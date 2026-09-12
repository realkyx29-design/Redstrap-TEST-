//! Roblox install/update pipeline: mirror race, version check, parallel
//! download, parallel extraction, then launch customization (flags, mods,
//! settings file, GPU preferences).
//!
//! Downloads are content-verified (MD5) and resumable-by-skip: any package
//! whose recorded signature matches is never fetched again. Progress flows
//! to the caller through a single `FnMut` sink.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::consts::*;
use crate::deployment;
use crate::error::{Error, Result};
use crate::manifest::Package;
use crate::paths::Layout;
use crate::roblox::LaunchMode;
use crate::settings::{GpuPreference, Settings};
use crate::state::DistributionState;
use tokio::task::JoinSet;

/// Progress events delivered to the caller's sink.
#[derive(Debug, Clone)]
pub enum Progress {
    /// A new textual stage began.
    Stage(String),
    Downloading {
        done_bytes: u64,
        total_bytes: u64,
        files_done: usize,
        files_total: usize,
        bytes_per_sec: f64,
    },
    Extracting {
        files_done: usize,
        files_total: usize,
    },
    Done,
}

/// Outcome of [`ensure_installed`].
#[derive(Debug, Clone)]
pub struct InstallOutcome {
    pub version_guid: String,
    pub version_label: String,
    pub version_dir: PathBuf,
    pub exe: PathBuf,
    pub did_install: bool,
}

/// Report from [`apply_customization`].
#[derive(Debug, Clone, Default)]
pub struct CustomReport {
    pub flags_written: bool,
    pub mods_copied: usize,
    pub mod_failures: Vec<String>,
    pub gbs_updated: bool,
}

/// Ensure `mode` is installed and customized, downloading when needed.
///
/// - `version_pin` forces an exact `version-<hash>` instead of the channel's
///   current deploy.
/// - `force` reinstalls even when the recorded version matches.
/// - When [`Settings::auto_update_roblox`] is off, an existing install is
///   reused as-is and a missing one fails with [`Error::NotInstalledUpdatesBlocked`].
pub async fn ensure_installed(
    client: &reqwest::Client,
    layout: &Layout,
    settings: &Settings,
    mode: LaunchMode,
    version_pin: Option<&str>,
    force: bool,
    progress: &mut impl FnMut(Progress),
) -> Result<InstallOutcome> {
    progress(Progress::Stage(String::from("Connecting to Roblox...")));
    let base_url = deployment::initialize_connectivity(client).await?;

    let state_path = dist_state_path(layout, mode);
    let (mut dist, _) = DistributionState::load(&state_path);

    // Resolve the target version.
    let (version_guid, version_label) = match version_pin {
        Some(pin) if !pin.trim().is_empty() => (pin.trim().to_string(), pin.trim().to_string()),
        _ => {
            progress(Progress::Stage(String::from("Checking for updates...")));
            let info = deployment::get_info(
                client,
                &settings.roblox_domain,
                &settings.channel,
                mode.binary_type(),
                &settings.channel_token,
                true,
            )
            .await?;
            if info.behind_default && !settings.channel.eq_ignore_ascii_case(DEFAULT_CHANNEL) {
                tracing::warn!(
                    "channel '{}' is behind production ({})",
                    settings.channel,
                    info.version
                );
            }
            (info.version_guid.clone(), info.version.clone())
        }
    };

    let (version_dir, exe) =
        crate::roblox::version_paths(layout, mode, &version_guid, settings.static_directory);

    let up_to_date =
        !force && dist.version_guid == version_guid && dist.is_installed() && exe.is_file();

    if up_to_date {
        progress(Progress::Stage(String::from("Already up to date.")));
    } else if !settings.auto_update_roblox && version_pin.is_none() {
        // Updates blocked: reuse any usable install, else fail honestly.
        if dist.is_installed() {
            let (dir, exe_path) = crate::roblox::version_paths(
                layout,
                mode,
                &dist.version_guid,
                settings.static_directory,
            );
            if exe_path.is_file() {
                tracing::info!("automatic updates disabled; using installed {}", dist.version_guid);
                let report = apply_customization(layout, settings, mode, &dir)?;
                log_custom_report(mode, &report);
                progress(Progress::Done);
                return Ok(InstallOutcome {
                    version_guid: dist.version_guid.clone(),
                    version_label: dist.version_guid.clone(),
                    version_dir: dir,
                    exe: exe_path,
                    did_install: false,
                });
            }
        }
        return Err(Error::NotInstalledUpdatesBlocked);
    } else {
        install_version(
            client,
            layout,
            settings,
            mode,
            &base_url,
            &version_guid,
            &version_dir,
            &mut dist,
            progress,
        )
        .await?;
        dist.version_guid = version_guid.clone();
        dist.size_bytes = dir_size(&version_dir);
        dist.save(&state_path)?;
    }

    progress(Progress::Stage(String::from("Applying settings...")));
    let report = apply_customization(layout, settings, mode, &version_dir)?;
    log_custom_report(mode, &report);

    prune_old_versions(layout, &version_guid);

    progress(Progress::Done);
    Ok(InstallOutcome {
        version_guid,
        version_label,
        version_dir,
        exe,
        did_install: !up_to_date,
    })
}

