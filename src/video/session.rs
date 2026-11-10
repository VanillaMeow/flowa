//! The application's handle on video playback: state, controls, and the shown texture

use std::path::Path;

use anyhow::{Context, Result, bail};

use super::{PlayerCommand, VideoState, playback};
use crate::settings::VideoDecoder;

/// Coordinates playback state and the texture displayed by the image canvas
#[derive(Default)]
pub struct VideoSession {
    pub state: VideoState,
    pub texture: Option<egui::TextureId>,
    player: Option<playback::Player>,
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
            .and_then(|graphics| playback::Player::new(graphics, &cc.egui_ctx));

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
