//! Logs page: live view of the in-memory log ring.

use eframe::egui::{self, Color32, RichText};

use crate::app::RedStrapApp;
use crate::ui::{theme, widgets};

pub(crate) fn show(app: &mut RedStrapApp, ui: &mut egui::Ui) {
    theme::page_title(ui, "Logs", "This session's log, live");

    ui.horizontal(|ui| {
        ui.label("Level:");
        egui::ComboBox::from_id_source("log-level")
            .selected_text(&app.log_level)
            .show_ui(ui, |ui| {
                for level in ["All", "ERROR", "WARN", "INFO", "DEBUG"] {
                    ui.selectable_value(&mut app.log_level, level.to_string(), level);
                }
            });
        ui.label("Filter:");
        ui.text_edit_singleline(&mut app.log_filter);
        ui.checkbox(&mut app.log_autoscroll, "Follow");
    });
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        if ui.add(theme::ghost_button("Copy visible")).clicked() {
            let text = visible_lines(app)
                .into_iter()
                .map(|(_, rendered)| rendered)
                .collect::<Vec<_>>()
                .join("\n");
            ui.ctx().copy_text(text);
            app.toast("Logs copied");
        }
        if ui.add(theme::ghost_button("Clear view")).clicked() {
            app.ring.clear();
        }
        if ui.add(theme::ghost_button("Open log folder")).clicked() {
            if let Err(e) = redstrap_core::util::reveal_in_file_manager(&app.layout.logs) {
                app.show_error("Could not open folder", e.to_string());
            }
        }
        if ui
            .checkbox(&mut app.settings.verbose_logging, "Verbose logging")
            .on_hover_text("Debug-level output. Takes effect on restart.")
            .changed()
        {
            app.save_settings();
        }
    });
    ui.add_space(6.0);

    let lines = visible_lines(app);
    egui::ScrollArea::vertical()
        .stick_to_bottom(app.log_autoscroll)
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
            for line in &lines {
                let color = level_color(&line.0);
                ui.horizontal(|ui| {
                    ui.label(RichText::new(&line.0).small().color(color).strong());
                    ui.label(RichText::new(&line.1).monospace().small());
                });
            }
            if lines.is_empty() {
                widgets::hint(ui, "No log lines match.");
            }
        });
}

/// (level, rendered) pairs after filtering, newest last, capped.
fn visible_lines(app: &RedStrapApp) -> Vec<(String, String)> {
    const CAP: usize = 500;
    let query = app.log_filter.trim().to_ascii_lowercase();
    let level = app.log_level.clone();
    app.ring
        .snapshot()
        .into_iter()
        .filter(|line| {
            if level != "All" && line.level != level {
                return false;
            }
            if query.is_empty() {
                return true;
            }
            line.message.to_ascii_lowercase().contains(&query)
                || line.target.to_ascii_lowercase().contains(&query)
        })
        .rev()
        .take(CAP)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .map(|line| {
            let rendered = if line.target.is_empty() {
                line.message.clone()
            } else {
                format!("[{}] {}", line.target, line.message)
            };
            (line.level.clone(), rendered)
        })
        .collect()
}

fn level_color(level: &str) -> Color32 {
    match level {
        "ERROR" => widgets::err_red(),
        "WARN" => widgets::warn_amber(),
        "DEBUG" => Color32::from_rgb(0x8A, 0x8A, 0x96),
        _ => Color32::from_rgb(0x8A, 0xE0, 0x9B),
    }
}
