//! Red Strap bootstrapper: install, update, launch, and watch Roblox.
//!
//! Flag-based CLI kept compatible with the previous bootstrapper family:
//! protocol URIs launch straight into games, `--player`/`--studio` start the
//! apps, `--settings` opens the desktop UI, and `--watcher` attaches the
//! activity/RPC monitor to a running instance.

mod install;
mod launch;
mod rpc;
mod watcher;

use std::io::IsTerminal;

use clap::Parser;
use redstrap_core::consts::APP_NAME;
use redstrap_core::error::{Error, Result};
use redstrap_core::roblox::LaunchMode;

#[derive(Debug, Parser)]
#[command(name = "RedStrap", version, about = "Fast, lightweight Roblox bootstrapper")]
pub(crate) struct Cli {
    /// Protocol URI (roblox://...) or `version-<hash>` pin to launch.
    #[arg(value_name = "TARGET")]
    pub(crate) target: Option<String>,

    /// Launch Roblox Player.
    #[arg(long)]
    pub(crate) player: bool,

    /// Launch Roblox Studio.
    #[arg(long)]
    pub(crate) studio: bool,

    /// Open the Red Strap settings window.
    #[arg(long)]
    pub(crate) settings: bool,

    /// Install or repair the Red Strap installation.
    #[arg(long)]
    pub(crate) install: bool,

    /// Remove Red Strap (asks unless `--yes` is given).
    #[arg(long)]
    pub(crate) uninstall: bool,

    /// Skip confirmation prompts.
    #[arg(long)]
    pub(crate) yes: bool,

    /// Run the activity/RPC watcher against a running Roblox process.
    #[arg(long)]
    pub(crate) watcher: bool,

    /// Process ID to watch (with `--watcher`).
    #[arg(long, value_name = "PID")]
    pub(crate) pid: Option<u32>,

    /// `player` or `studio` (with `--watcher` / `--sync-roblox`).
    #[arg(long, value_name = "MODE")]
    pub(crate) mode: Option<String>,

    /// Raw launch arguments appended to the Roblox command line.
    #[arg(long, value_name = "ARGS")]
    pub(crate) launch_args: Option<String>,

    /// `placeId[;jobId[;accessCode]]` to join (implies `--player`).
    #[arg(long, value_name = "DATA")]
    pub(crate) gameshortcut: Option<String>,

    /// Pin a specific `version-<hash>` instead of the channel's latest.
    #[arg(long, value_name = "GUID")]
    pub(crate) version_guid: Option<String>,

    /// Install/update Roblox without launching it.
    #[arg(long)]
    pub(crate) sync_roblox: bool,

    /// Silent self-update check, used by the scheduled task.
    #[arg(long)]
    pub(crate) background_updater: bool,

    /// Marker passed to the relaunched binary after a self-update.
    #[arg(long, hide = true)]
    pub(crate) updated: bool,

    /// Suppress console output (file logging continues).
    #[arg(long)]
    pub(crate) quiet: bool,

    /// Print detailed progress information.
    #[arg(long)]
    pub(crate) verbose: bool,
}

#[tokio::main(flavor = "multi_thread", worker_threads = 4)]
async fn main() {
    let cli = Cli::parse();
    if let Err(e) = run(cli).await {
        eprintln!("{APP_NAME}: {e}");
        std::process::exit(1);
    }
}

