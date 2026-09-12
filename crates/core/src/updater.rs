//! Red Strap self-updates from GitHub releases.
//!
//! Update checks compare the running version against the latest release tag
//! (`v<semver>`). Installing downloads the new executable, swaps it into
//! place via a `.old` rename (the running process keeps its own handle, so
//! the swap always succeeds), and relaunches. A failed swap restores the
//! previous binary automatically.

use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::error::{Error, Result};

/// Parsed GitHub release (only the fields we use).
#[derive(Debug, Clone, Deserialize)]
pub struct Release {
    #[serde(rename = "tag_name")]
    pub tag: String,
    #[serde(default)]
    pub prerelease: bool,
    #[serde(default)]
    pub draft: bool,
    #[serde(default)]
    pub assets: Vec<ReleaseAsset>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ReleaseAsset {
    pub name: String,
    #[serde(rename = "browser_download_url")]
    pub download_url: String,
}

impl Release {
    /// `v1.2.3` -> `1.2.3`.
    pub fn version(&self) -> &str {
        self.tag.strip_prefix('v').unwrap_or(&self.tag)
    }

    /// Find an asset by exact file name (case-insensitive).
    pub fn asset_named(&self, file_name: &str) -> Option<&ReleaseAsset> {
        self.assets
            .iter()
            .find(|a| a.name.eq_ignore_ascii_case(file_name))
    }

