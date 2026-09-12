//! Appearance page: scaling, custom font, tray behaviour, palette.

use eframe::egui::{self, Color32, RichText};

use crate::app::RedStrapApp;
use crate::ui::{theme, widgets};

pub(crate) fn show(app: &mut RedStrapApp, ui: &mut egui::Ui, ctx: &egui::Context) {
    theme::page_title(ui, "Appearance", "Scale, font, and tray behaviour");

    widgets::section(ui, "Interface scale");
    widgets::setting_row(
        ui,
        "UI scale",
        "Scales every control live (0.5 - 3.0).",
        |ui| {
            let mut scale = app.settings.ui_scale;
            if ui
                .add(egui::Slider::new(&mut scale, 0.5..=3.0).step_by(0.05))
                .changed()
            {
                app.settings.ui_scale = scale;
                app.save_settings();
                theme::apply(ctx, scale, &app.settings.custom_font_path);
            }
        },
    );

    widgets::section(ui, "Font");
    widgets::setting_row(
        ui,
        "Custom font",
        "TTF or OTF file replacing Inter. Empty uses the bundled font.",
        |ui| {
            ui.horizontal(|ui| {
                if ui
                    .text_edit_singleline(&mut app.settings.custom_font_path)
                    .changed()
                {
                    app.save_settings();
                }
                if ui.small_button("Browse").clicked() {
                    if let Some(file) = rfd::FileDialog::new()
                        .add_filter("Fonts", &["ttf", "otf"])
                        .pick_file()
                    {
                        app.settings.custom_font_path =
                            file.to_string_lossy().into_owned();
                        app.save_settings();
                        apply_font(app, ctx);
                    }
                }
                if ui.small_button("Apply").clicked() {
                    apply_font(app, ctx);
                }
                if ui.small_button("Reset").clicked() {
                    app.settings.custom_font_path.clear();
                    app.save_settings();
                    apply_font(app, ctx);
                }
            });
        },
    );
    ui.add_space(2.0);
    ui.label(RichText::new("The quick brown fox jumps over 0123456789").size(16.0));
    ui.label(RichText::new("fn main() { println!(\"mono\"); }").monospace());

    widgets::section(ui, "Tray");
    widgets::setting_row(
        ui,
        "Minimize to tray",
        "Closing the window hides it instead of quitting (tray icon stays).",
        |ui| {
            if ui
                .checkbox(&mut app.settings.minimize_to_tray, "")
                .changed()
            {
                app.save_settings();
            }
        },
    );

    widgets::section(ui, "Palette");
    ui.horizontal(|ui| {
        swatch(ui, "Accent", theme::ACCENT);
        swatch(ui, "Hover", theme::ACCENT_HOVER);
        swatch(ui, "Active", theme::ACCENT_ACTIVE);
        swatch(ui, "OK", widgets::ok_green());
        swatch(ui, "Warning", widgets::warn_amber());
        swatch(ui, "Error", widgets::err_red());
    });
}

fn apply_font(app: &mut RedStrapApp, ctx: &egui::Context) {
    if let Some(warning) = theme::apply(
        ctx,
        app.settings.ui_scale,
        &app.settings.custom_font_path,
    ) {
        app.show_error("Font not applied", warning);
    } else {
        app.toast("Font applied");
    }
}

fn swatch(ui: &mut egui::Ui, label: &str, color: Color32) {
    ui.vertical(|ui| {
        let (rect, _) = ui.allocate_exact_size(egui::vec2(44.0, 28.0), egui::Sense::hover());
        ui.painter()
            .rect_filled(rect, egui::Rounding::same(6.0), color);
        ui.label(RichText::new(label).small());
    });
}
