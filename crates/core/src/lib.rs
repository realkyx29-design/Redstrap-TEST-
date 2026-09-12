//! redstrap-core: shared logic for the Red Strap launcher.
//!
//! This crate holds everything that is UI-agnostic: settings, paths, the
//! Roblox deployment client, mod handling, the settings-file editor,
//! process/registry helpers, and the self-updater. Both the background
//! bootstrapper binary and the desktop settings app build on it.
//!
//! Async functions assume a Tokio runtime provided by the caller.

pub mod allowlist;
pub mod consts;
pub mod deployment;
pub mod error;
pub mod fastflags;
pub mod gbs;
pub mod http;
pub mod instance;
pub mod logging;
pub mod manifest;
pub mod mods;
pub mod paths;
pub mod pipeline;
pub mod process;
pub mod registry;
pub mod roblox;
pub mod settings;
pub mod shortcut;
pub mod shortcuts;
pub mod state;
pub mod updater;
pub mod util;
pub mod version;

pub use error::{Error, Result};
