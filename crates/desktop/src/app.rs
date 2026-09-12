//! Application shell: navigation, background-task inbox, tray, dialogs.
//!
//! Pages render from this single state object and mutate it directly; every
//! mutation path ends in an immediate atomic save, so there is no "unsaved
//! changes" concept anywhere in the UI.

use std::collections::{BTreeSet, HashMap};
use std::sync::mpsc::{Receiver, Sender};
use std::time::{Duration, Instant};

use eframe::egui::{self, Color32, RichText, TextureHandle};
use redstrap_core::fastflags::FlagStore;
use redstrap_core::logging::LogRing;
use redstrap_core::mods::ModDir;
use redstrap_core::paths::Layout;
use redstrap_core::roblox::LaunchMode;
use redstrap_core::settings::Settings;
use redstrap_core::shortcuts::GameShortcut;
use redstrap_core::state::{DistributionState, State};

use crate::tasks::{ReleaseInfo, TaskMsg};
use crate::tray::{Tray, TrayAction};
use crate::ui::pages;
use crate::ui::theme;
use crate::ui::widgets;

// ---------------------------------------------------------------------------
// Navigation
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Page {
    Overview,
    Launch,
    Performance,
    Flags,
    Mods,
    Integrations,
    Shortcuts,
    Appearance,
    Logs,
    About,
}

impl Page {
    fn all() -> &'static [Page] {
        use Page::*;
        &[
            Overview,
            Launch,
            Performance,
            Flags,
            Mods,
            Integrations,
            Shortcuts,
            Appearance,
            Logs,
            About,
        ]
    }

    fn label(self) -> &'static str {
        match self {
            Page::Overview => "Overview",
            Page::Launch => "Launch",
            Page::Performance => "Performance",
            Page::Flags => "Fast Flags",
            Page::Mods => "Mods",
            Page::Integrations => "Integrations",
            Page::Shortcuts => "Shortcuts",
            Page::Appearance => "Appearance",
            Page::Logs => "Logs",
            Page::About => "About",
        }
    }

    fn id(self) -> &'static str {
        match self {
            Page::Overview => "overview",
            Page::Launch => "launch",
            Page::Performance => "performance",
            Page::Flags => "flags",
            Page::Mods => "mods",
            Page::Integrations => "integrations",
            Page::Shortcuts => "shortcuts",
            Page::Appearance => "appearance",
            Page::Logs => "logs",
            Page::About => "about",
        }
    }

    fn from_id(id: &str) -> Self {
        match id {
            "launch" => Page::Launch,
            "performance" => Page::Performance,
            "flags" => Page::Flags,
            "mods" => Page::Mods,
            "integrations" => Page::Integrations,
            "shortcuts" => Page::Shortcuts,
            "appearance" => Page::Appearance,
            "logs" => Page::Logs,
            "about" => Page::About,
            _ => Page::Overview,
        }
    }
}

// ---------------------------------------------------------------------------
// Supporting state
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default)]
pub(crate) struct SyncJob {
    pub text: String,
    pub fraction: Option<f32>,
}

#[derive(Debug, Clone)]
pub(crate) struct GamePreview {
    pub place_id: u64,
    pub name: String,
    pub texture: Option<TextureHandle>,
}

/// Best-effort mirror of the watcher's `ServerDetails.json` snapshot.
#[derive(Debug, Clone, Default, serde::Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub(crate) struct ServerDetails {
    pub active: bool,
    pub mode: String,
    pub place_id: u64,
    pub job_id: String,
    pub server_ip: String,
    pub server_port: u16,
    pub machine_address: String,
    pub universe_id: u64,
    pub game_name: String,
    pub game_icon_url: String,
    pub connected: bool,
}

struct Toast {
    text: String,
    until: Instant,
}

