//! Launch page: confirmation, multi-instance, priority, channel, updates.

use eframe::egui::self;

use crate::app::RedStrapApp;
use crate::ui::{theme, widgets};

pub(crate) fn show(app: &mut RedStrapApp, ui: &mut egui::Ui) {
    theme::page_title(ui, "Launch", "How Roblox starts");

    widgets::section(ui, "Behaviour");
    widgets::setting_row(
        ui,
        "Confirm before launching",
        "Ask every time a game or app launch is requested.",
        |ui| {
            if ui
                .checkbox(&mut app.settings.confirm_launches, "")
                .changed()
            {
                app.save_settings();
            }
        },
    );
    widgets::setting_row(
        ui,
        "Process priority",
        "OS scheduling priority applied to Roblox after launch.",
        |ui| {
            if widgets::enum_combo(ui, "priority", &mut app.settings.process_priority) {
                app.save_settings();
            }
        },
    );
    widgets::setting_row(
        ui,
        "Auto-close crash handler",
        "Dismiss Roblox's crash window automatically after launch.",
        |ui| {
            if ui
                .checkbox(&mut app.settings.auto_close_crash_handler, "")
                .changed()
            {
                app.save_settings();
            }
        },
    );

    widgets::section(ui, "Multi-instance");
    widgets::setting_row(
        ui,
        "Launch multiple players",
        "Start several Player instances for the same launch.",
        |ui| {
            if ui
                .checkbox(&mut app.settings.multi_instance_launching, "")
                .changed()
            {
                app.save_settings();
            }
        },
    );
    widgets::setting_row(
        ui,
        "Instance count",
        "How many players to start (1-8).",
        |ui| {
            if ui
                .add(
                    egui::DragValue::new(&mut app.settings.instances_count)
                        .range(1..=8),
                )
                .changed()
            {
                app.save_settings();
            }
        },
    );
    widgets::setting_row(
        ui,
        "Delay between instances",
        "Milliseconds to wait before starting the next one.",
        |ui| {
            if ui
                .add(
                    egui::DragValue::new(&mut app.settings.instance_delay_ms)
                        .range(0..=30_000),
                )
                .changed()
            {
                app.save_settings();
            }
        },
    );

    widgets::section(ui, "Roblox deployment");
    widgets::setting_row(ui, "Channel", "Release channel, e.g. production.", |ui| {
        ui.horizontal(|ui| {
            let response = ui.text_edit_singleline(&mut app.settings.channel);
            if response.changed() {
                app.channel_check = None;
                app.save_settings();
            }
            let checking = app.channel_checking;
            if ui
                .add_enabled(!checking, theme::ghost_button("Validate"))
                .clicked()
            {
                app.channel_checking = true;
                app.channel_check = None;
                crate::tasks::spawn_channel_check(
                    &app.rt,
                    app.http.clone(),
                    app.tx.clone(),
                    app.settings.roblox_domain.clone(),
                    app.settings.channel.clone(),
                    app.settings.channel_token.clone(),
                );
            }
        });
        if app.channel_checking {
            ui.add(egui::Spinner::new());
        }
        if let Some((ok, detail)) = app.channel_check.clone() {
            if ok {
                widgets::status_pill(ui, &format!("OK: {detail}"), widgets::ok_green());
            } else {
                widgets::status_pill(ui, &detail, widgets::err_red());
            }
        }
    });
    widgets::setting_row(
        ui,
        "Channel token",
        "Private-channel access token (rarely needed).",
        |ui| {
            if ui
                .text_edit_singleline(&mut app.settings.channel_token)
                .changed()
            {
                app.save_settings();
            }
        },
    );
    widgets::setting_row(
        ui,
        "Roblox domain",
        "API domain for version checks and game data.",
        |ui| {
            if ui
                .text_edit_singleline(&mut app.settings.roblox_domain)
                .changed()
            {
                app.save_settings();
            }
        },
    );
    widgets::setting_row(
        ui,
        "Keep Roblox updated",
        "Install new versions automatically on launch. Off reuses the installed build.",
        |ui| {
            if ui
                .checkbox(&mut app.settings.auto_update_roblox, "")
                .changed()
            {
                app.save_settings();
            }
        },
    );
    widgets::setting_row(
        ui,
        "Static install directory",
        "Install into Versions/Roblox instead of a versioned folder (helps some tools).",
        |ui| {
            if ui
                .checkbox(&mut app.settings.static_directory, "")
                .changed()
            {
                app.save_settings();
            }
        },
    );
    widgets::setting_row(
        ui,
        "Custom launch command",
        "Prefix for non-Windows systems, e.g. wine. Empty runs the binary directly.",
        |ui| {
            if ui
                .text_edit_singleline(&mut app.settings.custom_launch_command)
                .changed()
            {
                app.save_settings();
            }
        },
    );

    widgets::section(ui, "Cleaning");
    widgets::setting_row(
        ui,
        "Clean Roblox debris",
        "Remove temp files and stale logs around sessions.",
        |ui| {
            if widgets::enum_combo(ui, "cleaner", &mut app.settings.cleaner) {
                app.save_settings();
            }
        },
    );
    widgets::setting_row(
        ui,
        "Extra directories",
        "Additional folders whose contents are wiped when cleaning.",
        |ui| {
            ui.vertical(|ui| {
                let mut remove = None;
                for (index, dir) in app.settings.cleaner_extra_dirs.iter_mut().enumerate() {
                    ui.horizontal(|ui| {
                        if ui.text_edit_singleline(dir).changed() {
                            app.save_settings();
                        }
                        if ui.small_button("Remove").clicked() {
                            remove = Some(index);
                        }
                    });
                }
                if let Some(index) = remove {
                    app.settings.cleaner_extra_dirs.remove(index);
                    app.save_settings();
                }
                if ui.add(theme::ghost_button("Add folder")).clicked() {
                    if let Some(folder) = rfd::FileDialog::new().pick_folder() {
                        app.settings
                            .cleaner_extra_dirs
                            .push(folder.to_string_lossy().into_owned());
                        app.save_settings();
                    }
                }
            });
        },
    );

    widgets::section(ui, "Red Strap updates");
    widgets::setting_row(
        ui,
        "Check for",
        "Which releases the self-updater considers.",
        |ui| {
            if widgets::enum_combo(ui, "update-channel", &mut app.settings.update_check) {
                app.save_settings();
            }
        },
    );
    widgets::setting_row(
        ui,
        "Install in background",
        "Stage new versions silently; they apply on next start.",
        |ui| {
            if ui
                .checkbox(&mut app.settings.background_updates, "")
                .changed()
            {
                app.save_settings();
            }
        },
    );
    widgets::setting_row(ui, "Release source", "GitHub owner/repo to check.", |ui| {
        if ui
            .text_edit_singleline(&mut app.settings.update_repo)
            .changed()
        {
            app.save_settings();
        }
    });
    ui.add_space(4.0);
    widgets::hint(
        ui,
        "Update checks run at startup and from the About page.",
    );
}