async fn run(cli: Cli) -> Result<()> {
    // Layout first: everything else (logs, settings) hangs off it.
    let base = redstrap_core::paths::resolve_base_dir();
    redstrap_core::paths::init(&base);
    let layout = redstrap_core::paths::get();
    layout.ensure_dirs()?;
    let (settings, settings_corrupt) =
        redstrap_core::settings::Settings::load(&layout.settings_file);
    if settings_corrupt {
        say(
            cli.quiet,
            "Settings file was corrupt; a backup was kept and defaults were restored.",
        );
    }
    let (_ring, _log_guard) = redstrap_core::logging::init(
        &layout.logs,
        redstrap_core::consts::LOG_FILE_PREFIX,
        200,
        settings.verbose_logging || cli.verbose,
    )?;
    let (mut state, _) = redstrap_core::state::State::load(&layout.state_file);

    let client = redstrap_core::http::build_client()?;

    if cli.watcher {
        let pid = cli.pid.ok_or_else(|| {
            Error::Other(String::from("--watcher requires --pid <PID>"))
        })?;
        let mode = parse_mode(cli.mode.as_deref())?;
        return watcher::run(&client, layout, &settings, &layout.state_file, pid, mode).await;
    }

    if cli.uninstall {
        return install::uninstall(layout, &settings, cli.yes || !std::io::stdin().is_terminal(), cli.quiet);
    }

    if cli.background_updater {
        return background_update(&client, &layout, &settings).await;
    }

    if cli.install {
        install::install(layout, &settings, cli.quiet)?;
        state.save(&layout.state_file)?;
        return Ok(());
    }

    if cli.updated {
        // Post-update: drop stale backups, then show what's new via settings.
        if let Ok(exe) = std::env::current_exe() {
            if let Some(dir) = exe.parent() {
                redstrap_core::updater::cleanup_leftovers(dir);
            }
        }
        say(cli.quiet, "Red Strap updated successfully.");
        return open_settings_ui(layout, cli.quiet);
    }

    if cli.settings {
        return open_settings_ui(layout, cli.quiet);
    }

    if cli.sync_roblox {
        let mode = parse_mode(cli.mode.as_deref())?;
        let outcome = redstrap_core::pipeline::ensure_installed(
            &client,
            layout,
            &settings,
            mode,
            cli.version_guid.as_deref(),
            false,
            &mut console_progress(cli.quiet, cli.verbose),
        )
        .await?;
        say(
            cli.quiet,
            &format!(
                "{} is ready at {}",
                mode.label(),
                outcome.version_dir.display()
            ),
        );
        return Ok(());
    }

    // Launch flows.
    if let Some(data) = cli.gameshortcut.as_deref() {
        let link = redstrap_core::roblox::parse_gameshortcut(data).ok_or_else(|| {
            Error::Other(format!(
                "could not parse --gameshortcut '{data}' (expected placeId[;jobId[;accessCode]])"
            ))
        })?;
        return launch::launch(
            &client,
            layout,
            &settings,
            &mut state,
            LaunchMode::Player,
            &link,
            cli.version_guid.as_deref(),
            &cli,
        )
        .await;
    }

    if let Some(target) = cli.target.as_deref() {
        if let Some(mode) = redstrap_core::roblox::classify_protocol_arg(target) {
            return launch::launch(
                &client,
                layout,
                &settings,
                &mut state,
                mode,
                target,
                cli.version_guid.as_deref(),
                &cli,
            )
            .await;
        }
        if target.starts_with("version-") {
            return launch::launch(
                &client,
                layout,
                &settings,
                &mut state,
                LaunchMode::Player,
                cli.launch_args.as_deref().unwrap_or(""),
                Some(target),
                &cli,
            )
            .await;
        }
        return Err(Error::Other(format!(
            "unrecognised launch target '{target}' (expected roblox://... or version-<hash>)"
        )));
    }

    if cli.player || cli.studio {
        let mode = if cli.studio {
            LaunchMode::Studio
        } else {
            LaunchMode::Player
        };
        return launch::launch(
            &client,
            layout,
            &settings,
            &mut state,
            mode,
            cli.launch_args.as_deref().unwrap_or(""),
            cli.version_guid.as_deref(),
            &cli,
        )
        .await;
    }

    // No arguments: first run installs, otherwise open settings.
    if !install::is_installed(layout) {
        install::install(layout, &settings, cli.quiet)?;
        state.save(&layout.state_file)?;
    }
    open_settings_ui(layout, cli.quiet)
}

fn parse_mode(text: Option<&str>) -> Result<LaunchMode> {
    match text {
        None => Ok(LaunchMode::Player),
        Some(t) => LaunchMode::parse(t).ok_or_else(|| {
            Error::Other(format!("unknown mode '{t}' (expected 'player' or 'studio')"))
        }),
    }
}

pub(crate) fn say(quiet: bool, message: &str) {
    if !quiet {
        println!("{message}");
    }
}

/// Build a pipeline progress sink that prints single-line console updates.
pub(crate) fn console_progress(quiet: bool, verbose: bool) -> impl FnMut(redstrap_core::pipeline::Progress) {
    let mut last_line_len = 0usize;
    let mut last_tick = std::time::Instant::now();
    move |progress: redstrap_core::pipeline::Progress| {
        if quiet {
            return;
        }
        let line = progress_line(&progress, verbose);
        // Throttle byte-level updates so the console doesn't churn.
        let throttled = matches!(
            progress,
            redstrap_core::pipeline::Progress::Downloading { .. } | redstrap_core::pipeline::Progress::Extracting { .. }
        ) && last_tick.elapsed() < std::time::Duration::from_millis(120);
        if throttled {
            return;
        }
        last_tick = std::time::Instant::now();
        // Pad with spaces to overwrite the previous (possibly longer) line.
        let padding = last_line_len.saturating_sub(line.len());
        print!("\r{line}{}", " ".repeat(padding));
        let _ = std::io::Write::flush(&mut std::io::stdout());
        last_line_len = line.len();
        if matches!(progress, redstrap_core::pipeline::Progress::Done) {
            println!();
            last_line_len = 0;
        }
    }
}