fn log_custom_report(mode: LaunchMode, report: &CustomReport) {
    tracing::info!(
        "{} customization: flags_written={} mods_copied={} gbs_updated={}",
        mode.label(),
        report.flags_written,
        report.mods_copied,
        report.gbs_updated
    );
    for failure in &report.mod_failures {
        tracing::warn!("mod apply failure: {failure}");
    }
}

fn dist_state_path(layout: &Layout, mode: LaunchMode) -> PathBuf {
    if mode.is_studio() {
        layout.studio_state_file.clone()
    } else {
        layout.player_state_file.clone()
    }
}

// ---------------------------------------------------------------------------
// Download + extract
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
async fn install_version(
    client: &reqwest::Client,
    layout: &Layout,
    settings: &Settings,
    mode: LaunchMode,
    base_url: &str,
    version_guid: &str,
    version_dir: &Path,
    dist: &mut DistributionState,
    progress: &mut impl FnMut(Progress),
) -> Result<()> {
    progress(Progress::Stage(format!(
        "Fetching package list for {}...",
        mode.label()
    )));
    let manifest_url = deployment::location(
        base_url,
        &settings.channel,
        &format!("/{version_guid}-rbxPkgManifest.txt"),
    );
    let manifest_text = crate::http::get_bytes(client, &manifest_url)
        .await
        .map(|b| String::from_utf8_lossy(&b).into_owned())?;
    let packages = crate::manifest::parse_package_manifest(&manifest_text)?;
    if packages.is_empty() {
        return Err(Error::Manifest(String::from("package manifest lists no files")));
    }

    let stage_dir = layout.downloads.join(version_guid);
    std::fs::create_dir_all(&stage_dir).map_err(|e| Error::with_path(&stage_dir.clone(), e))?;

    // Decide what to fetch: skip packages whose recorded signature matches
    // a staged file of the expected size.
    let mut to_fetch = Vec::new();
    for package in &packages {
        let staged = stage_dir.join(&package.name);
        let fresh = dist
            .package_hashes
            .get(&package.name)
            .map(|h| h == &package.signature)
            .unwrap_or(false)
            && staged.is_file()
            && staged.metadata().map(|m| m.len()).unwrap_or(0) == package.packed_size;
        if !fresh {
            to_fetch.push(package.clone());
        }
    }

    if !to_fetch.is_empty() {
        download_packages(client, base_url, &settings.channel, &stage_dir, &to_fetch, dist, progress)
            .await?;
    } else {
        tracing::info!("all {} packages already staged", packages.len());
    }

    progress(Progress::Stage(String::from("Installing packages...")));
    extract_packages(&stage_dir, &packages, version_dir, progress).await?;

    // The install is unusable without its executable — fail loudly.
    let exe_name = mode.exe_name();
    if !version_dir.join(exe_name).is_file() {
        return Err(Error::Extraction(format!(
            "{exe_name} is missing after install; the download may be incomplete"
        )));
    }
    Ok(())
}

