//! Persisted user settings (`Settings.json`).
//!
//! The file uses camelCase JSON keys and is loaded defensively: unknown or
//! missing fields fall back to defaults, and numbers are clamped into sane
//! ranges by [`Settings::validate`]. Every setting maps to real behaviour —
//! there are intentionally no decorative toggles.

use serde::{Deserialize, Serialize};

use crate::consts::*;
use crate::error::Result;
use crate::util::{load_json_or_default, save_json};

// ---------------------------------------------------------------------------
// Enums
// ---------------------------------------------------------------------------

macro_rules! setting_enum {
    ($(#[$meta:meta])* $name:ident { $($( #[$vmeta:meta] )* $variant:ident => $label:expr,)* }) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
        #[serde(rename_all = "camelCase")]
        pub enum $name {
            $($( #[$vmeta] )* $variant,)*
        }

        impl $name {
            /// All variants in display order.
            pub fn all() -> &'static [$name] {
                &[$($name::$variant,)*]
            }

            /// Human-readable label for the settings UI.
            pub fn label(self) -> &'static str {
                match self {
                    $($name::$variant => $label,)*
                }
            }
        }
    };
}

setting_enum! {
    /// Which releases the self-updater considers.
    UpdateChannel {
        Disabled => "Disabled",
        #[default]
        Stable => "Stable releases",
        PreRelease => "Pre-releases",
        Both => "Both",
    }
}

setting_enum! {
    /// When the bundled cleaner removes Roblox temp/cache/log files.
    CleanerMode {
        #[default]
        Never => "Never",
        OnLaunch => "When Roblox launches",
        OnClose => "When Roblox closes",
    }
}

setting_enum! {
    /// OS process priority applied to Roblox after launch.
    ProcessPriority {
        Low => "Low",
        BelowNormal => "Below normal",
        #[default]
        Normal => "Normal",
        AboveNormal => "Above normal",
        High => "High",
        RealTime => "Real time",
    }
}

setting_enum! {
    /// Rendering backend forced through fast flags.
    RenderingMode {
        #[default]
        Default => "Default (DirectX 11)",
        Vulkan => "Vulkan",
        OpenGL => "OpenGL",
        D3D11 => "DirectX 11 (forced)",
    }
}

setting_enum! {
    /// Forced MSAA sample count.
    MSAAMode {
        #[default]
        Default => "Default",
        X1 => "1x",
        X2 => "2x",
        X4 => "4x",
    }
}

setting_enum! {
    /// GPU preference written to the Windows graphics settings registry.
    GpuPreference {
        #[default]
        System => "Let Windows decide",
        Integrated => "Power saving (integrated)",
        HighPerformance => "High performance (discrete)",
    }
}

impl MSAAMode {
    /// Fast-flag value, or `None` when the flag should be removed.
    pub fn flag_value(self) -> Option<&'static str> {
        match self {
            MSAAMode::Default => None,
            MSAAMode::X1 => Some("1"),
            MSAAMode::X2 => Some("2"),
            MSAAMode::X4 => Some("4"),
        }
    }
}

// ---------------------------------------------------------------------------
// Custom integrations
// ---------------------------------------------------------------------------

/// An external program launched alongside Roblox (e.g. overlays, tools).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CustomIntegration {
    #[serde(default = "default_integration_name")]
    pub name: String,
    #[serde(default)]
    pub exe: String,
    #[serde(default)]
    pub args: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

fn default_integration_name() -> String {
    String::from("Custom tool")
}

fn default_true() -> bool {
    true
}

impl Default for CustomIntegration {
    fn default() -> Self {
        Self {
            name: default_integration_name(),
            exe: String::new(),
            args: String::new(),
            enabled: true,
        }
    }
}

// ---------------------------------------------------------------------------
// Settings
// ---------------------------------------------------------------------------

