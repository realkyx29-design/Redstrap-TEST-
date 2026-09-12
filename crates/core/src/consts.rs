//! Application-wide constants for Red Strap.
//!
//! Keeping these in one place makes rebranding, endpoint changes, and
//! tuning trivial, and avoids magic strings scattered through the code.

/// Human-readable application name shown in the UI, window titles, and logs.
pub const APP_NAME: &str = "RedStrap";

/// Lowercase identifier used for file names, mutex names, and directories.
pub const APP_ID: &str = "redstrap";

/// Current settings/state schema version. Bumped when a migration is needed.
pub const SCHEMA_VERSION: u32 = 1;

/// GitHub repository (`owner/name`) used by the self-updater by default.
/// This is also exposed as a setting so forks can point elsewhere.
pub const DEFAULT_UPDATE_REPO: &str = "realkyx29-design/Redstrap-TEST-";

/// File name of the Red Strap launcher executable.
#[cfg(windows)]
pub const LAUNCHER_EXE: &str = "RedStrap.exe";
/// File name of the Red Strap launcher executable.
#[cfg(not(windows))]
pub const LAUNCHER_EXE: &str = "RedStrap";
/// File name of the Red Strap settings executable.
#[cfg(windows)]
pub const SETTINGS_EXE: &str = "RedStrap-Settings.exe";
/// File name of the Red Strap settings executable.
#[cfg(not(windows))]
pub const SETTINGS_EXE: &str = "RedStrap-Settings";

/// File name of the Roblox player executable.
pub const ROBLOX_PLAYER_EXE: &str = "RobloxPlayerBeta.exe";
/// File name of the Roblox Studio executable.
pub const ROBLOX_STUDIO_EXE: &str = "RobloxStudioBeta.exe";

/// Default Roblox web domain (without scheme).
pub const DEFAULT_ROBLOX_DOMAIN: &str = "roblox.com";
/// Default Roblox release channel.
pub const DEFAULT_CHANNEL: &str = "production";
/// Alias that also means the default channel.
pub const LIVE_CHANNEL_ALIAS: &str = "live";

/// Expected body of the `versionStudio` connectivity probe. This is the
/// version hash of the last MFC Studio ever deployed, so it never changes.
pub const VERSION_STUDIO_HASH: &str = "version-012732894899482c";

/// Client-settings binary type identifiers.
pub const BINARY_TYPE_PLAYER: &str = "WindowsPlayer";
pub const BINARY_TYPE_STUDIO: &str = "WindowsStudio64";

/// Fast-flag allowlist application identifiers.
pub const ALLOWLIST_APP_PLAYER: &str = "PCDesktopClient";
pub const ALLOWLIST_APP_STUDIO: &str = "PCStudioClient";

/// Persisted file names (relative to the base directory unless noted).
pub const SETTINGS_FILE: &str = "Settings.json";
pub const STATE_FILE: &str = "State.json";
pub const PLAYER_STATE_FILE: &str = "PlayerState.json";
pub const STUDIO_STATE_FILE: &str = "StudioState.json";
pub const CLIENT_SETTINGS_FILE: &str = "ClientAppSettings.json";

/// Directory names relative to the base directory.
pub const DIR_DOWNLOADS: &str = "Downloads";
pub const DIR_CACHE: &str = "Cache";
pub const DIR_LOGS: &str = "Logs";
pub const DIR_VERSIONS: &str = "Versions";
pub const DIR_MODIFICATIONS: &str = "Modifications";
pub const DIR_FLAG_PROFILES: &str = "FlagProfiles";
pub const DIR_CLIENT_SETTINGS: &str = "ClientSettings";
pub const DIR_CURSOR_SETS: &str = "CursorSets";
pub const DIR_GAME_SHORTCUTS: &str = "GameShortcuts";
pub const DIR_INTEGRATIONS: &str = "Integrations";

/// Directory name used when "static install directory" is enabled.
pub const STATIC_VERSION_DIR: &str = "Roblox";

/// Name of the marker file that identifies an install directory.
pub const INSTALL_MARKER: &str = SETTINGS_FILE;

/// Log file name prefix (the appender adds the rolling date suffix).
pub const LOG_FILE_PREFIX: &str = "RedStrap.log";

/// Maximum number of package downloads running concurrently.
pub const DOWNLOAD_CONCURRENCY: usize = 4;
/// Maximum number of package extractions running concurrently.
pub const EXTRACT_CONCURRENCY: usize = 4;
/// HTTP request timeout, in seconds.
pub const HTTP_TIMEOUT_SECS: u64 = 60;
/// HTTP connect timeout, in seconds.
pub const HTTP_CONNECT_TIMEOUT_SECS: u64 = 15;
/// Chunk size used when hashing files, in bytes.
pub const HASH_CHUNK_SIZE: usize = 64 * 1024;

/// In-memory cap for the recent-log ring buffer shown in the UI.
pub const LOG_RING_CAPACITY: usize = 500;
/// Cap for stored fast-flag undo history entries.
pub const FLAG_UNDO_CAPACITY: usize = 50;

/// Discord IPC pipe / socket base name (`discord-ipc-0` .. `discord-ipc-9`).
pub const DISCORD_IPC_BASENAME: &str = "discord-ipc-";
/// Number of Discord IPC endpoints probed when connecting.
pub const DISCORD_IPC_PROBE_COUNT: u8 = 10;

/// Upper bound for automatic server rejoins in a single watcher session.
/// Prevents infinite rejoin loops when a server is genuinely unreachable.
pub const MAX_AUTO_REJOINS: u32 = 5;

/// Delay before an automatic rejoin is attempted, in seconds.
pub const AUTO_REJOIN_DELAY_SECS: u64 = 3;
