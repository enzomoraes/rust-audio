mod theme;
mod widgets;

use std::collections::VecDeque;
use std::sync::atomic::Ordering;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use eframe::egui;
use egui::{Align, CornerRadius, Layout, RichText, Stroke};

use crate::audio::Engine;
use crate::dsp::PipelineRemote;
use crate::telemetry::{Level, LogEntry};
use widgets::Meter;

const APP_NAME: &str = "Rust Phone";
const MAX_LOG_ENTRIES: usize = 500;
// Meters need a steady refresh even when nobody touches the window.
const REPAINT_EVERY: Duration = Duration::from_millis(33);

pub fn run(
    engine: Result<(Engine, PipelineRemote), String>,
    logs: mpsc::Receiver<LogEntry>,
    started: Instant,
) -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title(APP_NAME)
            .with_inner_size([980.0, 680.0])
            .with_min_inner_size([720.0, 480.0]),
        ..Default::default()
    };
    eframe::run_native(
        APP_NAME,
        options,
        Box::new(move |cc| {
            theme::apply(&cc.egui_ctx);
            Ok(Box::new(App::new(engine, logs, started)))
        }),
    )
}

struct Running {
    engine: Engine,
    remote: PipelineRemote,
}

/// Index of the effect being dragged in the chain.
struct DraggedEffect(usize);

struct App {
    running: Result<Running, String>,
    logs: mpsc::Receiver<LogEntry>,
    entries: VecDeque<LogEntry>,
    started: Instant,
    input_meter: Meter,
    output_meter: Meter,
    dropped_total: usize,
    dropped_last_second: usize,
    last_drop_check: Instant,
}

impl App {
    fn new(
        engine: Result<(Engine, PipelineRemote), String>,
        logs: mpsc::Receiver<LogEntry>,
        started: Instant,
    ) -> Self {
        Self {
            running: engine.map(|(engine, remote)| Running { engine, remote }),
            logs,
            entries: VecDeque::new(),
            started,
            input_meter: Meter::new(),
            output_meter: Meter::new(),
            dropped_total: 0,
            dropped_last_second: 0,
            last_drop_check: Instant::now(),
        }
    }

    fn poll(&mut self) {
        while let Ok(entry) = self.logs.try_recv() {
            self.push_entry(entry);
        }

        let Ok(running) = &self.running else {
            return;
        };
        let stats = running.engine.stats();
        self.input_meter.update(stats.input_peak.take());
        self.output_meter.update(stats.output_peak.take());

        if self.last_drop_check.elapsed() >= Duration::from_secs(1) {
            self.last_drop_check = Instant::now();
            let total = stats.dropped_frames.load(Ordering::Relaxed);
            self.dropped_last_second = total - self.dropped_total;
            self.dropped_total = total;
            if self.dropped_last_second > 0 {
                let ms = self.dropped_last_second as f32 / running.engine.info().sample_rate as f32
                    * 1000.0;
                let message = format!(
                    "dropped {} frames ({ms:.1} ms) in the last second: buffer full",
                    self.dropped_last_second
                );
                self.push_entry(LogEntry {
                    at: Instant::now(),
                    level: Level::Warn,
                    message,
                });
            }
        }
    }

    fn push_entry(&mut self, entry: LogEntry) {
        if self.entries.len() == MAX_LOG_ENTRIES {
            self.entries.pop_front();
        }
        self.entries.push_back(entry);
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.poll();
        ui.ctx().request_repaint_after(REPAINT_EVERY);

        egui::Panel::top("header")
            .frame(panel_frame(theme::BG, 14))
            .show(ui, |ui| self.header(ui));

        // Added before the side panel so it spans the whole window width.
        egui::Panel::bottom("logs")
            .resizable(true)
            .default_size(170.0)
            .min_size(90.0)
            .frame(panel_frame(theme::PANEL, 12))
            .show(ui, |ui| self.logs_panel(ui));

        egui::Panel::right("side")
            .exact_size(300.0)
            .frame(panel_frame(theme::PANEL, 16))
            .show(ui, |ui| self.side_panel(ui));

        egui::CentralPanel::default()
            .frame(panel_frame(theme::BG, 20))
            .show(ui, |ui| self.chain_panel(ui));
    }
}

