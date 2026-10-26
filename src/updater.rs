//! Update check and install from this repository's GitHub releases.
//!
//! Installing downloads the release zip, renames files that are in use (the running
//! exe and DLLs) to `*.old`, writes the new files next to the executable and restarts.
//! Leftover `*.old` files are removed on the next start.

use std::fs;
use std::io::{Cursor, Read};
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Channel {
    #[default]
    Stable,
    Beta,
}

#[derive(Clone, Debug)]
pub struct Release {
    pub tag: String,
    pub prerelease: bool,
    pub zip_url: Option<String>,
    pub page: String,
}

impl Release {
    pub fn version(&self) -> &str {
        self.tag.trim_start_matches('v')
    }
}

fn repository() -> Option<(&'static str, &'static str)> {
    let url = env!("CARGO_PKG_REPOSITORY").trim_end_matches('/');
    let mut parts = url.rsplit('/');
    let repo = parts.next()?;
    let owner = parts.next()?;
    Some((owner, repo))
}

pub fn releases_page() -> String {
    format!("{}/releases", env!("CARGO_PKG_REPOSITORY"))
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder().timeout_global(Some(Duration::from_secs(60))).build().into()
}

pub fn check() -> Result<Vec<Release>> {
    let (owner, repo) = repository().context("no repository configured")?;
    let url = format!("https://api.github.com/repos/{owner}/{repo}/releases?per_page=30");
    let text = agent()
        .get(&url)
        .header("User-Agent", "vrc-openface-updater")
        .header("Accept", "application/vnd.github+json")
        .call()?
        .body_mut()
        .read_to_string()?;
    let list: Vec<Value> = serde_json::from_str(&text)?;
    Ok(list
        .iter()
        .filter(|r| !r.get("draft").and_then(Value::as_bool).unwrap_or(false))
        .filter_map(|r| {
            let zip_url = r.get("assets")?.as_array()?.iter().find_map(|a| {
                let name = a.get("name")?.as_str()?;
                name.ends_with(".zip").then(|| a.get("browser_download_url")?.as_str().map(str::to_string))?
            });
            Some(Release {
                tag: r.get("tag_name")?.as_str()?.to_string(),
                prerelease: r.get("prerelease").and_then(Value::as_bool).unwrap_or(false),
                zip_url,
                page: r.get("html_url")?.as_str()?.to_string(),
            })
        })
        .collect())
}

/// Newest release on a channel; beta also accepts stable releases.
pub fn latest(releases: &[Release], channel: Channel) -> Option<&Release> {
    releases.iter().find(|r| channel == Channel::Beta || !r.prerelease)
}

/// Compares dotted versions numerically, e.g. "0.10.0" > "0.9.3".
pub fn is_newer(candidate: &str, current: &str) -> bool {
    let parse = |v: &str| -> Vec<u64> { v.split(['.', '-']).map(|p| p.parse().unwrap_or(0)).collect() };
    parse(candidate) > parse(current)
}

fn install_dir() -> Result<PathBuf> {
    Ok(std::env::current_exe()?.parent().context("executable has no folder")?.to_path_buf())
}

pub fn install(release: &Release) -> Result<()> {
    let url = release.zip_url.as_deref().context("this release has no zip download")?;
    let mut bytes = Vec::new();
    agent()
        .get(url)
        .header("User-Agent", "vrc-openface-updater")
        .call()?
        .body_mut()
        .with_config()
        .limit(512 * 1024 * 1024)
        .reader()
        .read_to_end(&mut bytes)?;
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes))?;

    // Release zips may wrap everything in one top-level folder; strip it.
    let names: Vec<String> = (0..archive.len())
        .filter_map(|i| archive.by_index(i).ok().and_then(|e| e.name().ok().map(|n| n.to_string())))
        .collect();
    let prefix = names
        .first()
        .and_then(|n| n.split_once('/').map(|(p, _)| format!("{p}/")))
        .filter(|p| names.iter().all(|n| n.starts_with(p.as_str())))
        .unwrap_or_default();
    let exe_name = std::env::current_exe()?.file_name().map(|n| n.to_owned());
    let exe_name = exe_name.map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    if !names.iter().any(|n| Path::new(n).file_name().is_some_and(|f| f.to_string_lossy() == exe_name)) {
        bail!("the release zip does not contain {exe_name}");
    }

    let dir = install_dir()?;
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i)?;
        if entry.is_dir() {
            continue;
        }
        let Some(relative) = entry.enclosed_name() else { continue };
        let relative = relative.strip_prefix(&prefix).unwrap_or(&relative).to_path_buf();
        let target = dir.join(&relative);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }
        if target.exists() {
            let old = target.with_extension(format!(
                "{}old",
                target.extension().map(|e| format!("{}.", e.to_string_lossy())).unwrap_or_default()
            ));
            let _ = fs::remove_file(&old);
            fs::rename(&target, &old).with_context(|| format!("replacing {}", target.display()))?;
        }
        let mut data = Vec::new();
        entry.read_to_end(&mut data)?;
        fs::write(&target, data)?;
    }
    restart()
}

pub fn restart() -> Result<()> {
    std::process::Command::new(std::env::current_exe()?).spawn()?;
    std::process::exit(0);
}

/// Removes `*.old` files left by a previous update.
pub fn cleanup() {
    let Ok(dir) = install_dir() else { return };
    let mut stack = vec![dir];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = fs::read_dir(&dir) else { continue };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|e| e == "old") {
                let _ = fs::remove_file(path);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compares_versions_numerically() {
        assert!(is_newer("0.10.0", "0.9.3"));
        assert!(!is_newer("0.2.0", "0.2.0"));
        assert!(is_newer("1.0.0", "0.99.0"));
    }

    #[test]
    fn repository_comes_from_cargo_metadata() {
        assert_eq!(repository(), Some(("setsumi-vrc", "VRC-Openface")));
    }
}