async fn download_packages(
    client: &reqwest::Client,
    base_url: &str,
    channel: &str,
    stage_dir: &Path,
    packages: &[Package],
    dist: &mut DistributionState,
    progress: &mut impl FnMut(Progress),
) -> Result<()> {
    let total_bytes: u64 = packages.iter().map(|p| p.packed_size).sum();
    let done_bytes = Arc::new(AtomicU64::new(0));
    let semaphore = Arc::new(tokio::sync::Semaphore::new(DOWNLOAD_CONCURRENCY));
    let started = std::time::Instant::now();

    let mut set = JoinSet::new();
    for package in packages {
        let permit = semaphore
            .clone()
            .acquire_owned()
            .await
            .map_err(|e| Error::Other(format!("download scheduler failed: {e}")))?;
        let client = client.clone();
        let url = deployment::location(base_url, channel, &format!("/{}", package.name));
        let dest = stage_dir.join(&package.name);
        let expected = package.signature.clone();
        let name = package.name.clone();
        let counter = done_bytes.clone();
        set.spawn(async move {
            let _permit = permit;
            // `on_chunk` reports cumulative bytes per file; convert to deltas
            // with task-local state captured by the closure.
            let mut last_total = 0u64;
            let outcome = crate::http::download_to_file(
                &client,
                &url,
                &dest,
                move |total| {
                    counter.fetch_add(total.saturating_sub(last_total), Ordering::Relaxed);
                    last_total = total;
                },
            )
            .await;
            (name, expected, dest, outcome)
        });
    }

    let mut files_done = 0usize;
    let files_total = packages.len();
    let mut ticker = tokio::time::interval(std::time::Duration::from_millis(150));
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    // NOTE: byte accounting below counts completed files (plus staged sizes)
    // rather than live chunks — see the download task above.

    loop {
        tokio::select! {
            _ = ticker.tick() => {
                progress(Progress::Downloading {
                    done_bytes: done_bytes.load(Ordering::Relaxed),
                    total_bytes,
                    files_done,
                    files_total,
                    bytes_per_sec: done_bytes.load(Ordering::Relaxed) as f64
                        / started.elapsed().as_secs_f64().max(0.001),
                });
            }
            outcome = set.join_next() => {
                match outcome {
                    Some(Ok((name, expected, dest, result))) => {
                        result?;
                        // Hash-verify every fresh download before trusting it.
                        let actual = crate::util::md5_hex_file(&dest)?;
                        if !actual.eq_ignore_ascii_case(&expected) {
                            let _ = std::fs::remove_file(&dest);
                            return Err(Error::Checksum {
                                file: name,
                                expected,
                                actual,
                            });
                        }
                        dist.package_hashes.insert(name, expected);
                        files_done += 1;
                        progress(Progress::Downloading {
                            done_bytes: done_bytes.load(Ordering::Relaxed),
                            total_bytes,
                            files_done,
                            files_total,
                            bytes_per_sec: done_bytes.load(Ordering::Relaxed) as f64
                                / started.elapsed().as_secs_f64().max(0.001),
                        });
                    }
                    Some(Err(e)) => {
                        return Err(Error::Other(format!("download task failed: {e}")));
                    }
                    None => break,
                }
            }
        }
    }

    Ok(())
}

