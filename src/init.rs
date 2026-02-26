use anyhow::Result;
use std::path::Path;
use tokio::fs;

const REQUIRED_DIRS: &[&str] = &[
    "movies",
    "output", 
    "tiktok_output",
    "movies_retired",
    "backgroundmusic",
    "scripts",
    "scripts/srt_files",
    "clips",
    "clips/audio",
    "resources",
];

pub async fn ensure_directories() -> Result<()> {
    for dir in REQUIRED_DIRS {
        if !Path::new(dir).exists() {
            fs::create_dir_all(dir).await?;
            eprintln!("[INFO] Created directory: {}", dir);
        }
    }
    Ok(())
}

pub async fn check_ffmpeg() -> bool {
    match tokio::process::Command::new("ffmpeg")
        .arg("-version")
        .output()
        .await
    {
        Ok(output) => output.status.success(),
        Err(_) => false,
    }
}
