use crate::config::Config;
use crate::{logw};
use anyhow::{Context, Result};
use reqwest::Client;
use std::path::Path;
use tokio::fs;

pub async fn elevenlabs_tts_to_mp3(
    client: &Client,
    cfg: &Config,
    text: &str,
    out_mp3_path: &Path,
) -> Result<bool> {
    let url = format!(
        "https://api.elevenlabs.io/v1/text-to-speech/{}?output_format=mp3_44100_128",
        cfg.eleven_voice_id
    );

    let body = serde_json::json!({
        "text": text,
        "model_id": cfg.eleven_model_id,
    });

    let resp = client
        .post(url)
        .header("Content-Type", "application/json")
        .header("xi-api-key", &cfg.elevenlabs_key)
        .json(&body)
        .timeout(std::time::Duration::from_secs(300))
        .send()
        .await
        .context("ElevenLabs request failed")?;

    if !resp.status().is_success() {
        logw(format!("ElevenLabs TTS failed HTTP {}", resp.status().as_u16()));
        return Ok(false);
    }

    let bytes = resp.bytes().await.context("ElevenLabs response read failed")?;
    if let Some(parent) = out_mp3_path.parent() {
        fs::create_dir_all(parent)
            .await
            .with_context(|| format!("Failed to create dir {}", parent.display()))?;
    }
    fs::write(out_mp3_path, &bytes).await?;

    Ok(fs::metadata(out_mp3_path).await.is_ok())
}