impl App {
    fn header(&self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            widgets::logo(ui);
            ui.label(RichText::new(APP_NAME).strong().size(20.0));
            ui.add_space(8.0);

            match &self.running {
                Ok(running) => {
                    widgets::chip(ui, "Running", theme::OK, true);
                    if running.engine.is_monitoring() {
                        widgets::chip(ui, "Listening to yourself", theme::ACCENT, false);
                    }
                }
                Err(_) => widgets::chip(ui, "Stopped", theme::ERROR, true),
            }

            if let Ok(running) = &self.running {
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    // Right-to-left layout: added last-to-first.
                    let info = running.engine.info();
                    ui.label(RichText::new(output_summary(&running.engine)).color(theme::TEXT_DIM));
                    widgets::arrow_right(ui);
                    ui.label(RichText::new(&info.input_device).color(theme::TEXT_DIM));
                });
            }
        });
    }

    fn side_panel(&self, ui: &mut egui::Ui) {
        if let Ok(running) = &self.running {
            output_section(ui, &running.engine);
            ui.add_space(18.0);
        }

        section_title(ui, "Levels");
        self.input_meter.show(ui, "Input");
        ui.add_space(6.0);
        self.output_meter.show(ui, "Output");

        ui.add_space(18.0);
        section_title(ui, "Engine");
        let Ok(running) = &self.running else {
            ui.label(RichText::new("Audio is not running.").color(theme::TEXT_DIM));
            return;
        };
        let info = running.engine.info();
        let rate = info.sample_rate as f32;
        let ms = |frames: usize| frames as f32 / rate * 1000.0;

        egui::Grid::new("engine_stats")
            .num_columns(2)
            .spacing([12.0, 6.0])
            .show(ui, |ui| {
                stat_row(ui, "Sample rate", format!("{} Hz", info.sample_rate));
                stat_row(
                    ui,
                    "Block",
                    format!(
                        "{} frames · {:.1} ms",
                        info.block_frames,
                        ms(info.block_frames)
                    ),
                );
                for queue in running.engine.queues() {
                    stat_row(
                        ui,
                        queue.name,
                        format!("{:.1} ms of {:.0} ms", ms(queue.queued), ms(queue.capacity)),
                    );
                }
                stat_row(
                    ui,
                    "Dropped",
                    format!("{} frames/s", self.dropped_last_second),
                );
            });
    }

    fn chain_panel(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label(RichText::new("Effect chain").strong().size(18.0));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.label(
                    RichText::new("Drag the handle to reorder · double-click a slider to reset")
                        .color(theme::TEXT_DIM),
                );
            });
        });
        ui.add_space(4.0);

        let Ok(running) = &mut self.running else {
            if let Err(err) = &self.running {
                ui.add_space(40.0);
                ui.vertical_centered(|ui| {
                    ui.label(RichText::new("Audio could not start").strong().size(16.0));
                    ui.label(RichText::new(err).color(theme::ERROR));
                });
            }
            return;
        };

        let mut pending_move = None;
        egui::ScrollArea::vertical().show(ui, |ui| {
            flow_label(ui, "Microphone", true);
            let count = running.remote.effects().len();
            for index in 0..count {
                let controls = running.remote.effects()[index].clone();
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
            flow_label(ui, &output_summary(&running.engine), false);
        });

        if let Some((from, to)) = pending_move {
            running.remote.move_effect(from, to);
        }
    }

    fn logs_panel(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            section_title(ui, "Logs");
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui.button("Clear").clicked() {
                    self.entries.clear();
                }
            });
        });

        egui::ScrollArea::both()
            .auto_shrink([false, false])
            .stick_to_bottom(true)
            .show(ui, |ui| {
                for entry in &self.entries {
                    let elapsed = entry.at.saturating_duration_since(self.started).as_secs();
                    let (tag, color) = match entry.level {
                        Level::Info => ("INFO ", theme::TEXT_DIM),
                        Level::Warn => ("WARN ", theme::WARN),
                        Level::Error => ("ERROR", theme::ERROR),
                    };
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new(format!("{:02}:{:02}", elapsed / 60, elapsed % 60))
                                .monospace()
                                .color(theme::TEXT_DIM),
                        );
                        ui.label(RichText::new(tag).monospace().color(color));
                        ui.label(RichText::new(&entry.message).monospace());
                    });
                }
            });
    }
}

