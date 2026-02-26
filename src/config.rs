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
        // Create default config if it doesn't exist
        Self::create_default_if_missing(&path).await?;
        
        let content = fs::read_to_string(&path)
            .await
            .with_context(|| format!("Failed to read config: {}", path.as_ref().display()))?;
        let config: Config = serde_json::from_str(&content)?;
        
        if config.openai_key.is_empty() {
            anyhow::bail!("config.json: open_api_key is empty. Please add your OpenAI API key.");
        }
        if config.elevenlabs_key.is_empty() {
            anyhow::bail!("config.json: elevenlabs_api_key is empty. Please add your ElevenLabs API key.");
        }
        
        Ok(config)
    }
    
    async fn create_default_if_missing<P: AsRef<Path>>(path: P) -> Result<()> {
        if !path.as_ref().exists() {
            let default_config = Config {
                openai_key: String::new(),
                elevenlabs_key: String::new(),
                eleven_voice_id: default_voice_id(),
                eleven_model_id: default_model_id(),
            };
            
            let json = serde_json::to_string_pretty(&default_config)?;
            fs::write(&path, json).await?;
            
            eprintln!("[INFO] Created default config.json at: {}", path.as_ref().display());
            eprintln!("[INFO] Please edit config.json and add your API keys before running.");
        }
        Ok(())
    }
}
