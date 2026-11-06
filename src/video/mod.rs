//! Video sessions with an optional native Vulkan playback backend

#[cfg(all(feature = "video", any(target_os = "linux", target_os = "windows")))]
mod ffmpeg;
#[cfg(all(feature = "video", any(target_os = "linux", target_os = "windows")))]
pub(crate) mod gpu;
#[cfg(all(feature = "video", any(target_os = "linux", target_os = "windows")))]
mod media;
#[cfg(all(feature = "video", any(target_os = "linux", target_os = "windows")))]
mod native;
#[cfg(all(feature = "video", any(target_os = "linux", target_os = "windows")))]
mod player;
#[cfg(all(feature = "video", any(target_os = "linux", target_os = "windows")))]
mod thumbnail;

use std::{path::Path, time::Duration};

use crate::settings::VideoDecoder;
use anyhow::{Result, bail};

/// Extracts filmstrip pixels without requiring an initialized playback device
pub(crate) fn thumbnail(
    path: &Path,
    generation: &std::sync::atomic::AtomicU64,
    expected: u64,
) -> Result<egui::ColorImage> {
    #[cfg(all(feature = "video", any(target_os = "linux", target_os = "windows")))]
    {
        thumbnail::extract(path, generation, expected)
    }

    #[cfg(not(all(
        feature = "video",
        any(target_os = "linux", target_os = "windows")
    )))]
    {
        let _ = (path, generation, expected);
        bail!("This build does not support video thumbnails")
    }
}

/// Source audio properties before playback resampling
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct AudioInfo {
    pub codec: String,
    pub channels: u32,
    pub sample_rate: u32,
    pub bit_rate: u64,
}

/// Playback properties reported by the FFmpeg player
///
/// Times are seconds, volume is a percentage, and size includes display aspect ratio
pub(crate) struct VideoState {
    pub active: bool,
    pub loading: bool,
    pub seeking: bool,
    pub paused: bool,
    pub position: f64,
    pub duration: f64,
    pub volume: f64,
    pub size: [u32; 2],
    pub decoder: String,
    pub codec: String,
    pub audio: Option<AudioInfo>,
    pub error: Option<String>,
}

