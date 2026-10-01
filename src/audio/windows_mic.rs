//! Windows can only create audio devices through a driver. A virtual cable driver
//! exposes a virtual speaker whose audio comes back out of a virtual mic, so the
//! app's job is just to find that speaker by name and play into it.

use cpal::traits::HostTrait;

use super::devices;

/// What the app plays into, and the mic users pick in Meet, Discord...
pub struct Cable {
    pub speaker: &'static str,
    pub mic: &'static str,
}

// Tried in order. The Rust Phone driver doesn't exist yet; VB-CABLE stands in for
// it so the rest of the app can be tested on Windows.
const KNOWN_CABLES: [Cable; 2] = [
    Cable {
        speaker: "Rust Phone (app output)",
        mic: "Rust Phone",
    },
    Cable {
        speaker: "CABLE Input",
        mic: "CABLE Output",
    },
];

pub fn find() -> Option<(cpal::Device, &'static Cable)> {
    let outputs: Vec<(cpal::Device, String)> = cpal::default_host()
        .output_devices()
        .ok()?
        .map(|device| {
            let name = devices::name(&device);
            (device, name)
        })
        .collect();

    // Windows names endpoints like "CABLE Input (VB-Audio Virtual Cable)", hence starts_with.
    KNOWN_CABLES.iter().find_map(|cable| {
        outputs
            .iter()
            .find(|(_, name)| name.starts_with(cable.speaker))
            .map(|(device, _)| (device.clone(), cable))
    })
}