    /// Pick the Windows executable asset (prefers `-win64` builds).
    pub fn windows_exe(&self) -> Option<&ReleaseAsset> {
        self.assets
            .iter()
            .filter(|a| {
                let name = a.name.to_ascii_lowercase();
                name.ends_with(".exe")
            })
            .max_by_key(|a| {
                let name = a.name.to_ascii_lowercase();
                if name.contains("win64") || name.contains("x64") {
                    2
                } else if name.contains("win") {
                    1
                } else {
                    0
                }
            })
    }
}

/// Fetch the latest release metadata for `owner/repo`.
pub async fn fetch_latest_release(
    client: &reqwest::Client,
    owner: &str,
    repo: &str,
) -> Result<Release> {
    let url = format!("https://api.github.com/repos/{owner}/{repo}/releases/latest");
    let release: Release = crate::http::get_json(client, &url).await?;
    if release.version().is_empty() {
        return Err(Error::Other(String::from(
            "latest release has no version tag",
        )));
    }
    Ok(release)
}

/// True when `latest` is newer than `current` (numeric comparison).
pub fn is_newer(current: &str, latest: &str) -> bool {
    crate::version::compare(current, latest) == std::cmp::Ordering::Less
}

/// List releases newest-first (optionally including drafts/prereleases).
pub async fn fetch_releases(
    client: &reqwest::Client,
    owner: &str,
    repo: &str,
) -> Result<Vec<Release>> {
    let url = format!("https://api.github.com/repos/{owner}/{repo}/releases?per_page=20");
    let releases: Vec<Release> = crate::http::get_json(client, &url).await?;
    Ok(releases.into_iter().filter(|r| !r.draft).collect())
}

/// Download `release` and swap it over the running executable, without
/// relaunching. Used by the silent background updater; the new binary takes
/// effect on the next start.
pub async fn download_and_swap(
    client: &reqwest::Client,
    release: &Release,
    on_progress: impl FnMut(u64),
) -> Result<PathBuf> {
    let asset = release.windows_exe().ok_or_else(|| {
        Error::Update(String::from(
            "this release has no Windows executable attached",
        ))
    })?;

    let current_exe = std::env::current_exe().map_err(|e| {
        Error::Update(format!("could not locate the running executable: {e}"))
    })?;
    let dir = current_exe
        .parent()
        .ok_or_else(|| Error::Update(String::from("executable has no parent directory")))?;
    let new_file = dir.join(format!(
        "{}-{}",
        current_exe
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("RedStrap.exe"),
        "new"
    ));

    swap_with_download(client, &asset.download_url, &current_exe, on_progress).await?;
    Ok(current_exe)
}

/// Download `url` and swap it over `target` (`.old` backup, auto-restore).
/// Returns `true` when an asset named `file_name` exists in `release` and
/// `target` was updated; `false` when there is nothing to do (missing asset
/// or `target` was never installed — never creates stray files).
pub async fn update_companion(
    client: &reqwest::Client,
    release: &Release,
    target: &Path,
    file_name: &str,
    on_progress: impl FnMut(u64),
) -> Result<bool> {
    let Some(asset) = release.asset_named(file_name) else {
        return Ok(false);
    };
    if !target.is_file() {
        return Ok(false);
    }
    swap_with_download(client, &asset.download_url, target, on_progress).await?;
    Ok(true)
}

/// Download `url` to a `-new` sidecar, then atomically swap it over `target`.
pub async fn swap_with_download(
    client: &reqwest::Client,
    url: &str,
    target: &Path,
    on_progress: impl FnMut(u64),
) -> Result<()> {
    let dir = target
        .parent()
        .ok_or_else(|| Error::Update(String::from("executable has no parent directory")))?;
    let new_file = dir.join(format!(
        "{}-{}",
        target
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("RedStrap.exe"),
        "new"
    ));
    crate::http::download_to_file(client, url, &new_file, on_progress).await?;
    swap_executable(target, &new_file)
}

/// Atomically replace `current` with `downloaded` via a `.old` backup.
/// Restores the backup when the final rename fails.
fn swap_executable(current: &Path, downloaded: &Path) -> Result<()> {
    let backup = backup_path(current);
    if backup.exists() {
        std::fs::remove_file(&backup).map_err(|e| Error::with_path(&backup, e))?;
    }
    std::fs::rename(current, &backup).map_err(|e| Error::with_path(&current.to_path_buf(), e))?;
    match std::fs::rename(downloaded, current) {
        Ok(()) => {
            // Best effort: the new process removes leftovers on startup too.
            let _ = std::fs::remove_file(&backup);
            Ok(())
        }
        Err(e) => {
            let _ = std::fs::rename(&backup, current);
            Err(Error::with_path(&current.to_path_buf(), e))
        }
    }
}

fn backup_path(current: &Path) -> PathBuf {
    let mut name = current
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("RedStrap.exe")
        .to_string();
    name.push_str(".old");
    current.with_file_name(name)
}

/// Remove stale update leftovers (`.old` backups, `*-new` downloads).
/// Called at startup; failures are ignored.
pub fn cleanup_leftovers(exe_dir: &Path) {
    let entries = match std::fs::read_dir(exe_dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = match path.file_name().and_then(|n| n.to_str()) {
            Some(n) => n,
            None => continue,
        };
        if name.ends_with(".old") || name.ends_with("-new") {
            let _ = std::fs::remove_file(&path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn release_parses_and_picks_exe() {
        let json = r#"{
            "tag_name": "v2.1.0",
            "assets": [
                {"name": "RedStrap-linux", "browser_download_url": "https://x/linux"},
                {"name": "RedStrap-win64.exe", "browser_download_url": "https://x/win.exe"}
            ]
        }"#;
        let release: Release = serde_json::from_str(json).expect("parse");
        assert_eq!(release.version(), "2.1.0");
        assert_eq!(
            release.windows_exe().map(|a| a.download_url.as_str()),
            Some("https://x/win.exe")
        );
    }

    #[test]
    fn version_newness() {
        assert!(is_newer("1.0.0", "1.0.1"));
        assert!(is_newer("1.9.0", "1.10.0"));
        assert!(!is_newer("2.0.0", "2.0.0"));
        assert!(!is_newer("2.0.1", "2.0.0"));
    }

    #[test]
    fn swap_roundtrip() {
        let dir = std::env::temp_dir().join(format!("redstrap-swap-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("mkdir");
        let current = dir.join("app.exe");
        let downloaded = dir.join("app.exe-new");
        std::fs::write(&current, b"old").expect("write");
        std::fs::write(&downloaded, b"new").expect("write");

        swap_executable(&current, &downloaded).expect("swap");
        assert_eq!(std::fs::read(&current).expect("read"), b"new");
        assert!(!backup_path(&current).exists());

        let _ = std::fs::remove_dir_all(&dir);
    }
}
