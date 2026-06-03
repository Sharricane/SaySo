mod audio;
mod encode;
mod hotkey;
mod paste;
mod stt;

use anyhow::{Context, Result};
use hotkey::HotkeyEvent;
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

    tracing::info!(
        "SaySo ready — hold {hotkey_name:?} to talk, release to transcribe & paste. Ctrl-C to quit."
    );

    let stt_cfg = stt::Config::from_env(api_key);
    let mut hotkey_rx = hotkey::spawn_listener(target_key);

    loop {
        if let Err(e) = handle_one_session(&mut hotkey_rx, &stt_cfg).await {
            tracing::error!("session error: {e:#}");
        }
    }
}

async fn handle_one_session(
    rx: &mut tokio::sync::mpsc::UnboundedReceiver<HotkeyEvent>,
    stt_cfg: &stt::Config,
) -> Result<()> {
    // 等用户按下快捷键
    hotkey::wait_for(rx, HotkeyEvent::Press).await?;

    let started = Instant::now();
    let (stop_tx, audio_handle) = audio::start_recording()?;
    tracing::info!("● recording…");

    // 等用户松开
    hotkey::wait_for(rx, HotkeyEvent::Release).await?;
    let _ = stop_tx.send(());

    let recording = tokio::task::spawn_blocking(move || {
        audio_handle
            .join()
            .map_err(|_| anyhow::anyhow!("audio thread panicked"))?
    })
    .await
    .context("recording join task panicked")??;

    let dur = started.elapsed();
    tracing::info!(
        "captured {:.2}s ({} samples @ {}Hz, peak={})",
        dur.as_secs_f32(),
        recording.samples.len(),
        recording.sample_rate,
        recording.peak_amplitude()
    );

    if recording.is_silent() {
        tracing::warn!("recording looks silent — skipping Whisper to avoid hallucination");
        return Ok(());
    }
    if dur.as_secs_f32() < 0.3 {
        tracing::warn!("recording too short ({:.2}s) — skipping", dur.as_secs_f32());
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
