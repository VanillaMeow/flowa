//! Configuration values, defaults, and validation

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
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
    #[strum(to_string = "System / Qt", serialize = "System/Qt")]
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
    pub compact_video_controls: bool,
    pub info: bool,
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
            compact_video_controls: true,
            info: false,
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

        if !RendererPreference::SUPPORTED.contains(&self.renderer) {
            self.renderer = RendererPreference::Auto;
        }
    }

    pub fn remember(&mut self, path: PathBuf) {
        self.recent.retain(|p| *p != path);
        self.recent.insert(0, path);
        self.recent.truncate(12);
    }
}
