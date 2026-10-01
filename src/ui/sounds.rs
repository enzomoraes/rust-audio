use std::path::{Path, PathBuf};
use std::sync::{Arc, mpsc};

use eframe::egui;
use egui::{Align, CornerRadius, Layout, RichText, Stroke};

use super::theme;
use crate::dsp::Frame;
use crate::soundboard::{Sound, SoundboardRemote, decode, library};
use crate::telemetry::Logger;

const EXTENSIONS: [&str; 2] = ["mp3", "wav"];
const TILE_SIZE: egui::Vec2 = egui::vec2(132.0, 54.0);
const MAX_NAME_CHARS: usize = 16;

enum State {
    Loading,
    Ready(Arc<Sound>),
    Failed(String),
}

struct Entry {
    path: PathBuf,
    name: String,
    state: State,
    /// Copies of this sound the mixer is playing right now.
    playing: usize,
}

type Decoded = (PathBuf, Result<Vec<Frame>, String>);

/// The soundboard section: the list of clips, loading them, and the buttons that play them.
pub struct Sounds {
    entries: Vec<Entry>,
    sample_rate: u32,
    decoded_tx: mpsc::Sender<Decoded>,
    decoded_rx: mpsc::Receiver<Decoded>,
    logger: Logger,
}

impl Sounds {
    pub fn new(sample_rate: u32, logger: Logger) -> Self {
        let (decoded_tx, decoded_rx) = mpsc::channel();
        let mut sounds = Self {
            entries: Vec::new(),
            sample_rate,
            decoded_tx,
            decoded_rx,
            logger,
        };
        for path in library::load() {
            sounds.add(path, false);
        }
        sounds
    }

    pub fn is_supported(path: &Path) -> bool {
        path.extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| EXTENSIONS.contains(&extension.to_lowercase().as_str()))
    }

    pub fn add_files(&mut self, paths: impl IntoIterator<Item = PathBuf>) {
        let mut added = false;
        for path in paths {
            if Self::is_supported(&path) {
                added |= self.add(path, true);
            } else {
                self.logger
                    .warn(format!("{} isn't an MP3 or WAV file", path.display()));
            }
        }
        if added {
            self.save();
        }
    }

    fn add(&mut self, path: PathBuf, announce: bool) -> bool {
        if self.entries.iter().any(|entry| entry.path == path) {
            return false;
        }
        if announce {
            self.logger
                .info(format!("Loading sound {}", library::short_name(&path)));
        }

        // Decoding a long file takes a moment; keep it off both the UI and audio threads.
        let tx = self.decoded_tx.clone();
        let sample_rate = self.sample_rate;
        let thread_path = path.clone();
        std::thread::spawn(move || {
            let result = decode::decode_file(&thread_path, sample_rate);
            let _ = tx.send((thread_path, result));
        });

        self.entries.push(Entry {
            name: library::short_name(&path),
            path,
            state: State::Loading,
            playing: 0,
        });
        true
    }

    fn remove(&mut self, index: usize) {
        // If it's playing, the mixer still holds its own Arc and hands it back later.
        self.entries.remove(index);
        self.save();
    }

    fn save(&self) {
        let paths: Vec<&Path> = self
            .entries
            .iter()
            .map(|entry| entry.path.as_path())
            .collect();
        if let Err(err) = library::save(&paths) {
            self.logger
                .error(format!("could not save the sound list: {err}"));
        }
    }

    pub fn poll(&mut self, remote: Option<&mut SoundboardRemote>) {
        while let Ok((path, result)) = self.decoded_rx.try_recv() {
            let Some(entry) = self.entries.iter_mut().find(|entry| entry.path == path) else {
                continue;
            };
            entry.state = match result {
                Ok(frames) => State::Ready(Arc::new(Sound { frames })),
                Err(err) => {
                    self.logger
                        .error(format!("could not load {}: {err}", entry.name));
                    State::Failed(err)
                }
            };
        }

        if let Some(remote) = remote {
            // Dropping these here, on the UI thread, is the point of sending them back.
            for finished in remote.finished() {
                let entry = self.entries.iter_mut().find(|entry| {
                    matches!(&entry.state, State::Ready(sound) if Arc::ptr_eq(sound, &finished))
                });
                if let Some(entry) = entry {
                    entry.playing = entry.playing.saturating_sub(1);
                }
            }
        }
    }

    pub fn show(&mut self, ui: &mut egui::Ui, mut remote: Option<&mut SoundboardRemote>) {
        ui.horizontal(|ui| {
            ui.label(RichText::new("Sounds").strong().size(18.0));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui.button("Add sound").clicked() {
                    let picked = rfd::FileDialog::new()
                        .add_filter("Audio", &EXTENSIONS)
                        .pick_files();
                    if let Some(paths) = picked {
                        self.add_files(paths);
                    }
                }
                let any_playing = self.entries.iter().any(|entry| entry.playing > 0);
                if ui
                    .add_enabled(any_playing, egui::Button::new("Stop all"))
                    .clicked()
                    && let Some(remote) = remote.as_deref_mut()
                {
                    remote.stop_all();
                }
            });
        });
        ui.label(
            RichText::new(
                "Played clean, without the effects. Click again to restart, right-click to remove.",
            )
            .color(theme::TEXT_DIM),
        );

        if let Some(remote) = remote.as_deref() {
            let volume = remote.volume();
            let mut value = volume.get();
            ui.spacing_mut().slider_width = (ui.available_width() - 150.0).max(60.0);
            let slider = egui::Slider::new(&mut value, volume.min..=volume.max)
                .text(volume.name)
                .suffix(volume.unit)
                .fixed_decimals(0);
            let response = ui.add(slider);
            if response.double_clicked() {
                volume.set(volume.default);
            } else if response.changed() {
                volume.set(value);
            }
        }
        ui.add_space(6.0);

        let dragging_files = ui.input(|input| !input.raw.hovered_files.is_empty());
        if self.entries.is_empty() || dragging_files {
            drop_hint(ui, dragging_files);
        }

        let mut to_remove = None;
        ui.horizontal_wrapped(|ui| {
            for (index, entry) in self.entries.iter_mut().enumerate() {
                let response = tile(ui, entry, remote.is_some(), self.sample_rate);
                if response.clicked()
                    && let (State::Ready(sound), Some(remote)) =
                        (&entry.state, remote.as_deref_mut())
                    && remote.play(sound)
                {
                    entry.playing += 1;
                }
                response.context_menu(|ui| {
                    if ui.button("Remove").clicked() {
                        to_remove = Some(index);
                        ui.close();
                    }
                });
            }
        });
        if let Some(index) = to_remove {
            self.remove(index);
        }
    }
}

