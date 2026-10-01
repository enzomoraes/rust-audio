use eframe::egui;
use egui::{Align, CornerRadius, Layout, RichText, Stroke};

use super::{theme, widgets};
use crate::dsp::PipelineRemote;
use crate::dsp::controls::EffectControls;

/// Index of the effect being dragged in the chain.
struct DraggedEffect(usize);

/// The effect cards, in order, between the source and destination labels.
pub fn show(ui: &mut egui::Ui, remote: &mut PipelineRemote, destination: &str) {
    ui.horizontal(|ui| {
        ui.label(RichText::new("Effects").strong().size(18.0));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.label(RichText::new("Drag the handle to reorder").color(theme::TEXT_DIM))
                .on_hover_text("Double-click a slider to reset it.");
        });
    });
    ui.add_space(4.0);

    let mut pending_move = None;
    flow_label(ui, "Microphone", true);
    for index in 0..remote.effects().len() {
        let controls = remote.effects()[index].clone();
        let card = effect_card(ui, index, &controls);

        // While something is dragged over this card, show where it would land.
        if let (Some(dragged), Some(pointer)) = (
            card.dnd_hover_payload::<DraggedEffect>(),
            ui.input(|input| input.pointer.interact_pos()),
        ) {
            let above = pointer.y < card.rect.center().y;
            let y = if above {
                card.rect.top() - 4.0
            } else {
                card.rect.bottom() + 4.0
            };
            ui.painter()
                .hline(card.rect.x_range(), y, Stroke::new(3.0, theme::ACCENT));

            if card.dnd_release_payload::<DraggedEffect>().is_some() {
                let insert_at = if above { index } else { index + 1 };
                // Removing the dragged card first shifts everything after it up by one.
                let to = if insert_at > dragged.0 {
                    insert_at - 1
                } else {
                    insert_at
                };
                pending_move = Some((dragged.0, to));
            }
        }
    }
    flow_label(ui, destination, false);

    if let Some((from, to)) = pending_move {
        remote.move_effect(from, to);
    }
}

fn effect_card(ui: &mut egui::Ui, index: usize, controls: &EffectControls) -> egui::Response {
    let mut enabled = controls.is_enabled();
    let border = if enabled {
        theme::ACCENT.gamma_multiply(0.5)
    } else {
        theme::CARD_BORDER
    };

    egui::Frame::NONE
        .fill(theme::CARD)
        .stroke(Stroke::new(1.0, border))
        .corner_radius(CornerRadius::same(10))
        .inner_margin(egui::Margin::symmetric(10, 8))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.dnd_drag_source(
                    egui::Id::new(("effect", index)),
                    DraggedEffect(index),
                    |ui| {
                        widgets::grip(ui);
                    },
                );
                ui.label(
                    RichText::new(format!("{}", index + 1))
                        .monospace()
                        .color(theme::TEXT_DIM),
                );
                ui.label(RichText::new(controls.name).strong())
                    .on_hover_text(controls.description);
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if widgets::toggle(ui, &mut enabled).changed() {
                        controls.set_enabled(enabled);
                    }
                });
            });

            // Leave room for the value box and the label next to the slider.
            ui.spacing_mut().slider_width = (ui.available_width() - 150.0).max(60.0);
            for param in &controls.params {
                let mut value = param.get();
                let slider = egui::Slider::new(&mut value, param.min..=param.max)
                    .text(param.name)
                    .suffix(param.unit)
                    .fixed_decimals(1);
                let response = ui.add_enabled(enabled, slider);
                if response.double_clicked() {
                    param.set(param.default);
                } else if response.changed() {
                    param.set(value);
                }
            }
        })
        .response
}

/// The source/destination at the ends of the chain, with an arrow pointing into it.
fn flow_label(ui: &mut egui::Ui, text: &str, is_source: bool) {
    ui.vertical_centered(|ui| {
        if !is_source {
            widgets::arrow_down(ui);
        }
        ui.label(RichText::new(text).color(theme::TEXT_DIM));
        if is_source {
            widgets::arrow_down(ui);
        }
    });
}
