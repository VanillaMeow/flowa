//! Configuration values, defaults, and validation

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Theme {
    #[default]
    System,
    Charcoal,
    Midnight,
    Nord,
    Catppuccin,
    Dracula,
    Forest,
    Oled,
    Paper,
    Sand,
    Latte,
    HighContrast,
}

impl Theme {
    pub const ALL: [Self; 12] = [
        Self::System,
        Self::Charcoal,
        Self::Midnight,
        Self::Nord,
        Self::Catppuccin,
        Self::Dracula,
        Self::Forest,
        Self::Oled,
        Self::Paper,
        Self::Sand,
        Self::Latte,
        Self::HighContrast,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::System => "System / Qt",
            Self::Charcoal => "Charcoal",
            Self::Midnight => "Midnight",
            Self::Nord => "Nord",
            Self::Catppuccin => "Catppuccin",
            Self::Dracula => "Dracula",
            Self::Forest => "Forest",
            Self::Oled => "OLED",
            Self::Paper => "Paper",
            Self::Sand => "Sand",
            Self::Latte => "Latte",
            Self::HighContrast => "High contrast",
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Accent {
    #[default]
    Theme,
    Violet,
    Iris,
    Blue,
    Sky,
    Teal,
    Mint,
    Green,
    Amber,
    Peach,
    Rose,
    Coral,
}

impl Accent {
    pub const ALL: [Self; 12] = [
        Self::Theme,
        Self::Violet,
        Self::Iris,
        Self::Blue,
        Self::Sky,
        Self::Teal,
        Self::Mint,
        Self::Green,
        Self::Amber,
        Self::Peach,
        Self::Rose,
        Self::Coral,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::Theme => "Theme",
            Self::Violet => "Violet",
            Self::Iris => "Iris",
            Self::Blue => "Blue",
            Self::Sky => "Sky",
            Self::Teal => "Teal",
            Self::Mint => "Mint",
            Self::Green => "Green",
            Self::Amber => "Amber",
            Self::Peach => "Peach",
            Self::Rose => "Rose",
            Self::Coral => "Coral",
        }
    }
}

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
    pub fn validate(&mut self) {
        // Hand-edited or older preferences must stay inside supported bounds
        self.ui_scale = self.ui_scale.clamp(0.75, 2.0);
        self.memory_mb = self.memory_mb.clamp(128, 4096);
        self.checker_size = self.checker_size.clamp(4.0, 64.0);
        self.checker_contrast = self.checker_contrast.clamp(0.02, 0.4);
        self.slideshow_seconds = self.slideshow_seconds.clamp(1.0, 120.0);
        self.window_size[0] = self.window_size[0].clamp(640.0, 7680.0);
        self.window_size[1] = self.window_size[1].clamp(400.0, 4320.0);
        self.recent.truncate(12);
    }

    pub fn remember(&mut self, path: PathBuf) {
        self.recent.retain(|p| *p != path);
        self.recent.insert(0, path);
        self.recent.truncate(12);
    }
}

#[cfg(test)]
mod tests {
    use anyhow::Result;

    use super::*;

    #[test]
    fn old_preferences_get_new_defaults() -> Result<()> {
        let s: Settings = serde_json::from_str("{\"filmstrip\":true}")?;
        assert!(s.filmstrip);
        assert!(!s.checkerboard);
        Ok(())
    }
}
