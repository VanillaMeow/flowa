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

/// Where the video controls sit and how they share space with the video
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
pub enum VideoControlsStyle {
    /// The timeline on its own row above the buttons, below the video
    #[strum(to_string = "Two rows")]
    TwoRows,
    /// The timeline between the buttons on one row, below the video
    Compact,
    /// Floating over the bottom of the video and hidden while the pointer rests
    #[default]
    Overlay,
}

impl VideoControlsStyle {
    /// One line on how the style looks
    pub fn description(self) -> &'static str {
        match self {
            Self::TwoRows => "A full-width timeline above the buttons, below the video",
            Self::Compact => "The timeline and buttons on one row, below the video",
            Self::Overlay => "Floats over the video and hides when the pointer rests",
        }
    }
}

/// The gradient behind overlaid video controls
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
pub enum OverlayBackdrop {
    /// Black with light controls, readable over any video
    #[default]
    Black,
    /// The theme's panel color with its own text color
    Theme,
}

/// What happens to the pitch of audio when a video plays faster or slower
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
pub enum SpeedPitch {
    /// Pitch rises and falls with speed, like a tape played faster or slower
    #[strum(to_string = "Follow speed")]
    Follow,
    /// Slower speeds lower the pitch, while faster ones keep it
    #[default]
    #[strum(to_string = "Lower only")]
    Lower,
    /// Voices keep their pitch, as audio is stretched in time
    #[strum(to_string = "Keep")]
    Keep,
}

impl SpeedPitch {
    /// Whether audio at this speed is stretched in time to keep its pitch, rather
    /// than resampled
    #[cfg_attr(not(video_playback), allow(dead_code))]
    pub fn stretches(self, rate: f64) -> bool {
        match self {
            Self::Follow => false,
            Self::Lower => rate > 1.,
            Self::Keep => true,
        }
    }

    /// A one-word name for tight spaces such as the speed popup
    pub fn short_label(self) -> &'static str {
        match self {
            Self::Follow => "Follow",
            Self::Lower => "Lower",
            Self::Keep => "Keep",
        }
    }

    /// One line on what the choice sounds like
    pub fn description(self) -> &'static str {
        match self {
            Self::Follow => "Slower sounds deeper and faster higher, like a record",
            Self::Lower => "Slower sounds deeper; faster keeps voices at their pitch",
            Self::Keep => "Voices sound the same at any speed",
        }
    }
}

// Bounds shared by validation and the preference controls
pub const UI_SCALE: RangeInclusive<f32> = 0.75..=2.0;
pub const MEMORY_MB: RangeInclusive<u64> = 128..=4096;
pub const CHECKER_SIZE: RangeInclusive<f32> = 4.0..=64.0;
pub const CHECKER_CONTRAST: RangeInclusive<f32> = 0.02..=0.4;
pub const SLIDESHOW_SECONDS: RangeInclusive<f32> = 1.0..=120.0;
pub const WINDOW_WIDTH: RangeInclusive<f32> = 640.0..=7680.0;
pub const WINDOW_HEIGHT: RangeInclusive<f32> = 400.0..=4320.0;
/// The 58-point toolbar and 32-point status bar leave a 1280 × 720 canvas, which a
/// 16:9 video fills edge to edge
const DEFAULT_WINDOW_SIZE: [f32; 2] = [1280.0, 810.0];
/// The default before it was sized for 16:9 videos
const OLD_DEFAULT_WINDOW_SIZE: [f32; 2] = [1200.0, 820.0];
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
    pub video_controls: VideoControlsStyle,
    pub overlay_backdrop: OverlayBackdrop,
    /// The checkbox that preceded `video_controls`, read once from older preferences
    #[serde(rename = "compact_video_controls", skip_serializing)]
    pub(crate) legacy_compact_controls: Option<bool>,
    pub ui_scale: f32,
    pub fit_small: bool,
    /// Fitted images keep a margin from the canvas edges outside fullscreen
    pub image_margin: bool,
    /// Fitted videos keep a margin from the canvas edges outside fullscreen
    pub video_margin: bool,
    pub remember_view: bool,

    pub sort: SortOrder,
    pub reverse_sort: bool,
    pub wrap_navigation: bool,
    pub slideshow_seconds: f32,
    pub nearest: bool,
    pub renderer: RendererPreference,
    pub video_decoder: VideoDecoder,
    pub speed_pitch: SpeedPitch,
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
            video_controls: VideoControlsStyle::Overlay,
            overlay_backdrop: OverlayBackdrop::Black,
            legacy_compact_controls: None,
            ui_scale: 1.0,
            fit_small: false,
            image_margin: true,
            video_margin: false,
            remember_view: false,
            sort: SortOrder::Name,
            reverse_sort: false,
            wrap_navigation: true,
            slideshow_seconds: 5.0,
            nearest: false,
            renderer: RendererPreference::Auto,
            video_decoder: VideoDecoder::Auto,
            speed_pitch: SpeedPitch::Lower,
            memory_mb: 1024,
            recent: Vec::new(),
            last_viewed: None,
            reopen_last: false,
            window_size: DEFAULT_WINDOW_SIZE,
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

        // A window still at the old default was most likely never resized, so it moves
        // to the new one; any other size was chosen and stays
        if self.window_size == OLD_DEFAULT_WINDOW_SIZE {
            self.window_size = DEFAULT_WINDOW_SIZE;
        }

        if !RendererPreference::SUPPORTED.contains(&self.renderer) {
            self.renderer = RendererPreference::Auto;
        }

        // Older preferences saved a checkbox that was on by default, so only turning it
        // off was a choice; that keeps two rows and the default moves to the overlay
        if let Some(compact) = self.legacy_compact_controls.take() {
            self.video_controls = if compact {
                VideoControlsStyle::Overlay
            } else {
                VideoControlsStyle::TwoRows
            };
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
