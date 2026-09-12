//! Overview: install status, quick launch, live session, playtime.

use eframe::egui::{self, Color32, RichText};
use redstrap_core::roblox::LaunchMode;
use redstrap_core::state::DistributionState;

use crate::app::{RedStrapApp, SyncJob};
use crate::ui::{theme, widgets};

pub(crate) fn show(app: &mut RedStrapApp, ui: &mut egui::Ui, _ctx: &egui::Context) {
    theme::page_title(ui, "Overview", "Install status, quick launch, and live session");

    // Update banner.
    if let Some(release) = app.update_release.clone() {
        ui.horizontal(|ui| {
            widgets::status_pill(
                ui,
                &format!("Red Strap {} is available", release.version),
                widgets::warn_amber(),
            );
            if ui.add(theme::ghost_button("View")).clicked() {
                app.goto(crate::app::Page::About);
            }
        });
        ui.add_space(8.0);
    }

    // Install cards.
    ui.columns(2, |columns| {
        install_card(app, &mut columns[0], LaunchMode::Player);
        install_card(app, &mut columns[1], LaunchMode::Studio);
    });
    ui.add_space(8.0);

    // Live session.
    widgets::section(ui, "Now playing");
    match app.server.clone() {
        Some(server) if server.active => {
            ui.horizontal(|ui| {
                let key = format!("server-icon-{}", server.game_icon_url);
                if !server.game_icon_url.is_empty() {
                    if let Some(texture) = app.cached_texture(&key) {
                        ui.image(egui::load::SizedTexture::new(
                            texture.id(),
                            egui::vec2(56.0, 56.0),
                        ));
                    }
                }
                ui.vertical(|ui| {
                    let title = if server.game_name.is_empty() {
                        format!("Roblox {}", server.mode)
                    } else {
                        server.game_name.clone()
                    };
                    ui.label(RichText::new(title).size(17.0).strong());
                    ui.label(
                        RichText::new(format!(
                            "Place {} · Job {}",
                            server.place_id,
                            short_job(&server.job_id)
                        ))
                        .monospace()
                        .small(),
                    );
                    if server.connected {
                        widgets::status_pill(ui, "Connected", widgets::ok_green());
                    } else {
                        widgets::status_pill(ui, "Connecting...", widgets::warn_amber());
                    }
                });
            });
            ui.add_space(4.0);
            server_row(ui, app, "Server", &format!("{}:{}", server.server_ip, server.server_port));
            if !server.machine_address.is_empty() {
                server_row(ui, app, "Machine", &server.machine_address);
            }
            server_row(ui, app, "Job ID", &server.job_id);
            if server.universe_id > 0 {
                ui.label(
                    RichText::new(format!("Universe {}", server.universe_id))
                        .small()
                        .color(widgets::muted()),
                );
            }
        }
        _ => {
            widgets::hint(ui, "Launch a game to see live server details here.");
        }
    }

    // Playtime + stats.
    ui.add_space(4.0);
    widgets::section(ui, "Statistics");
    ui.horizontal(|ui| {
        stat_box(
            ui,
            "Total playtime",
            &redstrap_core::util::format_duration_secs(app.state.playtime_total_secs),
        );
        stat_box(ui, "Fast flags", &app.flags.len().to_string());
        let enabled_mods = app
            .state
            .mods
            .iter()
            .filter(|m| m.enabled)
            .count();
        stat_box(
            ui,
            "Mods enabled",
            &format!("{enabled_mods}/{}", app.mods.len()),
        );
        let install_bytes = app.player_dist.size_bytes + app.studio_dist.size_bytes;
        stat_box(
            ui,
            "Roblox size",
            &redstrap_core::util::format_bytes(install_bytes),
        );
    });
}