#[derive(Debug, Clone)]
pub(crate) enum ConfirmAction {
    Uninstall,
    DeleteMod(String),
    RemoveUnknownFlags(usize),
    DeleteProfile(String),
    DeleteGameShortcut(String),
    ResetAllFlags,
}

// ---------------------------------------------------------------------------
// App
// ---------------------------------------------------------------------------

pub struct RedStrapApp {
    pub(crate) settings: Settings,
    pub(crate) state: State,
    pub(crate) flags: FlagStore,
    pub(crate) layout: Layout,
    pub(crate) page: Page,
    pub(crate) rt: tokio::runtime::Runtime,
    pub(crate) http: reqwest::Client,
    pub(crate) tx: Sender<TaskMsg>,
    rx: Receiver<TaskMsg>,
    pub(crate) ring: LogRing,
    tray: Option<Tray>,

    // Overview
    pub(crate) player_dist: DistributionState,
    pub(crate) studio_dist: DistributionState,
    pub(crate) server: Option<ServerDetails>,
    server_checked_at: Instant,
    server_icon_url: String,

    // Flags page
    pub(crate) flag_search: String,
    pub(crate) flag_new_name: String,
    pub(crate) flag_new_value: String,
    pub(crate) flag_unknown_only: bool,
    pub(crate) allowlist: Option<BTreeSet<String>>,
    pub(crate) allowlist_loading: bool,
    pub(crate) editing_flag: Option<String>,
    pub(crate) editing_value: String,
    pub(crate) profile_name: String,

    // Mods page
    pub(crate) mods: Vec<ModDir>,
    pub(crate) recolor: Color32,
    pub(crate) recolor_groups: [bool; 3],
    pub(crate) recolor_extras: String,
    pub(crate) recolor_player: bool,
    pub(crate) recolor_studio: bool,

    // Shortcuts page
    pub(crate) gs_name: String,
    pub(crate) gs_place: String,
    pub(crate) gs_job: String,
    pub(crate) gs_access: String,
    pub(crate) gs_preview: Option<GamePreview>,
    pub(crate) gs_loading: bool,
    pub(crate) game_shortcuts: Vec<(String, GameShortcut)>,

    // Logs page
    pub(crate) log_filter: String,
    pub(crate) log_level: String,
    pub(crate) log_autoscroll: bool,

    // Jobs
    pub(crate) update_checking: bool,
    pub(crate) update_release: Option<ReleaseInfo>,
    pub(crate) update_skipped: Option<String>,
    pub(crate) update_error: Option<String>,
    pub(crate) update_staging: bool,
    pub(crate) update_staged: Option<String>,
    pub(crate) sync_player: Option<SyncJob>,
    pub(crate) sync_studio: Option<SyncJob>,
    pub(crate) applying_now: bool,
    pub(crate) gs_creating: bool,
    pub(crate) mod_installing: bool,
    pub(crate) recolor_pending: u32,
    pub(crate) channel_check: Option<(bool, String)>,
    pub(crate) channel_checking: bool,

    // Dialogs
    toast: Option<Toast>,
    error_dialog: Option<(String, String)>,
    confirm: Option<(String, String, String, ConfirmAction)>,

    textures: HashMap<String, TextureHandle>,
}

