//! Background jobs for the settings UI.
//!
//! Every job runs on the shared Tokio runtime and reports back through a
//! plain `std::mpsc` channel that the UI drains once per frame. Network and
//! blocking work never touch the UI thread, so the window stays responsive
//! while updates download or versions install.

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::mpsc::Sender;

use redstrap_core::error::{Error, Result};
use redstrap_core::paths::Layout;
use redstrap_core::roblox::LaunchMode;
use redstrap_core::settings::{Settings, UpdateChannel};

/// Results delivered back to the UI thread.
#[derive(Debug)]
pub enum TaskMsg {
    UpdateCheck {
        current: String,
        release: Option<ReleaseInfo>,
        error: Option<String>,
    },
    UpdateStaged {
        version: String,
        error: Option<String>,
    },
    Allowlist {
        flags: BTreeSet<String>,
        error: Option<String>,
    },
    SyncProgress {
        mode: String,
        text: String,
        fraction: Option<f32>,
    },
    SyncDone {
        mode: String,
        version: String,
        error: Option<String>,
    },
    GameShortcutDone {
        path: String,
        error: Option<String>,
    },
    GamePreview {
        place_id: u64,
        name: String,
        icon_png: Vec<u8>,
    },
    ChannelChecked {
        channel: String,
        ok: bool,
        detail: String,
    },
    RecolorDone {
        target: String,
        recolored: usize,
        missing: usize,
        failed: Vec<String>,
    },
    ModInstalled {
        name: String,
        error: Option<String>,
    },
    AppliedNow {
        summary: Vec<String>,
    },
    IconFetched {
        url: String,
        png: Vec<u8>,
    },
}

/// Owned, UI-friendly release summary.
#[derive(Debug, Clone)]
pub struct ReleaseInfo {
    pub version: String,
    pub tag: String,
    pub prerelease: bool,
    pub launcher_url: String,
    pub settings_url: Option<String>,
}

impl ReleaseInfo {
    fn from_release(release: &redstrap_core::updater::Release) -> Option<Self> {
        // The launcher asset is required; without it there is no update.
        let launcher = release
            .asset_named(redstrap_core::consts::LAUNCHER_EXE)
            .or_else(|| release.windows_exe())?;
        let settings = release
            .asset_named(redstrap_core::consts::SETTINGS_EXE)
            .map(|a| a.download_url.clone());
        Some(Self {
            version: release.version().to_string(),
            tag: release.tag.clone(),
            prerelease: release.prerelease,
            launcher_url: launcher.download_url.clone(),
            settings_url: settings,
        })
    }
}

fn send(tx: &Sender<TaskMsg>, msg: TaskMsg) {
    let _ = tx.send(msg);
}

// ---------------------------------------------------------------------------
// Self-updates
// ---------------------------------------------------------------------------

pub fn spawn_update_check(
    rt: &tokio::runtime::Runtime,
    http: reqwest::Client,
    tx: Sender<TaskMsg>,
    repo: String,
    channel: UpdateChannel,
) {
    rt.spawn(async move {
        let current = env!("CARGO_PKG_VERSION").to_string();
        let outcome: Result<Option<ReleaseInfo>> = async {
            if channel == UpdateChannel::Disabled {
                return Ok(None);
            }
            let (owner, name) = repo.split_once('/').ok_or_else(|| {
                Error::Update(format!("malformed update repo '{repo}' (expected owner/name)"))
            })?;
            let release = match channel {
                UpdateChannel::Disabled => None,
                UpdateChannel::Stable => Some(
                    redstrap_core::updater::fetch_latest_release(&http, owner.trim(), name.trim())
                        .await?,
                ),
                UpdateChannel::PreRelease | UpdateChannel::Both => {
                    let releases =
                        redstrap_core::updater::fetch_releases(&http, owner.trim(), name.trim())
                            .await?;
                    releases.into_iter().find(|r| {
                        channel == UpdateChannel::Both || r.prerelease
                    })
                }
            };
            match release {
                Some(r) if redstrap_core::updater::is_newer(&current, r.version()) => {
                    Ok(ReleaseInfo::from_release(&r))
                }
                _ => Ok(None),
            }
        }
        .await;
        match outcome {
            Ok(release) => send(&tx, TaskMsg::UpdateCheck { current, release, error: None }),
            Err(e) => send(
                &tx,
                TaskMsg::UpdateCheck {
                    current,
                    release: None,
                    error: Some(e.to_string()),
                },
            ),
        }
    });
}

