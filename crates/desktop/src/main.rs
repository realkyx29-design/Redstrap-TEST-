//! Red Strap settings app: native red + dark desktop UI.
//!
//! Single-instance, tray-capable, and fully offline-friendly: every page
//! works without a network connection except the ones that inherently need
//! it (updates, allowlist, previews).

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod tasks;
mod tray;
mod ui;

use redstrap_core::error::{Error, Result};

fn main() {
    if let Err(e) = run() {
        eprintln!("Red Strap Settings: {e}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let base = redstrap_core::paths::resolve_base_dir();
    redstrap_core::paths::init(&base);
    let layout = redstrap_core::paths::get();
    layout.ensure_dirs()?;

    // Only one settings window at a time.
    let _instance = match redstrap_core::instance::InstanceLock::try_acquire("settings-ui")? {
        Some(guard) => guard,
        None => {
            println!("Red Strap settings is already open.");
            return Ok(());
        }
    };

    let (settings, settings_corrupt) =
        redstrap_core::settings::Settings::load(&layout.settings_file);
    let (ring, _log_guard) = redstrap_core::logging::init(
        &layout.logs,
        "RedStrap-Settings.log",
        500,
        settings.verbose_logging,
    )?;
    // The guard must outlive the process; leaking is intentional and tiny.
    std::mem::forget(_log_guard);
    if settings_corrupt {
        tracing::warn!("settings file was corrupt; defaults restored");
    }
    let (state, _) = redstrap_core::state::State::load(&layout.state_file);
    let (flags, _) = redstrap_core::fastflags::FlagStore::load(&layout.working_flags_file);

    let rt = match tokio::runtime::Runtime::new() {
        Ok(rt) => rt,
        Err(e) => {
            return Err(Error::Other(format!(
                "could not start background runtime: {e}"
            )));
        }
    };
    let http = redstrap_core::http::build_client()?;
    let (tx, rx) = std::sync::mpsc::channel();

    let tray = match tray::Tray::new() {
        Ok(tray) => Some(tray),
        Err(e) => {
            tracing::warn!("tray unavailable ({e}); continuing without it");
            None
        }
    };

    let mut viewport = eframe::egui::ViewportBuilder::default()
        .with_title("Red Strap")
        .with_inner_size([1060.0, 720.0])
        .with_min_inner_size([880.0, 600.0]);
    if let Ok(icon) = eframe::icon_data::from_png_bytes(crate::ui::theme::ICON_PNG) {
        viewport = viewport.with_icon(icon);
    }
    let options = eframe::NativeOptions {
        viewport,
        follow_system_theme: false,
        ..Default::default()
    };

    let layout_owned: redstrap_core::paths::Layout = (*layout).clone();
    let layout = layout_owned;
    eframe::run_native(
        "Red Strap",
        options,
        Box::new(move |cc| {
            let font_warning = crate::ui::theme::apply(
                &cc.egui_ctx,
                settings.ui_scale,
                &settings.custom_font_path,
            );
            if settings_corrupt {
                tracing::warn!("started with repaired default settings");
            }
            let app = app::RedStrapApp::new(
                layout,
                settings,
                state,
                flags,
                rt,
                http,
                tx,
                rx,
                ring,
                tray,
                font_warning,
            );
            Ok(Box::new(app) as Box<dyn eframe::App>)
        }),
    )
    .map_err(|e| Error::Other(format!("could not open the settings window: {e}")))
}