impl VideoState {
    pub fn codec_description(&self) -> &str {
        match self.codec.as_str() {
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

    /// Describes decoder selection before the first decoded frame arrives
    pub fn decoder_description(&self) -> String {
        if self.decoder.is_empty() || self.decoder == "no" {
            return "Selecting video decoder".into();
        }

        if self.decoder == "software" {
            return "Software decoding · CPU".into();
        }

        let name = match self.decoder.as_str() {
            "nvdec" => "NVDEC",
            "vulkan" => "Vulkan",
            "vaapi" => "VA-API",
            other => other,
        };
        format!("Hardware decoding · {name}")
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
            decoder: String::new(),
            codec: String::new(),
            audio: None,
            error: None,
        }
    }
}

/// Coordinates playback state and the texture displayed by the image canvas
///
/// The optional Rust player owns decoding and GPU resources for this window
#[derive(Default)]
pub(crate) struct VideoSession {
    pub state: VideoState,
    pub texture: Option<egui::TextureId>,
    #[cfg(all(feature = "video", any(target_os = "linux", target_os = "windows")))]
    player: Option<player::Player>,
    unavailable: Option<String>,
    decoders: Vec<VideoDecoder>,
}

impl VideoSession {
    /// Initializes playback only after the application's compatible device exists
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        #[cfg(all(feature = "video", any(target_os = "linux", target_os = "windows")))]
        if let Some(graphics) = &cc.wgpu_render_state {
            return match player::Player::new(graphics, &cc.egui_ctx) {
                Ok(player) => Self {
                    decoders: player.available_decoders(),
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
            };
        }

        let _ = cc;
        Self {
            unavailable: Some(
                "This build does not support Vulkan video playback".into(),
            ),
            ..Default::default()
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

        #[cfg(all(feature = "video", any(target_os = "linux", target_os = "windows")))]
        if let Some(player) = &mut self.player {
            if let Err(error) = player.load(path) {
                self.fail(error);
            } else if let Err(error) =
                player.command(Command::Volume(self.state.volume))
            {
                self.fail(error);
            }
            return;
        }

        let _ = path;
        self.state.loading = false;
        self.state.error = Some(
            self.unavailable
                .clone()
                .unwrap_or_else(|| "Video backend unavailable".into()),
        );
    }

    /// Stops playback before the browser changes its selection
    pub fn close(&mut self) {
        #[cfg(all(feature = "video", any(target_os = "linux", target_os = "windows")))]
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
        #[cfg(all(feature = "video", any(target_os = "linux", target_os = "windows")))]
        if let Some(player) = &mut self.player {
            player.poll(&mut self.state);

            if self.state.ready() {
                match player.render(self.state.size) {
                    Ok(texture) => self.texture = texture,
                    Err(error) => self.fail(error),
                }
            }
        }

        if self.state.active
            && (!self.state.paused || self.state.seeking)
            && self.state.error.is_none()
        {
            #[cfg(all(
                feature = "video",
                any(target_os = "linux", target_os = "windows")
            ))]
            let delay = self
                .player
                .as_ref()
                .map_or(Duration::from_millis(16), player::Player::repaint_after);
            #[cfg(not(all(
                feature = "video",
                any(target_os = "linux", target_os = "windows")
            )))]
            let delay = Duration::from_millis(16);
            ctx.request_repaint_after(delay);
        }
    }

    /// Reports whether this window has an initialized Vulkan playback backend
    pub fn available(&self) -> bool {
        #[cfg(all(feature = "video", any(target_os = "linux", target_os = "windows")))]
        {
            self.player.is_some()
        }

        #[cfg(not(all(
            feature = "video",
            any(target_os = "linux", target_os = "windows")
        )))]
        {
            false
        }
    }

    /// Explains why video is unavailable without hiding native initialization failures
    pub fn unavailable_reason(&self) -> &str {
        self.unavailable
            .as_deref()
            .unwrap_or("Video playback needs the optional Linux Vulkan backend")
    }

    pub fn available_decoders(&self) -> &[VideoDecoder] {
        &self.decoders
    }

    pub fn set_decoder(&mut self, preference: VideoDecoder) -> Result<()> {
        #[cfg(all(feature = "video", any(target_os = "linux", target_os = "windows")))]
        {
            self.command(Command::Decoder(preference))?;
            self.state.error = None;
            Ok(())
        }

        #[cfg(not(all(
            feature = "video",
            any(target_os = "linux", target_os = "windows")
        )))]
        {
            let _ = preference;
            bail!("Video backend unavailable")
        }
    }

    /// Changes pause state while preserving the current frame
    pub fn toggle_play(&mut self) -> Result<()> {
        self.command(Command::TogglePause)
    }

    /// Seeks to an absolute media time, bounded by the known duration
    pub fn seek(&mut self, seconds: f64) -> Result<()> {
        let position = seek_position(seconds, self.state.duration)?;
        self.command(Command::Seek(position))
    }

    /// Sets output volume as a percentage between zero and one hundred
    pub fn set_volume(&mut self, volume: f64) -> Result<()> {
        if !volume.is_finite() {
            bail!("Volume must be finite");
        }

        self.command(Command::Volume(volume.clamp(0., 100.)))
    }

    fn command(&mut self, command: Command) -> Result<()> {
        if let Command::Seek(value) | Command::Volume(value) = command
            && !value.is_finite()
        {
            bail!("Playback control must be finite");
        }

        #[cfg(all(feature = "video", any(target_os = "linux", target_os = "windows")))]
        if let Some(player) = &mut self.player {
            return player.command(command);
        }

        let _ = command;
        bail!("Video backend unavailable")
    }

    #[cfg(all(feature = "video", any(target_os = "linux", target_os = "windows")))]
    fn fail(&mut self, error: anyhow::Error) {
        log::error!("Video playback: {error:#}");
        self.state.loading = false;
        self.state.error = Some(format!("{error:#}"));
        self.texture = None;
    }
}

#[derive(Clone, Copy)]
pub(super) enum Command {
    TogglePause,
    Seek(f64),
    Volume(f64),
    #[cfg(all(feature = "video", any(target_os = "linux", target_os = "windows")))]
    Decoder(VideoDecoder),
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
