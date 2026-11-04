//! Video sessions with an optional native Vulkan playback backend

#[cfg(all(feature = "video", target_os = "linux"))]
pub(crate) mod gpu;
#[cfg(all(feature = "video", target_os = "linux"))]
mod mpv;
#[cfg(all(feature = "video", target_os = "linux"))]
mod player;

use std::{path::Path, time::Duration};

use anyhow::{Result, bail};

/// UI-facing playback properties observed from libmpv
///
/// Times are seconds, volume is a percentage, and size includes display aspect ratio
pub(crate) struct VideoState {
    pub active: bool,
    pub loading: bool,
    pub paused: bool,
    pub position: f64,
    pub duration: f64,
    pub volume: f64,
    pub size: [u32; 2],
    pub decoder: String,
    pub error: Option<String>,
}

impl VideoState {
    /// Controls become interactive after loading finishes without a playback error
    pub fn ready(&self) -> bool {
        self.active && !self.loading && self.error.is_none()
    }

    /// Describes the decoder without exposing an empty libmpv property to the UI
    pub fn decoder_description(&self) -> String {
        if self.decoder.is_empty() || self.decoder == "no" {
            return "Software decoding".into();
        }

        format!("Hardware decoding · {}", self.decoder)
    }
}

impl Default for VideoState {
    fn default() -> Self {
        Self {
            active: false,
            loading: false,
            paused: false,
            position: 0.,
            duration: 0.,
            volume: 100.,
            size: [0; 2],
            decoder: String::new(),
            error: None,
        }
    }
}

/// Coordinates playback state and the texture displayed by the image canvas
///
/// The optional native player owns all GPU and libmpv resources on the UI thread
#[derive(Default)]
pub(crate) struct VideoSession {
    pub state: VideoState,
    pub texture: Option<egui::TextureId>,
    #[cfg(all(feature = "video", target_os = "linux"))]
    player: Option<player::Player>,
    unavailable: Option<String>,
}

impl VideoSession {
    /// Initializes playback only after the application's compatible device exists
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        #[cfg(all(feature = "video", target_os = "linux"))]
        if let Some(graphics) = &cc.wgpu_render_state {
            return match player::Player::new(graphics, &cc.egui_ctx) {
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

        #[cfg(all(feature = "video", target_os = "linux"))]
        if let Some(player) = &mut self.player {
            if let Err(error) = player.load(path) {
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

    /// Stops playback asynchronously before the browser changes its selection
    pub fn close(&mut self) {
        #[cfg(all(feature = "video", target_os = "linux"))]
        if self.state.active
            && let Some(player) = &self.player
            && let Err(error) = player.command(&["stop"])
        {
            log::warn!("Could not stop video: {error:#}");
        }

        self.state = VideoState {
            volume: self.state.volume,
            ..Default::default()
        };
        self.texture = None;
    }

    /// Applies native events and prepares a frame without waiting for GPU completion
    pub fn poll(&mut self, ctx: &egui::Context) {
        #[cfg(all(feature = "video", target_os = "linux"))]
        if let Some(player) = &mut self.player {
            player.poll(&mut self.state);

            if self.state.ready() {
                match player.render(self.state.size) {
                    Ok(texture) => self.texture = texture,
                    Err(error) => self.fail(error),
                }
            }
        }

        if self.state.active && !self.state.paused && self.state.error.is_none() {
            ctx.request_repaint_after(Duration::from_millis(16));
        }
    }

    /// Reports whether this window has an initialized Vulkan playback backend
    pub fn available(&self) -> bool {
        #[cfg(all(feature = "video", target_os = "linux"))]
        {
            self.player.is_some()
        }

        #[cfg(not(all(feature = "video", target_os = "linux")))]
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

    /// Changes the engine's pause state through its asynchronous command queue
    pub fn toggle_play(&mut self) -> Result<()> {
        self.command(&["cycle", "pause"])
    }

    /// Seeks to an absolute media time, bounded by the known duration
    pub fn seek(&mut self, seconds: f64) -> Result<()> {
        let position = seek_position(seconds, self.state.duration)?;
        self.command(&["seek", &position.to_string(), "absolute+exact"])
    }

    /// Sets output volume as a percentage between zero and one hundred
    pub fn set_volume(&mut self, volume: f64) -> Result<()> {
        if !volume.is_finite() {
            bail!("Volume must be finite");
        }

        self.command(&["set", "volume", &volume.clamp(0., 100.).to_string()])
    }

    fn command(&self, args: &[&str]) -> Result<()> {
        #[cfg(all(feature = "video", target_os = "linux"))]
        if let Some(player) = &self.player {
            return player.command(args);
        }

        let _ = args;
        bail!("Video backend unavailable")
    }

    #[cfg(all(feature = "video", target_os = "linux"))]
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
