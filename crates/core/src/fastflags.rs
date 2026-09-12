//! Fast-flag store (`ClientAppSettings.json`).
//!
//! Roblox reads `ClientSettings/ClientAppSettings.json` from its version
//! directory at startup. Values are stored as strings (`"True"`, `"4"`).
//! This module owns the working copy, the named-preset table used by the
//! Performance page, on-disk profiles, and bounded undo/redo history.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::consts::FLAG_UNDO_CAPACITY;
use crate::error::Result;
use crate::settings::{MSAAMode, RenderingMode};
use crate::util::{atomic_write, sanitize_filename};

// ---------------------------------------------------------------------------
// Preset table (preset key -> real flag name)
// ---------------------------------------------------------------------------

/// Named presets shown in the UI, mapped to real Roblox flag names.
pub const PRESETS: &[(&str, &str)] = &[
    ("Rendering.ManualFullscreen", "FFlagHandleAltEnterFullscreenManually"),
    ("Rendering.PauseVoxelizer", "DFFlagDebugPauseVoxelizer"),
    ("Rendering.DisableScaling", "DFFlagDisableDPIScale"),
    (
        "Rendering.TextureQuality.OverrideEnabled",
        "DFFlagTextureQualityOverrideEnabled",
    ),
    (
        "Rendering.TextureQuality.Level",
        "DFIntTextureQualityOverride",
    ),
    ("Rendering.FrmQuality", "DFIntDebugFRMQualityLevelOverride"),
    (
        "Rendering.LowPolyMeshes1",
        "DFIntCSGLevelOfDetailSwitchingDistance",
    ),
    (
        "Rendering.LowPolyMeshes2",
        "DFIntCSGLevelOfDetailSwitchingDistanceL12",
    ),
    (
        "Rendering.LowPolyMeshes3",
        "DFIntCSGLevelOfDetailSwitchingDistanceL23",
    ),
    (
        "Rendering.LowPolyMeshes4",
        "DFIntCSGLevelOfDetailSwitchingDistanceL34",
    ),
    ("Rendering.Mode.D3D11", "FFlagDebugGraphicsPreferD3D11"),
    ("Rendering.Mode.Vulkan", "FFlagDebugGraphicsPreferVulkan"),
    ("Rendering.Mode.OpenGL", "FFlagDebugGraphicsPreferOpenGL"),
    ("Graphic.GraySky", "FFlagDebugSkyGray"),
    ("Rendering.MSAA1", "FIntDebugForceMSAASamples"),
    ("Rendering.RemoveGrass1", "FIntFRMMinGrassDistance"),
    ("Rendering.RemoveGrass2", "FIntFRMMaxGrassDistance"),
    (
        "Rendering.RemoveGrass3",
        "FIntGrassMovementReducedMotionFactor",
    ),
    ("Performance.FpsCap", "DFIntTaskSchedulerTargetFps"),
];

/// Look up the real flag name behind a preset key.
pub fn preset_flag(key: &str) -> Option<&'static str> {
    PRESETS
        .iter()
        .find(|(k, _)| *k == key)
        .map(|(_, flag)| *flag)
}

/// True when `flag` (case-insensitive) is managed by a preset.
pub fn is_preset_flag(flag: &str) -> bool {
    PRESETS
        .iter()
        .any(|(_, name)| name.eq_ignore_ascii_case(flag))
}

// ---------------------------------------------------------------------------
// Store
// ---------------------------------------------------------------------------

/// In-memory flag table with change tracking and undo/redo.
#[derive(Debug, Clone, Default)]
pub struct FlagStore {
    flags: BTreeMap<String, String>,
    original: BTreeMap<String, String>,
    undo: Vec<BTreeMap<String, String>>,
    redo: Vec<BTreeMap<String, String>>,
    suspend_undo: bool,
}

