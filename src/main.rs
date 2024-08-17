// Copyright (c) 2024 DDN. All rights reserved.
// Use of this source code is governed by a MIT-style
// license that can be found in the LICENSE file.

use anyhow::Result;
use clap::Parser;
use std::{path::PathBuf, process::ExitCode};
use tempfile::tempdir;

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
async fn main() -> Result<ExitCode> {
    let args = Args::parse();

    let config = Config::load(&args.config)?;

    // We create the pdf_temp_dir here so it will fall out of scope and be deleted when the process exits.
    let pdf_temp_dir = tempdir()?;

    let url_to_pdf = render_urls(&config, pdf_temp_dir.path()).await?;

    merge_pdfs(&config, url_to_pdf)
}
