//! Install-location resolution and on-disk layout.
//!
//! Red Strap keeps everything (settings, states, downloads, versions, mods,
//! logs) under a single base directory:
//!
//! * Windows: `%LOCALAPPDATA%\RedStrap`
//! * Linux: `~/.local/share/redstrap` (or `$XDG_DATA_HOME/redstrap`)
//! * macOS: `~/Library/Application Support/RedStrap`
//!
//! A directory counts as an install location when it contains
//! `Settings.json`. An explicit `REDSTRAP_HOME` environment variable always
//! wins, which keeps tests hermetic and enables portable installs.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use crate::consts::*;
use crate::error::Result;
use crate::util;

/// Fully-resolved directory layout. Cheap to clone (paths only).
#[derive(Debug, Clone)]
pub struct Layout {
    pub base: PathBuf,
    pub downloads: PathBuf,
    pub cache: PathBuf,
    pub logs: PathBuf,
    pub integrations: PathBuf,
    pub versions: PathBuf,
    pub modifications: PathBuf,
    pub preset_modifications: PathBuf,
    pub flag_profiles: PathBuf,
    pub client_settings: PathBuf,
    pub cursor_sets: PathBuf,
    pub game_shortcuts: PathBuf,
    /// Roblox's own data directory (`%LOCALAPPDATA%\Roblox` on Windows).
    pub roblox: PathBuf,
    pub roblox_logs: PathBuf,
    pub roblox_temp: PathBuf,
    /// Expected location of this application's executable once installed.
    pub application: PathBuf,
    pub settings_file: PathBuf,
    pub state_file: PathBuf,
    pub player_state_file: PathBuf,
    pub studio_state_file: PathBuf,
    pub working_flags_file: PathBuf,
}

impl Layout {
    pub fn new(base: &Path) -> Self {
        let exe_name = crate::consts::LAUNCHER_EXE;
        let roblox = roblox_data_dir();
        let client_settings = base.join(DIR_CLIENT_SETTINGS);
        Self {
            base: base.to_path_buf(),
            downloads: base.join(DIR_DOWNLOADS),
            cache: base.join(DIR_CACHE),
            logs: base.join(DIR_LOGS),
            integrations: base.join(DIR_INTEGRATIONS),
            versions: base.join(DIR_VERSIONS),
            modifications: base.join(DIR_MODIFICATIONS),
            preset_modifications: base.join(DIR_MODIFICATIONS).join("Preset Modifications"),
            flag_profiles: base.join(DIR_FLAG_PROFILES),
            working_flags_file: client_settings.join(CLIENT_SETTINGS_FILE),
            client_settings,
            cursor_sets: base.join(DIR_CURSOR_SETS),
            game_shortcuts: base.join(DIR_CACHE).join(DIR_GAME_SHORTCUTS),
            roblox_logs: roblox.join("logs"),
            roblox_temp: std::env::temp_dir().join("Roblox"),
            roblox,
            application: base.join(exe_name),
            settings_file: base.join(SETTINGS_FILE),
            state_file: base.join(STATE_FILE),
            player_state_file: base.join(PLAYER_STATE_FILE),
            studio_state_file: base.join(STUDIO_STATE_FILE),
        }
    }

    /// Installed location of the settings UI executable.
    pub fn settings_app(&self) -> PathBuf {
        self.base.join(crate::consts::SETTINGS_EXE)
    }

    /// Create every directory Red Strap writes to (idempotent).
    pub fn ensure_dirs(&self) -> Result<()> {
        for dir in [
            &self.base,
            &self.downloads,
            &self.cache,
            &self.logs,
            &self.integrations,
            &self.versions,
            &self.modifications,
            &self.flag_profiles,
            &self.client_settings,
            &self.cursor_sets,
            &self.game_shortcuts,
        ] {
            std::fs::create_dir_all(dir)
                .map_err(|e| crate::error::Error::with_path(&dir.clone(), e))?;
        }
        Ok(())
    }
}

/// Process-wide layout, initialised on first use.
static LAYOUT: OnceLock<Layout> = OnceLock::new();

/// Override the process-wide layout (used at startup once the real install
/// location is known). Subsequent calls are ignored.
pub fn init(base: &Path) {
    let _ = LAYOUT.set(Layout::new(base));
}

/// Access the process-wide layout, resolving a sensible default when the
/// application never called [`init`] (e.g. unit tests, first run).
pub fn get() -> &'static Layout {
    LAYOUT.get_or_init(|| Layout::new(&resolve_base_dir()))
}

/// Best-effort install-location resolution, in priority order:
///
/// 1. `REDSTRAP_HOME` environment variable (portable / test override).
/// 2. The executable's own directory, when it contains `Settings.json`.
/// 3. The Windows uninstall-registry `InstallLocation` value.
/// 4. The platform default data directory.
pub fn resolve_base_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("REDSTRAP_HOME").map(PathBuf::from) {
        if !dir.as_os_str().is_empty() {
            return dir;
        }
    }

    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            if dir.join(INSTALL_MARKER).is_file() {
                return dir.to_path_buf();
            }
        }
    }

    #[cfg(windows)]
    {
        if let Ok(Some(location)) = crate::registry::get_sz(
            &crate::registry::uninstall_key(),
            Some("InstallLocation"),
        ) {
            let dir = PathBuf::from(location);
            if dir.join(INSTALL_MARKER).is_file() {
                return dir;
            }
        }
    }

    default_base_dir()
}

/// Platform default base directory.
pub fn default_base_dir() -> PathBuf {
    let name = if cfg!(windows) { APP_NAME } else { APP_ID };
    util::local_app_data()
        .map(|d| d.join(name))
        .or_else(|| util::home_dir().map(|h| h.join(format!(".{}", APP_ID))))
        .unwrap_or_else(|| PathBuf::from(format!(".{}", APP_ID)))
}

/// Roblox's own data directory.
pub fn roblox_data_dir() -> PathBuf {
    #[cfg(windows)]
    {
        util::local_app_data()
            .map(|d| d.join("Roblox"))
            .unwrap_or_else(|| PathBuf::from("Roblox"))
    }
    #[cfg(not(windows))]
    {
        // Roblox only runs natively on Windows; on other systems these paths
        // are used for Wine prefix-relative lookups and log discovery.
        util::home_dir()
            .map(|h| h.join(".redstrap-roblox"))
            .unwrap_or_else(|| PathBuf::from(".redstrap-roblox"))
    }
}
