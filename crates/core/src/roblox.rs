//! Roblox launch targets, deeplinks, and client-log parsing.
//!
//! Log parsing is allocation-light on purpose: the watcher tails the log at
//! 1 Hz, so every matcher is a plain `starts_with` / manual scan — no regex
//! engine, no per-line regex compilation.

use std::path::{Path, PathBuf};

use crate::consts::*;

// ---------------------------------------------------------------------------
// Launch targets
// ---------------------------------------------------------------------------

/// Which Roblox application to launch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LaunchMode {
    #[default]
    Player,
    Studio,
    /// Studio opened through the `roblox-studio-auth:` protocol.
    StudioAuth,
}

impl LaunchMode {
    pub fn parse(text: &str) -> Option<Self> {
        match text.trim().to_ascii_lowercase().as_str() {
            "player" => Some(LaunchMode::Player),
            "studio" => Some(LaunchMode::Studio),
            "studioauth" | "studio-auth" => Some(LaunchMode::StudioAuth),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            LaunchMode::Player => "player",
            LaunchMode::Studio => "studio",
            LaunchMode::StudioAuth => "studio-auth",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            LaunchMode::Player => "Roblox Player",
            LaunchMode::Studio => "Roblox Studio",
            LaunchMode::StudioAuth => "Roblox Studio",
        }
    }

    pub fn exe_name(self) -> &'static str {
        match self {
            LaunchMode::Player => ROBLOX_PLAYER_EXE,
            LaunchMode::Studio | LaunchMode::StudioAuth => ROBLOX_STUDIO_EXE,
        }
    }

    pub fn binary_type(self) -> &'static str {
        match self {
            LaunchMode::Player => BINARY_TYPE_PLAYER,
            LaunchMode::Studio | LaunchMode::StudioAuth => BINARY_TYPE_STUDIO,
        }
    }

    pub fn is_studio(self) -> bool {
        !matches!(self, LaunchMode::Player)
    }
}

/// Resolve the version directory + executable for a launch.
pub fn version_paths(
    layout: &crate::paths::Layout,
    mode: LaunchMode,
    version_guid: &str,
    static_dir: bool,
) -> (PathBuf, PathBuf) {
    let version_dir = if static_dir {
        layout.versions.join(STATIC_VERSION_DIR)
    } else {
        layout.versions.join(version_guid)
    };
    let exe = version_dir.join(mode.exe_name());
    (version_dir, exe)
}

/// Build the OS command that starts Roblox. On non-Windows systems a
/// `custom_command` (e.g. `wine`) prefixes the executable.
pub fn launch_command(
    exe: &Path,
    args: &str,
    custom_command: &str,
    working_dir: &Path,
) -> std::process::Command {
    let mut command = if custom_command.trim().is_empty() {
        std::process::Command::new(exe)
    } else {
        let mut parts = custom_command.split_whitespace();
        let program = parts.next().unwrap_or("wine");
        let mut cmd = std::process::Command::new(program);
        for extra in parts {
            cmd.arg(extra);
        }
        cmd.arg(exe);
        cmd
    };
    if !args.trim().is_empty() {
        // Roblox expects the raw protocol string as one argument.
        command.arg(args.trim());
    }
    command.current_dir(working_dir);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // Avoid flashing a console window for GUI children.
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    command
}

// ---------------------------------------------------------------------------
// Deeplinks
// ---------------------------------------------------------------------------

/// `roblox://experiences/start?placeId=...` deeplink used by game shortcuts
/// and server rejoins.
pub fn start_deeplink(place_id: u64, job_id: &str, access_code: &str) -> String {
    let mut link = format!("roblox://experiences/start?placeId={place_id}");
    if !access_code.is_empty() {
        link.push_str("&accessCode=");
        link.push_str(access_code);
    } else if !job_id.is_empty() {
        link.push_str("&gameInstanceId=");
        link.push_str(job_id);
    }
    link
}

/// Parse a `-gameshortcut`-style payload: `placeId[;jobId[;accessCode]]`.
pub fn parse_gameshortcut(data: &str) -> Option<String> {
    let mut parts = data.split(';');
    let place: u64 = parts.next()?.trim().parse().ok()?;
    if place == 0 {
        return None;
    }
    let job = parts.next().unwrap_or("").trim();
    let access = parts.next().unwrap_or("").trim();
    Some(start_deeplink(place, job, access))
}

