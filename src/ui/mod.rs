mod chain;
mod sounds;
mod theme;
mod widgets;

use std::collections::VecDeque;
use std::sync::atomic::Ordering;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use eframe::egui;
use egui::{Align, Layout, RichText};

use crate::audio::{Engine, Remotes};
use crate::telemetry::{Level, LogEntry, Logger};
use sounds::Sounds;
use widgets::Meter;

const APP_NAME: &str = "Rust Phone";
const MAX_LOG_ENTRIES: usize = 500;
// Meters need a steady refresh even when nobody touches the window.
const REPAINT_EVERY: Duration = Duration::from_millis(33);
// Below this width, effects and sounds are stacked instead of side by side.
const TWO_COLUMNS_MIN_WIDTH: f32 = 760.0;
// Used to decode sounds when audio isn't running, so they're still listed.
const FALLBACK_SAMPLE_RATE: u32 = 48_000;

pub fn run(
    engine: Result<(Engine, Remotes), String>,
    logs: mpsc::Receiver<LogEntry>,
    logger: Logger,
    started: Instant,
) -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title(APP_NAME)
            .with_inner_size([1200.0, 760.0])
            .with_min_inner_size([760.0, 520.0]),
        ..Default::default()
    };
    eframe::run_native(
        APP_NAME,
        options,
        Box::new(move |cc| {
            theme::apply(&cc.egui_ctx);
            Ok(Box::new(App::new(engine, logs, logger, started)))
        }),
    )
}

struct Running {
    engine: Engine,
    remotes: Remotes,
}

struct App {
    running: Result<Running, String>,
    sounds: Sounds,
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
        engine: Result<(Engine, Remotes), String>,
        logs: mpsc::Receiver<LogEntry>,
        logger: Logger,
        started: Instant,
    ) -> Self {
        let sample_rate = engine
            .as_ref()
            .map(|(engine, _)| engine.info().sample_rate)
            .unwrap_or(FALLBACK_SAMPLE_RATE);
        Self {
            running: engine.map(|(engine, remotes)| Running { engine, remotes }),
            sounds: Sounds::new(sample_rate, logger),
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

    fn poll(&mut self, ctx: &egui::Context) {
        while let Ok(entry) = self.logs.try_recv() {
            self.push_entry(entry);
        }

        let dropped_files: Vec<_> = ctx.input(|input| {
            input
                .raw
                .dropped_files
                .iter()
                .filter_map(|file| file.path.clone())
                .collect()
        });
        if !dropped_files.is_empty() {
            self.sounds.add_files(dropped_files);
        }
        self.sounds.poll(
            self.running
                .as_mut()
                .ok()
                .map(|running| &mut running.remotes.soundboard),
        );

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
        self.poll(&ui.ctx().clone());
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
            .frame(panel_frame(theme::BG, 16))
            .show(ui, |ui| self.main_area(ui));
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

    /// Effects and sounds: side by side when there's room, stacked otherwise.
    fn main_area(&mut self, ui: &mut egui::Ui) {
        let running = match &mut self.running {
            Ok(running) => running,
            Err(err) => {
                ui.add_space(40.0);
                ui.vertical_centered(|ui| {
                    ui.label(RichText::new("Audio could not start").strong().size(16.0));
                    ui.label(RichText::new(err.as_str()).color(theme::ERROR));
                });
                return;
            }
        };
        let destination = output_summary(&running.engine);
        let pipeline = &mut running.remotes.pipeline;
        let soundboard = &mut running.remotes.soundboard;
        let sounds = &mut self.sounds;

        if ui.available_width() >= TWO_COLUMNS_MIN_WIDTH {
            ui.columns(2, |columns| {
                egui::ScrollArea::vertical()
                    .id_salt("effects")
                    .show(&mut columns[0], |ui| {
                        chain::show(ui, pipeline, &destination)
                    });
                egui::ScrollArea::vertical()
                    .id_salt("sounds")
                    .show(&mut columns[1], |ui| sounds.show(ui, Some(soundboard)));
            });
        } else {
            egui::ScrollArea::vertical().show(ui, |ui| {
                chain::show(ui, pipeline, &destination);
                ui.add_space(20.0);
                sounds.show(ui, Some(soundboard));
            });
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