impl RedStrapApp {
    pub(crate) fn new(
        layout: Layout,
        settings: Settings,
        state: State,
        flags: FlagStore,
        rt: tokio::runtime::Runtime,
        http: reqwest::Client,
        tx: Sender<TaskMsg>,
        rx: Receiver<TaskMsg>,
        ring: LogRing,
        tray: Option<Tray>,
        font_warning: Option<String>,
    ) -> Self {
        let page = Page::from_id(&state.last_page);
        let (r, g, b) = redstrap_core::mods::parse_hex_color(&settings.last_recolor)
            .unwrap_or((0xE1, 0x1D, 0x2E));
        let mut app = Self {
            settings,
            state,
            flags,
            layout,
            page,
            rt,
            http,
            tx,
            rx,
            ring,
            tray,
            player_dist: DistributionState::default(),
            studio_dist: DistributionState::default(),
            server: None,
            server_checked_at: Instant::now() - Duration::from_secs(10),
            server_icon_url: String::new(),
            flag_search: String::new(),
            flag_new_name: String::new(),
            flag_new_value: String::from("True"),
            flag_unknown_only: false,
            allowlist: None,
            allowlist_loading: false,
            editing_flag: None,
            editing_value: String::new(),
            profile_name: String::new(),
            mods: Vec::new(),
            recolor: Color32::from_rgb(r, g, b),
            recolor_groups: [true, true, true],
            recolor_extras: String::new(),
            recolor_player: true,
            recolor_studio: false,
            gs_name: String::new(),
            gs_place: String::new(),
            gs_job: String::new(),
            gs_access: String::new(),
            gs_preview: None,
            gs_loading: false,
            game_shortcuts: Vec::new(),
            log_filter: String::new(),
            log_level: String::from("All"),
            log_autoscroll: true,
            update_checking: false,
            update_release: None,
            update_skipped: None,
            update_error: None,
            update_staging: false,
            update_staged: None,
            sync_player: None,
            sync_studio: None,
            applying_now: false,
            gs_creating: false,
            mod_installing: false,
            recolor_pending: 0,
            channel_check: None,
            channel_checking: false,
            toast: None,
            error_dialog: None,
            confirm: None,
            textures: HashMap::new(),
        };
        app.refresh_dists();
        app.refresh_mods();
        app.refresh_game_shortcuts();
        if let Some(warning) = font_warning {
            app.toast(warning);
        }
        // Automatic update check on startup (when enabled).
        if app.settings.update_check != redstrap_core::settings::UpdateChannel::Disabled {
            app.start_update_check();
        }
        app
    }

    // -- persistence ---------------------------------------------------------

    pub(crate) fn save_settings(&mut self) {
        self.settings.validate();
        if let Err(e) = self.settings.save(&self.layout.settings_file) {
            self.show_error("Could not save settings", &e.to_string());
        }
    }

    pub(crate) fn save_state(&mut self) {
        if let Err(e) = self.state.save(&self.layout.state_file) {
            tracing::warn!("could not save state: {e}");
        }
    }

    pub(crate) fn save_flags(&mut self) {
        if let Err(e) = self.flags.save(&self.layout.working_flags_file) {
            self.show_error("Could not save fast flags", &e.to_string());
        } else {
            self.flags.mark_saved();
        }
    }

    // -- feedback ------------------------------------------------------------

    pub(crate) fn toast(&mut self, text: impl Into<String>) {
        self.toast = Some(Toast {
            text: text.into(),
            until: Instant::now() + Duration::from_secs(4),
        });
    }

    pub(crate) fn show_error(&mut self, title: impl Into<String>, message: impl Into<String>) {
        self.error_dialog = Some((title.into(), message.into()));
    }

    pub(crate) fn ask_confirm(
        &mut self,
        title: impl Into<String>,
        message: impl Into<String>,
        yes_label: impl Into<String>,
        action: ConfirmAction,
    ) {
        self.confirm = Some((title.into(), message.into(), yes_label.into(), action));
    }

    // -- refreshers ----------------------------------------------------------

    pub(crate) fn refresh_dists(&mut self) {
        let (player, _) = DistributionState::load(&self.layout.player_state_file);
        let (studio, _) = DistributionState::load(&self.layout.studio_state_file);
        self.player_dist = player;
        self.studio_dist = studio;
    }