/// Classify a protocol URI given on the command line.
pub fn classify_protocol_arg(arg: &str) -> Option<LaunchMode> {
    let lower = arg.to_ascii_lowercase();
    if lower.starts_with("roblox-player:") || lower.starts_with("roblox:") {
        Some(LaunchMode::Player)
    } else if lower.starts_with("roblox-studio-auth:") {
        Some(LaunchMode::StudioAuth)
    } else if lower.starts_with("roblox-studio:") {
        Some(LaunchMode::Studio)
    } else {
        None
    }
}

// ---------------------------------------------------------------------------
// Log discovery
// ---------------------------------------------------------------------------

/// Find the newest `.log` file in a directory (by modification time).
pub fn find_latest_log(logs_dir: &Path) -> Option<PathBuf> {
    let entries = std::fs::read_dir(logs_dir).ok()?;
    let mut best: Option<(std::time::SystemTime, PathBuf)> = None;
    for entry in entries.flatten() {
        let path = entry.path();
        let is_log = path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.eq_ignore_ascii_case("log"))
            .unwrap_or(false);
        if !is_log {
            continue;
        }
        let modified = entry.metadata().and_then(|m| m.modified()).ok()?;
        let replace = best.as_ref().map(|(t, _)| modified > *t).unwrap_or(true);
        if replace {
            best = Some((modified, path));
        }
    }
    best.map(|(_, path)| path)
}

// ---------------------------------------------------------------------------
// Log line parsing (pure functions, heavily tested)
// ---------------------------------------------------------------------------

/// Strip the `timestamp ...` prefix: everything up to the first space.
/// Roblox log lines look like `2024-01-01T00:00:00.000Z,0.000000,... [FLog::...] ...`.
/// The original watcher split at the first space, so markers below are
/// matched against the remainder of the line.
pub fn strip_log_prefix(line: &str) -> &str {
    match line.find(' ') {
        Some(idx) => &line[idx + 1..],
        None => line,
    }
}

// Marker prefixes (matched after the timestamp prefix is stripped).
pub const MARK_JOINING_GAME: &str = "[FLog::Output] ! Joining game";
pub const MARK_JOINED_SERVER: &str = "[FLog::Network] serverId:";
pub const MARK_DISCONNECTED: &str = "[FLog::Network] Time to disconnect replication data:";
pub const MARK_DISCONNECT_REASON: &str = "[FLog::Network] Sending disconnect with reason:";
pub const MARK_LEAVING_GAME: &str = "[FLog::SingleSurfaceApp] leaveUGCGameInternal";
pub const MARK_TELEPORT: &str = "[FLog::GameJoinUtil] GameJoinUtil::initiateTeleportToPlace";
pub const MARK_PRIVATE_JOIN: &str =
    "[FLog::GameJoinUtil] GameJoinUtil::joinGamePostPrivateServer";
pub const MARK_RESERVED_JOIN: &str =
    "[FLog::GameJoinUtil] GameJoinUtil::initiateTeleportToReservedServer";
pub const MARK_UNIVERSE_REPORT: &str = "[FLog::GameJoinLoadTime] Report game_join_loadtime:";
pub const MARK_UDMUX: &str = "[FLog::Network] UDMUX Address = ";
pub const MARK_RPC_LEGACY: &str = "[FLog::Output] [BloxstrapRPC]";
pub const MARK_RPC: &str = "[FLog::Output] [RedStrapRPC]";
pub const MARK_STUDIO_OPEN: &str = "[FLog::PlaceManager] Start to open place";
pub const MARK_STUDIO_CLOSE: &str = "[FLog::PlaceManager] PlaceManager::closeCurrentPlayDoc";

/// Parsed `! Joining game '{job}' place {place} at {ip}` line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JoinInfo {
    pub job_id: String,
    pub place_id: u64,
    pub machine_ip: String,
}

