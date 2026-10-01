//! The list of sound files, saved as one path per line so it survives restarts.

use std::path::{Path, PathBuf};

const LIBRARY_FILE: &str = "sounds.txt";

fn config_dir() -> Option<PathBuf> {
    if cfg!(target_os = "windows") {
        std::env::var_os("APPDATA").map(|dir| PathBuf::from(dir).join("Rust Phone"))
    } else {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
            .map(|dir| dir.join("rust-phone"))
    }
}

pub fn load() -> Vec<PathBuf> {
    let Some(path) = config_dir().map(|dir| dir.join(LIBRARY_FILE)) else {
        return Vec::new();
    };
    std::fs::read_to_string(path)
        .map(|contents| {
            contents
                .lines()
                .filter(|line| !line.trim().is_empty())
                .map(PathBuf::from)
                .collect()
        })
        .unwrap_or_default()
}

pub fn save(paths: &[&Path]) -> std::io::Result<()> {
    let dir = config_dir().ok_or_else(|| std::io::Error::other("no config directory"))?;
    std::fs::create_dir_all(&dir)?;
    let contents: String = paths
        .iter()
        .map(|path| format!("{}\n", path.display()))
        .collect();
    std::fs::write(dir.join(LIBRARY_FILE), contents)
}

/// The button label: the file name without its extension.
pub fn short_name(path: &Path) -> String {
    path.file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}
