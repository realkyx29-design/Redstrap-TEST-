//! Red + dark theme: custom `Visuals`, bundled fonts, UI scaling.
//!
//! Inter (variable) drives proportional text, JetBrains Mono drives code and
//! flags. A user-supplied font file can replace Inter at runtime.

use eframe::egui::{self, Color32, FontData, FontDefinitions, FontFamily, FontId, Stroke, TextStyle};

/// Brand red.
pub const ACCENT: Color32 = Color32::from_rgb(0xE1, 0x1D, 0x2E);
/// Brighter red for hovered controls.
pub const ACCENT_HOVER: Color32 = Color32::from_rgb(0xF2, 0x2D, 0x3F);
/// Deep red for pressed controls.
pub const ACCENT_ACTIVE: Color32 = Color32::from_rgb(0xB8, 0x14, 0x24);
/// Soft red tint for selection backgrounds.
pub const ACCENT_SOFT: Color32 = Color32::from_rgb(0x5A, 0x1A, 0x22);

const INTER: &[u8] = include_bytes!("../../../assets/fonts/Inter-Variable.ttf");
const JETBRAINS_REGULAR: &[u8] = include_bytes!("../../../assets/fonts/JetBrainsMono-Regular.ttf");

/// Window/tray icon source (256 px PNG with alpha).
pub const ICON_PNG: &[u8] = include_bytes!("../../../../assets/icon/icon.png");

/// Apply the full theme. Returns a warning when the custom font could not
/// be loaded (the bundled font is used instead).
pub fn apply(ctx: &egui::Context, ui_scale: f32, custom_font_path: &str) -> Option<String> {
    let scale = ui_scale.clamp(0.5, 3.0);
    ctx.set_pixels_per_point(scale);

    let mut warning = None;
    let custom = if custom_font_path.trim().is_empty() {
        None
    } else {
        match std::fs::read(custom_font_path.trim()) {
            Ok(bytes) if !bytes.is_empty() => Some(bytes),
            Ok(_) => {
                warning = Some(String::from("custom font file is empty; using Inter"));
                None
            }
            Err(e) => {
                warning = Some(format!("could not load custom font: {e}; using Inter"));
                None
            }
        }
    };

    ctx.set_fonts(build_fonts(custom));
    let mut style = (*ctx.style()).clone();
    style.visuals = build_visuals();
    tune_text_styles(&mut style);
    style.spacing.item_spacing = egui::vec2(10.0, 8.0);
    style.spacing.button_padding = egui::vec2(14.0, 6.0);
    style.spacing.indent = 22.0;
    style.spacing.scroll = egui::style::ScrollStyle {
        bar_width: 10.0,
        ..Default::default()
    };
    ctx.set_style(style);

    warning
}

fn build_fonts(custom_proportional: Option<Vec<u8>>) -> FontDefinitions {
    let mut fonts = FontDefinitions::default();
    let proportional = custom_proportional.unwrap_or_else(|| INTER.to_vec());
    fonts.font_data.insert(
        String::from("Inter"),
        FontData::from_owned(proportional),
    );
    fonts.font_data.insert(
        String::from("JetBrainsMono"),
        FontData::from_owned(JETBRAINS_REGULAR.to_vec()),
    );
    fonts
        .families
        .insert(FontFamily::Proportional, vec![String::from("Inter")]);
    fonts
        .families
        .insert(FontFamily::Monospace, vec![String::from("JetBrainsMono")]);
    fonts
}

fn tune_text_styles(style: &mut egui::Style) {
    use FontFamily as F;
    style.text_styles.insert(
        TextStyle::Heading,
        FontId::new(22.0, F::Proportional),
    );
    style.text_styles.insert(
        TextStyle::Body,
        FontId::new(15.0, F::Proportional),
    );
    style.text_styles.insert(
        TextStyle::Button,
        FontId::new(15.0, F::Proportional),
    );
    style.text_styles.insert(
        TextStyle::Small,
        FontId::new(12.5, F::Proportional),
    );
    style.text_styles.insert(
        TextStyle::Monospace,
        FontId::new(13.5, F::Monospace),
    );
}

