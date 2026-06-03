use anyhow::{Context, Result};
use reqwest::multipart;
use serde::Deserialize;
use std::time::Duration;

const DEFAULT_BASE_URL: &str = "https://api.groq.com/openai/v1";
const DEFAULT_MODEL: &str = "whisper-large-v3-turbo";

/// Whisper 接受最多 244 token 的 prompt 用来引导风格、补充术语。
/// 默认包含：一句典型中英混合技术对话示例（教标点位置与混搭风格）
/// + 高频技术词表（教 Whisper 这些词的拼写）。
const DEFAULT_PROMPT: &str = "以下是技术对话口述转写示例：我们今天修了登录模块的 token 逻辑，然后给 dashboard 加了个 loading 状态。涉及术语：模块、组件、用户、登录、注册、密码、token、API、接口、bug、merge、commit、PR、pull request、loading、error、warning、success、状态、React、Vue、TypeScript、Python、Rust、SDK、Redis、SQL、数据库、缓存、配置、settings、dashboard、frontend、backend、调试、部署、build、deploy。";

pub struct Config {
    pub base_url: String,
    pub model: String,
    pub api_key: String,
    pub prompt: Option<String>,
}

impl Config {
    pub fn from_env(api_key: String) -> Self {
        let prompt = match std::env::var("SAYSO_STT_PROMPT") {
            Ok(v) if v.trim().is_empty() => None,  // 空字符串 = 显式关闭
            Ok(v) => Some(v),
            Err(_) => Some(DEFAULT_PROMPT.into()),
        };
        Self {
            base_url: std::env::var("SAYSO_STT_BASE_URL")
                .unwrap_or_else(|_| DEFAULT_BASE_URL.into()),
            model: std::env::var("SAYSO_STT_MODEL").unwrap_or_else(|_| DEFAULT_MODEL.into()),
            api_key,
            prompt,
        }
    }
}

#[derive(Deserialize)]
struct WhisperResponse {
    text: String,
}

pub async fn transcribe(wav: Vec<u8>, cfg: &Config) -> Result<String> {
    let mut builder = reqwest::Client::builder().timeout(Duration::from_secs(30));

    if let Ok(proxy_url) = std::env::var("HTTPS_PROXY")
        .or_else(|_| std::env::var("https_proxy"))
        .or_else(|_| std::env::var("HTTP_PROXY"))
        .or_else(|_| std::env::var("http_proxy"))
    {
        tracing::info!("using HTTP proxy: {proxy_url}");
        let proxy = reqwest::Proxy::all(&proxy_url).context("invalid proxy URL")?;
        builder = builder.proxy(proxy);
    }

    let client = builder.build().context("reqwest client build")?;

    let file_part = multipart::Part::bytes(wav)
        .file_name("audio.wav")
        .mime_str("audio/wav")?;

    let mut form = multipart::Form::new()
        .part("file", file_part)
        .text("model", cfg.model.clone())
        .text("response_format", "json");

    if let Some(prompt) = &cfg.prompt {
        form = form.text("prompt", prompt.clone());
    }

    let url = format!("{}/audio/transcriptions", cfg.base_url.trim_end_matches('/'));
    let resp = client
        .post(&url)
        .bearer_auth(&cfg.api_key)
        .multipart(form)
        .send()
        .await
        .context("STT request failed")?;

    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        anyhow::bail!("STT {url} returned {status}: {body}");
    }

    let parsed: WhisperResponse = resp.json().await.context("parse STT response")?;
    Ok(parsed.text.trim().to_string())
}
