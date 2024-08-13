use anyhow::Result;
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
    url: String,
    /// The list of URLS generated from `url` and `vitepress_links`.
    #[serde(skip)]
    pub urls: IndexSet<String>,
    /// List of paths to JSON files that define the `VitePress` site.
    vitepress_links: Vec<PathBuf>,
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
