use anyhow::{Context, Result};
use reqwest::multipart;
use serde::Deserialize;
use std::time::Duration;

const DEFAULT_BASE_URL: &str = "https://api.groq.com/openai/v1";
const DEFAULT_MODEL: &str = "whisper-large-v3-turbo";

pub struct Config {
    pub base_url: String,
    pub model: String,
    pub api_key: String,
}

impl Config {
    pub fn from_env(api_key: String) -> Self {
        Self {
            base_url: std::env::var("SAYSO_STT_BASE_URL")
                .unwrap_or_else(|_| DEFAULT_BASE_URL.into()),
            model: std::env::var("SAYSO_STT_MODEL").unwrap_or_else(|_| DEFAULT_MODEL.into()),
            api_key,
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

    let form = multipart::Form::new()
        .part("file", file_part)
        .text("model", cfg.model.clone())
        .text("response_format", "json");

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
