//! Installation and removal of Red Strap itself.
//!
//! Install copies the launcher (plus the settings UI when shipped
//! alongside) into the data directory, registers the uninstall entry, URL
//! protocols and shortcuts. Uninstall removes all of it, including the
//! downloaded Roblox versions that live inside the install directory.

use std::path::Path;

use redstrap_core::error::{Error, Result};
use redstrap_core::paths::Layout;
use redstrap_core::settings::Settings;

/// True when the launcher executable exists at its installed location.
pub fn is_installed(layout: &Layout) -> bool {
    layout.application.is_file()
}

/// Install or repair. Safe to run over an existing install.
pub fn install(layout: &Layout, settings: &Settings, quiet: bool) -> Result<()> {
    layout.ensure_dirs()?;

    let current = std::env::current_exe().map_err(|e| {
        Error::Other(format!("could not locate the running executable: {e}"))
    })?;
    copy_self(&current, &layout.application)?;

    // Ship the settings UI alongside when present.
    if let Some(dir) = current.parent() {
        let shipped = dir.join(redstrap_core::consts::SETTINGS_EXE);
        if shipped.is_file() && shipped != layout.settings_app() {
            let dest = layout.settings_app();
            if let Err(e) = std::fs::copy(&shipped, &dest) {
                tracing::warn!("could not install settings UI: {e}");
            }
        }
    }

    #[cfg(windows)]
    {
        register_uninstall_entry(layout)?;
        register_protocols(layout)?;
    }
    // (Non-Windows systems have no URL-protocol registry; desktop entries
    // are written by the shortcut sync below.)
    redstrap_core::shortcuts::sync_managed(layout, settings)?;

    settings.save(&layout.settings_file)?;
    crate::say(quiet, "Red Strap installed successfully.");
    tracing::info!("installed to {}", layout.base.display());
    Ok(())
}

fn copy_self(current: &Path, dest: &Path) -> Result<()> {
    if current == dest {
        return Ok(());
    }
    if dest.is_file() {
        // The destination may be locked by a running instance.
        if let Err(e) = std::fs::copy(current, dest) {
            return Err(Error::Other(format!(
                "could not replace {} (is Red Strap still running?): {e}",
                dest.display()
            )));
        }
        return Ok(());
    }
    std::fs::copy(current, dest).map_err(|e| Error::with_path(&dest.to_path_buf(), e))?;
    Ok(())
}

#[cfg(windows)]
fn register_uninstall_entry(layout: &Layout) -> Result<()> {
    use redstrap_core::consts::APP_NAME;
    use redstrap_core::registry;

    let key = registry::uninstall_key();
    let exe = layout.application.to_string_lossy().into_owned();
    let base = layout.base.to_string_lossy().into_owned();
    registry::set_sz(&key, Some("DisplayName"), APP_NAME)?;
    registry::set_sz(&key, Some("DisplayVersion"), env!("CARGO_PKG_VERSION"))?;
    registry::set_sz(&key, Some("Publisher"), "RedStrap")?;
    registry::set_sz(&key, Some("InstallLocation"), &base)?;
    registry::set_sz(&key, Some("UninstallString"), &format!("\"{exe}\" --uninstall"))?;
    registry::set_sz(&key, Some("QuietUninstallString"), &format!("\"{exe}\" --uninstall --yes"))?;
    registry::set_sz(&key, Some("DisplayIcon"), &format!("{exe},0"))?;
    registry::set_dword(&key, Some("NoModify"), 1)?;
    registry::set_dword(&key, Some("NoRepair"), 1)?;
    Ok(())
}

/// `roblox://`, `roblox-player://` and Studio protocols pointing at us.
#[cfg(windows)]
fn register_protocols(layout: &Layout) -> Result<()> {
    use redstrap_core::registry;

    let exe = layout.application.to_string_lossy().into_owned();
    let protocols = [
        ("roblox-player", "Roblox Player"),
        ("roblox", "Roblox"),
        ("roblox-studio", "Roblox Studio"),
        ("roblox-studio-auth", "Roblox Studio Auth"),
    ];
    for (protocol, label) in protocols {
        let key = registry::protocol_key(protocol);
        registry::set_sz(&key, None, &format!("URL:{label}"))?;
        registry::set_sz(&key, Some("URL Protocol"), "")?;
        registry::set_sz(&format!("{key}\\DefaultIcon"), None, &format!("{exe},0"))?;
        registry::set_sz(
            &format!("{key}\\shell\\open\\command"),
            None,
            &format!("\"{exe}\" \"%1\""),
        )?;
    }
    Ok(())
}

