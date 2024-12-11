// Copyright (c) 2024 DDN. All rights reserved.
// Use of this source code is governed by a MIT-style
// license that can be found in the LICENSE file.

use anyhow::{anyhow, Result};
use clap::Parser;
use std::{fs, fs::File, io::Write, path::PathBuf, process::ExitCode};
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

    /// Overwrite the `output_pdf` defined in the config file
    #[arg(short = 'o', long)]
    output_pdf: Option<PathBuf>,

    /// Directory to save individual PDFs into.
    ///
    /// If this option is not defined individual PDFs will be removed.
    #[arg(short = 'k', long)]
    keep_pdfs: Option<PathBuf>,

    /// JSON HashMap of URL to individual PDFs. Used for development.
    ///
    /// Map is written when merge_only is false and read from when merge_only is true.
    #[arg(short = 'm', long)]
    map: Option<PathBuf>,

    /// Used to speed up merge development.
    ///
    /// This option skips PDF rendering and uses the saved PDFs and map.
    ///
    /// The idea is run `vitepress --keep_pdfs pdfs --map map.json` which
    /// will render out the pdfs then run `vitepress --merge-onlys --map map.json`
    #[arg(long, action)]
    merge_only: bool,
}

#[tokio::main]
async fn main() -> Result<ExitCode> {
    let args = Args::parse();

    if args.merge_only && args.map.is_none() {
        println!("--map must defined when --merge_only")
    }
    let mut config = Config::load(&args.config)?;

    if let Some(output_pdf) = args.output_pdf {
        config.output_pdf = output_pdf;
    }

    let temp_dir = tempdir()?;

    let path = match &args.keep_pdfs {
        None => {
            // We create the pdf_temp_dir here so it will fall out of scope and be deleted when the process exits.
            temp_dir.path()
        }
        Some(dir) => dir.as_path(),
    };

    let url_to_pdf: indexmap::IndexMap<String, PathBuf> = match args.merge_only {
        false => render_urls(&config, path).await?,
        true => serde_json::from_str::<indexmap::IndexMap<String, PathBuf>>(&fs::read_to_string(
            args.map
                .as_ref()
                .ok_or(anyhow!("Map must be defined when using merge_only"))?,
        )?)?,
    };

    if let Some(map) = args.map.as_ref() {
        if !args.merge_only {
            let mut output = File::create(map)?;
            write!(output, "{}", serde_json::to_string_pretty(&url_to_pdf)?)?;
        }
    }

    merge_pdfs(&config, url_to_pdf)
}

#[cfg(test)]
mod tests {
    use super::Args;
    use clap::CommandFactory as _;

    #[test]
    fn verify_cli() {
        Args::command().debug_assert()
    }
}
