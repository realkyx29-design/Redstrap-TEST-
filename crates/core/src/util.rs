//! Small, dependency-light helpers shared across the workspace.
//!
//! Everything here is deliberately synchronous and allocation-conscious:
//! hot paths (hashing, progress formatting) avoid needless copies.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::consts::HASH_CHUNK_SIZE;
use crate::error::{Error, Result};

// ---------------------------------------------------------------------------
// Hashing
// ---------------------------------------------------------------------------

/// Lowercase hex MD5 of an in-memory buffer (matches Roblox manifest hashes).
pub fn md5_hex_bytes(data: &[u8]) -> String {
    use md5::Digest;
    let mut hasher = md5::Md5::new();
    hasher.update(data);
    format!("{:x}", hasher.finalize())
}

/// Lowercase hex MD5 of a file, streamed in 64 KiB chunks so multi-hundred
/// megabyte packages never sit fully in memory.
pub fn md5_hex_file(path: &Path) -> Result<String> {
    use md5::Digest;
    use std::io::Read;

    let file = fs::File::open(path).map_err(|e| Error::with_path(&path.to_path_buf(), e))?;
    let mut reader = std::io::BufReader::with_capacity(HASH_CHUNK_SIZE, file);
    let mut hasher = md5::Md5::new();
    let mut buf = vec![0u8; HASH_CHUNK_SIZE];

    loop {
        let n = reader
            .read(&mut buf)
            .map_err(|e| Error::with_path(&path.to_path_buf(), e))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }

    Ok(format!("{:x}", hasher.finalize()))
}

// ---------------------------------------------------------------------------
// Atomic JSON persistence
// ---------------------------------------------------------------------------

/// Write bytes to `path` atomically: the data is fsync'd to a temporary
/// sibling file first, then renamed over the destination, so a crash or
/// power loss can never leave a half-written config behind.
pub fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent).map_err(|e| Error::with_path(&parent.to_path_buf(), e))?;
        }
    }

    let tmp = tmp_sibling(path);
    {
        let mut file =
            fs::File::create(&tmp).map_err(|e| Error::with_path(&tmp.clone(), e))?;
        file.write_all(bytes)
            .map_err(|e| Error::with_path(&tmp.clone(), e))?;
        // Best effort: durability matters for configs, but a failed sync
        // must not fail the whole save on exotic file systems.
        let _ = file.sync_all();
    }

    fs::rename(&tmp, path).map_err(|e| Error::with_path(&path.to_path_buf(), e))?;
    Ok(())
}

/// Serialize `value` as pretty JSON and store it atomically.
pub fn save_json<T: Serialize + ?Sized>(path: &Path, value: &T) -> Result<()> {
    let text = serde_json::to_string_pretty(value)?;
    atomic_write(path, text.as_bytes())
}

/// Load JSON from `path`, returning `T::default()` when the file is missing.
///
/// When the file exists but fails to parse, it is moved aside to
/// `<name>.corrupt-<unix-seconds>` and the default is returned together
/// with `corrupted = true`, so callers can inform the user instead of
/// crashing or silently discarding data.
pub fn load_json_or_default<T: DeserializeOwned + Default>(path: &Path) -> (T, bool) {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return (T::default(), false),
        Err(e) => {
            tracing::warn!("failed to read {}: {}", path.display(), e);
            return (T::default(), false);
        }
    };

    match serde_json::from_slice::<T>(&bytes) {
        Ok(value) => (value, false),
        Err(e) => {
            tracing::warn!("{} is corrupt ({}); moving it aside", path.display(), e);
            let backup = corrupt_sibling(path);
            if let Err(move_err) = fs::rename(path, &backup) {
                tracing::warn!("could not move corrupt file aside: {}", move_err);
            } else {
                tracing::info!("corrupt file preserved at {}", backup.display());
            }
            (T::default(), true)
        }
    }
}

fn tmp_sibling(path: &Path) -> PathBuf {
    let mut name = path
        .file_name()
        .map(|s| s.to_owned())
        .unwrap_or_else(|| std::ffi::OsString::from("tmp"));
    name.push(format!(".tmp-{}", std::process::id()));
    path.with_file_name(name)
}

fn corrupt_sibling(path: &Path) -> PathBuf {
    let mut name = path
        .file_name()
        .map(|s| s.to_owned())
        .unwrap_or_else(|| std::ffi::OsString::from("file"));
    name.push(format!(".corrupt-{}", unix_seconds()));
    path.with_file_name(name)
}

/// Seconds since the Unix epoch (saturates to 0 on broken clocks).
pub fn unix_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

// ---------------------------------------------------------------------------
// Formatting
// ---------------------------------------------------------------------------

/// `1536` -> `"1.5 KB"`. Used for download / install sizes.
pub fn format_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{} {}", bytes, UNITS[unit])
    } else {
        format!("{:.1} {}", value, UNITS[unit])
    }
}

