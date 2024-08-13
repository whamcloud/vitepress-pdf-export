use anyhow::Result;
use clap::Parser;
use std::path::PathBuf;

mod config;
use config::Config;
mod merge;
use merge::merge_pdfs;
mod render;
use render::render_urls;

/// A program to convert a `VitePress` web site into a single PDF
#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Args {
    /// Configuration File
    #[arg(short = 'c', long)]
    config: PathBuf,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    let config = Config::load(&args.config)?;
    let url_to_pdf = render_urls(&config).await?;
    merge_pdfs(&config, url_to_pdf)?;

    Ok(())
}
