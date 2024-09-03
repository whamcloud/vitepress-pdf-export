// Copyright (c) 2024 DDN. All rights reserved.
// Use of this source code is governed by a MIT-style
// license that can be found in the LICENSE file.
use anyhow::{anyhow, Result};
use headless_chrome::types::PrintToPdfOptions;
use indexmap::{indexset, set::IndexSet};
use serde::Deserialize;
use std::{fs, path::PathBuf};

// Represents the whole file. Used because if`Config` was the top level struct
// Deserialization fails if you put variables after the `pdf_options` map.
#[derive(Debug, Deserialize)]
struct ConfigFile {
    pub config: Config,
}

/// Page Number Color
#[derive(Debug, Deserialize)]
pub struct Color {
    pub r: f64,
    pub g: f64,
    pub b: f64,
}

impl Color {
    fn valid(&self) -> Result<()> {
        let mut invalid = vec![];
        if self.r < 0.0 || self.r > 1.0 {
            invalid.push("r");
        }
        if self.g < 0.0 || self.g > 1.0 {
            invalid.push("g");
        }
        if self.b < 0.0 || self.b > 1.0 {
            invalid.push("b");
        }
        if !invalid.is_empty() {
            return Err(anyhow!(
                "Channel values must for be in range 0.0 to 1.0. Invalid channel(s) {}",
                invalid.join(",")
            ));
        }
        Ok(())
    }
}

/// Page Numbers Style
#[derive(Debug, Deserialize)]
pub struct PageNumber {
    /// Font Color
    pub color: Color,
    /// Font Name
    pub font: String,
    /// Font size
    pub size: i16,
    /// Page Number X offset (in inches) from the top left corner
    pub x: f64,
    /// Page Number Y offset (in inches) from the top left corner
    pub y: f64,
}

impl PageNumber {
    fn valid(&self) -> Result<()> {
        self.color.valid()?;
        let type1_fonts = [
            "Times−Roman",
            "Times−Bold",
            "Times−Italic",
            "Times−BoldItalic",
            "Helvetica",
            "Helvetica−Bold",
            "Helvetica−Oblique",
            "Helvetica−BoldOblique",
            "Courier",
            "Courier−Bold",
            "Courier−Oblique",
            "Courier−BoldOblique",
        ];
        if !type1_fonts.contains(&self.font.as_str()) {
            return Err(anyhow!(
                "Invalid font name {}. Only PDF Type 1 Fonts are supported",
                self.font
            ));
        }
        Ok(())
    }
}

/// We expect `vitepress-pdf-export` to be run as part of a CI actions so all options
/// are handled by a TOML configuration file.
#[derive(Debug, Deserialize)]
pub struct Config {
    /// Where to download Chrome builds to
    #[serde(default = "default_cache_path")]
    pub chrome_cache: PathBuf,
    /// Pin Chrome to a specific revision, e.g. `1336641`. If unset we use that latest known good build.
    pub chrome_version: Option<String>,
    /// The merged PDF file  
    pub output_pdf: PathBuf,
    /// `VitePress` Dev URL e.g., `http://localhost:5173``.
    pub url: String,
    /// The list of URLS generated from `url` and `vitepress_links`.
    #[serde(skip)]
    pub urls: IndexSet<String>,
    /// List of paths to JSON files that define the `VitePress` site.
    pub vitepress_links: Vec<PathBuf>,
    /// Page Number Style - if not defined page numbers will not be inserted.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub page_number: Option<PageNumber>,
    /// PDF Generation options see [Chrome DevTool Protocol](https://chromedevtools.github.io/devtools-protocol/tot/Page/#method-printToPDF) for documentation.
    pub print_to_pdf: PrintToPdfOptions,
}

fn default_cache_path() -> PathBuf {
    PathBuf::from("/tmp")
}

// VitePress defines the struct of the site in JSON files
#[derive(Debug, Deserialize)]
struct VitePressLinks {
    link: String,
    #[serde(default)]
    items: Vec<VitePressLinks>,
}

// Converts relative URLs into absoute URLs.
fn build_links(vp: &VitePressLinks, url: &String, links: &mut IndexSet<String>) {
    let mut link = url.clone();
    link.push_str(&vp.link);

    if link.ends_with('/') {
        link.push_str("index.html");
    } else if !link.ends_with(".html") {
        link.push_str(".html");
    }
    links.insert(link);

    for item in &vp.items {
        build_links(item, url, links)
    }
}

impl Config {
    /// Loads the TOML file and generates the list of URLS to render into PDFs
    pub fn load(path: &PathBuf) -> Result<Self> {
        let mut conf: Config = toml::from_str::<ConfigFile>(&fs::read_to_string(path)?)?.config;

        if let Some(page_number) = &conf.page_number {
            page_number.valid()?;
        }

        let mut index = conf.url.clone();
        index.push_str("/index.html");

        let mut links = indexset! {index};

        for path in &conf.vitepress_links {
            let vp: VitePressLinks =
                serde_json::from_str::<VitePressLinks>(&fs::read_to_string(path)?)?;
            build_links(&vp, &conf.url, &mut links);
        }

        conf.urls = links;
        Ok(conf)
    }
}