    pub(crate) fn refresh_mods(&mut self) {
        self.mods = redstrap_core::mods::scan_mods(&self.layout.modifications);
        // Register newcomers, prune the vanished.
        let mut dirty = false;
        for m in &self.mods {
            if self.state.mod_entry(&m.name).is_none() {
                self.state.ensure_mod(&m.name);
                dirty = true;
            }
        }
        let before = self.state.mods.len();
        let names: Vec<&str> = self.mods.iter().map(|m| m.name.as_str()).collect();
        self.state.mods.retain(|m| names.contains(&m.file.as_str()));
        if self.state.mods.len() != before {
            dirty = true;
        }
        if dirty {
            self.save_state();
        }
    }

    pub(crate) fn refresh_game_shortcuts(&mut self) {
        self.game_shortcuts = redstrap_core::shortcuts::list_game_shortcuts(&self.layout);
    }

    fn refresh_server(&mut self) {
        let path = self.layout.cache.join("ServerDetails.json");
        let bytes = match std::fs::read(&path) {
            Ok(b) => b,
            Err(_) => {
                self.server = None;
                return;
            }
        };
        match serde_json::from_slice::<ServerDetails>(&bytes) {
            Ok(details) if details.active => {
                // Fetch the game icon once per URL for the overview card.
                if !details.game_icon_url.is_empty() && details.game_icon_url != self.server_icon_url
                {
                    self.server_icon_url = details.game_icon_url.clone();
                    crate::tasks::spawn_fetch_icon(
                        &self.rt,
                        self.http.clone(),
                        self.tx.clone(),
                        details.game_icon_url.clone(),
                    );
                }
                self.server = Some(details);
            }
            _ => {
                self.server = None;
            }
        }
    }

    // -- actions --------------------------------------------------------------

    pub(crate) fn goto(&mut self, page: Page) {
        if self.page != page {
            self.page = page;
            self.state.last_page = page.id().to_string();
            self.save_state();
        }
    }

