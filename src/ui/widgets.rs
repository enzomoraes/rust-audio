use std::time::{Duration, Instant};

use eframe::egui;
use egui::{Color32, CornerRadius, Response, RichText, Sense, Stroke, StrokeKind, Ui};

use super::theme;

/// An iOS-style on/off switch.
pub fn toggle(ui: &mut Ui, on: &mut bool) -> Response {
    let size = egui::vec2(36.0, 20.0);
    let (rect, mut response) = ui.allocate_exact_size(size, Sense::click());
    if response.clicked() {
        *on = !*on;
        response.mark_changed();
    }

    if ui.is_rect_visible(rect) {
        let how_on = ui.ctx().animate_bool_responsive(response.id, *on);
        let track = if *on {
            theme::ACCENT
        } else {
            theme::CARD_BORDER
        };
        let radius = rect.height() / 2.0;
        ui.painter()
            .rect(rect, radius, track, Stroke::NONE, StrokeKind::Inside);
        let x = egui::lerp((rect.left() + radius)..=(rect.right() - radius), how_on);
        ui.painter().circle(
            egui::pos2(x, rect.center().y),
            radius - 3.0,
            Color32::WHITE,
            Stroke::NONE,
        );
    }
    response
}

/// A rounded label; `dot` adds a status light in front of the text.
pub fn chip(ui: &mut Ui, text: &str, color: Color32, dot: bool) {
    egui::Frame::NONE
        .fill(color.gamma_multiply(0.18))
        .stroke(Stroke::new(1.0, color.gamma_multiply(0.6)))
        .corner_radius(CornerRadius::same(10))
        .inner_margin(egui::Margin::symmetric(8, 2))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                if dot {
                    let (rect, _) = ui.allocate_exact_size(egui::vec2(8.0, 8.0), Sense::hover());
                    ui.painter().circle_filled(rect.center(), 4.0, color);
                }
                ui.label(RichText::new(text).color(color).size(12.0));
            });
        });
}

// Icons are painted instead of typed: egui's bundled fonts don't have glyphs
// like ⠿ or arrows, and they'd show up as empty boxes.

/// Two concentric circles, used as the app's logo.
pub fn logo(ui: &mut Ui) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(22.0, 22.0), Sense::hover());
    let painter = ui.painter();
    painter.circle_stroke(rect.center(), 9.0, Stroke::new(2.0, theme::ACCENT));
    painter.circle_filled(rect.center(), 4.0, theme::ACCENT);
}

/// Six dots in two columns: the handle you grab to drag a card.
pub fn grip(ui: &mut Ui) -> Response {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(14.0, 22.0), Sense::hover());
    let color = if response.hovered() {
        Color32::WHITE
    } else {
        theme::TEXT_DIM
    };
    for column in [-3.0, 3.0] {
        for row in [-6.0, 0.0, 6.0] {
            ui.painter()
                .circle_filled(rect.center() + egui::vec2(column, row), 1.7, color);
        }
    }
    response.on_hover_cursor(egui::CursorIcon::Grab)
}

pub fn arrow_down(ui: &mut Ui) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(12.0, 16.0), Sense::hover());
    let stroke = Stroke::new(1.5, theme::TEXT_DIM);
    let tip = rect.center_bottom();
    let painter = ui.painter();
    painter.line_segment([rect.center_top(), tip], stroke);
    painter.line_segment([tip, tip + egui::vec2(-4.0, -4.0)], stroke);
    painter.line_segment([tip, tip + egui::vec2(4.0, -4.0)], stroke);
}

pub fn arrow_right(ui: &mut Ui) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(16.0, 12.0), Sense::hover());
    let stroke = Stroke::new(1.5, theme::TEXT_DIM);
    let tip = rect.right_center();
    let painter = ui.painter();
    painter.line_segment([rect.left_center(), tip], stroke);
    painter.line_segment([tip, tip + egui::vec2(-4.0, -4.0)], stroke);
    painter.line_segment([tip, tip + egui::vec2(-4.0, 4.0)], stroke);
}

const METER_FLOOR_DB: f32 = -60.0;
// How fast the bar falls after a peak, like a hardware meter.
const METER_FALL_DB_PER_SEC: f32 = 30.0;
const PEAK_HOLD: Duration = Duration::from_millis(1200);

/// Smooths the raw peaks from the audio thread into something readable.
pub struct Meter {
    level_db: f32,
    hold_db: f32,
    hold_until: Instant,
    updated: Instant,
}

impl Meter {
    pub fn new() -> Self {
        Self {
            level_db: METER_FLOOR_DB,
            hold_db: METER_FLOOR_DB,
            hold_until: Instant::now(),
            updated: Instant::now(),
        }
    }

    pub fn update(&mut self, peak: f32) {
        let now = Instant::now();
        let dt = now.duration_since(self.updated).as_secs_f32();
        self.updated = now;

        let db = if peak > 0.0 {
            (20.0 * peak.log10()).max(METER_FLOOR_DB)
        } else {
            METER_FLOOR_DB
        };
        self.level_db = db.max(self.level_db - METER_FALL_DB_PER_SEC * dt);

        if db >= self.hold_db || now >= self.hold_until {
            self.hold_db = db;
            self.hold_until = now + PEAK_HOLD;
        }
    }

    pub fn show(&self, ui: &mut Ui, label: &str) {
        ui.horizontal(|ui| {
            ui.label(RichText::new(label).color(theme::TEXT_DIM));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let text = if self.hold_db <= METER_FLOOR_DB {
                    "silent".to_string()
                } else {
                    format!("{:.1} dB", self.hold_db)
                };
                ui.label(
                    RichText::new(text)
                        .monospace()
                        .color(level_color(self.hold_db)),
                );
            });
        });

        let width = ui.available_width();
        let (rect, _) = ui.allocate_exact_size(egui::vec2(width, 10.0), Sense::hover());
        let painter = ui.painter();
        painter.rect_filled(rect, 5.0, theme::BG);

        let fraction = |db: f32| ((db - METER_FLOOR_DB) / -METER_FLOOR_DB).clamp(0.0, 1.0);
        let mut bar = rect;
        bar.set_width(rect.width() * fraction(self.level_db));
        painter.rect_filled(bar, 5.0, level_color(self.level_db));

        let hold_x = rect.left() + rect.width() * fraction(self.hold_db);
        if self.hold_db > METER_FLOOR_DB {
            painter.vline(hold_x, rect.y_range(), Stroke::new(2.0, Color32::WHITE));
        }
    }
}

fn level_color(db: f32) -> Color32 {
    if db > -3.0 {
        theme::ERROR
    } else if db > -12.0 {
        theme::WARN
    } else {
        theme::OK
    }
}