fn progress_line(progress: &redstrap_core::pipeline::Progress, verbose: bool) -> String {
    use redstrap_core::util::{format_bytes, format_throughput};
    match progress {
        redstrap_core::pipeline::Progress::Stage(text) => text.clone(),
        redstrap_core::pipeline::Progress::Downloading {
            done_bytes,
            total_bytes,
            files_done,
            files_total,
            bytes_per_sec,
        } => {
            let pct = if *total_bytes > 0 {
                (*done_bytes as f64 / *total_bytes as f64 * 100.0) as u64
            } else {
                100
            };
            if verbose {
                format!(
                    "Downloading {files_done}/{files_total} files: {} / {} ({pct}%, {})",
                    format_bytes(*done_bytes),
                    format_bytes(*total_bytes),
                    format_throughput(*bytes_per_sec),
                )
            } else {
                format!(
                    "Downloading: {} / {} ({pct}%)",
                    format_bytes(*done_bytes),
                    format_bytes(*total_bytes),
                )
            }
        }
        redstrap_core::pipeline::Progress::Extracting {
            files_done,
            files_total,
        } => format!("Installing: {files_done}/{files_total} packages"),
        redstrap_core::pipeline::Progress::Done => String::from("Ready"),
    }
}

/// Ask `prompt` on the console. Returns true when stdin is not interactive
/// (protocol launches must never block).
pub(crate) fn confirm(prompt: &str) -> bool {
    if !std::io::stdin().is_terminal() {
        return true;
    }
    print!("{prompt} [Y/n] ");
    let _ = std::io::Write::flush(&mut std::io::stdout());
    let mut line = String::new();
    if std::io::stdin().read_line(&mut line).is_err() {
        return false;
    }
    let answer = line.trim().to_ascii_lowercase();
    answer.is_empty() || answer == "y" || answer == "yes"
}

fn open_settings_ui(
    layout: &redstrap_core::paths::Layout,
    pub(crate) quiet: bool,
) -> Result<()> {
    let exe_name = if cfg!(windows) {
        "RedStrap-Settings.exe"
    } else {
        "redstrap-settings"
    };
    let mut candidates = Vec::new();
    if let Ok(current) = std::env::current_exe() {
        if let Some(dir) = current.parent() {
            candidates.push(dir.join(exe_name));
        }
    }
    if let Some(dir) = layout.application.parent() {
        candidates.push(dir.join(exe_name));
    }
    for candidate in candidates {
        if candidate.is_file() {
            say(quiet, "Opening Red Strap settings...");
            std::process::Command::new(&candidate)
                .spawn()
                .map_err(|e| Error::Launch {
                    exe: candidate.to_string_lossy().into_owned(),
                    reason: e.to_string(),
                })?;
            return Ok(());
        }
    }
    Err(Error::Other(format!(
        "could not find {exe_name} next to the launcher — please reinstall Red Strap"
    )))
}

async fn background_update(
    client: &reqwest::Client,
    layout: &redstrap_core::paths::Layout,
    pub(crate) settings: &redstrap_core::settings::Settings,
) -> Result<()> {
    use redstrap_core::settings::UpdateChannel;

    let channel = settings.update_check;
    if channel == UpdateChannel::Disabled {
        return Ok(());
    }
    let (owner, repo) = match settings.update_repo.split_once('/') {
        Some((o, r)) if !o.is_empty() && !r.is_empty() => (o.trim(), r.trim()),
        _ => {
            tracing::warn!("ignoring malformed update repo '{}'", settings.update_repo);
            return Ok(());
        }
    };

    let current = env!("CARGO_PKG_VERSION");
    let release = match channel {
        UpdateChannel::Disabled => return Ok(()),
        UpdateChannel::Stable => Some(redstrap_core::updater::fetch_latest_release(client, owner, repo).await?),
        UpdateChannel::PreRelease | UpdateChannel::Both => {
            let releases = redstrap_core::updater::fetch_releases(client, owner, repo).await?;
            releases.into_iter().find(|r| {
                channel == UpdateChannel::Both || r.prerelease
            })
        }
    };
    let release = match release {
        Some(r) => r,
        None => return Ok(()),
    };

    if !redstrap_core::updater::is_newer(current, release.version()) {
        tracing::info!("Red Strap is up to date ({current})");
        return Ok(());
    }
    if !settings.background_updates {
        tracing::info!(
            "Red Strap {} is available (background updates disabled)",
            release.version()
        );
        return Ok(());
    }

    tracing::info!("installing Red Strap {} in the background", release.version());
    redstrap_core::updater::download_and_swap(client, &release, |_| {}).await?;
    // Keep the settings UI on the same version when it ships in the release.
    if redstrap_core::updater::update_companion(
        client,
        &release,
        &layout.settings_app(),
        redstrap_core::consts::SETTINGS_EXE,
        |_| {},
    )
    .await?
    {
        tracing::info!("settings UI updated alongside the launcher");
    }
    tracing::info!("background update staged; it applies on next start");
    Ok(())
}
