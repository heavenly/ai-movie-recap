use anyhow::Result;
use ai_movie_shorts::generator::run_generation;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();
    let code = run_generation().await?;
    std::process::exit(code);
}