/// `5025` -> `"1h 23m"`. Used for playtime and uptime displays.
pub fn format_duration_secs(total_secs: u64) -> String {
    let hours = total_secs / 3600;
    let minutes = (total_secs % 3600) / 60;
    let secs = total_secs % 60;
    if hours > 0 {
        format!("{}h {:02}m", hours, minutes)
    } else if minutes > 0 {
        format!("{}m {:02}s", minutes, secs)
    } else {
        format!("{}s", secs)
    }
}

/// `1_500_000.0` -> `"1.4 MB/s"`. Used for download throughput.
pub fn format_throughput(bytes_per_sec: f64) -> String {
    if !bytes_per_sec.is_finite() || bytes_per_sec < 0.0 {
        return String::from("--/s");
    }
    format!("{}/s", format_bytes(bytes_per_sec as u64))
}

// ---------------------------------------------------------------------------
// File names
// ---------------------------------------------------------------------------

/// Make arbitrary text safe for use as a file name: strips characters that
/// are illegal on Windows, trims trailing dots/spaces, caps length, and
/// guarantees a non-empty result.
pub fn sanitize_filename(input: &str) -> String {
    let mut out: String = input
        .chars()
        .map(|c| match c {
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' => '_',
            c if c.is_control() => '_',
            c => c,
        })
        .collect();

    // Trim trailing dots/spaces (illegal on Windows) and leading whitespace.
    while out.ends_with('.') || out.ends_with(' ') {
        out.pop();
    }
    let trimmed = out.trim_start().to_string();
    out = trimmed;

    // Cap length (in chars, to stay well under OS byte limits).
    if out.chars().count() > 80 {
        out = out.chars().take(80).collect();
        while out.ends_with('.') || out.ends_with(' ') {
            out.pop();
        }
    }

    if out.is_empty() {
        out.push_str("unnamed");
    }
    // Avoid reserved device names on Windows.
    const RESERVED: [&str; 22] = [
        "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7",
        "COM8", "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
    ];
    if RESERVED.iter().any(|r| out.eq_ignore_ascii_case(r)) {
        out.push('_');
    }
    out
}

// ---------------------------------------------------------------------------
// Well-known directories (no external crates needed)
// ---------------------------------------------------------------------------

fn env_dir(var: &str) -> Option<PathBuf> {
    std::env::var_os(var).map(PathBuf::from)
}

/// Current user's home directory.
pub fn home_dir() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        env_dir("USERPROFILE").or_else(home_dir_unix)
    }
    #[cfg(not(windows))]
    {
        home_dir_unix()
    }
}

fn home_dir_unix() -> Option<PathBuf> {
    env_dir("HOME")
}

/// Per-user local (non-roaming) application data directory.
pub fn local_app_data() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        env_dir("LOCALAPPDATA")
    }
    #[cfg(target_os = "macos")]
    {
        home_dir().map(|h| h.join("Library/Application Support"))
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        env_dir("XDG_DATA_HOME").or_else(|| home_dir().map(|h| h.join(".local/share")))
    }
    #[cfg(not(any(windows, unix)))]
    {
        None
    }
}

/// Per-user cache directory.
pub fn cache_dir() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        env_dir("LOCALAPPDATA")
    }
    #[cfg(target_os = "macos")]
    {
        home_dir().map(|h| h.join("Library/Caches"))
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        env_dir("XDG_CACHE_HOME").or_else(|| home_dir().map(|h| h.join(".cache")))
    }
    #[cfg(not(any(windows, unix)))]
    {
        None
    }
}

/// Current user's desktop directory, if it exists.
pub fn desktop_dir() -> Option<PathBuf> {
    let dir = home_dir()?.join("Desktop");
    if dir.is_dir() {
        Some(dir)
    } else {
        // Still return it: callers create it on demand when writing shortcuts.
        Some(dir)
    }
}

/// Windows Start Menu `Programs` directory. Always `None` on other systems.
pub fn start_menu_programs_dir() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        env_dir("APPDATA").map(|a| a.join("Microsoft/Windows/Start Menu/Programs"))
    }
    #[cfg(not(windows))]
    {
        None
    }
}

// ---------------------------------------------------------------------------
// Opening URLs / files with the OS default handler
// ---------------------------------------------------------------------------

/// Open a URL in the default web browser.
pub fn open_url(url: &str) -> Result<()> {
    if url.is_empty() {
        return Err(Error::Other(String::from("cannot open an empty URL")));
    }
    #[cfg(windows)]
    {
        shell_execute("open", url)
    }
    #[cfg(target_os = "macos")]
    {
        run_opener("open", url)
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        run_opener("xdg-open", url)
    }
    #[cfg(not(any(windows, unix)))]
    {
        Err(Error::Unsupported(String::from(
            "opening URLs is unavailable on this platform",
        )))
    }
}

