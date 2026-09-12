//! Ephemeral-but-persisted application state.
//!
//! Unlike [`crate::settings::Settings`], this data is managed by Red Strap
//! itself: installed Roblox versions, package hashes, enabled mods, playtime
//! totals, and UI memory (last opened page).

use std::collections::HashMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::consts::SCHEMA_VERSION;
use crate::error::Result;
use crate::util::{load_json_or_default, save_json};

// ---------------------------------------------------------------------------
// Mod configuration
// ---------------------------------------------------------------------------

/// Per-mod enablement and install targets, keyed by mod folder name.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModConfig {
    #[serde(default)]
    pub file: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_true")]
    pub player: bool,
    #[serde(default)]
    pub studio: bool,
}

fn default_true() -> bool {
    true
}

impl Default for ModConfig {
    fn default() -> Self {
        Self {
            file: String::new(),
            enabled: true,
            player: true,
            studio: false,
        }
    }
}

// ---------------------------------------------------------------------------
// Global state
// ---------------------------------------------------------------------------

/// `State.json`: UI memory, pending reinstall flags, mods, playtime.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct State {
    pub schema_version: u32,
    pub last_page: String,
    pub force_reinstall: bool,
    pub mods: Vec<ModConfig>,
    pub playtime_total_secs: u64,
    pub skipped_update_version: String,
}

impl Default for State {
    fn default() -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            last_page: String::new(),
            force_reinstall: false,
            mods: Vec::new(),
            playtime_total_secs: 0,
            skipped_update_version: String::new(),
        }
    }
}

impl State {
    pub fn load(path: &Path) -> (Self, bool) {
        let (mut state, corrupted): (Self, bool) = load_json_or_default(path);
        if state.schema_version == 0 {
            state.schema_version = SCHEMA_VERSION;
        }
        // Drop entries that lost their folder name.
        state.mods.retain(|m| !m.file.trim().is_empty());
        (state, corrupted)
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        save_json(path, self)
    }

    /// Ensure a config entry exists for a mod folder.
    pub fn ensure_mod(&mut self, file: &str) {
        if !self.mods.iter().any(|m| m.file == file) {
            self.mods.push(ModConfig {
                file: file.to_string(),
                ..ModConfig::default()
            });
        }
    }

    /// Look up the config entry for a mod folder, if present.
    pub fn mod_entry_mut(&mut self, file: &str) -> Option<&mut ModConfig> {
        self.mods.iter_mut().find(|m| m.file == file)
    }

    /// Immutable counterpart of [`State::mod_entry_mut`].
    pub fn mod_entry(&self, file: &str) -> Option<&ModConfig> {
        self.mods.iter().find(|m| m.file == file)
    }
}

// ---------------------------------------------------------------------------
// Distribution state (one file per Roblox app)
// ---------------------------------------------------------------------------

/// Installed-version record for the player or Studio.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct DistributionState {
    /// Installed `version-<hash>` GUID, empty when never installed.
    pub version_guid: String,
    /// Downloaded package name -> manifest signature.
    pub package_hashes: HashMap<String, String>,
    /// Approximate on-disk size in bytes.
    pub size_bytes: u64,
}

impl DistributionState {
    pub fn load(path: &Path) -> (Self, bool) {
        load_json_or_default(path)
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        save_json(path, self)
    }

    /// True when a version was previously installed.
    pub fn is_installed(&self) -> bool {
        !self.version_guid.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mod_entry_is_created_on_demand() {
        let mut s = State::default();
        s.ensure_mod("my-mod");
        if let Some(entry) = s.mod_entry_mut("my-mod") {
            entry.enabled = false;
        }
        assert_eq!(s.mods.len(), 1);
        assert!(!s.mods[0].enabled);
        // Second lookup returns the same entry.
        s.ensure_mod("my-mod");
        assert_eq!(s.mods.len(), 1);
        assert_eq!(s.mod_entry("my-mod").map(|m| m.enabled), Some(false));
    }

    #[test]
    fn distribution_defaults_to_uninstalled() {
        assert!(!DistributionState::default().is_installed());
    }
}
