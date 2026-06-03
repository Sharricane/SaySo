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

#[derive(Debug, Clone, Copy)]
enum TriggerMode {
    /// 按住录音，松开停止（默认）
    Hold,
    /// 点一下开始，再点一下停止
    Toggle,
}

impl TriggerMode {
    fn parse(s: &str) -> Result<Self> {
        match s.trim().to_lowercase().as_str() {
            "hold" | "push" | "ptt" => Ok(Self::Hold),
            "toggle" | "tap" | "click" => Ok(Self::Toggle),
            other => anyhow::bail!("unknown trigger mode '{other}' (try: hold | toggle)"),
        }
    }
}

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

    let trigger_mode = TriggerMode::parse(
        &std::env::var("SAYSO_TRIGGER_MODE").unwrap_or_else(|_| "hold".into()),
    )?;

    let silence_threshold: i16 = std::env::var("SAYSO_SILENCE_THRESHOLD")
        .ok()
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(500);

    let engine = Arc::new(AudioEngine::start()?);
    let stt_cfg = stt::Config::from_env(api_key.clone());
    let llm_cfg = llm::Config::from_env(api_key);

    tracing::info!(
        "SaySo ready — {} {} | silence<{} | polish={}",
        match trigger_mode {
            TriggerMode::Hold => "hold",
            TriggerMode::Toggle => "tap",
        },
        hotkey_name,
        silence_threshold,
        if llm_cfg.enabled { llm_cfg.model.as_str() } else { "off" }
    );

    let mut hotkey_rx = hotkey::spawn_listener(target_key);

    loop {
        if let Err(e) = handle_one_session(
            &mut hotkey_rx,
            &engine,
            &stt_cfg,
            &llm_cfg,
            trigger_mode,
            silence_threshold,
        )
        .await
        {
            tracing::error!("session error: {e:#}");
        }
    }
}

async fn handle_one_session(
    rx: &mut tokio::sync::mpsc::UnboundedReceiver<HotkeyEvent>,
    engine: &AudioEngine,
    stt_cfg: &stt::Config,
    llm_cfg: &llm::Config,
    trigger_mode: TriggerMode,
    silence_threshold: i16,
) -> Result<()> {
    // 等"开始"信号
    hotkey::wait_for(rx, HotkeyEvent::Press).await?;
    engine.begin_session();
    let started = Instant::now();
    tracing::info!("● recording…");

    // 等"停止"信号——hold 看 Release，toggle 看下一次 Press
    let stop_event = match trigger_mode {
        TriggerMode::Hold => HotkeyEvent::Release,
        TriggerMode::Toggle => HotkeyEvent::Press,
    };
    hotkey::wait_for(rx, stop_event).await?;

    let recording = engine.end_session();
    let dur = started.elapsed();

    tracing::info!(
        "captured {:.2}s ({} samples @ {}Hz, peak={})",
        recording.duration_secs(),
        recording.samples.len(),
        recording.sample_rate,
        recording.peak_amplitude()
    );

    if recording.is_silent(silence_threshold) {
        tracing::warn!(
            "peak {} < threshold {} — skipping Whisper",
            recording.peak_amplitude(),
            silence_threshold
        );
        return Ok(());
    }
    if dur.as_secs_f32() < 0.3 {
        tracing::warn!("recording only {:.2}s — too short, skipping", dur.as_secs_f32());
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
