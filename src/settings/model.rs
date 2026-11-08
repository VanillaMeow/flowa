//! Configuration values, defaults, and validation

use std::{
    ops::RangeInclusive,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

use crate::utils::units::MIB;
use strum::{AsRefStr, EnumString, VariantArray};

#[derive(
    Debug,
    Clone,
    Copy,
    Default,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    AsRefStr,
    EnumString,
    VariantArray,
)]
#[strum(ascii_case_insensitive)]
pub enum Theme {
    #[default]
    #[strum(
        to_string = "System / Qt",
        serialize = "System/Qt",
        serialize = "System"
    )]
    System,
    Charcoal,
    Midnight,
    Sweet,
    Nord,
    Catppuccin,
    Dracula,
    Forest,
    #[strum(to_string = "OLED")]
    Oled,
    Paper,
    Sand,
    Latte,
    #[strum(to_string = "High contrast", serialize = "HighContrast")]
    HighContrast,
}

#[derive(
    Debug,
    Clone,
    Copy,
    Default,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    AsRefStr,
    VariantArray,
)]
pub enum Accent {
    #[default]
    Theme,
    /// The magenta of the Sweet theme
    Sweet,
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

#[derive(
    Clone,
    Copy,
    Debug,
    Default,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    AsRefStr,
    VariantArray,
)]
pub enum Canvas {
    #[default]
    Theme,
    #[strum(to_string = "Neutral dark")]
    Dark,
    Black,
    White,
    Custom,
}

#[derive(
    Clone,
    Copy,
    Debug,
    Default,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    AsRefStr,
    VariantArray,
)]
pub enum SortOrder {
    #[default]
    Name,
    #[strum(to_string = "Date modified")]
    Modified,
    #[strum(to_string = "File size")]
    Size,
}

#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, AsRefStr,
)]
pub enum RendererPreference {
    #[default]
    Auto,
    Vulkan,
    #[strum(to_string = "OpenGL")]
    OpenGl,
    Metal,
    #[strum(to_string = "DirectX 12")]
    DirectX12,
}

impl RendererPreference {
    pub const SUPPORTED: &[Self] = &[
        Self::Auto,
        #[cfg(any(target_os = "linux", target_os = "windows"))]
        Self::Vulkan,
        #[cfg(target_os = "windows")]
        Self::DirectX12,
        #[cfg(target_os = "macos")]
        Self::Metal,
        #[cfg(any(target_os = "linux", target_os = "windows"))]
        Self::OpenGl,
    ];
}

#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, AsRefStr,
)]
pub enum VideoDecoder {
    #[default]
    Auto,
    #[strum(to_string = "NVDEC")]
    Nvdec,
    Vulkan,
    #[strum(to_string = "VA-API")]
    Vaapi,
    Software,
}

// Bounds shared by validation and the preference controls
pub const UI_SCALE: RangeInclusive<f32> = 0.75..=2.0;
pub const MEMORY_MB: RangeInclusive<u64> = 128..=4096;
pub const CHECKER_SIZE: RangeInclusive<f32> = 4.0..=64.0;
pub const CHECKER_CONTRAST: RangeInclusive<f32> = 0.02..=0.4;
pub const SLIDESHOW_SECONDS: RangeInclusive<f32> = 1.0..=120.0;
pub const WINDOW_WIDTH: RangeInclusive<f32> = 640.0..=7680.0;
pub const WINDOW_HEIGHT: RangeInclusive<f32> = 400.0..=4320.0;
pub const RECENT_LIMIT: usize = 12;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
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
    pub compact_video_controls: bool,
    pub ui_scale: f32,
    pub fit_small: bool,
    pub remember_view: bool,

    pub sort: SortOrder,
    pub reverse_sort: bool,
    pub wrap_navigation: bool,
    pub slideshow_seconds: f32,
    pub nearest: bool,
    pub renderer: RendererPreference,
    pub video_decoder: VideoDecoder,
    pub memory_mb: u64,

    /// Files and folders opened explicitly, newest first
    pub recent: Vec<PathBuf>,
    /// The file shown when the app last closed, used by `reopen_last`
    pub last_viewed: Option<PathBuf>,
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
            compact_video_controls: true,
            ui_scale: 1.0,
            fit_small: false,
            remember_view: false,
            sort: SortOrder::Name,
            reverse_sort: false,
            wrap_navigation: true,
            slideshow_seconds: 5.0,
            nearest: false,
            renderer: RendererPreference::Auto,
            video_decoder: VideoDecoder::Auto,
            memory_mb: 1024,
            recent: Vec::new(),
            last_viewed: None,
            reopen_last: false,
            window_size: [1200.0, 820.0],
        }
    }
}

impl Settings {
    pub fn validate(&mut self) {
        // Hand-edited or older preferences must stay inside supported bounds
        self.ui_scale = clamp(self.ui_scale, &UI_SCALE);
        self.memory_mb = clamp(self.memory_mb, &MEMORY_MB);
        self.checker_size = clamp(self.checker_size, &CHECKER_SIZE);
        self.checker_contrast = clamp(self.checker_contrast, &CHECKER_CONTRAST);
        self.slideshow_seconds = clamp(self.slideshow_seconds, &SLIDESHOW_SECONDS);
        self.window_size[0] = clamp(self.window_size[0], &WINDOW_WIDTH);
        self.window_size[1] = clamp(self.window_size[1], &WINDOW_HEIGHT);
        self.recent.truncate(RECENT_LIMIT);

        if !RendererPreference::SUPPORTED.contains(&self.renderer) {
            self.renderer = RendererPreference::Auto;
        }
    }

    /// The decoded image memory limit in bytes
    pub fn memory_budget(&self) -> u64 {
        self.memory_mb * MIB
    }

    /// Moves an explicitly opened file or folder to the front of the recent list
    pub fn remember(&mut self, path: PathBuf) {
        self.recent.retain(|recent| *recent != path);
        self.recent.insert(0, path);
        self.recent.truncate(RECENT_LIMIT);
    }

    /// Keeps recent entries pointing at a file after it is renamed
    pub fn rename_recent(&mut self, old: &Path, new: &Path) {
        for recent in &mut self.recent {
            if recent == old {
                *recent = new.to_path_buf();
            }
        }
    }

    pub fn forget(&mut self, path: &Path) {
        self.recent.retain(|recent| recent != path);
    }
}

fn clamp<T: PartialOrd + Copy>(value: T, range: &RangeInclusive<T>) -> T {
    if value < *range.start() {
        *range.start()
    } else if value > *range.end() {
        *range.end()
    } else {
        value
    }
}