/// `! Joining game 'uuid' place 123 at 1.2.3.4` -> [`JoinInfo`].
pub fn parse_joining_game(message: &str) -> Option<JoinInfo> {
    let rest = message.strip_prefix(MARK_JOINING_GAME)?.trim();
    // rest := "'<job>' place <place> at <ip> ..."
    let job = rest
        .strip_prefix('\'')?
        .split('\'')
        .next()
        .filter(|s| !s.is_empty())?;
    let after_job = rest.splitn(2, "' place ").nth(1)?;
    let mut words = after_job.split_whitespace();
    let place: u64 = words.next()?.parse().ok()?;
    if words.next() != Some("at") {
        return None;
    }
    let ip = words.next()?.trim_matches(|c| c == ',' || c == '\'');
    if ip.is_empty() {
        return None;
    }
    Some(JoinInfo {
        job_id: job.to_string(),
        place_id: place,
        machine_ip: ip.to_string(),
    })
}

/// `serverId: 1.2.3.4|53640` -> `(ip, port)`.
pub fn parse_server_id(message: &str) -> Option<(String, u16)> {
    let rest = message.strip_prefix(MARK_JOINED_SERVER)?.trim();
    let (ip, port) = rest.split_once('|')?;
    Some((ip.trim().to_string(), port.trim().parse().ok()?))
}

/// `Sending disconnect with reason: 277` -> `277`.
pub fn parse_disconnect_reason(message: &str) -> Option<u32> {
    message
        .strip_prefix(MARK_DISCONNECT_REASON)?
        .trim()
        .split(|c: char| !c.is_ascii_digit())
        .find(|s| !s.is_empty())?
        .parse()
        .ok()
}

/// Disconnect reasons that qualify for automatic rejoin:
/// 1 = inactivity timeout, 277 = connection lost.
pub fn is_rejoinable_reason(reason: u32) -> bool {
    matches!(reason, 1 | 277)
}

/// `Report game_join_loadtime: ... universeid:123, ... userid:456 ...`.
///
/// The report is comma-separated `key:value` pairs; scan them directly
/// instead of using regular expressions.
pub fn parse_universe_report(message: &str) -> Option<(u64, u64)> {
    let rest = message.strip_prefix(MARK_UNIVERSE_REPORT)?;
    let mut universe = None;
    let mut user = None;
    for pair in rest.split(',') {
        let (key, value) = pair.split_once(':')?;
        let key = key.trim().trim_matches('"');
        let value = value.trim().trim_matches('"');
        match key {
            "universeid" => universe = value.parse().ok(),
            "userid" => user = value.parse().ok(),
            _ => {}
        }
    }
    match (universe, user) {
        (Some(u), Some(id)) => Some((u, id)),
        _ => None,
    }
}

/// `UDMUX Address = 1.2.3.4, Port = 53640 | RCC Server Address = 5.6.7.8, Port = 53640`
/// -> `(udmux_ip, rcc_ip)`.
pub fn parse_udmux(message: &str) -> Option<(String, String)> {
    let rest = message.strip_prefix(MARK_UDMUX)?;
    let mut halves = rest.split('|');
    let udmux = halves.next()?.split(',').next()?;
    let rcc = halves.next()?.split("Address = ").nth(1)?.split(',').next()?;
    let udmux = udmux.trim();
    let rcc = rcc.trim();
    if udmux.is_empty() || rcc.is_empty() {
        return None;
    }
    Some((udmux.to_string(), rcc.to_string()))
}

/// Extract the JSON payload of an in-game RPC message, accepting both the
/// legacy `[BloxstrapRPC]` marker (emitted by existing games) and the new
/// `[RedStrapRPC]` marker.
pub fn parse_rpc_message(message: &str) -> Option<&str> {
    message
        .strip_prefix(MARK_RPC_LEGACY)
        .or_else(|| message.strip_prefix(MARK_RPC))
        .map(str::trim)
        .filter(|s| !s.is_empty())
}