async fn extract_packages(
    stage_dir: &Path,
    packages: &[Package],
    version_dir: &Path,
    progress: &mut impl FnMut(Progress),
) -> Result<()> {
    std::fs::create_dir_all(version_dir).map_err(|e| Error::with_path(&version_dir.to_path_buf(), e))?;

    // Extraction is CPU + I/O bound: run zips on the blocking pool with a
    // small concurrency cap so the machine stays responsive.
    let semaphore = Arc::new(tokio::sync::Semaphore::new(EXTRACT_CONCURRENCY));
    let mut set = JoinSet::new();
    for package in packages {
        let permit = semaphore
            .clone()
            .acquire_owned()
            .await
            .map_err(|e| Error::Other(format!("extract scheduler failed: {e}")))?;
        let zip_path = stage_dir.join(&package.name);
        let dest = version_dir.to_path_buf();
        let name = package.name.clone();
        set.spawn_blocking(move || {
            let _permit = permit;
            extract_zip(&zip_path, &dest).map(|count| (name, count))
        });
    }

    let mut files_done = 0usize;
    while let Some(outcome) = set.join_next().await {
        let (name, _count) = outcome
            .map_err(|e| Error::Other(format!("extract task failed: {e}")))??;
        files_done += 1;
        tracing::debug!("extracted {name}");
        progress(Progress::Extracting {
            files_done,
            files_total: packages.len(),
        });
    }
    Ok(())
}

/// Extract one Roblox package zip. Returns the extracted file count.
fn extract_zip(zip_path: &Path, dest_root: &Path) -> Result<usize> {
    let file =
        std::fs::File::open(zip_path).map_err(|e| Error::with_path(&zip_path.to_path_buf(), e))?;
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|e| Error::Extraction(format!("could not open {}: {e}", zip_path.display())))?;
    if archive.is_empty() {
        return Err(Error::Extraction(format!(
            "{} is empty",
            zip_path.display()
        )));
    }

    let mut count = 0;
    for index in 0..archive.len() {
        let mut entry = archive
            .by_index(index)
            .map_err(|e| Error::Extraction(format!("could not read zip entry: {e}")))?;
        let relative = match entry.enclosed_name() {
            Some(p) => p.to_path_buf(),
            None => {
                return Err(Error::Extraction(format!(
                    "zip entry '{}' would escape the install folder",
                    entry.name()
                )))
            }
        };
        let out_path = dest_root.join(relative);
        if entry.is_dir() {
            std::fs::create_dir_all(&out_path)
                .map_err(|e| Error::with_path(&out_path.clone(), e))?;
            continue;
        }
        if let Some(parent) = out_path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| Error::with_path(&parent.to_path_buf(), e))?;
        }
        let mut out_file =
            std::fs::File::create(&out_path).map_err(|e| Error::with_path(&out_path.clone(), e))?;
        std::io::copy(&mut entry, &mut out_file)
            .map_err(|e| Error::with_path(&out_path.clone(), e))?;
        count += 1;
    }
    Ok(count)
}

// ---------------------------------------------------------------------------
// Launch customization (also reused by the desktop app for live installs)
// ---------------------------------------------------------------------------

