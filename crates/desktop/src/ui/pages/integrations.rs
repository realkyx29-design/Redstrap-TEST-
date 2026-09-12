//! Integrations page: activity tracking, Discord RPC, auto-rejoin,
//! and external programs launched alongside Roblox.

use eframe::egui::{self};

use crate::app::RedStrapApp;
use crate::ui::{theme, widgets};

pub(crate) fn show(app: &mut RedStrapApp, ui: &mut egui::Ui) {
    theme::page_title(ui, "Integrations", "Activity, Discord, rejoin, external tools");

    widgets::section(ui, "Activity");
    widgets::setting_row(
        ui,
        "Track activity",
        "Resolve game names and feed the session watcher.",
        |ui| {
            if ui.checkbox(&mut app.settings.activity_tracking, "").changed() {
                app.save_settings();
            }
        },
    );
    widgets::setting_row(
        ui,
        "Playtime counter",
        "Accumulate total playtime shown on the overview.",
        |ui| {
            if ui.checkbox(&mut app.settings.playtime_counter, "").changed() {
                app.save_settings();
            }
        },
    );
    widgets::setting_row(
        ui,
        "Show server details",
        "Record the live server snapshot the overview displays.",
        |ui| {
            if ui.checkbox(&mut app.settings.show_server_details, "").changed() {
                app.save_settings();
            }
        },
    );
    widgets::setting_row(
        ui,
        "Auto-rejoin",
        "Rejoin automatically after crashes and connection loss (up to 5 times).",
        |ui| {
            if ui.checkbox(&mut app.settings.auto_rejoin, "").changed() {
                app.save_settings();
            }
        },
    );
    widgets::setting_row(
        ui,
        "Close on leave",
        "Quit Roblox when you leave the game (skips the home screen).",
        |ui| {
            if ui
                .checkbox(&mut app.settings.close_on_leave_game, "")
                .changed()
            {
                app.save_settings();
            }
        },
    );

    widgets::section(ui, "Discord Rich Presence");
    widgets::setting_row(
        ui,
        "Enable Discord RPC",
        "Publish the current game to Discord while playing.",
        |ui| {
            if ui.checkbox(&mut app.settings.discord_rpc, "").changed() {
                app.save_settings();
            }
        },
    );
    widgets::setting_row(
        ui,
        "Application ID",
        "Your Discord application's client ID (required for RPC).",
        |ui| {
            if ui
                .text_edit_singleline(&mut app.settings.discord_client_id)
                .changed()
            {
                app.save_settings();
            }
        },
    );
    if app.settings.discord_client_id.trim().is_empty() {
        widgets::hint(
            ui,
            "Create an application at discord.com/developers and paste its Application ID here.",
        );
    }
    widgets::setting_row(
        ui,
        "Hide buttons",
        "Omit the View Game button from the presence.",
        |ui| {
            if ui.checkbox(&mut app.settings.rpc_hide_buttons, "").changed() {
                app.save_settings();
            }
        },
    );
    widgets::setting_row(
        ui,
        "Show game name",
        "Include the current game's name in the presence.",
        |ui| {
            if ui.checkbox(&mut app.settings.rpc_show_game_name, "").changed() {
                app.save_settings();
            }
        },
    );
    widgets::setting_row(
        ui,
        "Show account",
        "Include your Roblox username in the presence.",
        |ui| {
            if ui.checkbox(&mut app.settings.rpc_show_account, "").changed() {
                app.save_settings();
            }
        },
    );

    widgets::section(ui, "External programs");
    widgets::hint(ui, "Programs started automatically alongside Roblox.");
    ui.add_space(4.0);
    let mut changed = false;
    let mut remove = None;
    let mut browse = None;
    for (index, integration) in app.settings.custom_integrations.iter_mut().enumerate() {
        egui::Frame::group(ui.style())
            .inner_margin(egui::Margin::same(8))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label("Name:");
                    if ui.text_edit_singleline(&mut integration.name).changed() {
                        changed = true;
                    }
                    if ui.checkbox(&mut integration.enabled, "Enabled").changed() {
                        changed = true;
                    }
                });
                ui.horizontal(|ui| {
                    ui.label("Program:");
                    if ui.text_edit_singleline(&mut integration.exe).changed() {
                        changed = true;
                    }
                    if ui.small_button("Browse").clicked() {
                        browse = Some(index);
                    }
                });
                ui.horizontal(|ui| {
                    ui.label("Arguments:");
                    if ui.text_edit_singleline(&mut integration.args).changed() {
                        changed = true;
                    }
                    if ui.small_button("Remove").clicked() {
                        remove = Some(index);
                    }
                });
            });
        ui.add_space(4.0);
    }
    if let Some(index) = browse {
        if let Some(file) = rfd::FileDialog::new().pick_file() {
            if let Some(integration) = app.settings.custom_integrations.get_mut(index) {
                integration.exe = file.to_string_lossy().into_owned();
                if integration.name.trim().is_empty()
                    || integration.name == "Custom tool"
                {
                    if let Some(stem) = file.file_stem().and_then(|s| s.to_str()) {
                        integration.name = stem.to_string();
                    }
                }
                changed = true;
            }
        }
    }
    if let Some(index) = remove {
        app.settings.custom_integrations.remove(index);
        changed = true;
    }
    if ui.add(theme::ghost_button("Add program")).clicked() {
        app.settings
            .custom_integrations
            .push(redstrap_core::settings::CustomIntegration::default());
        changed = true;
    }
    if changed {
        app.save_settings();
    }
}
