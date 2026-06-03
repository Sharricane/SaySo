mod audio;
mod encode;
mod hotkey;
mod llm;
mod paste;
mod stt;

use anyhow::{Context, Result};
use audio::AudioEngine;
use hotkey::HotkeyEvent;
use std::sync::Arc;
use std::time::Instant;

#[tokio::main]
async fn main() -> Result<()> {
    let _ = dotenvy::dotenv();
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info".into()),
        )
        .init();

    let api_key = std::env::var("GROQ_API_KEY")
        .context("GROQ_API_KEY not set — put it in .env at the project root")?;
    if !(api_key.len() == 56 && api_key.starts_with("gsk_")) {
        anyhow::bail!("GROQ_API_KEY format looks wrong (len={})", api_key.len());
    }

    let hotkey_name = std::env::var("SAYSO_HOTKEY").unwrap_or_else(|_| "RightAlt".into());
    let target_key = hotkey::parse_hotkey(&hotkey_name)?;

    let engine = Arc::new(AudioEngine::start()?);

    let stt_cfg = stt::Config::from_env(api_key.clone());
    let llm_cfg = llm::Config::from_env(api_key);

    tracing::info!(
        "SaySo ready — hold {} to talk, release to transcribe & paste. Ctrl-C to quit.",
        hotkey_name
    );
    if llm_cfg.enabled {
        tracing::info!("polish enabled: {}", llm_cfg.model);
    }

    let mut hotkey_rx = hotkey::spawn_listener(target_key);

    loop {
        if let Err(e) = handle_one_session(&mut hotkey_rx, &engine, &stt_cfg, &llm_cfg).await {
            tracing::error!("session error: {e:#}");
        }
    }
}

async fn handle_one_session(
    rx: &mut tokio::sync::mpsc::UnboundedReceiver<HotkeyEvent>,
    engine: &AudioEngine,
    stt_cfg: &stt::Config,
    llm_cfg: &llm::Config,
) -> Result<()> {
    hotkey::wait_for(rx, HotkeyEvent::Press).await?;
    engine.begin_session();
    let started = Instant::now();
    tracing::info!("● recording…");

    hotkey::wait_for(rx, HotkeyEvent::Release).await?;
    let recording = engine.end_session();
    let dur = started.elapsed();

    tracing::info!(
        "captured {:.2}s ({} samples @ {}Hz, peak={})",
        recording.duration_secs(),
        recording.samples.len(),
        recording.sample_rate,
        recording.peak_amplitude()
    );

    if recording.is_silent() {
        tracing::warn!("recording looks silent — skipping Whisper to avoid hallucination");
        return Ok(());
    }
    if dur.as_secs_f32() < 0.3 {
        tracing::warn!("hotkey held only {:.2}s — too short, skipping", dur.as_secs_f32());
        return Ok(());
    }

    let wav = encode::pcm_to_wav(&recording.samples, recording.sample_rate)?;
    tracing::info!("uploading {}KB to Groq Whisper…", wav.len() / 1024);

    let raw = stt::transcribe(wav, stt_cfg).await?;
    let raw = raw.trim();
    if raw.is_empty() {
        tracing::warn!("Whisper returned empty text");
        return Ok(());
    }
    tracing::info!("raw:     {raw}");

    // LLM 润色失败不阻断流程——退化到 raw 文本继续粘贴
    let final_text = match llm::polish(raw, llm_cfg).await {
        Ok(polished) => {
            if polished != raw {
                tracing::info!("polished: {polished}");
            }
            polished
        }
        Err(e) => {
            tracing::warn!("LLM polish failed ({e:#}), falling back to raw");
            raw.to_string()
        }
    };

    paste::paste(&final_text).context("paste failed")?;
    Ok(())
}