/// Apply flags, mods, the settings file, and OS preferences to an installed
/// version directory. Idempotent: safe to run on every launch.
pub fn apply_customization(
    layout: &Layout,
    settings: &Settings,
    mode: LaunchMode,
    version_dir: &Path,
) -> Result<CustomReport> {
    let mut report = CustomReport::default();

    // 1. Fast flags: merge user flags + performance settings into the copy
    //    Roblox actually reads (inside the version directory).
    let version_flags = version_dir.join(CLIENT_SETTINGS_FILE);
    if settings.use_flag_manager {
        let (mut store, _) =
            crate::fastflags::FlagStore::load(&layout.working_flags_file);
        settings.apply_performance_flags(&mut store);
        store.save(&version_flags)?;
        report.flags_written = true;
    } else if version_flags.is_file() {
        std::fs::remove_file(&version_flags)
            .map_err(|e| Error::with_path(&version_flags.clone(), e))?;
    }

    // 2. Mods enabled for this app.
    {
        let (mut state, _) = crate::state::State::load(&layout.state_file);
        let found = crate::mods::scan_mods(&layout.modifications);
        let mut dirty = false;
        for m in &found {
            if state.mod_entry(&m.name).is_none() {
                state.ensure_mod(&m.name);
                dirty = true;
            }
        }
        // Prune entries whose folders vanished.
        let before = state.mods.len();
        state.mods.retain(|m| found.iter().any(|f| f.name == m.file));
        if state.mods.len() != before {
            dirty = true;
        }
        if dirty {
            // Best effort: a failed save here must not block a launch.
            if let Err(e) = state.save(&layout.state_file) {
                tracing::warn!("could not persist mod list: {e}");
            }
        }
        let mut active = Vec::new();
        for m in &found {
            let enabled = state
                .mod_entry(&m.name)
                .map(|e| {
                    e.enabled && (!mode.is_studio() && e.player || mode.is_studio() && e.studio)
                })
                .unwrap_or(false);
            if enabled {
                active.push(m.path.clone());
            }
        }
        let (copied, failures) = crate::mods::apply_mods(&active, version_dir);
        report.mods_copied = copied;
        report.mod_failures = failures;
    }

    // 3. Roblox settings file (GlobalBasicSettings).
    {
        let updates = gbs_updates(settings);
        if !updates.is_empty() {
            let path = layout.roblox.join("GlobalBasicSettings_13.xml");
            crate::gbs::write_properties(&path, &updates)?;
            report.gbs_updated = true;
        }
    }

    // 4. OS-level preferences for the Roblox executable.
    {
        let exe = version_dir.join(mode.exe_name());
        if let Err(e) = apply_os_preferences(&exe, settings) {
            // Never fatal: a game must still launch when a tweak fails.
            tracing::warn!("could not apply OS preferences for {}: {e}", exe.display());
        }
    }

    Ok(report)
}

/// Translate GBS-related settings into `(class, name) -> value` updates.
///
/// Only properties with known value semantics are written: an explicit frame
/// cap, the MSAA sample count, and the VSync kill-switch.
fn gbs_updates(settings: &Settings) -> std::collections::BTreeMap<(String, String), String> {
    use crate::gbs::*;
    let mut updates = std::collections::BTreeMap::new();
    let mut put = |key: (&str, &str), value: &str| {
        updates.insert((key.1.to_string(), key.0.to_string()), value.to_string());
    };
    if let Some(cap) = settings.gbs_framerate_cap {
        put(PROP_FRAMERATE_CAP, &cap.to_string());
    }
    // The FPS cap mirrors into the settings file so builds that ignore the
    // fast flag still obey it; an explicit GBS cap takes precedence.
    if settings.use_flag_manager && settings.fps_cap > 0 {
        updates
            .entry((
                PROP_FRAMERATE_CAP.1.to_string(),
                PROP_FRAMERATE_CAP.0.to_string(),
            ))
            .or_insert_with(|| settings.fps_cap.to_string());
    }
    if let Some(level) = settings.msaa.flag_value() {
        put(PROP_MSAALEVEL, level);
    }
    if settings.disable_vsync {
        put(PROP_VSYNC, "True");
    }
    updates
}

