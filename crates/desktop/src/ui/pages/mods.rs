//! Mods page: file-mod management plus the UI recolor tool.

use eframe::egui::{self, RichText};
use redstrap_core::roblox::LaunchMode;

use crate::app::{ConfirmAction, RedStrapApp};
use crate::ui::{theme, widgets};

pub(crate) fn show(app: &mut RedStrapApp, ui: &mut egui::Ui) {
    theme::page_title(ui, "Mods", "File mods and interface recoloring");

    // -- install ---------------------------------------------------------------
    ui.horizontal(|ui| {
        if ui
            .add_enabled(!app.mod_installing, theme::accent_button("Install from ZIP"))
            .clicked()
        {
            if let Some(zip) = rfd::FileDialog::new()
                .add_filter("ZIP archive", &["zip"])
                .set_title("Install mod from ZIP")
                .pick_file()
            {
                install_zip(app, zip);
            }
        }
        if ui
            .add_enabled(!app.mod_installing, theme::ghost_button("Install files"))
            .on_hover_text("Pick loose files to bundle as a mod")
            .clicked()
        {
            if let Some(files) = rfd::FileDialog::new()
                .set_title("Pick mod files")
                .pick_files()
            {
                if !files.is_empty() {
                    install_paths(app, files);
                }
            }
        }
        if ui
            .add_enabled(!app.mod_installing, theme::ghost_button("Install folder"))
            .on_hover_text("Bundle a whole folder as a mod")
            .clicked()
        {
            if let Some(folder) = rfd::FileDialog::new()
                .set_title("Pick mod folder")
                .pick_folder()
            {
                install_paths(app, vec![folder]);
            }
        }
        if app.mod_installing {
            ui.add(egui::Spinner::new());
            ui.label("Installing...");
        }
    });

    // -- list ---------------------------------------------------------------------
    ui.add_space(4.0);
    widgets::section(ui, &format!("Installed ({})", app.mods.len()));
    if app.mods.is_empty() {
        widgets::hint(ui, "No mods installed. Mods mirror Roblox's version folder layout.");
    }
    let mut delete = None;
    let mut changed = false;
    for m in app.mods.clone() {
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.set_min_width(220.0);
                ui.label(RichText::new(&m.name).strong());
                ui.label(
                    RichText::new(format!(
                        "{} files · {}",
                        m.file_count,
                        redstrap_core::util::format_bytes(m.total_bytes)
                    ))
                    .small()
                    .color(widgets::muted()),
                );
            });
            // Toggle state lives in State; ensure the entry exists.
            app.state.ensure_mod(&m.name);
            if let Some(entry) = app.state.mod_entry_mut(&m.name) {
                if ui.checkbox(&mut entry.enabled, "On").changed() {
                    changed = true;
                }
                if ui.checkbox(&mut entry.player, "Player").changed() {
                    changed = true;
                }
                if ui.checkbox(&mut entry.studio, "Studio").changed() {
                    changed = true;
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.small_button("Delete").clicked() {
                    delete = Some(m.name.clone());
                }
                if ui.small_button("Open folder").clicked() {
                    if let Err(e) = redstrap_core::util::reveal_in_file_manager(&m.path) {
                        app.show_error("Could not open folder", e.to_string());
                    }
                }
            });
        });
        ui.separator();
    }
    if changed {
        app.save_state();
    }
    if let Some(name) = delete {
        app.ask_confirm(
            "Delete mod",
            format!("Delete mod '{name}' and all its files?"),
            "Delete",
            ConfirmAction::DeleteMod(name),
        );
    }

    // -- recolor --------------------------------------------------------------------
    ui.add_space(4.0);
    widgets::section(ui, "Recolor interface");
    widgets::hint(
        ui,
        "Tint Roblox's built-in UI textures (cursors, shift-lock, emote wheel). Re-apply after Roblox updates.",
    );
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        ui.label("Color:");
        if ui
            .color_edit_button_srgba(&mut app.recolor)
            .changed()
        {
            let c = app.recolor;
            app.settings.last_recolor = format!("#{:02x}{:02x}{:02x}", c.r(), c.g(), c.b());
            app.save_settings();
        }
        ui.label(
            RichText::new(&app.settings.last_recolor)
                .monospace()
                .color(widgets::muted()),
        );
    });
    ui.add_space(2.0);
    ui.label(RichText::new("Targets:").strong());
    for (index, group) in redstrap_core::mods::RECOLOR_GROUPS.iter().enumerate() {
        if index < app.recolor_groups.len() {
            ui.checkbox(&mut app.recolor_groups[index], group.label);
        }
    }
    ui.add_space(2.0);
    ui.label(RichText::new("Extra texture paths (one per line, relative to the version folder):").small());
    ui.text_edit_multiline(&mut app.recolor_extras);
    ui.add_space(2.0);
    ui.horizontal(|ui| {
        ui.checkbox(&mut app.recolor_player, "Player");
        ui.checkbox(&mut app.recolor_studio, "Studio");
    });
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        let busy = app.recolor_pending > 0;
        if ui
            .add_enabled(!busy, theme::accent_button("Apply recolor"))
            .clicked()
        {
            apply_recolor(app);
        }
        if busy {
            ui.add(egui::Spinner::new());
            ui.label("Recoloring...");
        }
    });
}

