//! Game launching: pipeline, spawning, priorities, integrations.
//!
//! One launch runs the install pipeline (fast no-op when up to date),
//! optionally wipes temp files, spawns Roblox, applies the configured
//! process priority, starts custom integrations, and detaches a `--watcher`
//! child for activity tracking / RPC / auto-rejoin.

use std::path::Path;

use redstrap_core::error::{Error, Result};
use redstrap_core::paths::Layout;
use redstrap_core::pipeline;
use redstrap_core::roblox::LaunchMode;
use redstrap_core::settings::{CleanerMode, ProcessPriority, Settings};
use redstrap_core::state::State;

/// Launch Roblox (player or studio) with the given protocol/launch args.
pub async fn launch(
    client: &reqwest::Client,
    layout: &Layout,
    settings: &Settings,
    state: &mut State,
    mode: LaunchMode,
    launch_args: &str,
    version_pin: Option<&str>,
    cli: &crate::Cli,
) -> Result<()> {
    if settings.confirm_launches && !crate::confirm(&format!("Launch {}?", mode.label())) {
        return Err(Error::Cancelled);
    }

    let force = state.force_reinstall;
    let mut progress = crate::console_progress(cli.quiet, cli.verbose);
    let outcome = pipeline::ensure_installed(
        client,
        layout,
        settings,
        mode,
        version_pin,
        force,
        &mut progress,
    )
    .await?;
    if force {
        state.force_reinstall = false;
        if let Err(e) = state.save(&layout.state_file) {
            tracing::warn!("could not clear force-reinstall flag: {e}");
        }
    }
    tracing::info!(
        "launching {} {} from {}",
        mode.label(),
        outcome.version_guid,
        outcome.exe.display()
    );

    if settings.cleaner == CleanerMode::OnLaunch {
        run_cleaner(layout, settings, cli.quiet);
    }

    // Multi-instance: same game N times with a stagger delay (player only).
    let instances = if settings.multi_instance_launching && !mode.is_studio() {
        settings.instances_count.max(1) as usize
    } else {
        1
    };

    // The watcher tracks the lead instance; extra multi-instances run
    // untracked so only one Discord presence is ever published.
    let mut lead_pid = 0u32;
    for index in 0..instances {
        if index > 0 {
            tokio::time::sleep(std::time::Duration::from_millis(
                settings.instance_delay_ms,
            ))
            .await;
        }
        let spawned = spawn_instance(settings, &outcome, launch_args, cli.quiet)?;
        if index == 0 {
            lead_pid = spawned.pid;
        }
    }

    if settings.auto_close_crash_handler {
        schedule_crash_handler_close();
    }

    spawn_custom_integrations(settings, cli.quiet);
    spawn_watcher(settings, mode, lead_pid, cli.quiet);

    crate::say(cli.quiet, &format!("{} launched.", mode.label()));
    Ok(())
}

struct Spawned {
    pid: u32,
}

fn spawn_instance(
    settings: &Settings,
    outcome: &pipeline::InstallOutcome,
    launch_args: &str,
    quiet: bool,
) -> Result<Spawned> {
    let mut command = redstrap_core::roblox::launch_command(
        &outcome.exe,
        launch_args,
        &settings.custom_launch_command,
        &outcome.version_dir,
    );
    let mut child = command.spawn().map_err(|e| Error::Launch {
        exe: outcome.exe.to_string_lossy().into_owned(),
        reason: e.to_string(),
    })?;
    let pid = child.id();
    // Detach: the launcher must exit while Roblox keeps running.
    std::mem::forget(child);

    if settings.process_priority != ProcessPriority::Normal {
        // The process may need a moment to initialize before accepting a
        // priority change; one retry keeps this best-effort and quiet.
        for attempt in 0..2 {
            match redstrap_core::process::set_priority(pid, settings.process_priority) {
                Ok(()) => break,
                Err(e) => {
                    if attempt == 1 {
                        crate::say(
                            quiet,
                            &format!("Note: could not set process priority: {e}"),
                        );
                    } else {
                        std::thread::sleep(std::time::Duration::from_millis(500));
                    }
                }
            }
        }
    }

    tracing::info!("spawned {} (PID {pid})", outcome.exe.display());
    Ok(Spawned { pid })
}

/// Close Roblox's crash-handler window shortly after launch, if enabled.
/// Runs on a detached thread: fire-and-forget by design.
fn schedule_crash_handler_close() {
    let started = std::thread::Builder::new()
        .name(String::from("crash-handler-closer"))
        .spawn(|| {
            std::thread::sleep(std::time::Duration::from_secs(10));
            let killed = redstrap_core::process::kill_all_by_name("RobloxCrashHandler");
            if killed > 0 {
                tracing::info!("closed {killed} crash handler window(s)");
            }
        });
    if let Err(e) = started {
        tracing::warn!("could not start crash-handler closer: {e}");
    }
}

fn spawn_custom_integrations(settings: &Settings, quiet: bool) {
    for integration in &settings.custom_integrations {
        if !integration.enabled || integration.exe.trim().is_empty() {
            continue;
        }
        let mut command = std::process::Command::new(integration.exe.trim());
        if !integration.args.trim().is_empty() {
            for arg in split_args(integration.args.trim()) {
                command.arg(arg);
            }
        }
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            command.creation_flags(CREATE_NO_WINDOW);
        }
        match command.spawn() {
            Ok(_) => tracing::info!("started integration '{}'", integration.name),
            Err(e) => crate::say(
                quiet,
                &format!(
                    "Note: could not start integration '{}': {e}",
                    integration.name
                ),
            ),
        }
    }
}