/// Complete user configuration. `#[serde(default)]` on the struct keeps old
/// files loadable after new fields are added.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    pub schema_version: u32,

    // Updates
    pub update_check: UpdateChannel,
    pub background_updates: bool,
    pub update_repo: String,

    // Launching
    pub confirm_launches: bool,
    pub multi_instance_launching: bool,
    pub instances_count: u32,
    pub instance_delay_ms: u64,
    pub process_priority: ProcessPriority,
    pub auto_close_crash_handler: bool,
    pub cleaner: CleanerMode,
    pub cleaner_extra_dirs: Vec<String>,

    // Roblox deployment
    pub channel: String,
    pub channel_token: String,
    pub roblox_domain: String,
    pub auto_update_roblox: bool,
    pub static_directory: bool,
    /// Custom launcher prefix for non-Windows systems, e.g. `wine`.
    /// Empty means "execute the Roblox binary directly".
    pub custom_launch_command: String,

    // Fast flags
    pub use_flag_manager: bool,
    pub alt_enter_fix: bool,

    // Performance (each maps to real flags / registry values)
    /// 0 disables the cap, otherwise target frames per second.
    pub fps_cap: u32,
    pub rendering_mode: RenderingMode,
    pub msaa: MSAAMode,
    /// 0 disables the override, otherwise FRM quality level 1-21.
    pub frm_quality: u8,
    /// 0 disables the override, otherwise texture quality level.
    pub texture_quality: u8,
    pub low_poly_meshes: bool,
    pub remove_grass: bool,
    pub pause_voxelizer: bool,
    pub disable_dpi_scaling: bool,
    pub gray_sky: bool,
    pub gpu_preference: GpuPreference,
    pub disable_fullscreen_optimizations: bool,

    // Roblox client settings file (GlobalBasicSettings). `None` = untouched.
    pub gbs_framerate_cap: Option<i32>,
    /// Maps to `RenderSettings.VSyncDisabled`.
    pub disable_vsync: bool,

    // Integrations
    pub activity_tracking: bool,
    pub discord_rpc: bool,
    /// Discord application ID. Empty disables rich presence.
    pub discord_client_id: String,
    pub rpc_hide_buttons: bool,
    pub rpc_show_game_name: bool,
    pub rpc_show_account: bool,
    pub playtime_counter: bool,
    pub auto_rejoin: bool,
    pub show_server_details: bool,
    pub close_on_leave_game: bool,
    pub custom_integrations: Vec<CustomIntegration>,

    // Mods
    pub last_recolor: String,

    // Shortcuts (managed: UI toggles create/delete the .lnk files)
    pub shortcut_desktop: bool,
    pub shortcut_start_menu: bool,
    pub shortcut_player: bool,
    pub shortcut_studio: bool,
    pub shortcut_settings: bool,

    // Appearance
    pub custom_font_path: String,
    pub ui_scale: f32,

    // Diagnostics
    pub verbose_logging: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            update_check: UpdateChannel::default(),
            background_updates: false,
            update_repo: DEFAULT_UPDATE_REPO.to_string(),
            confirm_launches: true,
            multi_instance_launching: false,
            instances_count: 2,
            instance_delay_ms: 1500,
            process_priority: ProcessPriority::default(),
            auto_close_crash_handler: false,
            cleaner: CleanerMode::default(),
            cleaner_extra_dirs: Vec::new(),
            channel: DEFAULT_CHANNEL.to_string(),
            channel_token: String::new(),
            roblox_domain: DEFAULT_ROBLOX_DOMAIN.to_string(),
            auto_update_roblox: true,
            static_directory: false,
            custom_launch_command: String::new(),
            use_flag_manager: true,
            alt_enter_fix: true,
            fps_cap: 0,
            rendering_mode: RenderingMode::default(),
            msaa: MSAAMode::default(),
            frm_quality: 0,
            texture_quality: 0,
            low_poly_meshes: false,
            remove_grass: false,
            pause_voxelizer: false,
            disable_dpi_scaling: false,
            gray_sky: false,
            gpu_preference: GpuPreference::default(),
            disable_fullscreen_optimizations: false,
            gbs_framerate_cap: None,
            disable_vsync: false,
            activity_tracking: true,
            discord_rpc: true,
            discord_client_id: String::new(),
            rpc_hide_buttons: true,
            rpc_show_game_name: true,
            rpc_show_account: false,
            playtime_counter: true,
            auto_rejoin: false,
            show_server_details: true,
            close_on_leave_game: false,
            custom_integrations: Vec::new(),
            last_recolor: String::from("#e11d2e"),
            shortcut_desktop: true,
            shortcut_start_menu: true,
            shortcut_player: false,
            shortcut_studio: false,
            shortcut_settings: false,
            custom_font_path: String::new(),
            ui_scale: 1.0,
            minimize_to_tray: true,
            verbose_logging: false,
        }
    }
}

impl Settings {
    /// Load from disk, repairing out-of-range values. Returns the settings
    /// and whether the file had to be quarantined as corrupt.
    pub fn load(path: &std::path::Path) -> (Self, bool) {
        let (mut settings, corrupted): (Self, bool) = load_json_or_default(path);
        settings.validate();
        (settings, corrupted)
    }

