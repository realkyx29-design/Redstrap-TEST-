//! Shared settings-UI building blocks: enum combos, setting rows, dialogs.
//!
//! Every control here is dumb by design: pages own the state, widgets only
//! render it and report changes.

use eframe::egui::{self, Color32, RichText};

use crate::ui::theme::{self, ACCENT};

// ---------------------------------------------------------------------------
// Enum combos
// ---------------------------------------------------------------------------

/// Display metadata for a settings enum, powering [`enum_combo`].
pub trait SettingOption: Copy + PartialEq {
    fn all() -> &'static [Self];
    fn label(self) -> &'static str;
}

impl SettingOption for redstrap_core::settings::UpdateChannel {
    fn all() -> &'static [Self] {
        Self::all()
    }
    fn label(self) -> &'static str {
        self.label()
    }
}

impl SettingOption for redstrap_core::settings::CleanerMode {
    fn all() -> &'static [Self] {
        Self::all()
    }
    fn label(self) -> &'static str {
        self.label()
    }
}

impl SettingOption for redstrap_core::settings::ProcessPriority {
    fn all() -> &'static [Self] {
        Self::all()
    }
    fn label(self) -> &'static str {
        self.label()
    }
}

impl SettingOption for redstrap_core::settings::RenderingMode {
    fn all() -> &'static [Self] {
        Self::all()
    }
    fn label(self) -> &'static str {
        self.label()
    }
}

impl SettingOption for redstrap_core::settings::MSAAMode {
    fn all() -> &'static [Self] {
        Self::all()
    }
    fn label(self) -> &'static str {
        self.label()
    }
}

impl SettingOption for redstrap_core::settings::GpuPreference {
    fn all() -> &'static [Self] {
        Self::all()
    }
    fn label(self) -> &'static str {
        self.label()
    }
}

/// Dropdown for a settings enum. Returns true when the value changed.
pub fn enum_combo<E: SettingOption>(
    ui: &mut egui::Ui,
    id: &str,
    current: &mut E,
) -> bool {
    let mut changed = false;
    egui::ComboBox::from_id_source(id)
        .selected_text(current.label())
        .show_ui(ui, |ui| {
            for option in E::all() {
                let selected = *current == *option;
                if ui
                    .selectable_label(selected, option.label())
                    .clicked()
                {
                    *current = *option;
                    changed = true;
                }
            }
        });
    changed
}

// ---------------------------------------------------------------------------
// Rows and sections
// ---------------------------------------------------------------------------

/// A labelled setting row: title + description on the left, control fitted
/// on the right. Returns the control closure's result.
pub fn setting_row<R>(
    ui: &mut egui::Ui,
    title: &str,
    description: &str,
    control: impl FnOnce(&mut egui::Ui) -> R,
) -> R {
    let result = ui
        .horizontal(|ui| {
            ui.vertical(|ui| {
                ui.set_min_width(260.0);
                ui.label(RichText::new(title).strong());
                if !description.is_empty() {
                    ui.label(
                        RichText::new(description)
                            .small()
                            .color(Color32::from_rgb(0x9A, 0x9A, 0xA5)),
                    );
                }
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                control(ui)
            })
            .inner
        })
        .inner;
    ui.add_space(2.0);
    result
}

/// A titled section with an accent caption.
pub fn section(ui: &mut egui::Ui, title: &str) {
    ui.add_space(10.0);
    ui.label(RichText::new(title).color(ACCENT).strong().size(16.0));
    ui.add_space(2.0);
    ui.separator();
    ui.add_space(6.0);
}

/// Small muted hint line.
pub fn hint(ui: &mut egui::Ui, text: &str) {
    ui.label(
        RichText::new(text)
            .small()
            .color(Color32::from_rgb(0x9A, 0x9A, 0xA5)),
    );
}

/// Status pill: colored dot + text.
pub fn status_pill(ui: &mut egui::Ui, text: &str, color: Color32) {
    ui.horizontal(|ui| {
        let (rect, _) = ui.allocate_exact_size(egui::vec2(8.0, 8.0), egui::Sense::hover());
        ui.painter().circle_filled(rect.center(), 4.0, color);
        ui.label(text);
    });
}

pub fn ok_green() -> Color32 {
    Color32::from_rgb(0x3D, 0xD6, 0x7A)
}

pub fn warn_amber() -> Color32 {
    Color32::from_rgb(0xE8, 0xA0, 0x3C)
}

pub fn err_red() -> Color32 {
    Color32::from_rgb(0xFF, 0x5A, 0x5A)
}

pub fn muted() -> Color32 {
    Color32::from_rgb(0x9A, 0x9A, 0xA5)
}

// ---------------------------------------------------------------------------
// Dialogs
// ---------------------------------------------------------------------------

/// Modal error dialog. Returns true while open.
pub fn error_dialog(
    ctx: &egui::Context,
    title: &str,
    message: &str,
    open: &mut bool,
) {
    if !*open {
        return;
    }
    let mut still_open = true;
    egui::Window::new(format!("{} ", title))
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .show(ctx, |ui| {
            ui.set_min_width(360.0);
            ui.label(message);
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                if ui.add(theme::accent_button("OK")).clicked() {
                    still_open = false;
                }
                if ui.add(theme::ghost_button("Copy")).clicked() {
                    ui.ctx().copy_text(message.to_string());
                }
            });
        });
    *open = still_open;
}

/// Modal confirmation dialog. Returns Some(true/false) once decided.
pub fn confirm_dialog(
    ctx: &egui::Context,
    title: &str,
    message: &str,
    yes_label: &str,
    open: &mut bool,
) -> Option<bool> {
    if !*open {
        return None;
    }
    let mut decision = None;
    egui::Window::new(format!("{title} "))
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .show(ctx, |ui| {
            ui.set_min_width(360.0);
            ui.label(message);
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                if ui.add(theme::accent_button(yes_label)).clicked() {
                    decision = Some(true);
                }
                if ui.add(theme::ghost_button("Cancel")).clicked() {
                    decision = Some(false);
                }
            });
        });
    if decision.is_some() {
        *open = false;
    }
    decision
}
