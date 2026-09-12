//! Fast Flags page: editor with search, undo, allowlist validation,
//! import/export, and named profiles.

use eframe::egui::{self, Color32, RichText};

use crate::app::{ConfirmAction, RedStrapApp};
use crate::ui::{theme, widgets};

pub(crate) fn show(app: &mut RedStrapApp, ui: &mut egui::Ui) {
    theme::page_title(ui, "Fast Flags", "ClientAppSettings editor with validation");

    if !app.settings.use_flag_manager {
        widgets::status_pill(
            ui,
            "Flag manager is off — flags are ignored until re-enabled in Performance.",
            widgets::warn_amber(),
        );
        ui.add_space(8.0);
    }

    // -- toolbar ------------------------------------------------------------
    ui.horizontal(|ui| {
        ui.label("Search:");
        ui.text_edit_singleline(&mut app.flag_search);
        if ui.small_button("Clear").clicked() {
            app.flag_search.clear();
        }
    });
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        if ui
            .add_enabled(app.flags.can_undo(), theme::ghost_button("Undo"))
            .clicked()
        {
            app.flags.undo();
            app.save_flags();
        }
        if ui
            .add_enabled(app.flags.can_redo(), theme::ghost_button("Redo"))
            .clicked()
        {
            app.flags.redo();
            app.save_flags();
        }
        if ui.add(theme::ghost_button("Import")).clicked() {
            import_flags(app);
        }
        if ui.add(theme::ghost_button("Export")).clicked() {
            export_flags(app);
        }
        if ui
            .add(theme::ghost_button("Reset all"))
            .on_hover_text("Delete every fast flag")
            .clicked()
        {
            let count = app.flags.len();
            app.ask_confirm(
                "Reset fast flags",
                format!("Delete all {count} fast flags? This cannot be undone."),
                "Delete all",
                ConfirmAction::ResetAllFlags,
            );
        }
    });

    // -- allowlist ------------------------------------------------------------
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        let loading = app.allowlist_loading;
        if ui
            .add_enabled(!loading, theme::ghost_button("Fetch allowlist"))
            .on_hover_text("Download the flags Roblox currently honours")
            .clicked()
        {
            app.allowlist_loading = true;
            crate::tasks::spawn_allowlist(
                &app.rt,
                app.http.clone(),
                app.tx.clone(),
                app.settings.roblox_domain.clone(),
                app.settings.channel.clone(),
            );
        }
        if loading {
            ui.add(egui::Spinner::new());
        }
        if app.allowlist.is_some() {
            if ui
                .checkbox(&mut app.flag_unknown_only, "Unknown only")
                .changed()
            {}
            let unknown = unknown_count(app);
            ui.label(format!("{unknown} unknown"));
            if unknown > 0 {
                if ui
                    .add(theme::ghost_button(&format!("Remove {unknown} unknown")))
                    .clicked()
                {
                    app.ask_confirm(
                        "Remove unknown flags",
                        format!(
                            "{unknown} flags are not in the allowlist and are ignored by Roblox. Remove them?"
                        ),
                        "Remove",
                        ConfirmAction::RemoveUnknownFlags(unknown),
                    );
                }
            }
        }
    });

    // -- add ------------------------------------------------------------------
    ui.add_space(4.0);
    widgets::section(ui, "Add flag");
    ui.horizontal(|ui| {
        ui.label("Name:");
        ui.text_edit_singleline(&mut app.flag_new_name);
        ui.label("Value:");
        ui.text_edit_singleline(&mut app.flag_new_value);
        if ui.add(theme::accent_button("Add")).clicked() {
            let name = app.flag_new_name.trim().to_string();
            let value = app.flag_new_value.trim().to_string();
            if name.is_empty() {
                app.show_error("Invalid flag", "Flag name must not be empty.");
            } else if value.is_empty() {
                app.show_error("Invalid flag", "Flag value must not be empty.");
            } else {
                app.flags.set(&name, Some(&value));
                app.save_flags();
                app.flag_new_name.clear();
                app.toast(format!("Flag '{name}' added"));
            }
        }
    });

    // -- edit panel --------------------------------------------------------------
    if let Some(name) = app.editing_flag.clone() {
        ui.add_space(4.0);
        widgets::section(ui, "Edit flag");
        ui.horizontal(|ui| {
            ui.label(RichText::new(&name).monospace().strong());
            ui.label("Value:");
            ui.text_edit_singleline(&mut app.editing_value);
            if ui.add(theme::accent_button("Save")).clicked() {
                let value = app.editing_value.trim().to_string();
                if value.is_empty() {
                    app.show_error("Invalid flag", "Flag value must not be empty.");
                } else {
                    app.flags.set(&name, Some(&value));
                    app.save_flags();
                    app.editing_flag = None;
                    app.toast(format!("Flag '{name}' updated"));
                }
            }
            if ui.add(theme::ghost_button("Cancel")).clicked() {
                app.editing_flag = None;
            }
        });
    }

    // -- profiles ---------------------------------------------------------------
    ui.add_space(4.0);
    widgets::section(ui, "Profiles");
    ui.horizontal(|ui| {
        ui.text_edit_singleline(&mut app.profile_name);
        if ui.add(theme::ghost_button("Save as")).clicked() {
            let name = app.profile_name.trim().to_string();
            if name.is_empty() {
                app.show_error("Invalid profile", "Profile name must not be empty.");
            } else {
                match redstrap_core::fastflags::save_profile(
                    &app.layout.flag_profiles,
                    &name,
                    &app.flags,
                ) {
                    Ok(_) => app.toast(format!("Profile '{name}' saved")),
                    Err(e) => app.show_error("Could not save profile", e.to_string()),
                }
            }
        }
    });
    ui.add_space(2.0);
    let profiles = redstrap_core::fastflags::list_profiles(&app.layout.flag_profiles);
    if profiles.is_empty() {
        widgets::hint(ui, "No saved profiles yet.");
    } else {
        for profile in profiles {
            ui.horizontal(|ui| {
                ui.label(RichText::new(&profile).strong());
                if ui.small_button("Load").clicked() {
                    let (store, _) = redstrap_core::fastflags::load_profile(
                        &app.layout.flag_profiles,
                        &profile,
                    );
                    app.flags = store;
                    app.save_flags();
                    app.toast(format!("Profile '{profile}' loaded"));
                }
                if ui.small_button("Delete").clicked() {
                    app.ask_confirm(
                        "Delete profile",
                        format!("Delete profile '{profile}'?"),
                        "Delete",
                        ConfirmAction::DeleteProfile(profile.clone()),
                    );
                }
            });
        }
    }

    // -- list ---------------------------------------------------------------------
    ui.add_space(4.0);
    widgets::section(ui, &format!("Flags ({})", app.flags.len()));
    let query = app.flag_search.trim().to_ascii_lowercase();
    let allow = app.allowlist.clone();
    let unknown_only = app.flag_unknown_only;
    let mut rows: Vec<(String, String, bool)> = app
        .flags
        .iter()
        .filter(|(name, value)| {
            if unknown_only && is_known(allow.as_ref(), name) {
                return false;
            }
            if query.is_empty() {
                return true;
            }
            name.to_ascii_lowercase().contains(&query)
                || value.to_ascii_lowercase().contains(&query)
        })
        .map(|(name, value)| {
            let known = is_known(allow.as_ref(), name);
            (name.clone(), value.clone(), known)
        })
        .collect();
    rows.sort_by(|a, b| a.0.to_ascii_lowercase().cmp(&b.0.to_ascii_lowercase()));

    if rows.is_empty() {
        widgets::hint(ui, "No flags match.");
    } else {
        egui::ScrollArea::vertical()
            .max_height(340.0)
            .auto_shrink([false, false])
            .show(ui, |ui| {
                let mut delete = None;
                let mut edit = None;
                for (name, value, known) in &rows {
                    ui.horizontal(|ui| {
                        if allow.is_some() && !known {
                            ui.label(
                                RichText::new("?")
                                    .color(widgets::warn_amber())
                                    .strong(),
                            )
                            .on_hover_text("Not in the allowlist — Roblox ignores it");
                        }
                        ui.label(RichText::new(name).monospace());
                        ui.label(RichText::new("=").color(widgets::muted()));
                        ui.label(
                            RichText::new(value).monospace().color(Color32::from_rgb(
                                0x8A, 0xE0, 0x9B,
                            )),
                        );
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.small_button("Delete").clicked() {
                                delete = Some(name.clone());
                            }
                            if ui.small_button("Edit").clicked() {
                                edit = Some((name.clone(), value.clone()));
                            }
                        });
                    });
                    ui.separator();
                }
                if let Some(name) = delete {
                    app.flags.set(&name, None);
                    app.save_flags();
                }
                if let Some((name, value)) = edit {
                    app.editing_flag = Some(name);
                    app.editing_value = value;
                }
            });
    }
}

