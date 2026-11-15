//! Video sessions backed by FFmpeg decoding and Vulkan presentation when the build has them

mod session;
pub(crate) mod subtitles;

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

#[cfg(test)]
mod tests;
#[cfg(not(video_playback))]
mod unsupported;
#[cfg(not(video_playback))]
pub(crate) use unsupported::gpu;
#[cfg(not(video_playback))]
use unsupported::{playback, thumbnail};

use std::path::Path;

use anyhow::Result;

pub(crate) use session::VideoSession;

use crate::{
    services::worker::Generation, settings::VideoDecoder,
    viewer::adjustments::Adjustments,
};

/// Extracts filmstrip pixels without requiring an initialized playback device
pub fn thumbnail(
    path: &Path,
    generation: &Generation,
    expected: u64,
) -> Result<egui::ColorImage> {
    thumbnail::extract(path, generation, expected)
}

/// One audio or subtitle stream the user can choose
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Track {
    /// The stream's index in its container
    pub index: usize,
    /// Title, language, and format, as far as the file describes them
    pub label: String,
    /// The file marks this stream as the one to use by default
    pub default: bool,
    /// Subtitles meant to be shown even to viewers who did not ask for subtitles
    pub forced: bool,
    /// Read from a subtitle file beside the video rather than from the container
    pub external: bool,
}

/// The audio and subtitle streams of the playing video
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Tracks {
    pub audio: Vec<Track>,
    pub subtitles: Vec<Track>,
}

impl Tracks {
    /// The subtitles to show without being asked: forced ones, then ones the file
    /// marks as default, then a subtitle file placed beside the video
    pub fn preferred_subtitles(&self) -> Option<usize> {
        let pick = |wanted: fn(&Track) -> bool| {
            self.subtitles
                .iter()
                .find(|track| wanted(track))
                .map(|track| track.index)
        };

        pick(|track| track.forced)
            .or_else(|| pick(|track| track.default))
            .or_else(|| pick(|track| track.external))
    }
}

/// Playback speeds offered by the controls, slowest first
pub const PLAYBACK_RATES: &[f64] =
    &[0.25, 0.5, 0.75, 1.0, 1.25, 1.5, 1.75, 2.0, 3.0, 4.0];

/// The offered speed `steps` places faster or slower than `rate`, stopping at the ends
pub fn step_rate(rate: f64, steps: i32) -> f64 {
    let nearest = PLAYBACK_RATES
        .iter()
        .enumerate()
        .min_by(|(_, a), (_, b)| (*a - rate).abs().total_cmp(&(*b - rate).abs()))
        .map_or(0, |(index, _)| index);
    let index = nearest.saturating_add_signed(steps as isize);

    PLAYBACK_RATES[index.min(PLAYBACK_RATES.len() - 1)]
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
/// Times are seconds, volume is a percentage, rate is a speed multiplier, and size
/// includes display aspect ratio
#[derive(Clone, Debug, PartialEq)]
pub struct VideoState {
    pub active: bool,
    pub loading: bool,
    pub seeking: bool,
    pub paused: bool,
    pub position: f64,
    pub duration: f64,
    pub volume: f64,
    pub rate: f64,
    pub size: [u32; 2],
    /// The decoder producing frames, or `None` while candidates are still being tried
    pub decoder: Option<VideoDecoder>,
    /// Set once frames come from a different decoder than expected
    pub fallback: Option<DecoderFallback>,
    /// Why software decoding took over, one line per hardware decoder that failed
    pub software_reason: Option<String>,
    pub codec: &'static str,
    pub audio: Option<AudioInfo>,
    pub tracks: Tracks,
    /// The container index of the audio stream being played
    pub audio_track: Option<usize>,
    /// The subtitle track being shown; files continue after the container's streams
    pub subtitle_track: Option<usize>,
    /// The cues to draw over the current frame
    pub subtitles: subtitles::ActiveSubtitles,
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
            rate: 1.,
            size: [0; 2],
            decoder: None,
            fallback: None,
            software_reason: None,
            codec: "",
            audio: None,
            tracks: Tracks::default(),
            audio_track: None,
            subtitle_track: None,
            subtitles: subtitles::ActiveSubtitles::default(),
            error: None,
        }
    }
}

/// Requests sent from the UI to the playback engine
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum PlayerCommand {
    TogglePause,
    /// Pauses, then shows the next frame for positive counts or seeks back for negative
    Step(i32),
    Seek(f64),
    Volume(f64),
    /// Playback speed, which also applies to later videos
    Rate(f64),
    Decoder(VideoDecoder),
    /// Brightness, contrast, saturation, and gamma for every later frame
    Adjust(Adjustments),
    /// Plays the audio stream with this container index from the current position
    AudioTrack(usize),
    /// Shows this subtitle track, or none
    Subtitles(Option<usize>),
}
