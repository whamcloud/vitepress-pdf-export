use anyhow::Result;
use headless_chrome::types::PrintToPdfOptions;
use indexmap::{indexset, set::IndexSet};
use serde::Deserialize;
use std::{fs, path::PathBuf};

/// Represents the whole file. Used because if `Config` was the top level struct Deserialization fails if you put variables after the `pdf_options` map.
#[derive(Debug, Deserialize)]
pub struct ConfigFile {
    pub config: Config,
}

#[derive(Debug, Deserialize)]
pub struct Config {
    #[serde(default = "default_cache_path")]
    pub chrome_cache: PathBuf,
    pub chrome_version: Option<String>,
    pub output_pdf: PathBuf,
    url: String,
    #[serde(skip)]
    pub urls: IndexSet<String>,
    vitepress_links: Vec<PathBuf>,
    pub print_to_pdf: PrintToPdfOptions,
}

fn default_cache_path() -> PathBuf {
    PathBuf::from("/tmp")
}

#[derive(Debug, Deserialize)]
struct VitePressLinks {
    link: String,
    #[serde(default)]
    items: Vec<VitePressLinks>,
}

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
