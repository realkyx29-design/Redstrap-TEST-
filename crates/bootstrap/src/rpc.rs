//! Discord Rich Presence over the local IPC socket.
//!
//! Speaks the Discord IPC framing directly (named pipe on Windows, Unix
//! socket elsewhere): handshake, `SET_ACTIVITY`, reconnect-on-failure. No
//! third-party RPC crate needed — the protocol is three opcodes.

use redstrap_core::error::{Error, Result};
#[cfg(any(windows, unix))]
use redstrap_core::consts::{DISCORD_IPC_BASENAME, DISCORD_IPC_PROBE_COUNT};
#[cfg(any(windows, unix))]
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const OP_HANDSHAKE: u32 = 0;
const OP_FRAME: u32 = 1;

/// A presence update. `None` fields are omitted from the payload.
#[derive(Debug, Clone, Default)]
pub struct Presence {
    pub details: Option<String>,
    pub state: Option<String>,
    /// Unix seconds (milliseconds are accepted and converted).
    pub start: Option<i64>,
    pub end: Option<i64>,
    pub large_image: Option<String>,
    pub large_text: Option<String>,
    pub small_image: Option<String>,
    pub small_text: Option<String>,
    /// `(label, url)` pairs; Discord shows at most two.
    pub buttons: Vec<(String, String)>,
}

#[cfg(windows)]
type IpcStream = tokio::net::windows::named_pipe::NamedPipeClient;
#[cfg(all(unix, not(windows)))]
type IpcStream = tokio::net::UnixStream;

/// Connected Discord IPC client. Dropping closes the socket.
pub struct IpcClient {
    #[cfg(any(windows, unix))]
    stream: IpcStream,
    client_id: String,
    pid: u32,
}

impl IpcClient {
    /// Probe `discord-ipc-0..10` and handshake. Fails when Discord isn't
    /// running or rejects the application ID.
    pub async fn connect(client_id: &str, pid: u32) -> Result<Self> {
        if client_id.trim().is_empty() {
            return Err(Error::Rpc(String::from("no Discord application ID configured")));
        }
        #[cfg(not(any(windows, unix)))]
        {
            return Err(Error::Rpc(String::from(
                "Discord IPC is unavailable on this platform",
            )));
        }
        #[cfg(any(windows, unix))]
        {
            let mut last_error = String::from("no Discord IPC socket responded");
            for index in 0..DISCORD_IPC_PROBE_COUNT {
                match open_socket(index).await {
                    Ok(stream) => {
                        let mut client = Self {
                            stream,
                            client_id: client_id.trim().to_string(),
                            pid,
                        };
                        match client.handshake().await {
                            Ok(()) => return Ok(client),
                            Err(e) => {
                                last_error = e.to_string();
                            }
                        }
                        break;
                    }
                    Err(e) => {
                        last_error = e;
                    }
                }
            }
            Err(Error::Rpc(format!("could not reach Discord: {last_error}")))
        }
    }

    pub fn client_id(&self) -> &str {
        &self.client_id
    }

    async fn handshake(&mut self) -> Result<()> {
        let payload = serde_json::json!({"v": 1, "client_id": self.client_id});
        self.send_frame(OP_HANDSHAKE, &payload.to_string()).await?;
        let (_op, _body) = self.read_frame().await?;
        Ok(())
    }

    /// Publish (or replace) the current activity.
    pub async fn set_activity(&mut self, presence: &Presence) -> Result<()> {
        let mut activity = serde_json::Map::new();
        if let Some(v) = &presence.details {
            activity.insert(String::from("details"), json_str(v));
        }
        if let Some(v) = &presence.state {
            activity.insert(String::from("state"), json_str(v));
        }
        let mut timestamps = serde_json::Map::new();
        if let Some(v) = presence.start {
            timestamps.insert(String::from("start"), serde_json::Value::from(normalize_time(v)));
        }
        if let Some(v) = presence.end {
            timestamps.insert(String::from("end"), serde_json::Value::from(normalize_time(v)));
        }
        if !timestamps.is_empty() {
            activity.insert(
                String::from("timestamps"),
                serde_json::Value::Object(timestamps),
            );
        }
        let mut assets = serde_json::Map::new();
        if let Some(v) = &presence.large_image {
            assets.insert(String::from("large_image"), json_str(v));
        }
        if let Some(v) = &presence.large_text {
            assets.insert(String::from("large_text"), json_str(v));
        }
        if let Some(v) = &presence.small_image {
            assets.insert(String::from("small_image"), json_str(v));
        }
        if let Some(v) = &presence.small_text {
            assets.insert(String::from("small_text"), json_str(v));
        }
        if !assets.is_empty() {
            activity.insert(String::from("assets"), serde_json::Value::Object(assets));
        }
        if !presence.buttons.is_empty() {
            let buttons: Vec<serde_json::Value> = presence
                .buttons
                .iter()
                .take(2)
                .map(|(label, url)| {
                    serde_json::json!({"label": truncate(label, 32), "url": url})
                })
                .collect();
            activity.insert(String::from("buttons"), serde_json::Value::Array(buttons));
        }

        let payload = serde_json::json!({
            "cmd": "SET_ACTIVITY",
            "args": {"pid": self.pid, "activity": activity},
            "nonce": nonce(),
        });
        self.send_frame(OP_FRAME, &payload.to_string()).await?;
        // The reply confirms receipt; content is intentionally ignored.
        let _ = self.read_frame().await?;
        Ok(())
    }

