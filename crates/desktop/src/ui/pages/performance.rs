//! Performance page: frame caps, rendering, quality, OS tweaks.
//!
//! Every control maps to real fast flags, settings-file values, or registry
//! entries applied by the launch pipeline.

use eframe::egui::{self};

use crate::app::RedStrapApp;
use crate::ui::{theme, widgets};

pub(crate) fn show(app: &mut RedStrapApp, ui: &mut egui::Ui) {
    theme::page_title(ui, "Performance", "Frame rate, rendering, and graphics quality");

    widgets::setting_row(
        ui,
        "Use fast-flag manager",
        "Master switch: off leaves Roblox flags completely untouched.",
        |ui| {
            if ui
                .checkbox(&mut app.settings.use_flag_manager, "")
                .changed()
            {
                app.save_settings();
            }
        },
    );
    widgets::setting_row(
        ui,
        "FPS cap",
        "Target frames per second. 0 disables the cap.",
        |ui| {
            if ui
                .add(egui::DragValue::new(&mut app.settings.fps_cap).range(0..=1000))
                .changed()
            {
                app.save_settings();
            }
        },
    );
    widgets::setting_row(
        ui,
        "Rendering backend",
        "Graphics API forced through fast flags.",
        |ui| {
            if widgets::enum_combo(ui, "rendering", &mut app.settings.rendering_mode) {
                app.save_settings();
            }
        },
    );
    widgets::setting_row(ui, "MSAA", "Forced anti-aliasing sample count.", |ui| {
        if widgets::enum_combo(ui, "msaa", &mut app.settings.msaa) {
            app.save_settings();
        }
    });
    widgets::setting_row(
        ui,
        "Fix Alt+Enter",
        "Keep fullscreen working when frame-rate tools are active.",
        |ui| {
            if ui.checkbox(&mut app.settings.alt_enter_fix, "").changed() {
                app.save_settings();
            }
        },
    );

    widgets::section(ui, "Quality trade-offs");
    widgets::setting_row(
        ui,
        "Frame-rate manager quality",
        "Override Roblox's dynamic quality level (1-21). 0 disables.",
        |ui| {
            if ui
                .add(egui::DragValue::new(&mut app.settings.frm_quality).range(0..=21))
                .changed()
            {
                app.save_settings();
            }
        },
    );
    widgets::setting_row(
        ui,
        "Texture quality",
        "Override texture resolution level (0-4). 0 disables.",
        |ui| {
            if ui
                .add(egui::DragValue::new(&mut app.settings.texture_quality).range(0..=4))
                .changed()
            {
                app.save_settings();
            }
        },
    );
    widgets::setting_row(
        ui,
        "Low-poly meshes",
        "Force simplified mesh detail for weaker GPUs.",
        |ui| {
            if ui.checkbox(&mut app.settings.low_poly_meshes, "").changed() {
                app.save_settings();
            }
        },
    );
    widgets::setting_row(
        ui,
        "Remove grass",
        "Disable decorative terrain grass.",
        |ui| {
            if ui.checkbox(&mut app.settings.remove_grass, "").changed() {
                app.save_settings();
            }
        },
    );
    widgets::setting_row(
        ui,
        "Pause voxelizer",
        "Freeze terrain voxel updates (static geometry only).",
        |ui| {
            if ui.checkbox(&mut app.settings.pause_voxelizer, "").changed() {
                app.save_settings();
            }
        },
    );
    widgets::setting_row(
        ui,
        "Disable DPI scaling",
        "Stop Windows from scaling Roblox on high-DPI displays.",
        |ui| {
            if ui.checkbox(&mut app.settings.disable_dpi_scaling, "").changed() {
                app.save_settings();
            }
        },
    );
    widgets::setting_row(
        ui,
        "Gray sky",
        "Replace the skybox with flat gray (slightly cheaper rendering).",
        |ui| {
            if ui.checkbox(&mut app.settings.gray_sky, "").changed() {
                app.save_settings();
            }
        },
    );
    widgets::setting_row(
        ui,
        "Disable VSync",
        "Turn off vertical sync in the client settings file.",
        |ui| {
            if ui.checkbox(&mut app.settings.disable_vsync, "").changed() {
                app.save_settings();
            }
        },
    );

    widgets::section(ui, "Windows graphics");
    widgets::setting_row(
        ui,
        "Preferred GPU",
        "Written to Windows graphics settings per Roblox executable.",
        |ui| {
            if widgets::enum_combo(ui, "gpu", &mut app.settings.gpu_preference) {
                app.save_settings();
            }
        },
    );
    widgets::setting_row(
        ui,
        "Disable fullscreen optimizations",
        "Windows compatibility tweak applied per executable.",
        |ui| {
            if ui
                .checkbox(&mut app.settings.disable_fullscreen_optimizations, "")
                .changed()
            {
                app.save_settings();
            }
        },
    );

    widgets::section(ui, "Client settings file");
    widgets::setting_row(
        ui,
        "Frame cap override",
        "Explicit FramerateCap for GlobalBasicSettings. Empty follows the FPS cap.",
        |ui| {
            let mut enabled = app.settings.gbs_framerate_cap.is_some();
            let mut value = app.settings.gbs_framerate_cap.unwrap_or(240);
            let mut changed = false;
            if ui.checkbox(&mut enabled, "Override").changed() {
                changed = true;
            }
            if enabled {
                if ui
                    .add(egui::DragValue::new(&mut value).range(1..=1000))
                    .changed()
                {
                    changed = true;
                }
            }
            if changed {
                app.settings.gbs_framerate_cap = enabled.then_some(value);
                app.save_settings();
            }
        },
    );

    ui.add_space(12.0);
    ui.horizontal(|ui| {
        let busy = app.applying_now;
        if ui
            .add_enabled(!busy, theme::accent_button("Apply to installed versions"))
            .on_hover_text("Write flags, mods, and tweaks into existing installs right now")
            .clicked()
        {
            app.applying_now = true;
            crate::tasks::spawn_apply_now(
                &app.rt,
                app.tx.clone(),
                app.layout.clone(),
                app.settings.clone(),
            );
        }
        if busy {
            ui.add(egui::Spinner::new());
            ui.label("Applying...");
        }
    });
    ui.add_space(4.0);
    widgets::hint(
        ui,
        "Changes apply automatically on the next launch; Apply pushes them into existing installs immediately.",
    );
}