    pub(crate) fn launch(&mut self, mode: LaunchMode, extra_args: &str) {
        if !self.layout.application.is_file() {
            self.show_error(
                "Not installed",
                "Red Strap itself is not installed — run the launcher once first.",
            );
            return;
        }
        let mut command = std::process::Command::new(&self.layout.application);
        match mode {
            LaunchMode::Player => {
                command.arg("--player");
            }
            LaunchMode::Studio | LaunchMode::StudioAuth => {
                command.arg("--studio");
            }
        }
        if !extra_args.trim().is_empty() {
            command.arg("--launch-args").arg(extra_args.trim());
        }
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            command.creation_flags(CREATE_NO_WINDOW);
        }
        if let Err(e) = command.spawn() {
            self.show_error(
                format!("Could not launch {}", mode.label()),
                e.to_string(),
            );
        } else {
            self.toast(format!("{} is starting...", mode.label()));
        }
    }

    pub(crate) fn start_update_check(&mut self) {
        if self.update_checking {
            return;
        }
        self.update_checking = true;
        self.update_error = None;
        self.update_skipped = None;
        crate::tasks::spawn_update_check(
            &self.rt,
            self.http.clone(),
            self.tx.clone(),
            self.settings.update_repo.clone(),
            self.settings.update_check,
        );
    }

    /// Decode PNG bytes into a cached texture.
    pub(crate) fn texture_for_png(
        &mut self,
        ctx: &egui::Context,
        key: &str,
        png: &[u8],
    ) -> Option<TextureHandle> {
        if let Some(handle) = self.textures.get(key) {
            return Some(handle.clone());
        }
        let image = image::load_from_memory(png).ok()?;
        let rgba = image.to_rgba8();
        let (w, h) = (rgba.width(), rgba.height());
        if w == 0 || h == 0 || w > 1024 || h > 1024 {
            return None;
        }
        let color =
            egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], rgba.as_raw());
        let handle = ctx.load_texture(key, color, egui::TextureOptions::LINEAR);
        self.textures.insert(key.to_string(), handle.clone());
        Some(handle)
    }

    // -- inbox -----------------------------------------------------------------

    fn pump_inbox(&mut self, ctx: &egui::Context) {
        for _ in 0..40 {
            match self.rx.try_recv() {
                Ok(msg) => self.handle_msg(ctx, msg),
                Err(_) => break,
            }
        }
    }

    fn handle_msg(&mut self, ctx: &egui::Context, msg: TaskMsg) {
        match msg {
            TaskMsg::UpdateCheck {
                current,
                release,
                error,
            } => {
                self.update_checking = false;
                if let Some(error) = error {
                    self.update_error = Some(error);
                } else {
                    self.update_error = None;
                    self.update_skipped = None;
                    match release {
                        Some(info)
                            if info.version == self.state.skipped_update_version =>
                        {
                            tracing::info!(
                                "update {} available but skipped by user",
                                info.version
                            );
                            self.update_release = None;
                            self.update_skipped = Some(info.version);
                        }
                        other => {
                            self.update_release = other;
                            if self.update_release.is_none() {
                                tracing::info!("up to date ({current})");
                            }
                        }
                    }
                }
            }
            TaskMsg::UpdateStaged { version, error } => {
                self.update_staging = false;
                if let Some(error) = error {
                    self.show_error("Update failed", error);
                } else {
                    self.update_staged = Some(version.clone());
                    self.update_release = None;
                    self.update_skipped = None;
                    if !self.state.skipped_update_version.is_empty() {
                        self.state.skipped_update_version.clear();
                        self.save_state();
                    }
                    self.toast(format!("Red Strap {version} staged — restart to apply"));
                }
            }
            TaskMsg::Allowlist { flags, error } => {
                self.allowlist_loading = false;
                if let Some(error) = error {
                    self.show_error("Allowlist fetch failed", error);
                } else {
                    self.allowlist = Some(flags);
                    self.toast("Flag allowlist refreshed");
                }
            }
            TaskMsg::SyncProgress {
                mode,
                text,
                fraction,
            } => {
                let job = SyncJob { text, fraction };
                if mode.contains("Studio") {
                    self.sync_studio = Some(job);
                } else {
                    self.sync_player = Some(job);
                }
            }
            TaskMsg::SyncDone {
                mode,
                version,
                error,
            } => {
                if mode.contains("Studio") {
                    self.sync_studio = None;
                } else {
                    self.sync_player = None;
                }
                self.refresh_dists();
                if let Some(error) = error {
                    self.show_error(format!("{mode} sync failed"), error);
                } else {
                    self.toast(format!("{mode} ready ({version})"));
                }
            }
            TaskMsg::GameShortcutDone { path, error } => {
                self.gs_creating = false;
                self.refresh_game_shortcuts();
                if let Some(error) = error {
                    self.show_error("Could not create game shortcut", error);
                } else {
                    self.toast(format!("Shortcut created"));
                    let _ = path;
                }
            }
            TaskMsg::GamePreview {
                place_id,
                name,
                icon_png,
            } => {
                self.gs_loading = false;
                let wanted: u64 = self.gs_place.trim().parse().unwrap_or(0);
                if wanted != place_id {
                    return; // stale preview
                }
                let texture = if icon_png.is_empty() {
                    None
                } else {
                    let key = format!("gs-preview-{place_id}");
                    self.texture_for_png(ctx, &key, &icon_png)
                };
                self.gs_preview = Some(GamePreview {
                    place_id,
                    name,
                    texture,
                });
            }
            TaskMsg::ChannelChecked { channel: _, ok, detail } => {
                self.channel_checking = false;
                self.channel_check = Some((ok, detail));
            }
            TaskMsg::RecolorDone {
                target,
                recolored,
                missing,
                failed,
            } => {
                self.recolor_pending = self.recolor_pending.saturating_sub(1);
                if failed.is_empty() {
                    self.toast(format!("{target}: recolored {recolored} ({missing} missing)"));
                } else {
                    self.show_error(
                        format!("{target}: recolor finished with errors"),
                        format!("recolored {recolored}, {missing} missing:\n{}", failed.join("\n")),
                    );
                }
            }
            TaskMsg::ModInstalled { name, error } => {
                self.mod_installing = false;
                self.refresh_mods();
                if let Some(error) = error {
                    self.show_error("Could not install mod", error);
                } else {
                    self.toast(format!("Mod '{name}' installed"));
                }
            }
            TaskMsg::AppliedNow { summary } => {
                self.applying_now = false;
                self.toast(summary.join(" · "));
            }
            TaskMsg::IconFetched { url, png } => {
                let key = format!("server-icon-{url}");
                self.texture_for_png(ctx, &key, &png);
            }
        }
    }

    // -- tray ------------------------------------------------------------------

    fn poll_tray(&mut self, ctx: &egui::Context) {
        let actions = match &self.tray {
            Some(tray) => tray.poll(),
            None => return,
        };
        for action in actions {
            match action {
                TrayAction::Show => {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
                }
                TrayAction::LaunchPlayer => self.launch(LaunchMode::Player, ""),
                TrayAction::LaunchStudio => self.launch(LaunchMode::Studio, ""),
                TrayAction::CheckUpdates => {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
                    self.goto(Page::About);
                    self.start_update_check();
                }
                TrayAction::Quit => {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            }
        }
    }

    fn handle_close_request(&self, ctx: &egui::Context) {
        let close_requested = ctx.input(|i| i.viewport().close_requested());
        if close_requested && self.settings.minimize_to_tray && self.tray.is_some() {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
        }
    }
}

