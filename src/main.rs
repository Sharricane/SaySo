mod audio;
mod encode;
mod hotkey;
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

    // 启动即预热 cpal 流，消除按下快捷键时 100-300ms 的录音盲区
    let engine = Arc::new(AudioEngine::start()?);

    tracing::info!(
        "SaySo ready — hold {hotkey_name:?} to talk, release to transcribe & paste. Ctrl-C to quit."
    );

    let stt_cfg = stt::Config::from_env(api_key);
    let mut hotkey_rx = hotkey::spawn_listener(target_key);

    loop {
        if let Err(e) = handle_one_session(&mut hotkey_rx, &engine, &stt_cfg).await {
            tracing::error!("session error: {e:#}");
        }
    }
}

async fn handle_one_session(
    rx: &mut tokio::sync::mpsc::UnboundedReceiver<HotkeyEvent>,
    engine: &AudioEngine,
    stt_cfg: &stt::Config,
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

    let text = stt::transcribe(wav, stt_cfg).await?;
    let text = text.trim();
    if text.is_empty() {
        tracing::warn!("Whisper returned empty text");
        return Ok(());
    }

    tracing::info!("✓ {text}");
    paste::paste(text).context("paste failed")?;
    Ok(())
}
