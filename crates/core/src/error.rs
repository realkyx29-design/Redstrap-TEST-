//! Unified error type for Red Strap.
//!
//! Every fallible operation in the workspace funnels through [`Error`] so
//! failures always carry actionable context and are surfaced to the user
//! through dialogs / logs instead of panics.

use std::path::PathBuf;

use thiserror::Error;

/// Convenience alias used across the whole workspace.
pub type Result<T> = std::result::Result<T, Error>;

/// All recoverable failures Red Strap can report.
#[derive(Debug, Error)]
pub enum Error {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("failed to parse JSON: {0}")]
    Json(#[from] serde_json::Error),

    #[error("network request failed: {0}")]
    Http(#[from] reqwest::Error),

    #[error("failed to parse XML: {0}")]
    Xml(#[from] quick_xml::errors::Error),

    #[error("image error: {0}")]
    Image(#[from] image::ImageError),

    #[error("file system path error: {0}")]
    Path(String),

    #[error("configuration error: {0}")]
    Config(String),

    #[error("invalid version string '{0}'")]
    VersionParse(String),

    #[error("package manifest error: {0}")]
    Manifest(String),

    #[error("download checksum mismatch for '{file}': expected {expected}, got {actual}")]
    Checksum {
        file: String,
        expected: String,
        actual: String,
    },

    #[error("download was cancelled")]
    Cancelled,

    #[error("package extraction failed: {0}")]
    Extraction(String),

    #[error("unknown or private Roblox channel '{channel}' (HTTP {status})")]
    InvalidChannel { channel: String, status: u16 },

    #[error("Roblox is not installed and automatic updates are disabled")]
    NotInstalledUpdatesBlocked,

    #[error("no installed Roblox version found for {0}")]
    DistributionMissing(String),

    #[error("failed to launch '{exe}': {reason}")]
    Launch { exe: String, reason: String },

    #[error("process operation failed: {0}")]
    Process(String),

    #[error("another Red Strap window of this kind is already open")]
    AlreadyRunning,

    #[error("Windows API call failed with error code {0}")]
    Win32(u32),

    #[error("registry operation failed: {0}")]
    Registry(String),

    #[error("shortcut creation failed: {0}")]
    Shortcut(String),

    #[error("unsupported on this platform: {0}")]
    Unsupported(String),

    #[error("Discord rich presence error: {0}")]
    Rpc(String),

    #[error("update check failed: {0}")]
    Update(String),

    #[error("font error: {0}")]
    Font(String),

    #[error("mod error: {0}")]
    Mod(String),

    #[error("Roblox settings file error: {0}")]
    GameSettings(String),

    #[error("{0}")]
    Other(String),
}

impl Error {
    /// Attach a file path to an I/O-flavoured error for clearer messages.
    pub fn with_path(path: &PathBuf, err: std::io::Error) -> Self {
        Error::Other(format!("{}: {}", path.display(), err))
    }

    /// Short, user-friendly title describing the failure class.
    pub fn title(&self) -> &'static str {
        match self {
            Error::Io(_) => "File Error",
            Error::Json(_) => "Data Error",
            Error::Http(_) => "Network Error",
            Error::Xml(_) => "Data Error",
            Error::Image(_) => "Image Error",
            Error::Path(_) => "Path Error",
            Error::Config(_) => "Configuration Error",
            Error::VersionParse(_) => "Version Error",
            Error::Manifest(_) => "Download Error",
            Error::Checksum { .. } => "Download Error",
            Error::Cancelled => "Cancelled",
            Error::Extraction(_) => "Install Error",
            Error::InvalidChannel { .. } => "Channel Error",
            Error::NotInstalledUpdatesBlocked => "Not Installed",
            Error::DistributionMissing(_) => "Not Installed",
            Error::Launch { .. } => "Launch Error",
            Error::Process(_) => "Process Error",
            Error::AlreadyRunning => "Already Running",
            Error::Win32(_) => "System Error",
            Error::Registry(_) => "System Error",
            Error::Shortcut(_) => "Shortcut Error",
            Error::Unsupported(_) => "Unsupported",
            Error::Rpc(_) => "Discord Error",
            Error::Update(_) => "Update Error",
            Error::Font(_) => "Font Error",
            Error::Mod(_) => "Mod Error",
            Error::GameSettings(_) => "Settings Error",
            Error::Other(_) => "Error",
        }
    }
}