// ---------------------------------------------------------------------------
// eframe glue
// ---------------------------------------------------------------------------

impl eframe::App for RedStrapApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.pump_inbox(ctx);
        self.poll_tray(ctx);
        self.handle_close_request(ctx);

        // Periodic server snapshot (overview only, every 2 s).
        if self.page == Page::Overview && self.server_checked_at.elapsed() > Duration::from_secs(2)
        {
            self.server_checked_at = Instant::now();
            self.refresh_server();
        }

        // Expire toasts.
        if let Some(toast) = &self.toast {
            if Instant::now() >= toast.until {
                self.toast = None;
            }
        }

        // Keep timers alive even when idle.
        ctx.request_repaint_after(Duration::from_millis(500));

        // -- navigation -------------------------------------------------------
        egui::SidePanel::left("nav")
            .resizable(false)
            .exact_width(200.0)
            .show(ctx, |ui| {
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    ui.add_space(8.0);
                    ui.label(
                        RichText::new("Red Strap")
                            .size(20.0)
                            .strong()
                            .color(theme::ACCENT_HOVER),
                    );
                });
                ui.add_space(4.0);
                ui.separator();
                ui.add_space(4.0);
                for page in Page::all() {
                    let selected = self.page == *page;
                    if ui
                        .selectable_label(selected, RichText::new(page.label()).size(15.0))
                        .clicked()
                    {
                        self.goto(*page);
                    }
                }
                ui.with_layout(egui::Layout::bottom_up(egui::Align::LEFT), |ui| {
                    ui.add_space(8.0);
                    ui.label(
                        RichText::new(format!("v{}", env!("CARGO_PKG_VERSION")))
                            .small()
                            .color(widgets::muted()),
                    );
                    if self.update_release.is_some() {
                        if ui
                            .add(theme::ghost_button("Update ready"))
                            .on_hover_text("Open the About page to install")
                            .clicked()
                        {
                            self.goto(Page::About);
                        }
                    }
                });
            });

        // -- page ---------------------------------------------------------------
        egui::CentralPanel::default().show(ctx, |ui| {
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.add_space(16.0);
                    ui.horizontal(|ui| {
                        ui.add_space(12.0);
                        ui.vertical(|ui| {
                            ui.set_min_width(ui.available_width() - 12.0);
                            match self.page {
                                Page::Overview => pages::overview::show(self, ui, ctx),
                                Page::Launch => pages::launch::show(self, ui),
                                Page::Performance => pages::performance::show(self, ui),
                                Page::Flags => pages::flags::show(self, ui),
                                Page::Mods => pages::mods::show(self, ui),
                                Page::Integrations => pages::integrations::show(self, ui),
                                Page::Shortcuts => pages::shortcuts::show(self, ui, ctx),
                                Page::Appearance => pages::appearance::show(self, ui, ctx),
                                Page::Logs => pages::logs::show(self, ui),
                                Page::About => pages::about::show(self, ui),
                            }
                        });
                    });
                    ui.add_space(24.0);
                });
        });

        // -- toast ----------------------------------------------------------------
        if let Some(toast) = &self.toast {
            egui::Window::new("toast")
                .title_bar(false)
                .resizable(false)
                .collapsible(false)
                .anchor(egui::Align2::CENTER_BOTTOM, [0.0, -16.0])
                .show(ctx, |ui| {
                    ui.label(RichText::new(&toast.text).color(Color32::WHITE));
                });
        }

        // -- dialogs --------------------------------------------------------------
        if let Some((title, message)) = self.error_dialog.clone() {
            let mut open = true;
            widgets::error_dialog(ctx, &title, &message, &mut open);
            if !open {
                self.error_dialog = None;
            }
        }
        if let Some((title, message, yes, action)) = self.confirm.clone() {
            let mut open = true;
            let decision = widgets::confirm_dialog(ctx, &title, &message, &yes, &mut open);
            if !open {
                self.confirm = None;
            }
            if decision == Some(true) {
                self.confirm = None;
                self.run_confirm_action(action);
            }
        }
    }
}