    /// Persist to disk atomically.
    pub fn save(&self, path: &std::path::Path) -> Result<()> {
        save_json(path, self)
    }

    /// Clamp numbers, trim strings, and repair empty critical values.
    pub fn validate(&mut self) {
        if self.schema_version == 0 {
            self.schema_version = SCHEMA_VERSION;
        }
        self.instances_count = self.instances_count.clamp(1, 8);
        self.instance_delay_ms = self.instance_delay_ms.clamp(0, 30_000);
        self.fps_cap = self.fps_cap.min(1000);
        // 0 = off; otherwise 1..=21.
        if self.frm_quality > 21 {
            self.frm_quality = 21;
        }
        if self.texture_quality > 4 {
            self.texture_quality = 4;
        }
        if !(0.5..=3.0).contains(&self.ui_scale) || !self.ui_scale.is_finite() {
            self.ui_scale = 1.0;
        }

        self.channel = self.channel.trim().to_string();
        if self.channel.is_empty() {
            self.channel = DEFAULT_CHANNEL.to_string();
        }
        self.roblox_domain = self.roblox_domain.trim().to_string();
        if self.roblox_domain.is_empty() {
            self.roblox_domain = DEFAULT_ROBLOX_DOMAIN.to_string();
        }
        self.update_repo = self.update_repo.trim().to_string();
        if self.update_repo.is_empty() {
            self.update_repo = DEFAULT_UPDATE_REPO.to_string();
        }
        self.custom_launch_command = self.custom_launch_command.trim().to_string();
        self.discord_client_id = self.discord_client_id.trim().to_string();
        self.custom_font_path = self.custom_font_path.trim().to_string();

        if let Some(cap) = self.gbs_framerate_cap {
            // Roblox clamps to 240 in most builds; allow a little headroom.
            self.gbs_framerate_cap = Some(cap.clamp(1, 1000));
        }
        // Drop empty integrations left behind by the editor.
        self.custom_integrations
            .retain(|i| !i.name.trim().is_empty() || !i.exe.trim().is_empty());
        self.cleaner_extra_dirs.retain(|d| !d.trim().is_empty());
    }

    /// Apply the performance section to a flag store. Called by the launch
    /// pipeline (and the desktop sync action) when building the merged
    /// `ClientAppSettings.json` copy that Roblox reads — never written back
    /// into the user's editable flag file.
    pub fn apply_performance_flags(&self, flags: &mut crate::fastflags::FlagStore) {
        use crate::fastflags as ff;
        ff::apply_alt_enter_fix(flags, self.alt_enter_fix && self.use_flag_manager);
        ff::apply_fps_cap(flags, self.use_flag_manager.then_some(self.fps_cap));
        ff::apply_rendering_mode(flags, self.rendering_mode);
        ff::apply_msaa(flags, self.msaa);
        ff::apply_frm_quality(flags, self.frm_quality);
        ff::apply_texture_quality(flags, self.texture_quality);
        ff::apply_low_poly_meshes(flags, self.low_poly_meshes);
        ff::apply_remove_grass(flags, self.remove_grass);
        ff::apply_pause_voxelizer(flags, self.pause_voxelizer);
        ff::apply_dpi_scaling(flags, self.disable_dpi_scaling);
        ff::apply_gray_sky(flags, self.gray_sky);
    }
}