fn install_card(app: &mut RedStrapApp, ui: &mut egui::Ui, mode: LaunchMode) {
    let dist: DistributionState = if mode.is_studio() {
        app.studio_dist.clone()
    } else {
        app.player_dist.clone()
    };
    let job: Option<SyncJob> = if mode.is_studio() {
        app.sync_studio.clone()
    } else {
        app.sync_player.clone()
    };

    egui::Frame::group(ui.style())
        .fill(Color32::from_rgb(0x1E, 0x1E, 0x25))
        .rounding(egui::Rounding::same(8.0))
        .inner_margin(egui::Margin::same(12.0))
        .show(ui, |ui| {
            ui.set_min_width(200.0);
            ui.label(RichText::new(mode.label()).size(17.0).strong());
            ui.add_space(2.0);
            if dist.is_installed() {
                widgets::status_pill(ui, "Installed", widgets::ok_green());
                ui.label(
                    RichText::new(&dist.version_guid)
                        .monospace()
                        .small()
                        .color(widgets::muted()),
                );
                ui.label(
                    RichText::new(redstrap_core::util::format_bytes(dist.size_bytes))
                        .small()
                        .color(widgets::muted()),
                );
            } else {
                widgets::status_pill(ui, "Not installed", widgets::muted());
            }

            if let Some(ref job) = job {
                ui.add_space(4.0);
                ui.label(RichText::new(&job.text).small());
                match job.fraction {
                    Some(f) => {
                        ui.add(egui::ProgressBar::new(f).show_percentage());
                    }
                    None => {
                        ui.add(egui::Spinner::new());
                    }
                }
            }

            ui.add_space(8.0);
            ui.horizontal(|ui| {
                let busy = job.is_some();
                if ui
                    .add_enabled(!busy, theme::accent_button("Launch"))
                    .clicked()
                {
                    app.launch(mode, "");
                }
                if ui
                    .add_enabled(!busy, theme::ghost_button("Verify"))
                    .on_hover_text("Check and repair the installation")
                    .clicked()
                {
                    start_sync(app, mode, false);
                }
            });
            if dist.is_installed() {
                ui.add_space(4.0);
                if ui
                    .add_enabled(
                        job.is_none(),
                        theme::ghost_button("Reinstall"),
                    )
                    .on_hover_text("Force a clean reinstall")
                    .clicked()
                {
                    start_sync(app, mode, true);
                }
            }
        });
}

fn start_sync(app: &mut RedStrapApp, mode: LaunchMode, force: bool) {
    let job = SyncJob {
        text: String::from("Starting..."),
        fraction: None,
    };
    if mode.is_studio() {
        app.sync_studio = Some(job);
    } else {
        app.sync_player = Some(job);
    }
    crate::tasks::spawn_sync(
        &app.rt,
        app.http.clone(),
        app.tx.clone(),
        app.layout.clone(),
        app.settings.clone(),
        mode,
        force,
    );
}

fn server_row(ui: &mut egui::Ui, app: &mut RedStrapApp, label: &str, value: &str) {
    if value.trim().is_empty() || value == ":" || value == "0" {
        return;
    }
    ui.horizontal(|ui| {
        ui.label(RichText::new(label).strong());
        ui.label(RichText::new(value).monospace());
        if ui.small_button("Copy").clicked() {
            ui.ctx().copy_text(value.to_string());
            app.toast(format!("{label} copied"));
        }
    });
}

fn short_job(job: &str) -> String {
    if job.len() > 13 {
        format!("{}...", &job[..8])
    } else {
        job.to_string()
    }
}

fn stat_box(ui: &mut egui::Ui, label: &str, value: &str) {
    egui::Frame::group(ui.style())
        .fill(Color32::from_rgb(0x1E, 0x1E, 0x25))
        .rounding(egui::Rounding::same(8.0))
        .inner_margin(egui::Margin::same(10.0))
        .show(ui, |ui| {
            ui.set_min_width(110.0);
            ui.label(RichText::new(value).size(18.0).strong());
            ui.label(RichText::new(label).small().color(widgets::muted()));
        });
}