impl FlagStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Load from a `ClientAppSettings.json` file. Missing files yield an
    /// empty store; corrupt files are moved aside (see util) and reported.
    pub fn load(path: &Path) -> (Self, bool) {
        let bytes = match std::fs::read(path) {
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return (Self::new(), false)
            }
            Err(e) => {
                tracing::warn!("failed to read {}: {}", path.display(), e);
                return (Self::new(), false);
            }
        };

        let parsed: BTreeMap<String, Value> = match serde_json::from_slice(&bytes) {
            Ok(map) => map,
            Err(e) => {
                tracing::warn!("{} is corrupt ({}); starting empty", path.display(), e);
                let backup = path.with_extension(format!(
                    "corrupt-{}",
                    crate::util::unix_seconds()
                ));
                if std::fs::rename(path, &backup).is_ok() {
                    tracing::info!("corrupt flags preserved at {}", backup.display());
                }
                return (Self::new(), true);
            }
        };

        let mut flags = BTreeMap::new();
        for (key, value) in parsed {
            let text = match value {
                Value::String(s) => s,
                Value::Number(n) => n.to_string(),
                Value::Bool(b) => {
                    if b {
                        String::from("True")
                    } else {
                        String::from("False")
                    }
                }
                // Nulls, arrays, and objects are not valid flag values.
                _ => continue,
            };
            if !key.trim().is_empty() {
                flags.insert(key, text);
            }
        }

        let mut store = Self::new();
        store.flags = flags.clone();
        store.original = flags;
        (store, false)
    }

    /// Save all flags as string values, atomically.
    pub fn save(&self, path: &Path) -> Result<()> {
        let text = serde_json::to_string_pretty(&self.flags)
            .unwrap_or_else(|_| String::from("{}"));
        atomic_write(path, text.as_bytes())
    }

    /// Mark the current content as the new "saved" baseline.
    pub fn mark_saved(&mut self) {
        self.original.clone_from(&self.flags);
    }

    /// True when the store differs from the last saved/loaded baseline.
    pub fn changed(&self) -> bool {
        self.flags != self.original
    }

    pub fn len(&self) -> usize {
        self.flags.len()
    }

    pub fn is_empty(&self) -> bool {
        self.flags.is_empty()
    }

    /// Iterate over `(name, value)` pairs in sorted order.
    pub fn iter(&self) -> impl Iterator<Item = (&String, &String)> {
        self.flags.iter()
    }

    /// Get a flag value, if present.
    pub fn get(&self, key: &str) -> Option<&str> {
        self.flags.get(key).map(|s| s.as_str())
    }

    /// Set a flag, or delete it when `value` is `None`.
    pub fn set(&mut self, key: &str, value: Option<&str>) {
        let key = key.trim();
        if key.is_empty() {
            return;
        }
        if !self.suspend_undo {
            self.snapshot_undo();
        }
        match value {
            Some(v) => {
                self.flags.insert(key.to_string(), v.to_string());
            }
            None => {
                self.flags.remove(key);
            }
        }
    }

    /// Set a preset by preset key, or delete it when `value` is `None`.
    pub fn set_preset(&mut self, preset_key: &str, value: Option<&str>) {
        if let Some(flag) = preset_flag(preset_key) {
            self.set(flag, value);
        } else {
            tracing::warn!("unknown flag preset '{preset_key}'");
        }
    }

    /// Read a preset value by preset key.
    pub fn get_preset(&self, preset_key: &str) -> Option<&str> {
        preset_flag(preset_key).and_then(|flag| self.get(flag))
    }

    /// Set every preset whose key starts with `prefix` to `value`.
    pub fn set_preset_group(&mut self, prefix: &str, value: Option<&str>) {
        let flags: Vec<&'static str> = PRESETS
            .iter()
            .filter(|(k, _)| k.starts_with(prefix))
            .map(|(_, flag)| *flag)
            .collect();
        self.suspend_undo = true;
        // Single undo entry for the whole group.
        self.snapshot_undo();
        for flag in flags {
            self.set(flag, value);
        }
        self.suspend_undo = false;
    }

    /// Within a preset group, set the matching `target` entry and clear the
    /// rest (used for mutually-exclusive rendering backends).
    pub fn set_preset_exclusive(&mut self, prefix: &str, target: &str, value: Option<&str>) {
        let full_prefix = format!("{prefix}.{target}");
        let entries: Vec<(&'static str, &'static str)> = PRESETS
            .iter()
            .filter(|(k, _)| k.starts_with(prefix))
            .map(|(k, f)| (*k, *f))
            .collect();
        self.suspend_undo = true;
        self.snapshot_undo();
        for (key, flag) in entries {
            if key.starts_with(&full_prefix) {
                self.set(flag, value);
            } else {
                self.set(flag, None);
            }
        }
        self.suspend_undo = false;
    }

    /// Remove every flag not present in `allow` (case-insensitive).
    /// Returns the number of removed flags.
    pub fn retain_allowed(&mut self, allow: &BTreeSet<String>) -> usize {
        let lowered: BTreeSet<String> =
            allow.iter().map(|s| s.to_ascii_lowercase()).collect();
        let doomed: Vec<String> = self
            .flags
            .keys()
            .filter(|k| !lowered.contains(&k.to_ascii_lowercase()))
            .cloned()
            .collect();
        if doomed.is_empty() {
            return 0;
        }
        self.snapshot_undo();
        for key in &doomed {
            self.flags.remove(key);
        }
        doomed.len()
    }

    /// Split flag names into (known, unknown) against an allowlist.
    pub fn partition_by_allowlist(
        &self,
        allow: &BTreeSet<String>,
    ) -> (Vec<String>, Vec<String>) {
        let lowered: BTreeSet<String> =
            allow.iter().map(|s| s.to_ascii_lowercase()).collect();
        let mut known = Vec::new();
        let mut unknown = Vec::new();
        for key in self.flags.keys() {
            if lowered.contains(&key.to_ascii_lowercase()) {
                known.push(key.clone());
            } else {
                unknown.push(key.clone());
            }
        }
        (known, unknown)
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    pub fn undo(&mut self) {
        if let Some(previous) = self.undo.pop() {
            self.redo.push(std::mem::take(&mut self.flags));
            if self.redo.len() > FLAG_UNDO_CAPACITY {
                self.redo.remove(0);
            }
            self.flags = previous;
        }
    }

    pub fn redo(&mut self) {
        if let Some(next) = self.redo.pop() {
            self.undo.push(std::mem::take(&mut self.flags));
            if self.undo.len() > FLAG_UNDO_CAPACITY {
                self.undo.remove(0);
            }
            self.flags = next;
        }
    }

    fn snapshot_undo(&mut self) {
        if self.undo.last().map(|s| *s == self.flags).unwrap_or(false) {
            return;
        }
        self.undo.push(self.flags.clone());
        if self.undo.len() > FLAG_UNDO_CAPACITY {
            self.undo.remove(0);
        }
        self.redo.clear();
    }
}