/// Extract `accessCode":"<uuid>"` from a private-server join line.
pub fn parse_access_code(message: &str) -> Option<String> {
    let key = "accessCode\":\"";
    let start = message.find(key)? + key.len();
    let end = message[start..].find('"')?;
    let code = &message[start..start + end];
    if code.is_empty() {
        None
    } else {
        Some(code.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn joining_game_parses() {
        let line = "[FLog::Output] ! Joining game '7d3f8a12-4b6c-4d8e-9f0a-1b2c3d4e5f60' place 1818 at 128.116.123.45";
        let info = parse_joining_game(line).expect("parse");
        assert_eq!(info.place_id, 1818);
        assert_eq!(info.machine_ip, "128.116.123.45");
        assert!(info.job_id.starts_with("7d3f8a12"));
        assert!(parse_joining_game("[FLog::Output] something else").is_none());
    }

    #[test]
    fn server_id_parses() {
        assert_eq!(
            parse_server_id("[FLog::Network] serverId: 128.116.1.2|53640"),
            Some((String::from("128.116.1.2"), 53640))
        );
    }

    #[test]
    fn disconnect_reason_parses() {
        assert_eq!(
            parse_disconnect_reason("[FLog::Network] Sending disconnect with reason: 277"),
            Some(277)
        );
        assert!(is_rejoinable_reason(1));
        assert!(is_rejoinable_reason(277));
        assert!(!is_rejoinable_reason(266));
    }

    #[test]
    fn universe_report_parses() {
        let line = "[FLog::GameJoinLoadTime] Report game_join_loadtime: universeid:1818, userid:1234, placeid:1818";
        assert_eq!(parse_universe_report(line), Some((1818, 1234)));
        assert!(parse_universe_report("[FLog::GameJoinLoadTime] Report game_join_loadtime: nope").is_none());
    }

    #[test]
    fn udmux_parses() {
        let line = "[FLog::Network] UDMUX Address = 1.2.3.4, Port = 53640 | RCC Server Address = 5.6.7.8, Port = 53640";
        assert_eq!(
            parse_udmux(line),
            Some((String::from("1.2.3.4"), String::from("5.6.7.8")))
        );
    }

    #[test]
    fn rpc_markers_both_accepted() {
        assert_eq!(
            parse_rpc_message("[FLog::Output] [BloxstrapRPC] {\"a\":1}"),
            Some("{\"a\":1}")
        );
        assert_eq!(
            parse_rpc_message("[FLog::Output] [RedStrapRPC] {\"a\":1}"),
            Some("{\"a\":1}")
        );
        assert!(parse_rpc_message("[FLog::Output] hello").is_none());
    }

    #[test]
    fn access_code_parses() {
        let line = "GameJoinUtil::joinGamePostPrivateServer ... \"accessCode\":\"abc-123\" ...";
        assert_eq!(parse_access_code(line), Some(String::from("abc-123")));
    }

    #[test]
    fn deeplinks_roundtrip() {
        assert_eq!(
            start_deeplink(1818, "job-1", ""),
            "roblox://experiences/start?placeId=1818&gameInstanceId=job-1"
        );
        assert_eq!(
            start_deeplink(1818, "", "code-9"),
            "roblox://experiences/start?placeId=1818&accessCode=code-9"
        );
        assert_eq!(
            parse_gameshortcut("1818;job-1"),
            Some(String::from(
                "roblox://experiences/start?placeId=1818&gameInstanceId=job-1"
            ))
        );
        assert!(parse_gameshortcut("abc").is_none());
        assert!(parse_gameshortcut("0").is_none());
    }

    #[test]
    fn protocol_args_classify() {
        assert_eq!(
            classify_protocol_arg("roblox-player:1+launchmode:play"),
            Some(LaunchMode::Player)
        );
        assert_eq!(
            classify_protocol_arg("ROBLOX-STUDIO:1+task:EditPlace"),
            Some(LaunchMode::Studio)
        );
        assert_eq!(
            classify_protocol_arg("roblox-studio-auth:1+blah"),
            Some(LaunchMode::StudioAuth)
        );
        assert_eq!(classify_protocol_arg("--settings"), None);
    }

    #[test]
    fn launch_modes_map() {
        assert_eq!(LaunchMode::Player.exe_name(), "RobloxPlayerBeta.exe");
        assert_eq!(LaunchMode::Studio.binary_type(), "WindowsStudio64");
        assert!(LaunchMode::StudioAuth.is_studio());
        assert!(!LaunchMode::Player.is_studio());
    }
}