/// Remove everything Red Strap installed. `yes` skips the confirmation.
pub fn uninstall(layout: &Layout, settings: &Settings, yes: bool, quiet: bool) -> Result<()> {
    if !is_installed(layout) {
        crate::say(quiet, "Red Strap is not installed.");
        return Ok(());
    }

    if !yes && !crate::confirm(
        "Remove Red Strap and all downloaded Roblox versions? Running games will be closed",
    ) {
        return Err(Error::Cancelled);
    }

    crate::say(quiet, "Removing Red Strap...");

    // Release file locks: our own watchers plus any running Roblox.
    let own_pid = std::process::id();
    for pid in redstrap_core::process::find_pids_by_name("roblox") {
        if pid != own_pid {
            let _ = redstrap_core::process::kill_pid(pid, true);
        }
    }
    for pid in redstrap_core::process::find_pids_by_name("redstrap") {
        if pid != own_pid {
            let _ = redstrap_core::process::kill_pid(pid, true);
        }
    }
    std::thread::sleep(std::time::Duration::from_millis(800));

    #[cfg(windows)]
    {
        use redstrap_core::registry;
        let _ = registry::delete_tree(&registry::uninstall_key());
        for protocol in ["roblox-player", "roblox", "roblox-studio", "roblox-studio-auth"] {
            let _ = registry::delete_tree(&registry::protocol_key(protocol));
        }
        // Drop per-exe graphics / compatibility tweaks we may have written.
        remove_os_preferences(layout, settings);
    }

    // Remove managed shortcuts (best effort; user may have deleted them).
    let _ = redstrap_core::shortcuts::remove_managed(layout, settings);

    #[cfg(windows)]
    {
        // The running exe cannot delete itself: hand off to a batch file.
        let deleter = std::env::temp_dir().join(format!("redstrap-uninstall-{own_pid}.bat"));
        let script = format!(
            "@echo off\r\ntimeout /t 2 /nobreak >nul\r\nrmdir /s /q \"{}\"\r\ndel \"%~f0\"\r\n",
            layout.base.display()
        );
        if std::fs::write(&deleter, script).is_ok() {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            const DETACHED_PROCESS: u32 = 0x0000_0008;
            let _ = std::process::Command::new("cmd")
                .arg("/c")
                .arg(deleter)
                .creation_flags(CREATE_NO_WINDOW | DETACHED_PROCESS)
                .spawn();
        } else {
            // Fall back to deleting everything except the running exe.
            let _ = remove_all_except_self(&layout.base, own_pid);
        }
    }
    #[cfg(not(windows))]
    {
        // Unix executables can be unlinked while running.
        if let Err(e) = std::fs::remove_dir_all(&layout.base) {
            return Err(Error::Other(format!(
                "could not remove {}: {e}",
                layout.base.display()
            )));
        }
    }

    crate::say(quiet, "Red Strap has been removed.");
    Ok(())
}

#[cfg(windows)]
fn remove_os_preferences(layout: &Layout, settings: &Settings) {
    use redstrap_core::registry;
    use redstrap_core::roblox::LaunchMode;
    use redstrap_core::state::DistributionState;

    // Resolve the installed exes from both dist states.
    let mut exes = Vec::new();
    for (mode, file) in [
        (LaunchMode::Player, &layout.player_state_file),
        (LaunchMode::Studio, &layout.studio_state_file),
    ] {
        let (dist, _) = DistributionState::load(file);
        if dist.is_installed() {
            let (_, exe) = redstrap_core::roblox::version_paths(
                layout,
                mode,
                &dist.version_guid,
                settings.static_directory,
            );
            exes.push(exe.to_string_lossy().into_owned());
        }
    }
    for exe in exes {
        let _ = registry::delete_value(
            "Software\\Microsoft\\DirectX\\UserGpuPreferences",
            Some(&exe),
        );
        let _ = registry::delete_value(
            "Software\\Microsoft\\Windows NT\\CurrentVersion\\AppCompatFlags\\Layers",
            Some(&exe),
        );
    }
}

/// Delete a directory tree except the currently running executable.
#[cfg(windows)]
fn remove_all_except_self(dir: &Path, _own_pid: u32) -> Result<()> {
    let current = std::env::current_exe().unwrap_or_else(|_| dir.join("redstrap.exe"));
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(e) => return Err(Error::with_path(&dir.to_path_buf(), e)),
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path == current {
            continue;
        }
        if path.is_dir() {
            let _ = std::fs::remove_dir_all(&path);
        } else {
            let _ = std::fs::remove_file(&path);
        }
    }
    Ok(())
}