// ---------------------------------------------------------------------------
// Preset application helpers (used by the Performance page)
// ---------------------------------------------------------------------------

/// Alt+Enter fullscreen fix (`False` = let Red Strap handle it).
pub fn apply_alt_enter_fix(store: &mut FlagStore, enabled: bool) {
    store.set_preset(
        "Rendering.ManualFullscreen",
        enabled.then_some("False"),
    );
}

/// FPS cap. `None` disables the whole preset group cleanly.
pub fn apply_fps_cap(store: &mut FlagStore, cap: Option<u32>) {
    match cap {
        Some(0) | None => store.set_preset("Performance.FpsCap", None),
        Some(n) => store.set_preset("Performance.FpsCap", Some(&n.to_string())),
    }
}

pub fn apply_rendering_mode(store: &mut FlagStore, mode: RenderingMode) {
    match mode {
        RenderingMode::Default => {
            store.set_preset_group("Rendering.Mode", None);
        }
        RenderingMode::Vulkan => {
            store.set_preset_exclusive("Rendering.Mode", "Vulkan", Some("True"));
        }
        RenderingMode::OpenGL => {
            store.set_preset_exclusive("Rendering.Mode", "OpenGL", Some("True"));
        }
        RenderingMode::D3D11 => {
            store.set_preset_exclusive("Rendering.Mode", "D3D11", Some("True"));
        }
    }
}

