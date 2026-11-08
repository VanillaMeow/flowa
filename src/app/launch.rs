//! Startup options passed from the native entry point

use std::path::PathBuf;

use crate::settings::Settings;

pub struct Launch {
    pub path: Option<PathBuf>,
    pub preferences: bool,
    pub filmstrip: bool,
    pub info: bool,
    pub zoom: Option<f32>,
}

impl Launch {
    /// The file or folder shown at launch: the command-line path, or the reopened file
    pub fn initial_path(&self, settings: &Settings) -> Option<PathBuf> {
        if let Some(path) = &self.path {
            return Some(path.clone());
        }

        if !settings.reopen_last {
            return None;
        }

        // Older preferences have no last viewed file, only the recent list
        settings
            .last_viewed
            .clone()
            .or_else(|| settings.recent.first().cloned())
    }
}