    /// Clear the activity (removes the presence while staying connected).
    pub async fn clear_activity(&mut self) -> Result<()> {
        let payload = serde_json::json!({
            "cmd": "SET_ACTIVITY",
            "args": {"pid": self.pid, "activity": serde_json::Value::Null},
            "nonce": nonce(),
        });
        self.send_frame(OP_FRAME, &payload.to_string()).await?;
        let _ = self.read_frame().await?;
        Ok(())
    }

    #[cfg(any(windows, unix))]
    async fn send_frame(&mut self, opcode: u32, body: &str) -> Result<()> {
        let bytes = body.as_bytes();
        let mut frame = Vec::with_capacity(8 + bytes.len());
        frame.extend_from_slice(&opcode.to_le_bytes());
        frame.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
        frame.extend_from_slice(bytes);
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            self.stream.write_all(&frame).await?;
            self.stream.flush().await
        })
        .await
        .map_err(|_| Error::Rpc(String::from("Discord IPC write timed out")))?
        .map_err(|e| Error::Rpc(format!("Discord IPC write failed: {e}")))?;
        Ok(())
    }

    #[cfg(any(windows, unix))]
    async fn read_frame(&mut self) -> Result<(u32, String)> {
        let mut header = [0u8; 8];
        tokio::time::timeout(std::time::Duration::from_secs(5), self.stream.read_exact(&mut header))
            .await
            .map_err(|_| Error::Rpc(String::from("Discord IPC read timed out")))?
            .map_err(|e| Error::Rpc(format!("Discord IPC read failed: {e}")))?;
        let opcode = u32::from_le_bytes([header[0], header[1], header[2], header[3]]);
        let len = u32::from_le_bytes([header[4], header[5], header[6], header[7]]) as usize;
        if len > 1024 * 1024 {
            return Err(Error::Rpc(String::from("Discord IPC frame too large")));
        }
        let mut body = vec![0u8; len];
        if len > 0 {
            tokio::time::timeout(
                std::time::Duration::from_secs(5),
                self.stream.read_exact(&mut body),
            )
            .await
            .map_err(|_| Error::Rpc(String::from("Discord IPC read timed out")))?
            .map_err(|e| Error::Rpc(format!("Discord IPC read failed: {e}")))?;
        }
        Ok((opcode, String::from_utf8_lossy(&body).into_owned()))
    }
}

#[cfg(not(any(windows, unix)))]
impl IpcClient {
    async fn send_frame(&mut self, _opcode: u32, _body: &str) -> Result<()> {
        Err(Error::Rpc(String::from(
            "Discord IPC is unavailable on this platform",
        )))
    }

    async fn read_frame(&mut self) -> Result<(u32, String)> {
        Err(Error::Rpc(String::from(
            "Discord IPC is unavailable on this platform",
        )))
    }
}

fn json_str(value: &str) -> serde_json::Value {
    serde_json::Value::String(truncate(value, 128))
}

fn truncate(value: &str, max_chars: usize) -> String {
    if value.chars().count() <= max_chars {
        value.to_string()
    } else {
        value.chars().take(max_chars).collect()
    }
}

fn nonce() -> String {
    redstrap_core::util::unix_seconds().to_string()
}

/// Accept seconds or milliseconds since epoch; Discord wants seconds.
fn normalize_time(value: i64) -> i64 {
    if value > 1_000_000_000_000 {
        value / 1000
    } else {
        value
    }
}

#[cfg(windows)]
async fn open_socket(index: u8) -> std::result::Result<IpcStream, String> {
    use tokio::net::windows::named_pipe::ClientOptions;
    let path = format!("\\\\.\\pipe\\{DISCORD_IPC_BASENAME}{index}");
    ClientOptions::new()
        .open(&path)
        .map_err(|e| format!("{path}: {e}"))
}

#[cfg(all(unix, not(windows)))]
async fn open_socket(index: u8) -> std::result::Result<IpcStream, String> {
    let dir = std::env::var_os("XDG_RUNTIME_DIR")
        .map(std::path::PathBuf::from)
        .filter(|d| d.is_dir())
        .unwrap_or_else(|| std::path::PathBuf::from("/tmp"));
    let path = dir.join(format!("{DISCORD_IPC_BASENAME}{index}"));
    tokio::time::timeout(
        std::time::Duration::from_secs(2),
        tokio::net::UnixStream::connect(&path),
    )
    .await
    .map_err(|_| format!("{}: timed out", path.display()))?
    .map_err(|e| format!("{}: {e}", path.display()))
}
