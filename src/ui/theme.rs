use eframe::egui;
use egui::{Color32, CornerRadius, Stroke, Theme};

pub const BG: Color32 = Color32::from_rgb(14, 16, 22);
pub const PANEL: Color32 = Color32::from_rgb(20, 23, 32);
pub const CARD: Color32 = Color32::from_rgb(27, 31, 44);
pub const CARD_BORDER: Color32 = Color32::from_rgb(42, 48, 68);
pub const TEXT_DIM: Color32 = Color32::from_rgb(138, 147, 166);
pub const ACCENT: Color32 = Color32::from_rgb(139, 92, 246);
pub const ACCENT_SOFT: Color32 = Color32::from_rgb(58, 42, 110);
pub const OK: Color32 = Color32::from_rgb(34, 197, 94);
pub const WARN: Color32 = Color32::from_rgb(245, 158, 11);
pub const ERROR: Color32 = Color32::from_rgb(239, 68, 68);

pub fn apply(ctx: &egui::Context) {
    ctx.set_theme(Theme::Dark);
    ctx.style_mut_of(Theme::Dark, |style| {
        style.spacing.item_spacing = egui::vec2(8.0, 8.0);
        style.spacing.button_padding = egui::vec2(10.0, 5.0);
        style.spacing.slider_width = 260.0;

        let visuals = &mut style.visuals;
        visuals.panel_fill = PANEL;
        visuals.window_fill = PANEL;
        visuals.extreme_bg_color = BG;
        visuals.faint_bg_color = CARD;
        visuals.selection.bg_fill = ACCENT;
        visuals.selection.stroke = Stroke::new(1.0, Color32::WHITE);
        visuals.slider_trailing_fill = true;

        let radius = CornerRadius::same(6);
        for widget in [
            &mut visuals.widgets.noninteractive,
            &mut visuals.widgets.inactive,
            &mut visuals.widgets.hovered,
            &mut visuals.widgets.active,
            &mut visuals.widgets.open,
        ] {
            widget.corner_radius = radius;
        }
        visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0, CARD_BORDER);
        visuals.widgets.inactive.bg_fill = CARD_BORDER;
        visuals.widgets.inactive.weak_bg_fill = CARD_BORDER;
        visuals.widgets.hovered.bg_fill = ACCENT_SOFT;
        visuals.widgets.hovered.weak_bg_fill = ACCENT_SOFT;
        visuals.widgets.active.bg_fill = ACCENT;
        visuals.widgets.active.weak_bg_fill = ACCENT;
    });
}