/// Reveal a file in the system file manager (or open a directory).
pub fn reveal_in_file_manager(path: &Path) -> Result<()> {
    #[cfg(windows)]
    {
        // `explorer /select,"..."` highlights the file; fall back to opening
        // the parent directory if that fails.
        let arg = format!("/select,\"{}\"", path.display());
        match std::process::Command::new("explorer")
            .arg(arg)
            .spawn()
        {
            Ok(_) => Ok(()),
            Err(_) => {
                let parent = path.parent().unwrap_or(path);
                shell_execute("open", &parent.to_string_lossy())
            }
        }
    }
    #[cfg(target_os = "macos")]
    {
        let target = if path.is_dir() {
            path.to_path_buf()
        } else {
            path.parent().unwrap_or(path).to_path_buf()
        };
        run_opener("open", &target.to_string_lossy())
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let target = if path.is_dir() {
            path.to_path_buf()
        } else {
            path.parent().unwrap_or(path).to_path_buf()
        };
        run_opener("xdg-open", &target.to_string_lossy())
    }
    #[cfg(not(any(windows, unix)))]
    {
        Err(Error::Unsupported(String::from(
            "the file manager is unavailable on this platform",
        )))
    }
}

#[cfg(unix)]
fn run_opener(program: &str, target: &str) -> Result<()> {
    std::process::Command::new(program)
        .arg(target)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map(|_| ())
        .map_err(|e| Error::Other(format!("failed to run {}: {}", program, e)))
}

#[cfg(windows)]
fn shell_execute(_operation: &str, target: &str) -> Result<()> {
    std::process::Command::new("explorer")
        .arg(target)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map(|_| ())
        .map_err(|e| Error::Other(format!("failed to run explorer: {e}")))
}

// ---------------------------------------------------------------------------
// Windows string helpers
// ---------------------------------------------------------------------------

/// Encode a Rust string as NUL-terminated UTF-16 for Win32 APIs.
#[cfg(windows)]
pub fn to_wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Decode a NUL-terminated UTF-16 buffer from a Win32 API (lossy).
///
/// # Safety
/// `ptr` must point to at least `len` readable `u16` units.
#[cfg(windows)]
pub unsafe fn wide_to_string_lossy(ptr: *const u16, len: usize) -> String {
    if ptr.is_null() || len == 0 {
        return String::new();
    }
    let slice = std::slice::from_raw_parts(ptr, len);
    let end = slice.iter().position(|c| *c == 0).unwrap_or(len);
    String::from_utf16_lossy(&slice[..end])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn md5_matches_known_vector() {
        // MD5("abc") — the canonical test vector.
        assert_eq!(
            md5_hex_bytes(b"abc"),
            "900150983cd24fb0d6963f7d28e17f72"
        );
    }

    #[test]
    fn byte_formatting() {
        assert_eq!(format_bytes(0), "0 B");
        assert_eq!(format_bytes(512), "512 B");
        assert_eq!(format_bytes(1024), "1.0 KB");
        assert_eq!(format_bytes(1536), "1.5 KB");
        assert_eq!(format_bytes(5 * 1024 * 1024), "5.0 MB");
    }

    #[test]
    fn duration_formatting() {
        assert_eq!(format_duration_secs(45), "45s");
        assert_eq!(format_duration_secs(65), "1m 05s");
        assert_eq!(format_duration_secs(5025), "1h 23m");
    }

    #[test]
    fn filename_sanitizer() {
        assert_eq!(sanitize_filename("Hello: World?"), "Hello_ World_");
        assert_eq!(sanitize_filename("   "), "unnamed");
        assert_eq!(sanitize_filename("trailing..."), "trailing");
        assert_eq!(sanitize_filename("CON"), "CON_");
        assert!(sanitize_filename(&"a".repeat(200)).chars().count() <= 80);
    }

    #[test]
    fn atomic_json_roundtrip() {
        #[derive(Debug, Default, PartialEq, Serialize, serde::Deserialize)]
        struct Sample {
            #[serde(default)]
            name: String,
            #[serde(default)]
            count: u32,
        }

        let dir = std::env::temp_dir().join(format!("redstrap-test-{}", std::process::id()));
        let _ = fs::create_dir_all(&dir);
        let path = dir.join("sample.json");

        let value = Sample {
            name: String::from("hi"),
            count: 7,
        };
        save_json(&path, &value).expect("save");
        let (loaded, corrupted): (Sample, bool) = load_json_or_default(&path);
        assert!(!corrupted);
        assert_eq!(loaded, value);

        // Corrupt the file: loader must back it up and return the default.
        fs::write(&path, b"{not json").expect("corrupt");
        let (loaded, corrupted): (Sample, bool) = load_json_or_default(&path);
        assert!(corrupted);
        assert_eq!(loaded, Sample::default());
        assert!(!path.exists(), "corrupt file should be moved aside");

        let _ = fs::remove_dir_all(&dir);
    }
}
