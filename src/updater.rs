//! Update check and install from this repository's GitHub releases.
//!
//! Installing downloads the release archive for this system (the zip on Windows, the
//! `-linux-x86_64.tar.gz` on Linux), renames files that are in use (the running exe and
//! DLLs) to `*.old`, writes the new files next to the executable and restarts. Leftover
//! `*.old` files are removed on the next start. If writing fails part way, the previous
//! files are put back.

use std::cmp::Ordering;
use std::ffi::OsString;
use std::fs;
use std::io::{Cursor, Read};
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::Value;

const USER_AGENT: &str = "vrc-openface-updater";

/// How this system's download ends. Versions up to 0.7.0 install the first `.zip` of a
/// release, so other systems' archives must not be zips.
fn asset_suffix() -> String {
    if cfg!(windows) { ".zip".into() } else { format!("-{}-{}.tar.gz", std::env::consts::OS, std::env::consts::ARCH) }
}

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
    /// The archive for this system, if the release has one.
    pub download_url: Option<String>,
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
    let suffix = asset_suffix();
    Ok(list
        .iter()
        .filter(|r| !r.get("draft").and_then(Value::as_bool).unwrap_or(false))
        .filter_map(|r| {
            let download_url = r.get("assets")?.as_array()?.iter().find_map(|a| {
                let name = a.get("name")?.as_str()?;
                name.ends_with(&suffix).then(|| a.get("browser_download_url")?.as_str().map(str::to_string))?
            });
            Some(Release {
                tag: r.get("tag_name")?.as_str()?.to_string(),
                prerelease: r.get("prerelease").and_then(Value::as_bool).unwrap_or(false),
                download_url,
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
    let url = release.download_url.as_deref().context("this release has no download for this system")?;
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
    let mut files = unpack(bytes)?;

    // Release archives may wrap everything in one top-level folder; strip it.
    let top = files.first().and_then(|f| f.path.iter().next()).map(PathBuf::from);
    if let Some(top) = top.filter(|top| files.iter().all(|f| f.path.starts_with(top) && f.path != *top)) {
        for file in &mut files {
            file.path = file.path.strip_prefix(&top).map(Path::to_path_buf).unwrap_or_default();
        }
    }
    // Taken before the files move: on Linux the running executable's path follows the rename.
    let exe = std::env::current_exe()?;
    let exe_name = exe.file_name().unwrap_or_default();
    if !files.iter().any(|f| f.path.file_name() == Some(exe_name)) {
        bail!("the release archive does not contain {}", exe_name.display());
    }

    let mut changed = Vec::new();
    if let Err(e) = extract(&files, &install_dir()?, &mut changed) {
        rollback(changed);
        return Err(e);
    }
    relaunch(&exe)
}

/// One file of a release archive, relative to the install folder.
struct Packed {
    path: PathBuf,
    data: Vec<u8>,
    /// Permission bits, so the executable stays executable.
    #[cfg(unix)]
    mode: u32,
}

#[cfg(windows)]
fn unpack(bytes: Vec<u8>) -> Result<Vec<Packed>> {
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes))?;
    let mut files = Vec::new();
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i)?;
        let Some(path) = entry.enclosed_name().filter(|_| !entry.is_dir()) else { continue };
        let mut data = Vec::new();
        entry.read_to_end(&mut data).with_context(|| format!("reading {}", path.display()))?;
        files.push(Packed { path, data });
    }
    Ok(files)
}

#[cfg(unix)]
fn unpack(bytes: Vec<u8>) -> Result<Vec<Packed>> {
    use std::path::Component;

    let mut archive = tar::Archive::new(flate2::read::GzDecoder::new(Cursor::new(bytes)));
    let mut files = Vec::new();
    for entry in archive.entries()? {
        let mut entry = entry?;
        if !entry.header().entry_type().is_file() {
            continue;
        }
        let path = entry.path()?.into_owned();
        // Nothing may land outside the install folder.
        if !path.components().all(|c| matches!(c, Component::Normal(_) | Component::CurDir)) {
            continue;
        }
        let mode = entry.header().mode()?;
        let mut data = Vec::new();
        entry.read_to_end(&mut data).with_context(|| format!("reading {}", path.display()))?;
        files.push(Packed { path, data, mode });
    }
    Ok(files)
}

/// Where a file in use is moved to while its replacement is written.
fn old_path(path: &Path) -> PathBuf {
    let mut old = OsString::from(path.as_os_str());
    old.push(".old");
    old.into()
}

/// Writes `files` under `dir`, moving the files in the way to `*.old`. `changed` gets each
/// written file and where its previous version went, for [`rollback`].
fn extract(files: &[Packed], dir: &Path, changed: &mut Vec<(PathBuf, Option<PathBuf>)>) -> Result<()> {
    for file in files {
        let target = dir.join(&file.path);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }
        let old = target.exists().then(|| old_path(&target));
        if let Some(old) = &old {
            let _ = fs::remove_file(old);
            fs::rename(&target, old).with_context(|| format!("replacing {}", target.display()))?;
        }
        changed.push((target.clone(), old));
        fs::write(&target, &file.data).with_context(|| format!("writing {}", target.display()))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&target, fs::Permissions::from_mode(file.mode & 0o777))
                .with_context(|| format!("writing {}", target.display()))?;
        }
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
    relaunch(&std::env::current_exe()?)
}

fn relaunch(exe: &Path) -> Result<()> {
    std::process::Command::new(exe).spawn()?;
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
    fn only_windows_downloads_a_zip() {
        // Versions up to 0.7.0 install the first zip they find, whatever system it is for.
        assert_eq!(asset_suffix().ends_with(".zip"), cfg!(windows));
    }

    #[cfg(unix)]
    #[test]
    fn tarballs_keep_the_executable_bit() {
        let mut builder = tar::Builder::new(flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast()));
        let mut header = tar::Header::new_gnu();
        header.set_size(2);
        header.set_mode(0o755);
        builder.append_data(&mut header, "VRC-Openface-1.0.0/vrc-openface", &b"#!"[..]).unwrap();
        let files = unpack(builder.into_inner().unwrap().finish().unwrap()).unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].path, Path::new("VRC-Openface-1.0.0/vrc-openface"));
        assert_eq!(files[0].mode & 0o777, 0o755);
        assert_eq!(files[0].data, b"#!");
    }

    #[test]
    fn repository_comes_from_cargo_metadata() {
        assert_eq!(repository(), Some(("setsumi-vrc", "VRC-Openface")));
    }
}