pub fn apply_msaa(store: &mut FlagStore, mode: MSAAMode) {
    store.set_preset("Rendering.MSAA1", mode.flag_value());
}

/// 0 disables; otherwise FRM quality level 1-21.
pub fn apply_frm_quality(store: &mut FlagStore, level: u8) {
    if level == 0 {
        store.set_preset("Rendering.FrmQuality", None);
    } else {
        store.set_preset("Rendering.FrmQuality", Some(&level.to_string()));
    }
}

/// 0 disables; otherwise enables the texture quality override at `level`.
pub fn apply_texture_quality(store: &mut FlagStore, level: u8) {
    if level == 0 {
        store.set_preset("Rendering.TextureQuality.OverrideEnabled", None);
        store.set_preset("Rendering.TextureQuality.Level", None);
    } else {
        store.set_preset("Rendering.TextureQuality.OverrideEnabled", Some("True"));
        store.set_preset(
            "Rendering.TextureQuality.Level",
            Some(&level.to_string()),
        );
    }
}

pub fn apply_low_poly_meshes(store: &mut FlagStore, enabled: bool) {
    if enabled {
        // Shorter LOD switching distances = simpler meshes sooner.
        store.set_preset("Rendering.LowPolyMeshes1", Some("50"));
        store.set_preset("Rendering.LowPolyMeshes2", Some("100"));
        store.set_preset("Rendering.LowPolyMeshes3", Some("200"));
        store.set_preset("Rendering.LowPolyMeshes4", Some("300"));
    } else {
        store.set_preset_group("Rendering.LowPolyMeshes", None);
    }
}

pub fn apply_remove_grass(store: &mut FlagStore, enabled: bool) {
    if enabled {
        store.set_preset("Rendering.RemoveGrass1", Some("0"));
        store.set_preset("Rendering.RemoveGrass2", Some("0"));
        store.set_preset("Rendering.RemoveGrass3", Some("0"));
    } else {
        store.set_preset_group("Rendering.RemoveGrass", None);
    }
}

pub fn apply_pause_voxelizer(store: &mut FlagStore, enabled: bool) {
    store.set_preset(
        "Rendering.PauseVoxelizer",
        enabled.then_some("True"),
    );
}

pub fn apply_dpi_scaling(store: &mut FlagStore, disabled: bool) {
    store.set_preset("Rendering.DisableScaling", disabled.then_some("True"));
}

pub fn apply_gray_sky(store: &mut FlagStore, enabled: bool) {
    store.set_preset("Graphic.GraySky", enabled.then_some("True"));
}

// ---------------------------------------------------------------------------
// Profiles
// ---------------------------------------------------------------------------

/// List saved profile names (without extension), sorted.
pub fn list_profiles(dir: &Path) -> Vec<String> {
    let mut out = Vec::new();
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return out,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
            out.push(stem.to_string());
        }
    }
    out.sort();
    out
}

/// Resolve a profile name to its file path (sanitized, always `.json`).
pub fn profile_path(dir: &Path, name: &str) -> PathBuf {
    dir.join(format!("{}.json", sanitize_filename(name)))
}

/// Save the current store as a named profile.
pub fn save_profile(dir: &Path, name: &str, store: &FlagStore) -> Result<PathBuf> {
    let path = profile_path(dir, name);
    store.save(&path)?;
    Ok(path)
}

/// Load a profile into a fresh store.
pub fn load_profile(dir: &Path, name: &str) -> (FlagStore, bool) {
    FlagStore::load(&profile_path(dir, name))
}

