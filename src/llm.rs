use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::time::Duration;

const DEFAULT_BASE_URL: &str = "https://api.groq.com/openai/v1";
const DEFAULT_MODEL: &str = "llama-3.3-70b-versatile";

/// LLM 润色的 system prompt。要点：
/// - 与输入同语言（不要翻译）
/// - 保留英文技术术语原貌（loading 不要变 "加载中"）
/// - 加合适标点，去明显口头禅
/// - 基于上下文修明显的同音/近音错字（"我们快" 在技术句里 → "模块"）
/// - 直接输出文本，不要前缀、不要 "以下是…"
const DEFAULT_SYSTEM_PROMPT: &str = r#"你是一个口述转写清理助手。输入是麦克风转写出的原始文本，可能包含：缺失的标点、口头禅、被吞掉的助词（了/的/个）、以及音近字识别错误。

任务：
- 在输入的原语言风格下加合适的标点（中文用，。？！；英文用 , . ? !）
- 删掉明显的口头禅（嗯、啊、那个、就是、um、uh、you know、like）
- 基于上下文修复明显的音近错字。例如"我们今天先把登录'我们快'的 token 逻辑修好"——"我们快"在"登录___的 token"语境下显然指"模块"
- 英文技术术语保持英文（loading、token、API、React、TypeScript、dashboard 等不要翻译）
- 输出语言与输入语言一致，不要做语言翻译
- 不要扩写、不要补充信息、不要改变说话人的语气

输出要求：
- 只输出清理后的文本本身，不要任何前缀（"以下是…"、"修订后："、引号）
- 不要解释你做了什么
- 不确定时保留原词，不要凭空创造内容
- 输入若是空字符串或完全无法理解，就原样输出
"#;

pub struct Config {
    pub base_url: String,
    pub model: String,
    pub api_key: String,
    pub system_prompt: String,
    pub enabled: bool,
}

impl Config {
    pub fn from_env(api_key: String) -> Self {
        let enabled = std::env::var("SAYSO_LLM_ENABLED")
            .map(|s| !matches!(s.trim().to_lowercase().as_str(), "false" | "0" | "off" | "no"))
            .unwrap_or(true);
        Self {
            base_url: std::env::var("SAYSO_LLM_BASE_URL").unwrap_or_else(|_| DEFAULT_BASE_URL.into()),
            model: std::env::var("SAYSO_LLM_MODEL").unwrap_or_else(|_| DEFAULT_MODEL.into()),
            api_key,
            system_prompt: std::env::var("SAYSO_LLM_PROMPT")
                .unwrap_or_else(|_| DEFAULT_SYSTEM_PROMPT.into()),
            enabled,
        }
    }
}

#[derive(Serialize)]
struct ChatRequest<'a> {
    model: &'a str,
    messages: Vec<ChatMessage<'a>>,
    temperature: f32,
}

#[derive(Serialize)]
struct ChatMessage<'a> {
    role: &'a str,
    content: &'a str,
}

#[derive(Deserialize)]
struct ChatResponse {
    choices: Vec<ChatChoice>,
}

#[derive(Deserialize)]
struct ChatChoice {
    message: ResponseMessage,
}

#[derive(Deserialize)]
struct ResponseMessage {
    content: String,
}

pub async fn polish(text: &str, cfg: &Config) -> Result<String> {
    if !cfg.enabled || text.trim().is_empty() {
        return Ok(text.to_string());
    }

    let mut builder = reqwest::Client::builder().timeout(Duration::from_secs(20));
    if let Ok(proxy_url) = std::env::var("HTTPS_PROXY")
        .or_else(|_| std::env::var("https_proxy"))
        .or_else(|_| std::env::var("HTTP_PROXY"))
        .or_else(|_| std::env::var("http_proxy"))
    {
        builder = builder.proxy(reqwest::Proxy::all(&proxy_url).context("invalid proxy URL")?);
    }
    let client = builder.build().context("reqwest client build")?;

    let req = ChatRequest {
        model: &cfg.model,
        messages: vec![
            ChatMessage {
                role: "system",
                content: &cfg.system_prompt,
            },
            ChatMessage {
                role: "user",
                content: text,
            },
        ],
        // 0.2：略带随机性让标点选择更自然，但避免随意改词
        temperature: 0.2,
    };

    let url = format!("{}/chat/completions", cfg.base_url.trim_end_matches('/'));
    let resp = client
        .post(&url)
        .bearer_auth(&cfg.api_key)
        .json(&req)
        .send()
        .await
        .context("LLM request failed")?;

    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        anyhow::bail!("LLM {url} returned {status}: {body}");
    }

    let parsed: ChatResponse = resp.json().await.context("parse LLM response")?;
    let content = parsed
        .choices
        .into_iter()
        .next()
        .context("LLM returned no choices")?
        .message
        .content;

    Ok(content.trim().to_string())
}