/// Import recognised keys from another bootstrapper's `Settings.json`
/// (Bloxstrap / Fishstrap / Froststrap family, PascalCase keys with numeric
/// enums). Unknown content is ignored; the result always validates.
pub fn import_from_foreign_json(text: &str) -> Settings {
    let mut out = Settings::default();
    let value: serde_json::Value = match serde_json::from_str(text) {
        Ok(v) => v,
        Err(_) => return out,
    };
    let obj = match value.as_object() {
        Some(o) => o,
        None => return out,
    };

    let get_bool = |key: &str| obj.get(key).and_then(|v| v.as_bool());
    let get_str = |key: &str| {
        obj.get(key)
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
    };
    let get_u32 = |key: &str| {
        obj.get(key)
            .and_then(|v| v.as_u64())
            .map(|n| n as u32)
    };

    if let Some(v) = get_bool("ConfirmLaunches") {
        out.confirm_launches = v;
    }
    if let Some(v) = get_bool("MultiInstanceLaunching") {
        out.multi_instance_launching = v;
    }
    if let Some(v) = get_u32("MultibloxInstanceCount") {
        out.instances_count = v;
    }
    if let Some(v) = obj
        .get("MultibloxDelayMs")
        .and_then(|v| v.as_u64())
    {
        out.instance_delay_ms = v;
    }
    if let Some(v) = get_bool("AutoCloseCrashHandler") {
        out.auto_close_crash_handler = v;
    }
    if let Some(v) = get_bool("UseFastFlagManager") {
        out.use_flag_manager = v;
    }
    if let Some(v) = get_bool("UseAltManually") {
        out.alt_enter_fix = v;
    }
    if let Some(v) = get_bool("UpdateRoblox") {
        out.auto_update_roblox = v;
    }
    if let Some(v) = get_bool("StaticDirectory") {
        out.static_directory = v;
    }
    if let Some(v) = get_bool("BackgroundUpdatesEnabled") {
        out.background_updates = v;
    }
    if let Some(v) = get_bool("EnableActivityTracking") {
        out.activity_tracking = v;
    }
    if let Some(v) = get_bool("UseDiscordRichPresence") {
        out.discord_rpc = v;
    }
    if let Some(v) = get_bool("HideRPCButtons") {
        out.rpc_hide_buttons = v;
    }
    if let Some(v) = get_bool("AutoRejoin") {
        out.auto_rejoin = v;
    }
    if let Some(v) = get_bool("PlaytimeCounter") {
        out.playtime_counter = v;
    }
    if let Some(v) = get_bool("UseDisableAppPatch") {
        out.close_on_leave_game = v;
    }
    if let Some(v) = get_str("Channel") {
        out.channel = v;
    }
    if let Some(v) = get_str("RobloxDomain") {
        out.roblox_domain = v;
    }
    if let Some(v) = get_str("CustomFontPath") {
        out.custom_font_path = v;
    }
    // Numeric enums (System.Text.Json default): map defensively.
    if let Some(v) = get_u32("SelectedProcessPriority") {
        out.process_priority = match v {
            0 => ProcessPriority::Low,
            1 => ProcessPriority::BelowNormal,
            3 => ProcessPriority::AboveNormal,
            4 => ProcessPriority::High,
            5 => ProcessPriority::RealTime,
            _ => ProcessPriority::Normal,
        };
    }
    if let Some(v) = get_u32("UpdateChecks") {
        out.update_check = match v {
            0 => UpdateChannel::Disabled,
            2 => UpdateChannel::PreRelease,
            3 => UpdateChannel::Both,
            _ => UpdateChannel::Stable,
        };
    }
    if let Some(v) = get_u32("CleanerOptions") {
        out.cleaner = match v {
            1 => CleanerMode::OnLaunch,
            2 => CleanerMode::OnClose,
            _ => CleanerMode::Never,
        };
    }

    out.validate();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_validate_cleanly() {
        let mut s = Settings::default();
        s.validate();
        assert_eq!(s.channel, "production");
        assert_eq!(s.instances_count, 2);
    }

    #[test]
    fn validation_repairs_garbage() {
        let mut s = Settings::default();
        s.instances_count = 99;
        s.frm_quality = 200;
        s.ui_scale = f32::NAN;
        s.channel = String::from("   ");
        s.validate();
        assert_eq!(s.instances_count, 8);
        assert_eq!(s.frm_quality, 21);
        assert_eq!(s.ui_scale, 1.0);
        assert_eq!(s.channel, "production");
    }

    #[test]
    fn foreign_import_maps_keys() {
        let foreign = r#"{
            "ConfirmLaunches": false,
            "Channel": "zekk",
            "SelectedProcessPriority": 4,
            "UpdateChecks": 0,
            "CleanerOptions": 2,
            "UnknownFutureKey": 123
        }"#;
        let s = import_from_foreign_json(foreign);
        assert!(!s.confirm_launches);
        assert_eq!(s.channel, "zekk");
        assert_eq!(s.process_priority, ProcessPriority::High);
        assert_eq!(s.update_check, UpdateChannel::Disabled);
        assert_eq!(s.cleaner, CleanerMode::OnClose);
    }

    #[test]
    fn settings_json_roundtrip() {
        let s = Settings::default();
        let text = serde_json::to_string(&s).expect("serialize");
        assert!(text.contains("\"confirmLaunches\""));
        let back: Settings = serde_json::from_str(&text).expect("deserialize");
        assert_eq!(back.channel, s.channel);
    }
}
