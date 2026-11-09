//! Video sessions backed by FFmpeg decoding and Vulkan presentation when the build has them

#[cfg(video_playback)]
mod ffmpeg;
#[cfg(video_playback)]
pub(crate) mod gpu;
#[cfg(video_playback)]
mod media;
#[cfg(video_playback)]
mod native;
#[cfg(video_playback)]
mod player;
#[cfg(video_playback)]
mod thumbnail;

#[cfg(not(video_playback))]
mod unsupported;
#[cfg(not(video_playback))]
pub(crate) use unsupported::gpu;
#[cfg(not(video_playback))]
use unsupported::{player, thumbnail};

use std::path::Path;

use anyhow::{Context, Result, bail};

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

/// Coordinates playback state and the texture displayed by the image canvas
#[derive(Default)]
pub struct VideoSession {
    pub state: VideoState,
    pub texture: Option<egui::TextureId>,
    player: Option<player::Player>,
    unavailable: Option<String>,
    notice: Option<String>,
}

impl VideoSession {
    /// Initializes playback only after the application's compatible device exists
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let player = cc
            .wgpu_render_state
            .as_ref()
            .context("Video playback needs the wgpu renderer")
            .and_then(|graphics| player::Player::new(graphics, &cc.egui_ctx));

        match player {
            Ok(player) => Self {
                player: Some(player),
                ..Default::default()
            },
            Err(error) => {
                log::info!("Video backend unavailable: {error:#}");
                Self {
                    unavailable: Some(format!("{error:#}")),
                    ..Default::default()
                }
            }
        }
    }

    /// Replaces the selected file while keeping the user's volume
    pub fn open(&mut self, path: &Path) {
        self.close();
        self.state = VideoState {
            active: true,
            loading: true,
            volume: self.state.volume,
            ..Default::default()
        };

        let Some(player) = &mut self.player else {
            self.state.loading = false;
            self.state.error = Some(self.unavailable_reason().to_owned());
            return;
        };

        let volume = self.state.volume;
        let opened = player
            .load(path)
            .and_then(|()| player.command(PlayerCommand::Volume(volume)));

        if let Err(error) = opened {
            self.fail(error);
        }
    }

    /// Stops playback before the browser changes its selection
    pub fn close(&mut self) {
        if self.state.active
            && let Some(player) = &mut self.player
        {
            player.stop();
        }

        self.state = VideoState {
            volume: self.state.volume,
            ..Default::default()
        };
        self.texture = None;
    }

    /// Advances playback and prepares a frame without waiting for GPU completion
    pub fn poll(&mut self, ctx: &egui::Context) {
        let Some(player) = &mut self.player else {
            return;
        };

        let previous_fallback = self.state.fallback;
        player.poll(&mut self.state);
        let repaint_after = player.repaint_after();

        if let Some(fallback) = self.state.fallback
            && previous_fallback != Some(fallback)
        {
            self.notice = Some(fallback.message());
        }

        if self.state.ready() {
            match player.render(self.state.size) {
                Ok(texture) => self.texture = texture,
                Err(error) => self.fail(error),
            }
        }

        let playing = !self.state.paused || self.state.seeking;

        if self.state.active && playing && self.state.error.is_none() {
            ctx.request_repaint_after(repaint_after);
        }
    }

    /// A message for the user about a change playback made on its own, shown once
    pub fn take_notice(&mut self) -> Option<String> {
        self.notice.take()
    }

    /// Reports whether this window has an initialized Vulkan playback backend
    pub fn available(&self) -> bool {
        self.player.is_some()
    }

    /// Explains why video is unavailable without hiding native initialization failures
    pub fn unavailable_reason(&self) -> &str {
        self.unavailable
            .as_deref()
            .unwrap_or("Video playback needs the Vulkan renderer")
    }

    /// Decoders this device supports, or `None` while detection runs or without video
    ///
    /// The first call starts detection in the background
    pub fn available_decoders(&self) -> Option<&[VideoDecoder]> {
        self.player.as_ref()?.available_decoders()
    }

    pub fn set_decoder(&mut self, preference: VideoDecoder) -> Result<()> {
        self.command(PlayerCommand::Decoder(preference))?;
        self.state.error = None;

        Ok(())
    }

    /// Changes pause state while preserving the current frame
    pub fn toggle_play(&mut self) -> Result<()> {
        self.command(PlayerCommand::TogglePause)
    }

    /// Seeks to an absolute media time, bounded by the known duration
    pub fn seek(&mut self, seconds: f64) -> Result<()> {
        let position = seek_position(seconds, self.state.duration)?;
        self.command(PlayerCommand::Seek(position))
    }

    /// Sets output volume as a percentage between zero and one hundred
    pub fn set_volume(&mut self, volume: f64) -> Result<()> {
        if !volume.is_finite() {
            bail!("Volume must be finite");
        }

        self.command(PlayerCommand::Volume(volume.clamp(0., 100.)))
    }

    fn command(&mut self, command: PlayerCommand) -> Result<()> {
        if let PlayerCommand::Seek(value) | PlayerCommand::Volume(value) = command
            && !value.is_finite()
        {
            bail!("Playback control must be finite");
        }

        self.player
            .as_mut()
            .context("Video backend unavailable")?
            .command(command)
    }

    fn fail(&mut self, error: anyhow::Error) {
        log::error!("Video playback: {error:#}");
        self.state.loading = false;
        self.state.error = Some(format!("{error:#}"));
        self.texture = None;
    }
}

fn seek_position(seconds: f64, duration: f64) -> Result<f64> {
    if !seconds.is_finite() || !duration.is_finite() || duration <= 0. {
        bail!("The video does not have a seekable duration");
    }

    Ok(seconds.clamp(0., duration))
}

#[cfg(test)]
mod tests {
    use super::seek_position;

    #[test]
    fn seeks_are_bounded_and_reject_invalid_times() {
        assert_eq!(seek_position(-5., 20.).ok(), Some(0.));
        assert_eq!(seek_position(30., 20.).ok(), Some(20.));
        assert_eq!(seek_position(5., 20.).ok(), Some(5.));
        assert!(seek_position(f64::NAN, 20.).is_err());
        assert!(seek_position(2., 0.).is_err());
        assert!(seek_position(2., f64::INFINITY).is_err());
    }
}
