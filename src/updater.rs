//! Update check and install from this repository's GitHub releases.
//!
//! Installing downloads the release zip, renames files that are in use (the running
//! exe and DLLs) to `*.old`, writes the new files next to the executable and restarts.
//! Leftover `*.old` files are removed on the next start. If writing fails part way, the
//! previous files are put back.

use std::cmp::Ordering;
use std::ffi::OsString;
use std::fs;
use std::io::{self, Cursor, Read};
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use zip::ZipArchive;

const USER_AGENT: &str = "vrc-openface-updater";

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
        .header("User-Agent", USER_AGENT)
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

/// Compares versions the way SemVer orders them: numerically ("0.10.0" > "0.9.3"), and a
/// pre-release below its release ("0.7.0-beta.1" < "0.7.0").
pub fn is_newer(candidate: &str, current: &str) -> bool {
    compare_versions(candidate, current) == Ordering::Greater
}

fn compare_versions(a: &str, b: &str) -> Ordering {
    let split = |v: &str| {
        let (core, pre) = v.split_once('-').map_or((v, None), |(core, pre)| (core, Some(pre.to_string())));
        let mut numbers = [0u64; 3];
        for (n, part) in numbers.iter_mut().zip(core.split('.')) {
            *n = part.parse().unwrap_or(0);
        }
        (numbers, pre)
    };
    let ((a, a_pre), (b, b_pre)) = (split(a), split(b));
    a.cmp(&b).then_with(|| match (a_pre, b_pre) {
        (None, None) => Ordering::Equal,
        (None, Some(_)) => Ordering::Greater,
        (Some(_), None) => Ordering::Less,
        // Dot-separated identifiers: numbers numerically and below words, words by text.
        (Some(a), Some(b)) => {
            let ids = |s: &str| s.split('.').map(|p| p.parse::<u64>().map_err(|_| p.to_string())).collect::<Vec<_>>();
            ids(&a).cmp(&ids(&b))
        }
    })
}

fn install_dir() -> Result<PathBuf> {
    Ok(std::env::current_exe()?.parent().context("executable has no folder")?.to_path_buf())
}

pub fn install(release: &Release) -> Result<()> {
    let url = release.zip_url.as_deref().context("this release has no zip download")?;
    let mut bytes = Vec::new();
    agent()
        .get(url)
        .header("User-Agent", USER_AGENT)
        .call()?
        .body_mut()
        .with_config()
        .limit(512 * 1024 * 1024)
        .reader()
        .read_to_end(&mut bytes)?;
    let mut archive = ZipArchive::new(Cursor::new(bytes))?;

    // Release zips may wrap everything in one top-level folder; strip it.
    let names: Vec<String> = archive.file_names().map(|n| n.map(|n| n.into_owned())).collect::<Result<_, _>>()?;
    let prefix = names
        .first()
        .and_then(|n| n.split_once('/').map(|(p, _)| format!("{p}/")))
        .filter(|p| names.iter().all(|n| n.starts_with(p.as_str())))
        .unwrap_or_default();
    let exe_name = std::env::current_exe()?.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    if !names.iter().any(|n| Path::new(n).file_name().is_some_and(|f| f.to_string_lossy() == exe_name)) {
        bail!("the release zip does not contain {exe_name}");
    }

    let mut changed = Vec::new();
    if let Err(e) = extract(&mut archive, &prefix, &install_dir()?, &mut changed) {
        rollback(changed);
        return Err(e);
    }
    restart()
}

/// Where a file in use is moved to while its replacement is written.
fn old_path(path: &Path) -> PathBuf {
    let mut old = OsString::from(path.as_os_str());
    old.push(".old");
    old.into()
}

/// Writes every file of the archive under `dir`, moving the files in the way to `*.old`.
/// `changed` gets each written file and where its previous version went, for [`rollback`].
fn extract(
    archive: &mut ZipArchive<Cursor<Vec<u8>>>,
    prefix: &str,
    dir: &Path,
    changed: &mut Vec<(PathBuf, Option<PathBuf>)>,
) -> Result<()> {
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i)?;
        if entry.is_dir() {
            continue;
        }
        let Some(relative) = entry.enclosed_name() else { continue };
        let target = dir.join(relative.strip_prefix(prefix).unwrap_or(&relative));
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }
        let old = target.exists().then(|| old_path(&target));
        if let Some(old) = &old {
            let _ = fs::remove_file(old);
            fs::rename(&target, old).with_context(|| format!("replacing {}", target.display()))?;
        }
        changed.push((target.clone(), old));
        let mut file = fs::File::create(&target).with_context(|| format!("writing {}", target.display()))?;
        io::copy(&mut entry, &mut file).with_context(|| format!("writing {}", target.display()))?;
    }
    Ok(())
}

/// Removes what a failed [`extract`] wrote and puts the previous files back.
fn rollback(changed: Vec<(PathBuf, Option<PathBuf>)>) {
    for (target, old) in changed.into_iter().rev() {
        let _ = fs::remove_file(&target);
        if let Some(old) = old
            && let Err(e) = fs::rename(&old, &target)
        {
            log::warn!("restoring {}: {e}", target.display());
        }
    }
}

pub fn restart() -> Result<()> {
    std::process::Command::new(std::env::current_exe()?).spawn()?;
    std::process::exit(0);
}

/// Removes the `*.old` files a previous update left. Only the folders a release has are
/// looked at, and only files whose replacement is there: the executable may sit in a
/// folder with other files of the user's.
pub fn cleanup() {
    let Ok(dir) = install_dir() else { return };
    for folder in [dir.join("models"), dir] {
        let Ok(entries) = fs::read_dir(&folder) else { continue };
        for path in entries.flatten().map(|e| e.path()) {
            let replaced = path.to_str().and_then(|p| p.strip_suffix(".old")).map(Path::new);
            if path.is_file() && replaced.is_some_and(Path::is_file) {
                let _ = fs::remove_file(&path);
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
    fn pre_releases_come_before_their_release() {
        assert!(is_newer("0.7.0", "0.7.0-beta.1"));
        assert!(!is_newer("0.7.0-beta.1", "0.7.0"));
        assert!(is_newer("0.7.0-beta.2", "0.7.0-beta.1"));
        assert!(is_newer("0.7.0-beta.10", "0.7.0-beta.9"));
        assert!(is_newer("0.7.0-rc.1", "0.7.0-beta.3"));
        assert!(is_newer("0.7.0-beta.1", "0.6.9"));
    }

    #[test]
    fn old_files_keep_the_whole_name() {
        assert_eq!(old_path(Path::new("a/vrc-openface.exe")), Path::new("a/vrc-openface.exe.old"));
        assert_eq!(old_path(Path::new("LICENSE")), Path::new("LICENSE.old"));
    }

    #[test]
    fn repository_comes_from_cargo_metadata() {
        assert_eq!(repository(), Some(("setsumi-vrc", "VRC-Openface")));
    }
}