/// Download + stage `release`; the running binary is swapped on disk and the
/// update takes effect at the next start (the UI offers a restart button).
pub fn spawn_stage_update(
    rt: &tokio::runtime::Runtime,
    http: reqwest::Client,
    tx: Sender<TaskMsg>,
    release: ReleaseInfo,
    launcher_target: std::path::PathBuf,
) {
    rt.spawn(async move {
        let outcome: Result<(), redstrap_core::error::Error> = async {
            // The launcher first: it is never the running process here.
            redstrap_core::updater::swap_with_download(
                &http,
                &release.launcher_url,
                &launcher_target,
                |_| {},
            )
            .await?;
            // Then ourselves (renaming a running exe is safe on Windows).
            if let Some(url) = release.settings_url.as_deref() {
                if let Ok(current) = std::env::current_exe() {
                    redstrap_core::updater::swap_with_download(&http, url, &current, |_| {})
                        .await?;
                }
            }
            Ok(())
        }
        .await;
        match outcome {
            Ok(_) => send(
                &tx,
                TaskMsg::UpdateStaged {
                    version: release.version.clone(),
                    error: None,
                },
            ),
            Err(e) => send(
                &tx,
                TaskMsg::UpdateStaged {
                    version: release.version.clone(),
                    error: Some(e.to_string()),
                },
            ),
        }
    });
}

// ---------------------------------------------------------------------------
// Fast-flag allowlist
// ---------------------------------------------------------------------------

pub fn spawn_allowlist(
    rt: &tokio::runtime::Runtime,
    http: reqwest::Client,
    tx: Sender<TaskMsg>,
    domain: String,
    channel: String,
) {
    rt.spawn(async move {
        match redstrap_core::allowlist::fetch_combined(&http, &domain, &channel).await {
            Ok(flags) => send(&tx, TaskMsg::Allowlist { flags, error: None }),
            Err(e) => send(
                &tx,
                TaskMsg::Allowlist {
                    flags: BTreeSet::new(),
                    error: Some(e.to_string()),
                },
            ),
        }
    });
}

// ---------------------------------------------------------------------------
// Roblox install / repair
// ---------------------------------------------------------------------------

pub fn spawn_sync(
    rt: &tokio::runtime::Runtime,
    http: reqwest::Client,
    tx: Sender<TaskMsg>,
    layout: Layout,
    settings: Settings,
    mode: LaunchMode,
    force: bool,
) {
    rt.spawn(async move {
        let label = mode.label().to_string();
        let progress_tx = tx.clone();
        let mut sink = move |p: redstrap_core::pipeline::Progress| {
            use redstrap_core::pipeline::Progress as P;
            let (text, fraction) = match &p {
                P::Stage(t) => (t.clone(), None),
                P::Downloading {
                    done_bytes,
                    total_bytes,
                    files_done,
                    files_total,
                    ..
                } => (
                    format!(
                        "Downloading {files_done}/{files_total}: {} / {}",
                        redstrap_core::util::format_bytes(*done_bytes),
                        redstrap_core::util::format_bytes(*total_bytes),
                    ),
                    if *total_bytes > 0 {
                        Some(*done_bytes as f32 / *total_bytes as f32)
                    } else {
                        None
                    },
                ),
                P::Extracting {
                    files_done,
                    files_total,
                } => (
                    format!("Installing package {files_done}/{files_total}"),
                    if *files_total > 0 {
                        Some(*files_done as f32 / *files_total as f32)
                    } else {
                        None
                    },
                ),
                P::Done => (String::from("Ready"), Some(1.0)),
            };
            send(
                &progress_tx,
                TaskMsg::SyncProgress {
                    mode: label.clone(),
                    text,
                    fraction,
                },
            );
        };
        let outcome = redstrap_core::pipeline::ensure_installed(
            &http,
            &layout,
            &settings,
            mode,
            None,
            force,
            &mut sink,
        )
        .await;
        match outcome {
            Ok(o) => send(
                &tx,
                TaskMsg::SyncDone {
                    mode: mode.label().to_string(),
                    version: o.version_guid,
                    error: None,
                },
            ),
            Err(e) => send(
                &tx,
                TaskMsg::SyncDone {
                    mode: mode.label().to_string(),
                    version: String::new(),
                    error: Some(e.to_string()),
                },
            ),
        }
    });
}

