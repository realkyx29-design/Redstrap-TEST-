//! Session watcher: tails the Roblox client log at 1 Hz and reacts.
//!
//! One `--watcher` child is detached per launch. It tracks joins, disconnects
//! and teleports, publishes Discord Rich Presence (client + in-game RPC),
//! auto-rejoins dropped servers, records playtime, and writes the
//! `ServerDetails.json` snapshot the desktop app displays.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use redstrap_core::consts::{AUTO_REJOIN_DELAY_SECS, MAX_AUTO_REJOINS};
use redstrap_core::error::Result;
use redstrap_core::paths::Layout;
use redstrap_core::roblox::{self, LaunchMode};
use redstrap_core::settings::Settings;
use serde::{Deserialize, Serialize};

use crate::rpc::{IpcClient, Presence};

/// Attach to `pid` until it exits.
pub async fn run(
    client: &reqwest::Client,
    layout: &Layout,
    settings: &Settings,
    state_path: &Path,
    pid: u32,
    mode: LaunchMode,
) -> Result<()> {
    // Only one watcher per process; stray duplicates exit silently.
    let _lock = match redstrap_core::instance::InstanceLock::try_acquire(&format!("watcher-{pid}"))? {
        Some(guard) => guard,
        None => {
            tracing::info!("a watcher for PID {pid} is already running");
            return Ok(());
        }
    };

    if pid == 0 || !redstrap_core::process::process_exists(pid) {
        tracing::warn!("watcher started for dead PID {pid}; exiting");
        return Ok(());
    }

    tracing::info!("watching {} (PID {pid})", mode.label());
    let mut session = Session::new(pid, mode);
    let started = Instant::now();

    // Wait for the client log to appear (fresh launches need a moment).
    let mut log_path: Option<PathBuf> = None;
    for _ in 0..90 {
        if !redstrap_core::process::process_exists(pid) {
            break;
        }
        if let Some(found) = roblox::find_latest_log(&layout.roblox_logs) {
            tracing::info!("tailing {}", found.display());
            log_path = Some(found);
            break;
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }

    let mut tail = Tail::default();
    let mut ticker = tokio::time::interval(Duration::from_secs(1));
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    // Studio idles without game markers; publish its presence once upfront.
    if mode.is_studio() {
        session.presence_dirty = true;
    }

    loop {
        ticker.tick().await;

        if !redstrap_core::process::process_exists(pid) {
            tracing::info!("PID {pid} exited");
            break;
        }

        // (Re)discover the log if it vanished (rotation, cleaners).
        if log_path.as_ref().map(|p| !p.is_file()).unwrap_or(true) {
            if let Some(found) = roblox::find_latest_log(&layout.roblox_logs) {
                if log_path.as_ref() != Some(&found) {
                    tracing::info!("tailing {}", found.display());
                    tail = Tail::default();
                    log_path = Some(found);
                }
            }
        }

        if let Some(path) = &log_path {
            let lines = tail.poll(path);
            for line in lines {
                handle_line(client, layout, settings, &mut session, &line).await;
            }
        }

        maybe_rejoin(settings, &mut session).await;
        maybe_push_presence(settings, &mut session).await;
    }

    finalize(layout, settings, state_path, &mut session, started).await;
    Ok(())
}

// ---------------------------------------------------------------------------
// Log tailing
// ---------------------------------------------------------------------------

#[derive(Default)]
struct Tail {
    offset: u64,
    remainder: String,
}

impl Tail {
    /// Read newly appended lines (1 MB cap per tick against bursts).
    fn poll(&mut self, path: &Path) -> Vec<String> {
        const MAX_READ: u64 = 1024 * 1024;
        const MAX_LINE: usize = 64 * 1024;

        let len = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
        if len < self.offset {
            self.offset = 0; // truncated or rotated
            self.remainder.clear();
        }
        if len == self.offset {
            return Vec::new();
        }

        let mut file = match std::fs::File::open(path) {
            Ok(f) => f,
            Err(_) => return Vec::new(),
        };
        use std::io::{Read, Seek};
        if file.seek(std::io::SeekFrom::Start(self.offset)).is_err() {
            return Vec::new();
        }
        let mut buf = vec![0u8; (len - self.offset).min(MAX_READ) as usize];
        let mut read = 0;
        while read < buf.len() {
            match file.read(&mut buf[read..]) {
                Ok(0) => break,
                Ok(n) => read += n,
                Err(_) => break,
            }
        }
        self.offset += read as u64;

        let text = String::from_utf8_lossy(&buf[..read]);
        let mut combined = std::mem::take(&mut self.remainder);
        combined.push_str(&text);

        let mut lines = Vec::new();
        let mut start = 0;
        for (i, b) in combined.bytes().enumerate() {
            if b == b'\n' {
                let mut line = combined[start..i].trim_end_matches('\r').to_string();
                if line.len() > MAX_LINE {
                    line.truncate(MAX_LINE);
                }
                lines.push(line);
                start = i + 1;
            }
        }
        self.remainder = combined[start..].to_string();
        if self.remainder.len() > MAX_LINE {
            // A runaway line without newline: drop it rather than growing.
            self.remainder.clear();
        }
        lines
    }
}

// ---------------------------------------------------------------------------
// Session state
// ---------------------------------------------------------------------------

struct Session {
    pid: u32,
    mode: LaunchMode,
    place_id: u64,
    job_id: String,
    access_code: String,
    server_ip: String,
    server_port: u16,
    machine_address: String,
    universe_id: u64,
    user_id: u64,
    user_name: String,
    user_name_resolved: bool,
    game_name: String,
    game_icon_url: String,
    connected: bool,
    join_seq: u64,
    last_disconnect_seq: u64,
    rejoin_attempts: u32,
    rejoin_at: Option<Instant>,
    launch_data: String,
    studio_open: bool,
    presence_dirty: bool,
    last_presence_push: Option<Instant>,
    next_rpc_retry: Option<Instant>,
    rpc: Option<IpcClient>,
    game_rpc: GameRpcState,
    game_cache: HashMap<u64, (String, String)>,
}

impl Session {
    fn new(pid: u32, mode: LaunchMode) -> Self {
        Self {
            pid,
            mode,
            place_id: 0,
            job_id: String::new(),
            access_code: String::new(),
            server_ip: String::new(),
            server_port: 0,
            machine_address: String::new(),
            universe_id: 0,
            user_id: 0,
            user_name: String::new(),
            user_name_resolved: false,
            game_name: String::new(),
            game_icon_url: String::new(),
            connected: false,
            join_seq: 0,
            last_disconnect_seq: 0,
            rejoin_attempts: 0,
            rejoin_at: None,
            launch_data: String::new(),
            studio_open: false,
            presence_dirty: false,
            last_presence_push: None,
            next_rpc_retry: None,
            rpc: None,
            game_rpc: GameRpcState::default(),
            game_cache: HashMap::new(),
        }
    }

    fn deeplink(&self) -> Option<String> {
        if self.place_id == 0 {
            return None;
        }
        let mut link = roblox::start_deeplink(self.place_id, &self.job_id, &self.access_code);
        if !self.launch_data.is_empty() {
            link.push_str("&launchData=");
            link.push_str(&self.launch_data);
        }
        Some(link)
    }
}

#[derive(Debug, Default)]
struct GameRpcState {
    details: Option<String>,
    state: Option<String>,
    time_start: Option<i64>,
    time_end: Option<i64>,
    large_image: Option<String>,
    large_text: Option<String>,
    small_image: Option<String>,
    small_text: Option<String>,
}

// ---------------------------------------------------------------------------
// Line handling
// ---------------------------------------------------------------------------

async fn handle_line(
    client: &reqwest::Client,
    layout: &Layout,
    settings: &Settings,
    session: &mut Session,
    line: &str,
) {
    let message = roblox::strip_log_prefix(line);

    if session.mode.is_studio() {
        handle_studio_line(session, message);
        return;
    }

    if message.starts_with(roblox::MARK_JOINING_GAME) {
        if let Some(join) = roblox::parse_joining_game(message) {
            tracing::info!("joining place {} job {}", join.place_id, join.job_id);
            session.place_id = join.place_id;
            session.job_id = join.job_id;
            session.machine_address = join.machine_ip;
            session.access_code.clear();
            session.connected = false;
            session.join_seq += 1;
            session.rejoin_attempts = 0;
            session.rejoin_at = None;
            session.presence_dirty = true;
            write_server_details(layout, session);
        }
        return;
    }

    if message.starts_with(roblox::MARK_JOINED_SERVER) {
        if let Some((ip, port)) = roblox::parse_server_id(message) {
            session.server_ip = ip;
            session.server_port = port;
            session.connected = true;
            session.presence_dirty = true;
            write_server_details(layout, session);
        }
        return;
    }

    if message.starts_with(roblox::MARK_DISCONNECTED) {
        session.connected = false;
        session.last_disconnect_seq = session.join_seq;
        session.presence_dirty = true;
        write_server_details(layout, session);
        return;
    }

    if message.starts_with(roblox::MARK_DISCONNECT_REASON) {
        if let Some(reason) = roblox::parse_disconnect_reason(message) {
            tracing::info!("disconnect reason {reason}");
            session.connected = false;
            session.last_disconnect_seq = session.join_seq;
            if settings.auto_rejoin && roblox::is_rejoinable_reason(reason) {
                schedule_rejoin(session);
            }
            session.presence_dirty = true;
            write_server_details(layout, session);
        }
        return;
    }

    if message.starts_with(roblox::MARK_LEAVING_GAME) {
        tracing::info!("leaving game");
        session.connected = false;
        session.presence_dirty = true;
        if settings.close_on_leave_game {
            tracing::info!("close-on-leave enabled; terminating PID {}", session.pid);
            let _ = redstrap_core::process::kill_pid(session.pid, true);
        }
        write_server_details(layout, session);
        return;
    }

    if message.starts_with(roblox::MARK_TELEPORT) || message.starts_with(roblox::MARK_RESERVED_JOIN) {
        // A teleport starts a fresh join flow; the Joining-game line follows.
        session.connected = false;
        session.presence_dirty = true;
        return;
    }

    if message.starts_with(roblox::MARK_PRIVATE_JOIN) {
        if let Some(code) = roblox::parse_access_code(message) {
            session.access_code = code;
        }
        return;
    }

    if message.starts_with(roblox::MARK_UNIVERSE_REPORT) {
        if let Some((universe, user)) = roblox::parse_universe_report(message) {
            session.universe_id = universe;
            session.user_id = user;
            session.presence_dirty = true;
            if settings.activity_tracking {
                resolve_game_info(client, settings, session).await;
            }
            if settings.discord_rpc && settings.rpc_show_account {
                resolve_user_name(client, settings, session).await;
            }
            write_server_details(layout, session);
        }
        return;
    }

    if message.starts_with(roblox::MARK_UDMUX) {
        if let Some((udmux, _rcc)) = roblox::parse_udmux(message) {
            if session.server_ip.is_empty() {
                session.server_ip = udmux;
            }
        }
        return;
    }

    if let Some(payload) = roblox::parse_rpc_message(message) {
        apply_game_rpc(session, payload);
    }
}

fn handle_studio_line(session: &mut Session, message: &str) {
    if message.starts_with(roblox::MARK_STUDIO_OPEN) {
        if !session.studio_open {
            session.studio_open = true;
            session.presence_dirty = true;
        }
    } else if message.starts_with(roblox::MARK_STUDIO_CLOSE) {
        if session.studio_open {
            session.studio_open = false;
            session.presence_dirty = true;
        }
    }
}

// ---------------------------------------------------------------------------
// Game info (public Roblox APIs)
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct GameListResponse {
    #[serde(default)]
    data: Vec<GameEntry>,
}

#[derive(Debug, Deserialize)]
struct GameEntry {
    #[serde(default)]
    name: String,
}

#[derive(Debug, Deserialize)]
struct ThumbResponse {
    #[serde(default)]
    data: Vec<ThumbEntry>,
}

#[derive(Debug, Deserialize)]
struct ThumbEntry {
    #[serde(rename = "imageUrl", default)]
    image_url: String,
}

async fn resolve_game_info(client: &reqwest::Client, settings: &Settings, session: &mut Session) {
    let universe = session.universe_id;
    if universe == 0 {
        return;
    }
    if let Some((name, icon)) = session.game_cache.get(&universe) {
        session.game_name = name.clone();
        session.game_icon_url = icon.clone();
        session.presence_dirty = true;
        return;
    }

    let domain = settings.roblox_domain.trim();
    let domain = if domain.is_empty() { "roblox.com" } else { domain };

    let mut name = String::new();
    let mut icon = String::new();
    let games_url = format!("https://games.{domain}/v1/games?universeIds={universe}");
    match redstrap_core::http::get_json::<GameListResponse>(client, &games_url).await {
        Ok(list) => {
            if let Some(entry) = list.data.into_iter().next() {
                name = entry.name;
            }
        }
        Err(e) => tracing::debug!("game info lookup failed: {e}"),
    }
    let thumbs_url = format!(
        "https://thumbnails.{domain}/v1/games/icons?universeIds={universe}&size=512x512&format=Png&isCircular=false"
    );
    match redstrap_core::http::get_json::<ThumbResponse>(client, &thumbs_url).await {
        Ok(list) => {
            if let Some(entry) = list.data.into_iter().next() {
                icon = entry.image_url;
            }
        }
        Err(e) => tracing::debug!("game thumbnail lookup failed: {e}"),
    }

    if !name.is_empty() || !icon.is_empty() {
        tracing::info!("now playing '{name}' (universe {universe})");
    }
    session.game_cache.insert(universe, (name.clone(), icon.clone()));
    session.game_name = name;
    session.game_icon_url = icon;
    session.presence_dirty = true;
}

#[derive(Debug, Deserialize)]
struct UserResponse {
    #[serde(default)]
    name: String,
}

/// Resolve the player's username once per session through the public users
/// API (no login required). Backs the "show account" presence option.
async fn resolve_user_name(client: &reqwest::Client, settings: &Settings, session: &mut Session) {
    if session.user_name_resolved || session.user_id == 0 {
        return;
    }
    session.user_name_resolved = true;
    let domain = settings.roblox_domain.trim();
    let domain = if domain.is_empty() { "roblox.com" } else { domain };
    let url = format!("https://users.{domain}/v1/users/{}", session.user_id);
    match redstrap_core::http::get_json::<UserResponse>(client, &url).await {
        Ok(user) if !user.name.trim().is_empty() => {
            session.user_name = user.name;
            session.presence_dirty = true;
        }
        Ok(_) => {}
        Err(e) => tracing::debug!("username lookup failed: {e}"),
    }
}

// ---------------------------------------------------------------------------
// In-game RPC ([RedStrapRPC] / legacy [BloxstrapRPC])
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct RpcCommand {
    #[serde(default)]
    command: String,
    #[serde(default)]
    details: Option<String>,
    #[serde(default)]
    state: Option<String>,
    #[serde(rename = "timeStart", default)]
    time_start: Option<i64>,
    #[serde(rename = "timeEnd", default)]
    time_end: Option<i64>,
    #[serde(rename = "largeImage", default)]
    large_image: Option<ImageSpec>,
    #[serde(rename = "smallImage", default)]
    small_image: Option<ImageSpec>,
    #[serde(rename = "launchData", default)]
    launch_data: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ImageSpec {
    #[serde(rename = "assetId", default)]
    asset_id: Option<String>,
    #[serde(rename = "hoverText", default)]
    hover_text: Option<String>,
    #[serde(default)]
    clear: bool,
    #[serde(default)]
    reset: bool,
}

fn apply_game_rpc(session: &mut Session, payload: &str) {
    let command: RpcCommand = match serde_json::from_str(payload) {
        Ok(c) => c,
        Err(e) => {
            tracing::debug!("ignoring malformed game RPC: {e}");
            return;
        }
    };
    match command.command.as_str() {
        "SetRichPresence" => {
            let rpc = &mut session.game_rpc;
            if let Some(v) = command.details {
                rpc.details = Some(v);
            }
            if let Some(v) = command.state {
                rpc.state = Some(v);
            }
            if let Some(v) = command.time_start {
                rpc.time_start = Some(v);
            }
            if let Some(v) = command.time_end {
                rpc.time_end = Some(v);
            }
            apply_image(&mut rpc.large_image, &mut rpc.large_text, command.large_image);
            apply_image(&mut rpc.small_image, &mut rpc.small_text, command.small_image);
            session.presence_dirty = true;
        }
        "SetLaunchData" => {
            if let Some(data) = command.launch_data {
                session.launch_data = data;
            }
        }
        other => {
            tracing::debug!("ignoring unknown game RPC command '{other}'");
        }
    }
}

fn apply_image(image: &mut Option<String>, text: &mut Option<String>, spec: Option<ImageSpec>) {
    let Some(spec) = spec else {
        return;
    };
    if spec.clear || spec.reset {
        *image = None;
        *text = None;
        return;
    }
    if let Some(id) = spec.asset_id {
        if !id.trim().is_empty() {
            *image = Some(id);
        }
    }
    if let Some(hover) = spec.hover_text {
        *text = Some(hover);
    }
}

// ---------------------------------------------------------------------------
// Auto-rejoin
// ---------------------------------------------------------------------------

fn schedule_rejoin(session: &mut Session) {
    if session.place_id == 0 || session.deeplink().is_none() {
        return;
    }
    if session.rejoin_attempts >= MAX_AUTO_REJOINS {
        tracing::warn!("auto-rejoin limit ({MAX_AUTO_REJOINS}) reached; giving up");
        return;
    }
    session.rejoin_at = Some(Instant::now() + Duration::from_secs(AUTO_REJOIN_DELAY_SECS));
}

async fn maybe_rejoin(settings: &Settings, session: &mut Session) {
    let Some(at) = session.rejoin_at else {
        return;
    };
    if Instant::now() < at {
        return;
    }
    session.rejoin_at = None;

    // Abort when a new join already started or the client went away.
    if session.connected
        || session.join_seq != session.last_disconnect_seq
        || !redstrap_core::process::process_exists(session.pid)
    {
        return;
    }
    let Some(link) = session.deeplink() else {
        return;
    };
    if !settings.auto_rejoin {
        return;
    }
    session.rejoin_attempts += 1;
    tracing::info!(
        "auto-rejoining {} (attempt {}/{MAX_AUTO_REJOINS})",
        link,
        session.rejoin_attempts
    );
    let exe = match std::env::current_exe() {
        Ok(e) => e,
        Err(e) => {
            tracing::warn!("could not locate launcher for rejoin: {e}");
            return;
        }
    };
    let mut command = std::process::Command::new(exe);
    command.arg(link).arg("--quiet");
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    if let Err(e) = command.spawn() {
        tracing::warn!("auto-rejoin spawn failed: {e}");
    }
}

// ---------------------------------------------------------------------------
// Presence
// ---------------------------------------------------------------------------

async fn maybe_push_presence(settings: &Settings, session: &mut Session) {
    if !settings.discord_rpc || settings.discord_client_id.trim().is_empty() {
        return;
    }
    if !session.presence_dirty {
        return;
    }
    // Throttle: Discord rate-limits rapid updates.
    if let Some(last) = session.last_presence_push {
        if last.elapsed() < Duration::from_secs(2) {
            return;
        }
    }
    if let Some(retry) = session.next_rpc_retry {
        if Instant::now() < retry {
            return;
        }
    }

    if session.rpc.is_none() {
        match IpcClient::connect(settings.discord_client_id.trim(), session.pid).await {
            Ok(rpc) => {
                tracing::info!("connected to Discord IPC");
                session.rpc = Some(rpc);
            }
            Err(e) => {
                tracing::debug!("Discord IPC unavailable: {e}");
                session.next_rpc_retry = Some(Instant::now() + Duration::from_secs(30));
                return;
            }
        }
    }

    let presence = build_presence(settings, session);
    let ok = match session.rpc.as_mut() {
        Some(rpc) => rpc.set_activity(&presence).await.is_ok(),
        None => false,
    };
    if ok {
        session.presence_dirty = false;
        session.last_presence_push = Some(Instant::now());
    } else {
        tracing::debug!("Discord IPC write failed; will reconnect");
        session.rpc = None;
        session.next_rpc_retry = Some(Instant::now() + Duration::from_secs(15));
    }
}

fn build_presence(settings: &Settings, session: &Session) -> Presence {
    let game = &session.game_rpc;
    let mut presence = Presence::default();

    if session.mode.is_studio() {
        presence.details = Some(String::from("Roblox Studio"));
        presence.state = Some(if session.studio_open {
            String::from("Editing a place")
        } else {
            String::from("Idling")
        });
        presence.start = Some(redstrap_core::util::unix_seconds() as i64);
        return presence;
    }

    // Details: game RPC wins, then the resolved game name.
    let mut details = game.details.clone().filter(|s| !s.trim().is_empty());
    if details.is_none() && settings.rpc_show_game_name && !session.game_name.is_empty() {
        details = Some(session.game_name.clone());
    }
    presence.details = details.or_else(|| Some(String::from("Playing Roblox")));

    // State: game RPC wins; otherwise the account name (when enabled) or
    // the connection status.
    let mut state = game.state.clone().filter(|s| !s.trim().is_empty());
    if state.is_none() {
        if settings.rpc_show_account && !session.user_name.is_empty() {
            state = Some(format!("as {}", session.user_name));
        } else {
            state = Some(if session.connected {
                String::from("In a server")
            } else {
                String::from("In the menu")
            });
        }
    }
    presence.state = state;

    presence.start = game.time_start.or_else(|| {
        Some(redstrap_core::util::unix_seconds() as i64)
    });
    presence.end = game.time_end;

    presence.large_image = game.large_image.clone();
    presence.large_text = game.large_text.clone();
    presence.small_image = game.small_image.clone();
    presence.small_text = game.small_text.clone();

    // Buttons must be https URLs: link the game page (joins happen there).
    if !settings.rpc_hide_buttons && session.place_id > 0 {
        let url = format!(
            "https://www.{}/games/{}/x",
            settings.roblox_domain.trim(),
            session.place_id
        );
        presence.buttons.push((String::from("View Game"), url));
    }

    presence
}

// ---------------------------------------------------------------------------
// Server details snapshot
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ServerDetails {
    active: bool,
    mode: String,
    place_id: u64,
    job_id: String,
    server_ip: String,
    server_port: u16,
    machine_address: String,
    universe_id: u64,
    game_name: String,
    game_icon_url: String,
    connected: bool,
}

fn write_server_details(layout: &Layout, session: &Session) {
    if session.mode.is_studio() {
        return;
    }
    let details = ServerDetails {
        active: true,
        mode: session.mode.as_str().to_string(),
        place_id: session.place_id,
        job_id: session.job_id.clone(),
        server_ip: session.server_ip.clone(),
        server_port: session.server_port,
        machine_address: session.machine_address.clone(),
        universe_id: session.universe_id,
        game_name: session.game_name.clone(),
        game_icon_url: session.game_icon_url.clone(),
        connected: session.connected,
    };
    let path = layout.cache.join("ServerDetails.json");
    if let Err(e) = redstrap_core::util::save_json(&path, &details) {
        tracing::debug!("could not write server details: {e}");
    }
}

// ---------------------------------------------------------------------------
// Shutdown
// ---------------------------------------------------------------------------

async fn finalize(
    layout: &Layout,
    settings: &Settings,
    state_path: &Path,
    session: &mut Session,
    started: Instant,
) {
    // Mark the snapshot inactive.
    let path = layout.cache.join("ServerDetails.json");
    let inactive = serde_json::json!({"active": false});
    let _ = redstrap_core::util::save_json(&path, &inactive);

    // Clear Discord presence on a best-effort basis.
    if let Some(rpc) = session.rpc.as_mut() {
        let _ = rpc.clear_activity().await;
    }

    // Playtime accounting (player sessions with tracking enabled).
    if settings.playtime_counter && !session.mode.is_studio() {
        let secs = started.elapsed().as_secs();
        if secs > 0 {
            let (mut state, _) = redstrap_core::state::State::load(state_path);
            state.playtime_total_secs = state.playtime_total_secs.saturating_add(secs);
            if let Err(e) = state.save(state_path) {
                tracing::warn!("could not save playtime: {e}");
            } else {
                tracing::info!("session lasted {secs}s");
            }
        }
    }

    if settings.cleaner == redstrap_core::settings::CleanerMode::OnClose {
        crate::launch::run_cleaner(layout, settings, true);
    }
}
