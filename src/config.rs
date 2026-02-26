use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;
use tokio::fs;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    #[serde(rename = "open_api_key")]
    pub openai_key: String,
    #[serde(rename = "elevenlabs_api_key")]
    pub elevenlabs_key: String,
    #[serde(rename = "eleven_voice_id")]
    #[serde(default = "default_voice_id")]
    pub eleven_voice_id: String,
    #[serde(rename = "eleven_model_id")]
    #[serde(default = "default_model_id")]
    pub eleven_model_id: String,
}

fn default_voice_id() -> String {
    "JBFqnCBsd6RMkjVDRZzb".to_string()
}

fn default_model_id() -> String {
    "eleven_multilingual_v2".to_string()
}

impl Config {
    pub async fn load<P: AsRef<Path>>(path: P) -> Result<Self> {
        let content = fs::read_to_string(&path)
            .await
            .with_context(|| format!("Failed to read config: {}", path.as_ref().display()))?;
        let config: Config = serde_json::from_str(&content)?;
        
        if config.openai_key.is_empty() {
            anyhow::bail!("config.json: open_api_key missing");
        }
        if config.elevenlabs_key.is_empty() {
            anyhow::bail!("config.json: elevenlabs_api_key missing");
        }
        
        Ok(config)
    }
}