#[cfg(windows)]
fn apply_os_preferences(exe: &Path, settings: &Settings) -> Result<()> {
    use crate::registry;

    let exe_str = exe.to_string_lossy().into_owned();

    // GPU preference (Windows graphics settings).
    let gpu_key = "Software\\Microsoft\\DirectX\\UserGpuPreferences";
    match settings.gpu_preference {
        GpuPreference::System => {
            registry::delete_value(gpu_key, Some(&exe_str))?;
        }
        GpuPreference::Integrated => {
            registry::set_sz(gpu_key, Some(&exe_str), "GpuPreference=1;")?;
        }
        GpuPreference::HighPerformance => {
            registry::set_sz(gpu_key, Some(&exe_str), "GpuPreference=2;")?;
        }
    }

    // Compatibility-layer flags share one space-separated Layers value;
    // unknown tokens are preserved untouched.
    let layers_key = "Software\\Microsoft\\Windows NT\\CurrentVersion\\AppCompatFlags\\Layers";
    let mut tokens: Vec<String> = registry::get_sz(layers_key, Some(&exe_str))?
        .map(|v| v.split_whitespace().map(|s| s.to_string()).collect())
        .unwrap_or_default();
    // The leading `~` marks per-user layers; keep it when present.
    let mut set_flag = |flag: &str, on: bool| {
        tokens.retain(|t| t != flag);
        if on {
            tokens.push(flag.to_string());
        }
    };
    set_flag("HIGHDPIAWARE", settings.disable_dpi_scaling);
    set_flag(
        "DISABLEDXMAXIMIZEDWINDOWEDMODE",
        settings.disable_fullscreen_optimizations,
    );
    if tokens.iter().all(|t| t == "~") || tokens.is_empty() {
        registry::delete_value(layers_key, Some(&exe_str))?;
    } else {
        registry::set_sz(layers_key, Some(&exe_str), &tokens.join(" "))?;
    }
    Ok(())
}

#[cfg(not(windows))]
fn apply_os_preferences(_exe: &Path, _settings: &Settings) -> Result<()> {
    Ok(())
}

// ---------------------------------------------------------------------------
// Housekeeping
// ---------------------------------------------------------------------------

/// Delete version directories (and staged downloads) that no longer belong
/// to the active install. The static directory is always preserved.
fn prune_old_versions(layout: &Layout, keep_guid: &str) {
    let entries = match std::fs::read_dir(&layout.versions) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let name = match path.file_name().and_then(|n| n.to_str()) {
            Some(n) => n,
            None => continue,
        };
        if name == keep_guid || name == STATIC_VERSION_DIR {
            continue;
        }
        // Only touch directories that look like Roblox versions.
        if !name.starts_with("version-") && name != STATIC_VERSION_DIR {
            continue;
        }
        tracing::info!("removing old version directory {name}");
        if let Err(e) = std::fs::remove_dir_all(&path) {
            tracing::warn!("could not remove old version {name}: {e}");
        }
        let staged = layout.downloads.join(name);
        if staged.is_dir() {
            let _ = std::fs::remove_dir_all(&staged);
        }
    }

    // Prune staged downloads for versions we no longer track.
    let staged_entries = match std::fs::read_dir(&layout.downloads) {
        Ok(e) => e,
        Err(_) => return,
    };
    let mut known = HashMap::new();
    // Re-read both dist states cheaply to learn the live GUIDs.
    for file in [&layout.player_state_file, &layout.studio_state_file] {
        let (dist, _) = DistributionState::load(file);
        if dist.is_installed() {
            known.insert(dist.version_guid.clone(), true);
        }
    }
    for entry in staged_entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let name = match path.file_name().and_then(|n| n.to_str()) {
            Some(n) => n.to_string(),
            None => continue,
        };
        if name != keep_guid && !known.contains_key(&name) {
            let _ = std::fs::remove_dir_all(&path);
        }
    }
}

fn dir_size(dir: &Path) -> u64 {
    let mut total = 0;
    let mut stack = vec![dir.to_path_buf()];
    let mut guard = 0;
    while let Some(next) = stack.pop() {
        guard += 1;
        if guard > 100_000 {
            break;
        }
        let entries = match std::fs::read_dir(&next) {
            Ok(e) => e,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            if let Ok(ft) = entry.file_type() {
                if ft.is_dir() {
                    stack.push(entry.path());
                } else if ft.is_file() {
                    total += entry.metadata().map(|m| m.len()).unwrap_or(0);
                }
            }
        }
    }
    total
}