fn build_visuals() -> egui::Visuals {
    let mut visuals = egui::Visuals::dark();

    visuals.dark_mode = true;
    visuals.override_text_color = None;
    visuals.window_fill = Color32::from_rgb(0x14, 0x14, 0x18);
    visuals.panel_fill = Color32::from_rgb(0x1B, 0x1B, 0x21);
    visuals.faint_bg_color = Color32::from_rgb(0x23, 0x23, 0x2A);
    visuals.extreme_bg_color = Color32::from_rgb(0x0E, 0x0E, 0x12);
    visuals.code_bg_color = Color32::from_rgb(0x0E, 0x0E, 0x12);

    visuals.hyperlink_color = ACCENT_HOVER;
    visuals.warn_fg_color = Color32::from_rgb(0xE8, 0xA0, 0x3C);
    visuals.error_fg_color = Color32::from_rgb(0xFF, 0x5A, 0x5A);

    visuals.selection.bg_fill = ACCENT_SOFT;
    visuals.selection.stroke = Stroke::new(1.0, ACCENT);

    let rounding = egui::Rounding::same(7);

    visuals.widgets.noninteractive.bg_fill = Color32::from_rgb(0x23, 0x23, 0x2A);
    visuals.widgets.noninteractive.weak_bg_fill = Color32::from_rgb(0x23, 0x23, 0x2A);
    visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0, Color32::from_rgb(0x35, 0x35, 0x40));
    visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0, Color32::from_rgb(0xE8, 0xE8, 0xEC));
    visuals.widgets.noninteractive.rounding = rounding;

    visuals.widgets.inactive.bg_fill = Color32::from_rgb(0x2A, 0x2A, 0x33);
    visuals.widgets.inactive.weak_bg_fill = Color32::from_rgb(0x2A, 0x2A, 0x33);
    visuals.widgets.inactive.bg_stroke = Stroke::new(1.0, Color32::from_rgb(0x3D, 0x3D, 0x4A));
    visuals.widgets.inactive.fg_stroke = Stroke::new(1.0, Color32::from_rgb(0xF2, 0xF2, 0xF5));
    visuals.widgets.inactive.rounding = rounding;

    visuals.widgets.hovered.bg_fill = Color32::from_rgb(0x33, 0x33, 0x3E);
    visuals.widgets.hovered.weak_bg_fill = Color32::from_rgb(0x33, 0x33, 0x3E);
    visuals.widgets.hovered.bg_stroke = Stroke::new(1.0, ACCENT);
    visuals.widgets.hovered.fg_stroke = Stroke::new(1.0, Color32::WHITE);
    visuals.widgets.hovered.rounding = rounding;
    visuals.widgets.hovered.expansion = 1.0;

    visuals.widgets.active.bg_fill = ACCENT_ACTIVE;
    visuals.widgets.active.weak_bg_fill = ACCENT_ACTIVE;
    visuals.widgets.active.bg_stroke = Stroke::new(1.0, ACCENT_HOVER);
    visuals.widgets.active.fg_stroke = Stroke::new(1.0, Color32::WHITE);
    visuals.widgets.active.rounding = rounding;

    visuals.widgets.open.bg_fill = Color32::from_rgb(0x2A, 0x2A, 0x33);
    visuals.widgets.open.weak_bg_fill = Color32::from_rgb(0x2A, 0x2A, 0x33);
    visuals.widgets.open.bg_stroke = Stroke::new(1.0, ACCENT);
    visuals.widgets.open.fg_stroke = Stroke::new(1.0, Color32::WHITE);
    visuals.widgets.open.rounding = rounding;

    visuals.window_rounding = rounding;
    visuals.window_shadow = egui::epaint::Shadow {
        offset: [0, 8],
        blur: 24,
        spread: 0,
        color: Color32::from_black_alpha(120),
    };
    visuals.popup_shadow = visuals.window_shadow;

    visuals
}

/// Accent-filled button (primary actions).
pub fn accent_button(text: &str) -> egui::Button {
    egui::Button::new(
        egui::RichText::new(text).color(Color32::WHITE).strong(),
    )
    .fill(ACCENT)
    .stroke(Stroke::NONE)
    .rounding(egui::Rounding::same(7))
    .min_size(egui::vec2(120.0, 30.0))
}

/// Muted button (secondary actions).
pub fn ghost_button(text: &str) -> egui::Button {
    egui::Button::new(egui::RichText::new(text).color(Color32::from_rgb(0xD8, 0xD8, 0xDE)))
        .rounding(egui::Rounding::same(7))
        .min_size(egui::vec2(100.0, 28.0))
}

/// Page title with a red accent bar.
pub fn page_title(ui: &mut egui::Ui, title: &str, subtitle: &str) {
    ui.horizontal(|ui| {
        let bar_height = 40.0;
        let (rect, _) = ui.allocate_exact_size(egui::vec2(4.0, bar_height), egui::Sense::hover());
        ui.painter().rect_filled(
            rect,
            egui::Rounding::same(2),
            ACCENT,
        );
        ui.vertical(|ui| {
            ui.heading(title);
            if !subtitle.is_empty() {
                ui.label(
                    egui::RichText::new(subtitle)
                        .small()
                        .color(Color32::from_rgb(0x9A, 0x9A, 0xA5)),
                );
            }
        });
    });
    ui.add_space(4.0);
    ui.separator();
    ui.add_space(8.0);
}