impl RedStrapApp {
    fn run_confirm_action(&mut self, action: ConfirmAction) {
        match action {
            ConfirmAction::Uninstall => {
                let mut command =
                    std::process::Command::new(&self.layout.application);
                command.arg("--uninstall");
                #[cfg(windows)]
                {
                    use std::os::windows::process::CommandExt;
                    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
                    command.creation_flags(CREATE_NO_WINDOW);
                }
                match command.spawn() {
                    Ok(_) => std::process::exit(0),
                    Err(e) => self.show_error("Could not start uninstall", e.to_string()),
                }
            }
            ConfirmAction::DeleteMod(name) => {
                let path = self.layout.modifications.join(&name);
                if path.is_dir() {
                    if let Err(e) = std::fs::remove_dir_all(&path) {
                        self.show_error("Could not delete mod", e.to_string());
                        return;
                    }
                }
                self.state.mods.retain(|m| m.file != name);
                self.save_state();
                self.refresh_mods();
                self.toast(format!("Mod '{name}' deleted"));
            }
            ConfirmAction::RemoveUnknownFlags(count) => {
                if let Some(allow) = self.allowlist.clone() {
                    let removed = self.flags.retain_allowed(&allow);
                    self.save_flags();
                    self.toast(format!("Removed {removed} unknown flags"));
                    let _ = count;
                }
            }
            ConfirmAction::DeleteProfile(name) => {
                if let Err(e) =
                    redstrap_core::fastflags::delete_profile(&self.layout.flag_profiles, &name)
                {
                    self.show_error("Could not delete profile", e.to_string());
                } else {
                    self.toast(format!("Profile '{name}' deleted"));
                }
            }
            ConfirmAction::DeleteGameShortcut(record) => {
                if let Err(e) =
                    redstrap_core::shortcuts::delete_game_shortcut(&self.layout, &record)
                {
                    self.show_error("Could not delete shortcut", e.to_string());
                } else {
                    self.refresh_game_shortcuts();
                    self.toast("Game shortcut deleted");
                }
            }
            ConfirmAction::ResetAllFlags => {
                self.flags = FlagStore::new();
                self.save_flags();
                self.toast("All fast flags cleared");
            }
        }
    }
}
