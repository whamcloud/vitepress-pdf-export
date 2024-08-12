use crate::Config;
use anyhow::{anyhow, Result};
use headless_chrome::{FetcherOptions, LaunchOptions, Revision};
use indexmap::IndexMap;
use indicatif::style::ProgressStyle;
use indicatif::ProgressBar;
use serde::Deserialize;
use std::ffi::OsStr;
use std::fs::{self, create_dir_all};
use std::path::PathBuf;
use std::time::Duration;

#[cfg(target_os = "linux")]
const PLATFORM: &str = "linux";
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
const PLATFORM: &str = "mac_arm";
#[cfg(all(target_os = "macos", not(target_arch = "aarch64")))]
const PLATFORM: &str = "mac";

#[cfg(target_os = "linux")]
const PLATFORM_BIN: &str = "chrome-linux/chrome";
#[cfg(target_os = "macos")]
const PLATFORM_BIN: &str = "chrome-mac/Chromium.app/Contents/MacOS/Chromium";

#[derive(Deserialize)]
struct KnownGoodVersions {
    _timestamp: String,
    versions: Vec<Version>,
}

#[derive(Deserialize)]
struct Version {
    _version: String,
    revision: String,
}

pub async fn get_latest_revision() -> Result<String> {
    let resp = reqwest::get("https://googlechromelabs.github.io/chrome-for-testing/known-good-versions-with-downloads.json").await?;
    let kgv = resp.json::<KnownGoodVersions>().await?;
    Ok(kgv
        .versions
        .last()
        .ok_or(anyhow!("Unable to get latest Version"))?
        .revision
        .to_string())
}

pub async fn get_chrome(config: &Config) -> Result<headless_chrome::Browser> {
    let revision = match &config.chrome_version {
        Some(r) => r.to_string(),
        None => get_latest_revision().await?.to_string(),
    };

    if !config.chrome_cache.exists() {
        create_dir_all(&config.chrome_cache)?;
    }

    let chrome_path = config.chrome_cache.join(format!("{PLATFORM}-{revision}"));

    if chrome_path.exists() {
        println!("Using cached Chrome revision {}", &revision);
        headless_chrome::Browser::new(
            LaunchOptions::default_builder()
                .path(Some(chrome_path.join(PLATFORM_BIN).canonicalize()?))
                .args(vec![OsStr::new("--generate-pdf-document-outline")])
                .headless(true)
                .devtools(false)
                .build()
                .unwrap(),
        )
    } else {
        let pb = ProgressBar::new_spinner();
        pb.enable_steady_tick(Duration::from_millis(50));
        pb.set_style(ProgressStyle::with_template(&format!(
            "{{spinner:.green}} Downloading Chrome revision {}.",
            &revision
        ))?);
        let chrome = headless_chrome::Browser::new(
            LaunchOptions::default_builder()
                .fetcher_options(
                    FetcherOptions::default()
                        .with_revision(Revision::Specific(revision))
                        .with_install_dir(Some(config.chrome_cache.canonicalize()?)),
                )
                .args(vec![OsStr::new("--generate-pdf-document-outline")])
                .headless(true)
                .devtools(false)
                .build()?,
        );
        pb.finish_with_message("Finished Downloading Chrome");
        chrome
    }
}

pub async fn render_urls(config: &Config) -> Result<IndexMap<String, PathBuf>> {
    let chrome = get_chrome(config).await?;

    let pb = ProgressBar::new(config.urls.len() as u64);
    pb.enable_steady_tick(Duration::from_millis(50));

    let mut map: IndexMap<String, PathBuf> = IndexMap::new();
    for (i, url) in config.urls.iter().enumerate() {
        pb.set_style(ProgressStyle::with_template(&format!(
            "{{spinner}} {{bar:.cyan}} {{pos}}/{{len}} rendering {url}"
        ))?);
        let tab = chrome.new_tab()?;
        let page_pdf = tab
            .navigate_to(url)?
            .wait_until_navigated()?
            .print_to_pdf(Some(config.print_to_pdf.clone()))?;
        let path = PathBuf::from(format!("{i}.pdf"));
        fs::write(&path, page_pdf)?;
        map.insert(url.clone(), path);
        pb.inc(1);
    }
    pb.finish_with_message("Finished Rendering URLS");
    Ok(map)
}
