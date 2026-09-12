//! About page: version, updates, links, danger zone.

use eframe::egui::{self, RichText};

use crate::app::{ConfirmAction, RedStrapApp};
use crate::ui::{theme, widgets};

pub(crate) fn show(app: &mut RedStrapApp, ui: &mut egui::Ui) {
    theme::page_title(ui, "About", "Version, updates, and links");

    ui.label(
        RichText::new(format!("Red Strap v{}", env!("CARGO_PKG_VERSION")))
            .size(20.0)
            .strong(),
    );
    ui.label("A fast, lightweight Roblox bootstrapper written in Rust.");
    ui.add_space(2.0);
    ui.horizontal(|ui| {
        ui.hyperlink_to("Source code", "https://github.com/realkyx29-design/Redstrap-TEST-");
        ui.label("·");
        ui.hyperlink_to("Report an issue", "https://github.com/realkyx29-design/Redstrap-TEST-/issues");
    });

    widgets::section(ui, "Updates");
    ui.horizontal(|ui| {
        let busy = app.update_checking || app.update_staging;
        if ui
            .add_enabled(!busy, theme::ghost_button("Check for updates"))
            .clicked()
        {
            app.start_update_check();
        }
        if app.update_checking {
            ui.add(egui::Spinner::new());
            ui.label("Checking...");
        }
    });
    if let Some(error) = app.update_error.clone() {
        widgets::status_pill(ui, &format!("Check failed: {error}"), widgets::err_red());
    }
    if let Some(release) = app.update_release.clone() {
        ui.add_space(4.0);
        widgets::status_pill(
            ui,
            &format!(
                "Red Strap {} available{}",
                release.version,
                if release.prerelease { " (pre-release)" } else { "" }
            ),
            widgets::ok_green(),
        );
        ui.add_space(2.0);
        ui.horizontal(|ui| {
            let busy = app.update_staging;
            if ui
                .add_enabled(!busy, theme::accent_button("Download and install"))
                .clicked()
            {
                app.update_staging = true;
                crate::tasks::spawn_stage_update(
                    &app.rt,
                    app.http.clone(),
                    app.tx.clone(),
                    release,
                    app.layout.application.clone(),
                );
            }
            if ui
                .add_enabled(!busy, theme::ghost_button("Skip this version"))
                .on_hover_text("Don't offer this version again")
                .clicked()
            {
                let version = release.version.clone();
                app.state.skipped_update_version = version.clone();
                app.save_state();
                app.update_release = None;
                app.update_skipped = Some(version.clone());
                app.toast(format!("Skipped Red Strap {version}"));
            }
            if busy {
                ui.add(egui::Spinner::new());
                ui.label("Downloading...");
            }
        });
    } else if let Some(skipped) = app.update_skipped.clone() {
        if !app.update_checking {
            widgets::status_pill(
                ui,
                &format!("Red Strap {skipped} available — skipped"),
                widgets::warn_amber(),
            );
            if ui.add(theme::ghost_button("Unskip")).clicked() {
                app.state.skipped_update_version.clear();
                app.save_state();
                app.update_skipped = None;
                app.start_update_check();
            }
        }
    } else if !app.update_checking && app.update_error.is_none() {
        widgets::hint(ui, "You are on the latest version.");
    }
    if let Some(staged) = app.update_staged.clone() {
        ui.add_space(4.0);
        widgets::status_pill(
            ui,
            &format!("Red Strap {staged} staged — restart to apply"),
            widgets::warn_amber(),
        );
        if ui.add(theme::accent_button("Restart now")).clicked() {
            restart_into_new_binary(app);
        }
    }

    widgets::section(ui, "Installation");
    ui.horizontal(|ui| {
        ui.label(RichText::new(app.layout.base.to_string_lossy().as_ref()).monospace().small());
        if ui.small_button("Open folder").clicked() {
            if let Err(e) = redstrap_core::util::reveal_in_file_manager(&app.layout.base) {
                app.show_error("Could not open folder", e.to_string());
            }
        }
    });

    widgets::section(ui, "Credits");
    ui.label("Interface font: Inter (SIL Open Font License 1.1).");
    ui.label("Code font: JetBrains Mono (SIL Open Font License 1.1).");
    ui.label("Built with Rust, egui, and Tokio.");

    widgets::section(ui, "Danger zone");
    if ui
        .add(theme::ghost_button("Uninstall Red Strap"))
        .on_hover_text("Remove Red Strap and all downloaded Roblox versions")
        .clicked()
    {
        app.ask_confirm(
            "Uninstall Red Strap",
            "Remove Red Strap, all settings, and all downloaded Roblox versions? Running games will be closed.",
            "Uninstall",
            ConfirmAction::Uninstall,
        );
    }
}

/// Relaunch the (freshly swapped) launcher binary, then exit this process.
fn restart_into_new_binary(app: &mut RedStrapApp) {
    let mut command = std::process::Command::new(&app.layout.application);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    match command.spawn() {
        Ok(_) => std::process::exit(0),
        Err(e) => app.show_error("Could not restart", e.to_string()),
    }
}