/// Apply flags/mods/GBS/OS preferences to already-installed versions
/// without downloading anything (the "Apply now" button).
pub fn spawn_apply_now(
    rt: &tokio::runtime::Runtime,
    tx: Sender<TaskMsg>,
    layout: Layout,
    settings: Settings,
) {
    rt.spawn_blocking(move || {
        let mut summary = Vec::new();
        for (mode, state_file) in [
            (LaunchMode::Player, layout.player_state_file.clone()),
            (LaunchMode::Studio, layout.studio_state_file.clone()),
        ] {
            let (dist, _) = redstrap_core::state::DistributionState::load(&state_file);
            if !dist.is_installed() {
                summary.push(format!("{}: not installed, skipped", mode.label()));
                continue;
            }
            let (dir, exe) = redstrap_core::roblox::version_paths(
                &layout,
                mode,
                &dist.version_guid,
                settings.static_directory,
            );
            if !exe.is_file() {
                summary.push(format!("{}: install damaged, skipped", mode.label()));
                continue;
            }
            match redstrap_core::pipeline::apply_customization(&layout, &settings, mode, &dir) {
                Ok(report) => {
                    let mut parts = Vec::new();
                    if report.flags_written {
                        parts.push(String::from("flags"));
                    }
                    if report.mods_copied > 0 {
                        parts.push(format!("{} mod files", report.mods_copied));
                    }
                    if report.gbs_updated {
                        parts.push(String::from("settings file"));
                    }
                    if parts.is_empty() {
                        parts.push(String::from("preferences"));
                    }
                    let mut line = format!("{}: applied {}", mode.label(), parts.join(", "));
                    if !report.mod_failures.is_empty() {
                        line.push_str(&format!(" ({} mod errors)", report.mod_failures.len()));
                    }
                    summary.push(line);
                }
                Err(e) => summary.push(format!("{}: {e}", mode.label())),
            }
        }
        send(&tx, TaskMsg::AppliedNow { summary });
    });
}

// ---------------------------------------------------------------------------
// Game shortcuts
// ---------------------------------------------------------------------------

pub fn spawn_game_shortcut(
    rt: &tokio::runtime::Runtime,
    http: reqwest::Client,
    tx: Sender<TaskMsg>,
    layout: Layout,
    settings: Settings,
    name: String,
    place_id: u64,
    job_id: String,
    access_code: String,
) {
    rt.spawn(async move {
        match redstrap_core::shortcuts::create_game_shortcut(
            &http,
            &layout,
            &settings,
            &name,
            place_id,
            &job_id,
            &access_code,
        )
        .await
        {
            Ok(path) => send(
                &tx,
                TaskMsg::GameShortcutDone {
                    path: path.to_string_lossy().into_owned(),
                    error: None,
                },
            ),
            Err(e) => send(
                &tx,
                TaskMsg::GameShortcutDone {
                    path: String::new(),
                    error: Some(e.to_string()),
                },
            ),
        }
    });
}

/// Resolve a place's display name + icon for the shortcut creator preview.
pub fn spawn_game_preview(
    rt: &tokio::runtime::Runtime,
    http: reqwest::Client,
    tx: Sender<TaskMsg>,
    domain: String,
    place_id: u64,
) {
    rt.spawn(async move {
        let domain = if domain.trim().is_empty() {
            "roblox.com"
        } else {
            domain.trim()
        };
        let mut name = String::new();
        let mut icon_png = Vec::new();

        #[derive(serde::Deserialize)]
        struct UniverseResponse {
            #[serde(rename = "universeId", default)]
            universe_id: u64,
        }
        #[derive(serde::Deserialize)]
        struct GameList {
            #[serde(default)]
            data: Vec<GameEntry>,
        }
        #[derive(serde::Deserialize)]
        struct GameEntry {
            #[serde(default)]
            name: String,
        }
        #[derive(serde::Deserialize)]
        struct ThumbList {
            #[serde(default)]
            data: Vec<ThumbEntry>,
        }
        #[derive(serde::Deserialize)]
        struct ThumbEntry {
            #[serde(rename = "imageUrl", default)]
            image_url: String,
        }

        let universe_url =
            format!("https://apis.{domain}/universes/v1/places/{place_id}/universe");
        if let Ok(u) =
            redstrap_core::http::get_json::<UniverseResponse>(&http, &universe_url).await
        {
            if u.universe_id > 0 {
                let games_url =
                    format!("https://games.{domain}/v1/games?universeIds={}", u.universe_id);
                if let Ok(list) =
                    redstrap_core::http::get_json::<GameList>(&http, &games_url).await
                {
                    if let Some(entry) = list.data.into_iter().next() {
                        name = entry.name;
                    }
                }
                let thumbs_url = format!(
                    "https://thumbnails.{domain}/v1/games/icons?universeIds={}&size=256x256&format=Png&isCircular=false",
                    u.universe_id
                );
                if let Ok(list) =
                    redstrap_core::http::get_json::<ThumbList>(&http, &thumbs_url).await
                {
                    if let Some(entry) = list.data.into_iter().next() {
                        if !entry.image_url.is_empty() {
                            if let Ok(bytes) =
                                redstrap_core::http::get_bytes(&http, &entry.image_url).await
                            {
                                if bytes.len() <= 8 * 1024 * 1024 {
                                    icon_png = bytes;
                                }
                            }
                        }
                    }
                }
            }
        }

        send(&tx, TaskMsg::GamePreview { place_id, name, icon_png });
    });
}

