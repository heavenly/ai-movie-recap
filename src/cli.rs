use anyhow::Result;
use ai_movie_shorts::generator::run_generation;
use ai_movie_shorts::init;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();
    
    // Initialize directories first
    init::ensure_directories().await?;
    
    if !init::check_ffmpeg().await {
        eprintln!("[WARNING] FFmpeg not found in PATH. Please install FFmpeg.");
    }
    
    let code = run_generation().await?;
    std::process::exit(code);
}
