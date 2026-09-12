//! Shortcuts page: managed launcher entries and per-game join links.

use eframe::egui::{self, RichText};

use crate::app::{ConfirmAction, RedStrapApp};
use crate::ui::{theme, widgets};

pub(crate) fn show(app: &mut RedStrapApp, ui: &mut egui::Ui, _ctx: &egui::Context) {
    theme::page_title(ui, "Shortcuts", "Launcher entries and game join links");

    widgets::section(ui, "Launcher shortcuts");
    widgets::hint(ui, "Toggles create or delete the entries immediately.");
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        if ui
            .checkbox(&mut app.settings.shortcut_desktop, "Desktop")
            .changed()
        {
            sync_managed(app);
        }
        if ui
            .checkbox(&mut app.settings.shortcut_start_menu, "Start Menu")
            .changed()
        {
            sync_managed(app);
        }
    });
    ui.add_space(2.0);
    ui.horizontal(|ui| {
        if ui
            .checkbox(&mut app.settings.shortcut_player, "Player")
            .changed()
        {
            sync_managed(app);
        }
        if ui
            .checkbox(&mut app.settings.shortcut_studio, "Studio")
            .changed()
        {
            sync_managed(app);
        }
        if ui
            .checkbox(&mut app.settings.shortcut_settings, "Settings")
            .changed()
        {
            sync_managed(app);
        }
    });

    widgets::section(ui, "New game shortcut");
    ui.horizontal(|ui| {
        ui.label("Name:");
        ui.text_edit_singleline(&mut app.gs_name);
        ui.label("Place ID:");
        ui.text_edit_singleline(&mut app.gs_place);
    });
    ui.horizontal(|ui| {
        ui.label("Job ID:");
        ui.text_edit_singleline(&mut app.gs_job);
        ui.label("Access code:");
        ui.text_edit_singleline(&mut app.gs_access);
    });
    widgets::hint(ui, "Job ID joins a specific server; access code joins a private server. Both optional.");
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        let busy = app.gs_loading || app.gs_creating;
        if ui
            .add_enabled(!busy, theme::ghost_button("Preview"))
            .clicked()
        {
            let place = app.gs_place.trim().parse::<u64>().unwrap_or(0);
            if place == 0 {
                app.show_error("Invalid Place ID", "Enter a numeric Place ID first.");
            } else {
                app.gs_loading = true;
                app.gs_preview = None;
                crate::tasks::spawn_game_preview(
                    &app.rt,
                    app.http.clone(),
                    app.tx.clone(),
                    app.settings.roblox_domain.clone(),
                    place,
                );
            }
        }
        if ui
            .add_enabled(!busy, theme::accent_button("Create shortcut"))
            .clicked()
        {
            let name = app.gs_name.trim().to_string();
            let place = app.gs_place.trim().parse::<u64>().unwrap_or(0);
            if name.is_empty() {
                app.show_error("Invalid shortcut", "Give the shortcut a name first.");
            } else if place == 0 {
                app.show_error("Invalid Place ID", "Enter a numeric Place ID first.");
            } else {
                app.gs_creating = true;
                crate::tasks::spawn_game_shortcut(
                    &app.rt,
                    app.http.clone(),
                    app.tx.clone(),
                    app.layout.clone(),
                    app.settings.clone(),
                    name,
                    place,
                    app.gs_job.trim().to_string(),
                    app.gs_access.trim().to_string(),
                );
            }
        }
        if busy {
            ui.add(egui::Spinner::new());
        }
    });

    if let Some(preview) = app.gs_preview.clone() {
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            if let Some(texture) = preview.texture.clone() {
                ui.image(egui::load::SizedTexture::new(
                    texture.id(),
                    egui::vec2(64.0, 64.0),
                ));
            }
            ui.vertical(|ui| {
                if preview.name.is_empty() {
                    ui.label(RichText::new("Unknown game").strong());
                    widgets::hint(ui, "Could not resolve this Place ID — the shortcut still works.");
                } else {
                    ui.label(RichText::new(&preview.name).size(16.0).strong());
                }
                ui.label(
                    RichText::new(format!("Place {}", preview.place_id))
                        .monospace()
                        .small()
                        .color(widgets::muted()),
                );
                if app.gs_name.trim().is_empty() && !preview.name.is_empty() {
                    if ui.small_button("Use as name").clicked() {
                        app.gs_name = preview.name.clone();
                    }
                }
            });
        });
    }

    widgets::section(ui, &format!("Saved ({})", app.game_shortcuts.len()));
    if app.game_shortcuts.is_empty() {
        widgets::hint(ui, "No game shortcuts yet.");
    }
    let mut delete = None;
    for (record, shortcut) in app.game_shortcuts.clone() {
        ui.horizontal(|ui| {
            ui.label(RichText::new(&shortcut.name).strong());
            ui.label(
                RichText::new(format!("place {}", shortcut.place_id))
                    .small()
                    .color(widgets::muted()),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.small_button("Delete").clicked() {
                    delete = Some(record.clone());
                }
            });
        });
        ui.separator();
    }
    if let Some(record) = delete {
        app.ask_confirm(
            "Delete game shortcut",
            "Delete this shortcut, its icon, and its saved record?",
            "Delete",
            ConfirmAction::DeleteGameShortcut(record),
        );
    }
}

fn sync_managed(app: &mut RedStrapApp) {
    app.save_settings();
    if let Err(e) = redstrap_core::shortcuts::sync_managed(&app.layout, &app.settings) {
        app.show_error("Could not sync shortcuts", e.to_string());
    } else {
        app.toast("Shortcuts updated");
    }
}