// ---------------------------------------------------------------------------
// Channel validation
// ---------------------------------------------------------------------------

pub fn spawn_channel_check(
    rt: &tokio::runtime::Runtime,
    http: reqwest::Client,
    tx: Sender<TaskMsg>,
    domain: String,
    channel: String,
    token: String,
) {
    rt.spawn(async move {
        let outcome = redstrap_core::deployment::get_info(
            &http,
            &domain,
            &channel,
            redstrap_core::consts::BINARY_TYPE_PLAYER,
            &token,
            false,
        )
        .await;
        match outcome {
            Ok(info) => send(
                &tx,
                TaskMsg::ChannelChecked {
                    channel,
                    ok: true,
                    detail: format!("{} ({})", info.version, info.version_guid),
                },
            ),
            Err(Error::InvalidChannel { .. }) => send(
                &tx,
                TaskMsg::ChannelChecked {
                    channel,
                    ok: false,
                    detail: String::from("unknown or private channel"),
                },
            ),
            Err(e) => send(
                &tx,
                TaskMsg::ChannelChecked {
                    channel,
                    ok: false,
                    detail: e.to_string(),
                },
            ),
        }
    });
}

// ---------------------------------------------------------------------------
// Recoloring + mod installs (blocking filesystem/image work)
// ---------------------------------------------------------------------------

pub fn spawn_recolor(
    rt: &tokio::runtime::Runtime,
    tx: Sender<TaskMsg>,
    version_dir: PathBuf,
    target: String,
    color: (u8, u8, u8),
    groups: Vec<redstrap_core::mods::RecolorGroup>,
    extras: Vec<String>,
) {
    rt.spawn_blocking(move || {
        let report =
            redstrap_core::mods::recolor_pngs(&version_dir, color, &groups, &extras);
        send(
            &tx,
            TaskMsg::RecolorDone {
                target,
                recolored: report.recolored,
                missing: report.missing.len(),
                failed: report.failed,
            },
        );
    });
}

pub fn spawn_install_mod_zip(
    rt: &tokio::runtime::Runtime,
    tx: Sender<TaskMsg>,
    zip: PathBuf,
    mods_dir: PathBuf,
    name: String,
) {
    rt.spawn_blocking(move || {
        match redstrap_core::mods::install_from_zip(&zip, &mods_dir, &name) {
            Ok(path) => send(
                &tx,
                TaskMsg::ModInstalled {
                    name: path
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or(&name)
                        .to_string(),
                    error: None,
                },
            ),
            Err(e) => send(
                &tx,
                TaskMsg::ModInstalled {
                    name,
                    error: Some(e.to_string()),
                },
            ),
        }
    });
}

pub fn spawn_install_mod_paths(
    rt: &tokio::runtime::Runtime,
    tx: Sender<TaskMsg>,
    sources: Vec<PathBuf>,
    mods_dir: PathBuf,
    name: String,
) {
    rt.spawn_blocking(move || {
        match redstrap_core::mods::install_from_paths(&sources, &mods_dir, &name) {
            Ok(path) => send(
                &tx,
                TaskMsg::ModInstalled {
                    name: path
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or(&name)
                        .to_string(),
                    error: None,
                },
            ),
            Err(e) => send(
                &tx,
                TaskMsg::ModInstalled {
                    name,
                    error: Some(e.to_string()),
                },
            ),
        }
    });
}

// ---------------------------------------------------------------------------
// Generic image fetch (overview game icon)
// ---------------------------------------------------------------------------

pub fn spawn_fetch_icon(
    rt: &tokio::runtime::Runtime,
    http: reqwest::Client,
    tx: Sender<TaskMsg>,
    url: String,
) {
    rt.spawn(async move {
        if let Ok(bytes) = redstrap_core::http::get_bytes(&http, &url).await {
            if !bytes.is_empty() && bytes.len() <= 8 * 1024 * 1024 {
                send(&tx, TaskMsg::IconFetched { url, png: bytes });
            }
        }
    });
}