fn tile(ui: &mut egui::Ui, entry: &Entry, audio_running: bool, sample_rate: u32) -> egui::Response {
    let (label, color, ready) = match &entry.state {
        State::Loading => ("Loading...".to_string(), theme::TEXT_DIM, false),
        State::Ready(_) => (truncate(&entry.name), egui::Color32::WHITE, true),
        State::Failed(_) => (truncate(&entry.name), theme::ERROR, false),
    };
    let playing = entry.playing > 0;
    let (fill, border) = if playing {
        (theme::ACCENT, theme::ACCENT)
    } else {
        (theme::CARD, theme::CARD_BORDER)
    };

    let button = egui::Button::new(RichText::new(label).strong().color(color))
        .min_size(TILE_SIZE)
        .fill(fill)
        .stroke(Stroke::new(1.0, border))
        .corner_radius(CornerRadius::same(10));
    let response = ui.add_enabled(ready && audio_running, button);

    let tooltip = match &entry.state {
        State::Loading => format!("{}\nDecoding...", entry.path.display()),
        State::Ready(sound) => format!(
            "{}\n{:.1} s",
            entry.path.display(),
            sound.frames.len() as f32 / sample_rate as f32
        ),
        State::Failed(err) => format!("{}\n{err}", entry.path.display()),
    };
    response
        .on_hover_text(tooltip)
        .on_disabled_hover_text(match &entry.state {
            State::Failed(err) => err.clone(),
            _ => entry.path.display().to_string(),
        })
}

fn drop_hint(ui: &mut egui::Ui, dragging_files: bool) {
    let (stroke, text) = if dragging_files {
        (theme::ACCENT, "Drop to add these sounds")
    } else {
        (
            theme::CARD_BORDER,
            "Drop MP3 or WAV files here, or click Add sound",
        )
    };
    egui::Frame::NONE
        .stroke(Stroke::new(1.5, stroke))
        .corner_radius(CornerRadius::same(10))
        .inner_margin(egui::Margin::same(18))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.vertical_centered(|ui| {
                ui.label(RichText::new(text).color(theme::TEXT_DIM));
            });
        });
    ui.add_space(6.0);
}

fn truncate(name: &str) -> String {
    if name.chars().count() <= MAX_NAME_CHARS {
        name.to_string()
    } else {
        let short: String = name.chars().take(MAX_NAME_CHARS - 3).collect();
        format!("{short}...")
    }
}