/// Split an argument string on whitespace, honouring double quotes.
fn split_args(input: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;
    let mut has_token = false;
    for c in input.chars() {
        match c {
            '"' => {
                in_quotes = !in_quotes;
                has_token = true;
            }
            c if c.is_whitespace() && !in_quotes => {
                if has_token {
                    out.push(std::mem::take(&mut current));
                    has_token = false;
                }
            }
            c => {
                current.push(c);
                has_token = true;
            }
        }
    }
    if has_token {
        out.push(current);
    }
    out
}

/// Detach the `--watcher` child that tails the client log. Skipped entirely
/// when no watcher-backed feature is enabled, so launches stay lean.
fn spawn_watcher(settings: &Settings, mode: LaunchMode, pid: u32, quiet: bool) {
    let needed = settings.activity_tracking
        || settings.playtime_counter
        || settings.auto_rejoin
        || settings.close_on_leave_game
        || settings.show_server_details
        || (settings.discord_rpc && !settings.discord_client_id.trim().is_empty());
    if !needed {
        tracing::info!("watcher not needed; skipping");
        return;
    }

    let exe = match std::env::current_exe() {
        Ok(e) => e,
        Err(e) => {
            tracing::warn!("could not locate launcher for watcher spawn: {e}");
            return;
        }
    };

    let mut command = std::process::Command::new(exe);
    command
        .arg("--watcher")
        .arg("--pid")
        .arg(pid.to_string())
        .arg("--mode")
        .arg(mode.as_str())
        .arg("--quiet");
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    match command.spawn() {
        Ok(_) => tracing::info!("watcher detached for PID {pid} ({})", mode.as_str()),
        Err(e) => crate::say(quiet, &format!("Note: could not start watcher: {e}")),
    }
}

// ---------------------------------------------------------------------------
// Cleaner
// ---------------------------------------------------------------------------

/// Delete Roblox temp/cache/log debris plus user-configured extra dirs.
/// Only the *contents* of directories are removed, never the dirs themselves.
pub fn run_cleaner(layout: &Layout, settings: &Settings, quiet: bool) {
    let mut removed_files = 0u64;
    let mut removed_bytes = 0u64;

    let (files, bytes) = wipe_contents(&layout.roblox_temp);
    removed_files += files;
    removed_bytes += bytes;

    // Stale client logs (keep the newest handful for debugging).
    remove_old_logs(&layout.roblox_logs, 5);

    for dir in &settings.cleaner_extra_dirs {
        let path = Path::new(dir.trim());
        if path.is_dir() {
            let (files, bytes) = wipe_contents(path);
            removed_files += files;
            removed_bytes += bytes;
        }
    }

    crate::say(
        quiet,
        &format!(
            "Cleaned {} file(s), {}.",
            removed_files,
            redstrap_core::util::format_bytes(removed_bytes)
        ),
    );
    tracing::info!("cleaner removed {removed_files} files ({removed_bytes} bytes)");
}

fn wipe_contents(dir: &Path) -> (u64, u64) {
    let mut files = 0;
    let mut bytes = 0;
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return (0, 0),
    };
    for entry in entries.flatten() {
        let path = entry.path();
        // Never follow or delete symlinks as trees — remove the link itself.
        let file_type = match std::fs::symlink_metadata(&path).map(|m| m.file_type()) {
            Ok(t) => t,
            Err(_) => continue,
        };
        if file_type.is_symlink() {
            if std::fs::remove_file(&path).is_ok() {
                files += 1;
            }
            continue;
        }
        if file_type.is_dir() {
            let (f, b) = wipe_contents(&path);
            files += f;
            bytes += b;
            let _ = std::fs::remove_dir(&path);
        } else {
            bytes += entry.metadata().map(|m| m.len()).unwrap_or(0);
            if std::fs::remove_file(&path).is_ok() {
                files += 1;
            }
        }
    }
    (files, bytes)
}

/// Delete `.log` files in `dir` except the newest `keep`.
fn remove_old_logs(dir: &Path, keep: usize) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    let mut logs: Vec<(std::time::SystemTime, std::path::PathBuf)> = entries
        .flatten()
        .filter(|e| {
            e.path()
                .extension()
                .and_then(|x| x.to_str())
                .map(|x| x.eq_ignore_ascii_case("log"))
                .unwrap_or(false)
        })
        .filter_map(|e| {
            e.metadata()
                .and_then(|m| m.modified())
                .ok()
                .map(|t| (t, e.path()))
        })
        .collect();
    logs.sort_by(|a, b| b.0.cmp(&a.0));
    for (_, path) in logs.into_iter().skip(keep) {
        let _ = std::fs::remove_file(&path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arg_splitting() {
        assert_eq!(split_args("--a b"), vec!["--a", "b"]);
        assert_eq!(
            split_args("--path \"C:\\My Dir\\x.exe\" --flag"),
            vec!["--path", "C:\\My Dir\\x.exe", "--flag"]
        );
        assert!(split_args("   ").is_empty());
    }
}
