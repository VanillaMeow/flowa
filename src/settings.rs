use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::theme::{Accent, Theme};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Canvas {
    #[default]
    Theme,
    Dark,
    Black,
    White,
    Custom,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum SortOrder {
    #[default]
    Name,
    Modified,
    Size,
}

impl SortOrder {
    pub fn name(self) -> &'static str {
        match self {
            Self::Name => "Name",
            Self::Modified => "Date modified",
            Self::Size => "File size",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub theme: Theme,
    pub accent: Accent,
    pub canvas: Canvas,
    pub canvas_color: [u8; 3],

    pub checkerboard: bool,
    pub checker_size: f32,
    pub checker_contrast: f32,

    pub filmstrip: bool,
    pub info: bool,
    pub ui_scale: f32,
    pub fit_small: bool,
    pub remember_view: bool,

    pub sort: SortOrder,
    pub reverse_sort: bool,
    pub wrap_navigation: bool,
    pub slideshow_seconds: f32,
    pub nearest: bool,
    pub memory_mb: u64,

    pub recent: Vec<PathBuf>,
    pub reopen_last: bool,
    pub window_size: [f32; 2],
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            theme: Theme::System,
            accent: Accent::Theme,
            canvas: Canvas::Theme,
            canvas_color: [24, 25, 28],
            checkerboard: false,
            checker_size: 12.0,
            checker_contrast: 0.09,
            filmstrip: false,
            info: false,
            ui_scale: 1.0,
            fit_small: false,
            remember_view: false,
            sort: SortOrder::Name,
            reverse_sort: false,
            wrap_navigation: true,
            slideshow_seconds: 5.0,
            nearest: false,
            memory_mb: 1024,
            recent: Vec::new(),
            reopen_last: false,
            window_size: [1200.0, 820.0],
        }
    }
}

impl Settings {
    pub fn config_path() -> Option<PathBuf> {
        directories::ProjectDirs::from("", "", "flowa")
            .map(|p| p.config_dir().join("settings.json"))
    }

    pub fn load(path: &Path) -> Result<Self> {
        let mut settings: Self = serde_json::from_slice(
            &std::fs::read(path).context("Could not read preferences")?,
        )
        .context("Could not parse preferences")?;

        // Hand-edited or older preferences must stay inside supported bounds
        settings.ui_scale = settings.ui_scale.clamp(0.75, 2.0);
        settings.memory_mb = settings.memory_mb.clamp(128, 4096);
        settings.checker_size = settings.checker_size.clamp(4.0, 64.0);
        settings.checker_contrast = settings.checker_contrast.clamp(0.02, 0.4);
        settings.slideshow_seconds = settings.slideshow_seconds.clamp(1.0, 120.0);
        settings.window_size[0] = settings.window_size[0].clamp(640.0, 7680.0);
        settings.window_size[1] = settings.window_size[1].clamp(400.0, 4320.0);
        settings.recent.truncate(12);

        Ok(settings)
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        let parent = path.parent().context("Preferences path has no parent")?;
        std::fs::create_dir_all(parent)
            .context("Could not create preferences directory")?;

        // Keep the previous file intact if writing the new settings fails
        let temporary = path.with_extension("json.tmp");
        std::fs::write(&temporary, serde_json::to_vec_pretty(self)?)
            .context("Could not write preferences")?;
        std::fs::rename(temporary, path).context("Could not save preferences")
    }

    pub fn remember(&mut self, path: PathBuf) {
        self.recent.retain(|p| *p != path);
        self.recent.insert(0, path);
        self.recent.truncate(12);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_round_trip_and_preserve_user_preferences() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("settings.json");
        let mut s = Settings {
            theme: Theme::Sand,
            checkerboard: true,
            ..Default::default()
        };
        s.remember(PathBuf::from("picture.png"));
        s.save(&path)?;
        s.accent = Accent::Rose;
        s.save(&path)?;
        let loaded = Settings::load(&path)?;
        assert_eq!(loaded.theme, Theme::Sand);
        assert_eq!(loaded.accent, Accent::Rose);
        assert!(loaded.checkerboard);
        assert_eq!(loaded.recent.len(), 1);
        Ok(())
    }

    #[test]
    fn old_preferences_get_new_defaults() -> Result<()> {
        let s: Settings = serde_json::from_str("{\"filmstrip\":true}")?;
        assert!(s.filmstrip);
        assert!(!s.checkerboard);
        Ok(())
    }
}