fn install_zip(app: &mut RedStrapApp, zip: std::path::PathBuf) {
    let name = zip
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("mod")
        .to_string();
    app.mod_installing = true;
    crate::tasks::spawn_install_mod_zip(
        &app.rt,
        app.tx.clone(),
        zip,
        app.layout.modifications.clone(),
        name,
    );
}

fn install_paths(app: &mut RedStrapApp, sources: Vec<std::path::PathBuf>) {
    let name = sources
        .first()
        .and_then(|p| p.file_stem())
        .and_then(|s| s.to_str())
        .unwrap_or("mod")
        .to_string();
    app.mod_installing = true;
    crate::tasks::spawn_install_mod_paths(
        &app.rt,
        app.tx.clone(),
        sources,
        app.layout.modifications.clone(),
        name,
    );
}

fn apply_recolor(app: &mut RedStrapApp) {
    let color = app.recolor;
    let rgb = (color.r(), color.g(), color.b());
    let mut groups = Vec::new();
    for (index, group) in redstrap_core::mods::RECOLOR_GROUPS.iter().enumerate() {
        if app.recolor_groups.get(index).copied().unwrap_or(false) {
            groups.push(*group);
        }
    }
    if groups.is_empty() {
        app.show_error("Nothing selected", "Pick at least one recolor target.");
        return;
    }
    let extras: Vec<String> = app
        .recolor_extras
        .lines()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty())
        .collect();

    let mut started = false;
    for (mode, wanted, dist) in [
        (
            LaunchMode::Player,
            app.recolor_player,
            app.player_dist.clone(),
        ),
        (
            LaunchMode::Studio,
            app.recolor_studio,
            app.studio_dist.clone(),
        ),
    ] {
        if !wanted {
            continue;
        }
        if !dist.is_installed() {
            app.toast(format!("{} is not installed — skipped", mode.label()));
            continue;
        }
        let (dir, exe) = redstrap_core::roblox::version_paths(
            &app.layout,
            mode,
            &dist.version_guid,
            app.settings.static_directory,
        );
        if !exe.is_file() {
            app.toast(format!("{} install damaged — skipped", mode.label()));
            continue;
        }
        app.recolor_pending += 1;
        started = true;
        crate::tasks::spawn_recolor(
            &app.rt,
            app.tx.clone(),
            dir,
            mode.label().to_string(),
            rgb,
            groups.clone(),
            extras.clone(),
        );
    }
    if !started {
        app.show_error(
            "Nothing to do",
            "Select Player and/or Studio above (it must be installed).",
        );
    }
}