/// Delete a profile. Missing files are not an error.
pub fn delete_profile(dir: &Path, name: &str) -> Result<()> {
    let path = profile_path(dir, name);
    match std::fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(crate::error::Error::with_path(&path, e)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "redstrap-flags-test-{}-{tag}",
            std::process::id()
        ));
        let _ = std::fs::create_dir_all(&dir);
        dir
    }

    #[test]
    fn set_get_delete_and_changed() {
        let mut s = FlagStore::new();
        assert!(!s.changed());
        s.set("A", Some("1"));
        assert_eq!(s.get("A"), Some("1"));
        assert!(s.changed());
        s.mark_saved();
        assert!(!s.changed());
        s.set("A", None);
        assert_eq!(s.get("A"), None);
        assert!(s.changed());
    }

    #[test]
    fn presets_roundtrip() {
        let mut s = FlagStore::new();
        s.set_preset("Rendering.MSAA1", Some("4"));
        assert_eq!(s.get("FIntDebugForceMSAASamples"), Some("4"));
        assert_eq!(s.get_preset("Rendering.MSAA1"), Some("4"));
        assert!(is_preset_flag("fintdebugforcemsaasamples"));
        assert!(!is_preset_flag("SomeRandomFlag"));
    }

    #[test]
    fn exclusive_groups_clear_siblings() {
        let mut s = FlagStore::new();
        apply_rendering_mode(&mut s, RenderingMode::Vulkan);
        assert_eq!(s.get("FFlagDebugGraphicsPreferVulkan"), Some("True"));
        assert_eq!(s.get("FFlagDebugGraphicsPreferOpenGL"), None);
        apply_rendering_mode(&mut s, RenderingMode::Default);
        assert_eq!(s.get("FFlagDebugGraphicsPreferVulkan"), None);
    }

    #[test]
    fn undo_redo_work() {
        let mut s = FlagStore::new();
        s.set("A", Some("1"));
        s.set("B", Some("2"));
        assert!(s.can_undo());
        s.undo();
        assert_eq!(s.get("B"), None);
        assert_eq!(s.get("A"), Some("1"));
        s.redo();
        assert_eq!(s.get("B"), Some("2"));
        s.undo();
        s.undo();
        assert!(s.is_empty());
        assert!(!s.can_undo());
    }

    #[test]
    fn allowlist_partition() {
        let mut s = FlagStore::new();
        s.set("Known", Some("1"));
        s.set("Mystery", Some("2"));
        let allow: BTreeSet<String> = ["known".to_string()].into_iter().collect();
        let (known, unknown) = s.partition_by_allowlist(&allow);
        assert_eq!(known, vec![String::from("Known")]);
        assert_eq!(unknown, vec![String::from("Mystery")]);
        assert_eq!(s.retain_allowed(&allow), 1);
        assert_eq!(s.get("Mystery"), None);
    }

    #[test]
    fn profiles_save_list_load_delete() {
        let dir = scratch_dir("profiles");
        let mut s = FlagStore::new();
        s.set("A", Some("1"));
        save_profile(&dir, "gaming", &s).expect("save");
        assert_eq!(list_profiles(&dir), vec![String::from("gaming")]);
        let (loaded, _) = load_profile(&dir, "gaming");
        assert_eq!(loaded.get("A"), Some("1"));
        delete_profile(&dir, "gaming").expect("delete");
        assert!(list_profiles(&dir).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn load_coerces_types() {
        let dir = scratch_dir("coerce");
        let path = dir.join("ClientAppSettings.json");
        std::fs::write(&path, r#"{"a":"x","b":5,"c":true,"d":null,"e":[1]}"#)
            .expect("write");
        let (s, _) = FlagStore::load(&path);
        assert_eq!(s.get("a"), Some("x"));
        assert_eq!(s.get("b"), Some("5"));
        assert_eq!(s.get("c"), Some("True"));
        assert_eq!(s.get("d"), None);
        assert_eq!(s.get("e"), None);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
