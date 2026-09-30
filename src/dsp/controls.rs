//! Values shared between the UI thread (which writes them) and the audio thread
//! (which reads them once per block). Everything is atomic, so neither side ever
//! waits for the other.

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

/// An `f32` stored as its bit pattern in an `AtomicU32`; std has no atomic float.
#[derive(Default)]
pub struct AtomicF32(AtomicU32);

impl AtomicF32 {
    pub fn new(value: f32) -> Self {
        Self(AtomicU32::new(value.to_bits()))
    }

    pub fn load(&self) -> f32 {
        f32::from_bits(self.0.load(Ordering::Relaxed))
    }

    pub fn store(&self, value: f32) {
        self.0.store(value.to_bits(), Ordering::Relaxed);
    }

    /// Keeps the largest value stored since the last `take`. Only valid for values
    /// >= 0: for those, a bigger float also has a bigger bit pattern.
    pub fn store_max(&self, value: f32) {
        self.0.fetch_max(value.to_bits(), Ordering::Relaxed);
    }

    pub fn take(&self) -> f32 {
        f32::from_bits(self.0.swap(0, Ordering::Relaxed))
    }
}

pub struct Param {
    pub name: &'static str,
    pub unit: &'static str,
    pub min: f32,
    pub max: f32,
    pub default: f32,
    value: AtomicF32,
}

impl Param {
    pub fn new(name: &'static str, unit: &'static str, min: f32, max: f32, default: f32) -> Self {
        Self {
            name,
            unit,
            min,
            max,
            default,
            value: AtomicF32::new(default),
        }
    }

    pub fn get(&self) -> f32 {
        self.value.load()
    }

    pub fn set(&self, value: f32) {
        self.value.store(value.clamp(self.min, self.max));
    }
}

/// What the UI can see and change about one effect.
pub struct EffectControls {
    pub name: &'static str,
    pub description: &'static str,
    pub params: Vec<Param>,
    enabled: AtomicBool,
}

impl EffectControls {
    pub fn new(name: &'static str, description: &'static str, params: Vec<Param>) -> Self {
        Self {
            name,
            description,
            params,
            enabled: AtomicBool::new(true),
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled.load(Ordering::Relaxed)
    }

    pub fn set_enabled(&self, enabled: bool) {
        self.enabled.store(enabled, Ordering::Relaxed);
    }
}
