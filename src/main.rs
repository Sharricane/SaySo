use anyhow::Result;

#[tokio::main]
async fn main() -> Result<()> {
    let _ = dotenvy::dotenv();

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info".into()),
        )
        .init();

    tracing::info!("SaySo Phase 0 — hello from the Windows side");

    match std::env::var("GROQ_API_KEY") {
        Ok(k) if k.len() == 56 && k.starts_with("gsk_") => {
            tracing::info!(
                "GROQ_API_KEY ok (prefix={}…suffix={})",
                &k[..6],
                &k[k.len() - 4..]
            );
        }
        Ok(k) => {
            tracing::warn!("GROQ_API_KEY present but looks malformed (len={})", k.len());
        }
        Err(_) => {
            tracing::warn!("GROQ_API_KEY not found in env or .env file");
        }
    }

    Ok(())
}