fn is_known(allow: Option<&std::collections::BTreeSet<String>>, name: &str) -> bool {
    match allow {
        None => true,
        Some(set) => {
            let lower = name.to_ascii_lowercase();
            set.iter().any(|s| s.to_ascii_lowercase() == lower)
        }
    }
}

fn unknown_count(app: &RedStrapApp) -> usize {
    match &app.allowlist {
        None => 0,
        Some(allow) => {
            let (_, unknown) = app.flags.partition_by_allowlist(allow);
            unknown.len()
        }
    }
}

fn import_flags(app: &mut RedStrapApp) {
    let Some(path) = rfd::FileDialog::new()
        .add_filter("JSON", &["json"])
        .set_title("Import fast flags")
        .pick_file()
    else {
        return;
    };
    let bytes = match std::fs::read(&path) {
        Ok(b) => b,
        Err(e) => {
            app.show_error("Could not read file", e.to_string());
            return;
        }
    };
    let value: serde_json::Value = match serde_json::from_slice(&bytes) {
        Ok(v) => v,
        Err(e) => {
            app.show_error("Invalid JSON", e.to_string());
            return;
        }
    };
    let obj = match value.as_object() {
        Some(o) => o,
        None => {
            app.show_error("Invalid flags file", "Expected a JSON object of name/value pairs.");
            return;
        }
    };
    let mut count = 0;
    for (key, val) in obj {
        let text = match val {
            serde_json::Value::String(s) => s.clone(),
            serde_json::Value::Null => continue,
            other => other.to_string(),
        };
        app.flags.set(key, Some(text.as_str()));
        count += 1;
    }
    app.save_flags();
    app.toast(format!("Imported {count} flags"));
}

fn export_flags(app: &mut RedStrapApp) {
    let Some(path) = rfd::FileDialog::new()
        .add_filter("JSON", &["json"])
        .set_file_name("ClientAppSettings.json")
        .set_title("Export fast flags")
        .save_file()
    else {
        return;
    };
    let mut map = serde_json::Map::new();
    for (name, value) in app.flags.iter() {
        map.insert(name.clone(), serde_json::Value::String(value.clone()));
    }
    let text = serde_json::Value::Object(map);
    let pretty = serde_json::to_string_pretty(&text).unwrap_or_else(|_| String::from("{}"));
    if let Err(e) = std::fs::write(&path, pretty) {
        app.show_error("Could not write file", e.to_string());
    } else {
        app.toast("Flags exported");
    }
}
