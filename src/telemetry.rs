//! What the audio side reports to the UI: numbers through atomics (safe to update
//! from the audio thread) and text messages through a channel (for everything else).

use std::sync::atomic::AtomicUsize;
use std::sync::mpsc;
use std::time::Instant;

use crate::dsp::controls::AtomicF32;

#[derive(Default)]
pub struct Stats {
    pub dropped_frames: AtomicUsize,
    /// Loudest sample since the UI last read it, before and after the effects.
    pub input_peak: AtomicF32,
    pub output_peak: AtomicF32,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Level {
    Info,
    Warn,
    Error,
}

pub struct LogEntry {
    pub at: Instant,
    pub level: Level,
    pub message: String,
}

/// Cheap to clone; hand one to every part of the app that has something to say.
/// Don't use it inside the audio callbacks: sending allocates.
#[derive(Clone)]
pub struct Logger {
    tx: mpsc::Sender<LogEntry>,
}

pub fn log_channel() -> (Logger, mpsc::Receiver<LogEntry>) {
    let (tx, rx) = mpsc::channel();
    (Logger { tx }, rx)
}

impl Logger {
    pub fn info(&self, message: impl Into<String>) {
        self.log(Level::Info, message.into());
    }

    pub fn warn(&self, message: impl Into<String>) {
        self.log(Level::Warn, message.into());
    }

    pub fn error(&self, message: impl Into<String>) {
        self.log(Level::Error, message.into());
    }

    fn log(&self, level: Level, message: String) {
        // Also printed, so logs survive if the UI never opens.
        match level {
            Level::Info => println!("{message}"),
            Level::Warn | Level::Error => eprintln!("{message}"),
        }
        let _ = self.tx.send(LogEntry {
            at: Instant::now(),
            level,
            message,
        });
    }
}
