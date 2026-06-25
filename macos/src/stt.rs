use anyhow::{Context, Result};
use reqwest::multipart;
use serde::Deserialize;
use std::time::Duration;

pub const DEFAULT_BASE_URL: &str = "https://api.groq.com/openai/v1";
pub const DEFAULT_MODEL: &str = "whisper-large-v3-turbo";
pub const DEFAULT_MIN_LOGPROB: f64 = -0.5;

/// Whisper 接受最多 244 token 的 prompt 用来引导风格、补充术语。
/// 默认包含：一句典型中英混合技术对话示例（教标点位置与混搭风格）
/// + 高频技术词表（教 Whisper 这些词的拼写）。
pub const DEFAULT_PROMPT: &str = "以下是技术对话口述转写示例：我们今天修了登录模块的 token 逻辑，然后给 dashboard 加了个 loading 状态。涉及术语：模块、组件、用户、登录、注册、密码、token、API、接口、bug、merge、commit、PR、pull request、loading、error、warning、success、状态、React、Vue、TypeScript、Python、Rust、SDK、Redis、SQL、数据库、缓存、配置、settings、dashboard、frontend、backend、调试、部署、build、deploy。";

pub struct Config {
    pub base_url: String,
    pub model: String,
    pub api_key: String,
    pub proxy: Option<String>,
    pub prompt: Option<String>,
    /// 段平均 avg_logprob 低于此值的结果当作噪声幻听丢弃。实测（见 git 历史）：
    /// 纯噪声/静音 ≤ -0.74，真人语音 ≥ -0.31，-0.5 落在中间。调到很低（如 -100）= 关闭。
    pub min_logprob: f64,
}

#[derive(Deserialize)]
struct WhisperResponse {
    text: String,
    // verbose_json 才有；段级 avg_logprob 用来识别噪声幻听
    #[serde(default)]
    segments: Vec<Segment>,
}

#[derive(Deserialize)]
struct Segment {
    avg_logprob: f64,
}

pub async fn transcribe(wav: Vec<u8>, peak: i16, cfg: &Config) -> Result<String> {
    let mut builder = reqwest::Client::builder().timeout(Duration::from_secs(30));

    // 配置里的 proxy 优先，否则退回 HTTPS_PROXY 等环境变量
    let proxy_url = cfg
        .proxy
        .clone()
        .filter(|p| !p.trim().is_empty())
        .or_else(|| std::env::var("HTTPS_PROXY").ok())
        .or_else(|| std::env::var("https_proxy").ok());
    if let Some(proxy_url) = proxy_url {
        tracing::info!("using HTTP proxy: {proxy_url}");
        builder = builder.proxy(reqwest::Proxy::all(&proxy_url).context("invalid proxy URL")?);
    }

    let client = builder.build().context("reqwest client build")?;

    let url = format!("{}/audio/transcriptions", cfg.base_url.trim_end_matches('/'));
    // Cloudflare 偶发 403／网络抖动 → 重试几次。multipart 发一次就被消费，每次重建（克隆 wav）。
    let parsed: WhisperResponse = {
        let mut last = String::new();
        let mut got = None;
        for attempt in 0..5u32 {
            if attempt > 0 {
                tokio::time::sleep(Duration::from_millis(250 * attempt as u64)).await;
            }
            let file_part = multipart::Part::bytes(wav.clone())
                .file_name("audio.wav")
                .mime_str("audio/wav")?;
            // verbose_json 拿段级 avg_logprob 做噪声幻听过滤；temperature=0 降低乱编
            let mut form = multipart::Form::new()
                .part("file", file_part)
                .text("model", cfg.model.clone())
                .text("response_format", "verbose_json")
                .text("temperature", "0");
            if let Some(prompt) = &cfg.prompt {
                form = form.text("prompt", prompt.clone());
            }
            let resp = match client.post(&url).bearer_auth(&cfg.api_key).multipart(form).send().await {
                Ok(r) => r,
                Err(e) => {
                    last = format!("request failed: {e}");
                    continue;
                }
            };
            let status = resp.status();
            if !status.is_success() {
                last = format!("{status}");
                continue;
            }
            got = Some(resp.json().await.context("parse STT response")?);
            break;
        }
        match got {
            Some(p) => p,
            None => anyhow::bail!("STT {url} 重试 5 次仍失败：{last}"),
        }
    };
    let text = parsed.text.trim().to_string();

    let mean_logprob = if parsed.segments.is_empty() {
        None
    } else {
        Some(parsed.segments.iter().map(|s| s.avg_logprob).sum::<f64>() / parsed.segments.len() as f64)
    };

    if looks_like_hallucination(&text, mean_logprob, cfg.min_logprob, peak) {
        tracing::warn!(
            "dropping likely noise/hallucination (avg_logprob={:.3} peak={peak}): {text:?}",
            mean_logprob.unwrap_or(f64::NAN)
        );
        return Ok(String::new());
    }
    Ok(text)
}

/// 录得够响（peak 高）就是真有人在说话——哪怕磕磕绊绊、中英混杂、转写置信度低，
/// 也别当噪声丢。avg_logprob 区分不了"乱说的真话"和"噪声幻听"（两者都低），所以
/// 只在**音量偏低、模棱两可**时才用它过滤；够响的一律收。空文本和已知幻听短语
/// （Thank you./感谢观看 这类静音幻听）任何音量都丢。
const LOUD_PEAK: i16 = 2000;
fn looks_like_hallucination(text: &str, mean_logprob: Option<f64>, min_logprob: f64, peak: i16) -> bool {
    let norm: String = text
        .trim()
        .trim_matches(|c: char| {
            c.is_whitespace() || c.is_ascii_punctuation() || "。、，！？…：；".contains(c)
        })
        .to_lowercase();
    if norm.is_empty() {
        return true;
    }
    // 够响 = 真说话了，不靠 avg_logprob 否决（否则磕巴/混语的真话被当噪声丢）。
    if peak < LOUD_PEAK && mean_logprob.is_some_and(|lp| lp < min_logprob) {
        return true;
    }
    const BLOCKLIST: &[&str] = &[
        "thank you",
        "thanks for watching",
        "thank you for watching",
        "please subscribe",
        "you",
        "bye",
        "感谢观看",
        "请不吝点赞订阅转发打赏支持明镜与点点栏目",
        "字幕由amara.org社区提供",
    ];
    BLOCKLIST.contains(&norm.as_str())
}
