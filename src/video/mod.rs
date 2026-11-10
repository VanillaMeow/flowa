//! Video sessions backed by FFmpeg decoding and Vulkan presentation when the build has them

mod session;

#[cfg(video_playback)]
mod audio;
#[cfg(video_playback)]
mod decoding;
#[cfg(video_playback)]
mod demux;
#[cfg(video_playback)]
mod ffi;
#[cfg(video_playback)]
mod ffmpeg;
#[cfg(video_playback)]
pub(crate) mod gpu;
#[cfg(video_playback)]
mod playback;
#[cfg(video_playback)]
mod presentation;
#[cfg(video_playback)]
mod thumbnail;

#[cfg(not(video_playback))]
mod unsupported;
#[cfg(not(video_playback))]
pub(crate) use unsupported::gpu;
#[cfg(not(video_playback))]
use unsupported::{playback, thumbnail};

use std::path::Path;

use anyhow::Result;

pub(crate) use session::VideoSession;

use crate::{services::worker::Generation, settings::VideoDecoder};

/// Extracts filmstrip pixels without requiring an initialized playback device
pub fn thumbnail(
    path: &Path,
    generation: &Generation,
    expected: u64,
) -> Result<egui::ColorImage> {
    thumbnail::extract(path, generation, expected)
}

/// Source audio properties before playback resampling
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AudioInfo {
    pub codec: &'static str,
    pub channels: u32,
    pub sample_rate: u32,
    pub bit_rate: u64,
}

/// A decoder that couldn't play the video and the one that took over
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DecoderFallback {
    pub from: VideoDecoder,
    pub to: VideoDecoder,
}

impl DecoderFallback {
    pub fn message(self) -> String {
        let replacement = match self.to {
            VideoDecoder::Software => "software decoding".to_owned(),
            hardware => hardware.as_ref().to_owned(),
        };

        format!(
            "Switched to {replacement} because {} couldn't play this video",
            self.from.as_ref()
        )
    }
}

/// Playback properties reported by the FFmpeg player
///
/// Times are seconds, volume is a percentage, and size includes display aspect ratio
#[derive(Clone, Debug, PartialEq)]
pub struct VideoState {
    pub active: bool,
    pub loading: bool,
    pub seeking: bool,
    pub paused: bool,
    pub position: f64,
    pub duration: f64,
    pub volume: f64,
    pub size: [u32; 2],
    /// The decoder producing frames, or `None` while candidates are still being tried
    pub decoder: Option<VideoDecoder>,
    /// Set once frames come from a different decoder than expected
    pub fallback: Option<DecoderFallback>,
    /// Why software decoding took over, one line per hardware decoder that failed
    pub software_reason: Option<String>,
    pub codec: &'static str,
    pub audio: Option<AudioInfo>,
    pub error: Option<String>,
}

impl VideoState {
    pub fn codec_description(&self) -> &str {
        match self.codec {
            "h264" => "H.264 / AVC",
            "hevc" => "H.265 / HEVC",
            "av1" => "AV1",
            "vp9" => "VP9",
            "vp8" => "VP8",
            "mpeg4" => "MPEG-4",
            other => other,
        }
    }

    /// Controls become interactive after loading finishes without a playback error
    pub fn ready(&self) -> bool {
        self.active && !self.loading && self.error.is_none()
    }

    pub fn decoder_description(&self) -> String {
        match self.decoder {
            None | Some(VideoDecoder::Auto) => "Selecting video decoder".into(),
            Some(VideoDecoder::Software) => "Software decoding · CPU".into(),
            Some(hardware) => format!("Hardware decoding · {}", hardware.as_ref()),
        }
    }
}

impl Default for VideoState {
    fn default() -> Self {
        Self {
            active: false,
            loading: false,
            seeking: false,
            paused: false,
            position: 0.,
            duration: 0.,
            volume: 100.,
            size: [0; 2],
            decoder: None,
            fallback: None,
            software_reason: None,
            codec: "",
            audio: None,
            error: None,
        }
    }
}

/// Requests sent from the UI to the playback engine
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum PlayerCommand {
    TogglePause,
    Seek(f64),
    Volume(f64),
    Decoder(VideoDecoder),
}