/// Where the processed voice is going right now, e.g. "Rust Phone mic + speakers".
fn output_summary(engine: &Engine) -> String {
    let mut outputs = Vec::new();
    if let Some(mic) = &engine.info().virtual_mic {
        outputs.push(format!("{mic} mic"));
    }
    if engine.is_monitoring() {
        outputs.push("speakers".to_string());
    }
    if outputs.is_empty() {
        "nowhere (listening is off)".to_string()
    } else {
        outputs.join(" + ")
    }
}

fn output_section(ui: &mut egui::Ui, engine: &Engine) {
    section_title(ui, "Output");
    let info = engine.info();

    ui.horizontal(|ui| {
        ui.label("Virtual mic");
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if info.virtual_mic.is_some() {
                widgets::chip(ui, "Active", theme::OK, true);
            } else {
                widgets::chip(ui, "Not installed", theme::WARN, false);
            }
        });
    });
    let hint = match &info.virtual_mic {
        Some(mic) => format!("Pick \"{mic}\" as the microphone in Meet, Discord or any other app."),
        None if cfg!(target_os = "windows") => {
            "No virtual mic driver found. Install VB-CABLE to test until the Rust Phone driver is ready."
                .to_string()
        }
        None => "Virtual mics aren't supported on this system yet.".to_string(),
    };
    ui.label(RichText::new(hint).color(theme::TEXT_DIM));
    ui.add_space(8.0);

    ui.horizontal(|ui| {
        ui.label("Listen to myself");
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let mut on = engine.is_monitoring();
            let enabled = info.speaker_device.is_some();
            if ui
                .add_enabled_ui(enabled, |ui| widgets::toggle(ui, &mut on))
                .inner
                .changed()
            {
                engine.set_monitoring(on);
            }
        });
    });
    let hint = match &info.speaker_device {
        Some(speaker) => format!(
            "Plays your processed voice on {speaker}. Use headphones, or the mic will pick it up and loop."
        ),
        None => "No speakers found.".to_string(),
    };
    ui.label(RichText::new(hint).color(theme::TEXT_DIM));
}

fn effect_card(
    ui: &mut egui::Ui,
    index: usize,
    controls: &crate::dsp::controls::EffectControls,
) -> egui::Response {
    let mut enabled = controls.is_enabled();
    let border = if enabled {
        theme::ACCENT.gamma_multiply(0.5)
    } else {
        theme::CARD_BORDER
    };

    egui::Frame::NONE
        .fill(theme::CARD)
        .stroke(Stroke::new(1.0, border))
        .corner_radius(CornerRadius::same(12))
        .inner_margin(egui::Margin::same(14))
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
                ui.vertical(|ui| {
                    ui.label(RichText::new(controls.name).strong().size(15.0));
                    ui.label(RichText::new(controls.description).color(theme::TEXT_DIM));
                });
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if widgets::toggle(ui, &mut enabled).changed() {
                        controls.set_enabled(enabled);
                    }
                });
            });

            if !controls.params.is_empty() {
                ui.add_space(6.0);
            }
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

fn panel_frame(fill: egui::Color32, margin: i8) -> egui::Frame {
    egui::Frame::NONE
        .fill(fill)
        .inner_margin(egui::Margin::same(margin))
}

fn section_title(ui: &mut egui::Ui, title: &str) {
    ui.label(
        RichText::new(title.to_uppercase())
            .size(11.0)
            .strong()
            .color(theme::TEXT_DIM),
    );
}

fn stat_row(ui: &mut egui::Ui, label: &str, value: String) {
    ui.label(RichText::new(label).color(theme::TEXT_DIM));
    ui.label(RichText::new(value).monospace());
    ui.end_row();
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
