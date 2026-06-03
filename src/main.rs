mod audio;
mod encode;
mod stt;

use anyhow::{Context, Result};
use std::time::Duration;

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

    tracing::info!("SaySo Phase 1A — fixed 5s record → Whisper → print");
    tracing::info!("✱ recording 5 seconds, speak now ✱");

    let recording = tokio::task::spawn_blocking(|| audio::record(Duration::from_secs(5)))
        .await
        .context("recording task panicked")?
        .context("recording failed")?;

    tracing::info!(
        "captured {} samples @ {}Hz (~{:.1}s)",
        recording.samples.len(),
        recording.sample_rate,
        recording.samples.len() as f32 / recording.sample_rate as f32
    );

    let wav =
        encode::pcm_to_wav(&recording.samples, recording.sample_rate).context("WAV encode")?;
    tracing::info!("WAV blob: {} bytes", wav.len());

    tracing::info!("uploading to Groq Whisper...");
    let cfg = stt::Config::from_env(api_key);
    let text = stt::transcribe(wav, &cfg).await.context("transcribe")?;

    println!();
    println!("─── transcription ──────────────────────────────");
    println!("{}", text);
    println!("────────────────────────────────────────────────");

    Ok(())
}
