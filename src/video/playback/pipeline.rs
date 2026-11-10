//! Couples the playback engine with the presenter that draws its frames

use std::{path::Path, sync::Arc};

use anyhow::Result;
use eframe::egui_wgpu::RenderState;

use super::engine::{Engine, Snapshot};
use crate::video::{
    PlayerCommand, gpu,
    presentation::{Presenter, RenderError, Target},
};

/// Decodes and schedules frames with the engine, and draws them with the presenter
///
/// The presenter releases its frames before the engine drops the decoder that made them
pub(super) struct Pipeline {
    engine: Engine,
    presenter: Presenter,
    pub status: Snapshot,
    needs_redraw: bool,
}

impl Pipeline {
    pub fn new(graphics: &RenderState, ctx: &egui::Context) -> Result<Self> {
        let gpu = gpu::for_device(&graphics.device)?;

        Ok(Self {
            presenter: Presenter::new(graphics, Arc::clone(&gpu))?,
            engine: Engine::new(gpu, ctx.clone()),
            status: Snapshot::default(),
            needs_redraw: false,
        })
    }

    pub fn load(&mut self, path: &Path) -> Result<()> {
        self.presenter.reset_frames();
        self.engine.open(path)?;
        self.forget_status();
        Ok(())
    }

    pub fn stop(&mut self) {
        self.presenter.reset_frames();
        self.engine.stop();
        self.forget_status();
    }

    fn forget_status(&mut self) {
        self.presenter.forget_displayed();
        self.status = Snapshot::default();
        self.needs_redraw = false;
    }

    pub fn command(&mut self, command: PlayerCommand) -> Result<()> {
        match command {
            PlayerCommand::TogglePause => self.engine.toggle_pause(),
            PlayerCommand::Seek(seconds) => self.engine.seek(seconds),
            PlayerCommand::Volume(volume) => self.engine.set_volume(volume),
            PlayerCommand::Decoder(preference) => {
                self.engine.set_decoder(preference)?;
                self.needs_redraw = true;
                Ok(())
            }
        }
    }

    pub fn poll(&mut self) {
        let status = self.engine.poll();
        self.needs_redraw |= status.frame_serial != self.status.frame_serial;
        self.status = status;
    }

    /// Reuses the last frame unless the engine has a new one or the size changes
    pub fn render(&mut self, size: [u32; 2]) -> Result<Option<egui::TextureId>> {
        if size.contains(&0) || self.status.frame_serial == 0 {
            return Ok(None);
        }

        let rendered = {
            let current = self.engine.current_frame();
            let frame = current
                .as_ref()
                .map(|(frame, stream)| (*frame, stream.as_ref()));

            self.presenter.render(size, frame, self.needs_redraw)
        };

        let error = match rendered {
            Ok(texture) => {
                self.needs_redraw = false;
                return Ok(texture);
            }
            Err(error) => error,
        };

        let unsupported = matches!(
            error.downcast_ref::<RenderError>(),
            Some(RenderError::UnsupportedFrame(_))
        );
        self.engine.render_failed(error, unsupported)?;
        self.needs_redraw = false;

        Ok(self.presenter.displayed())
    }

    pub fn displayed(&self) -> Option<egui::TextureId> {
        self.presenter.displayed()
    }

    pub fn displayed_target(&self) -> Option<Target> {
        self.presenter.displayed_target()
    }

    /// Textures the UI may free now that a replacement is showing
    pub fn take_retired(&mut self) -> Vec<Target> {
        self.presenter.take_retired()
    }

    /// Stops playback and returns every texture for the UI to free
    pub fn finish(&mut self) -> Vec<Target> {
        self.stop();
        self.presenter.release_targets()
    }
}

impl Drop for Pipeline {
    fn drop(&mut self) {
        self.presenter.reset_frames();
        self.engine.stop();
    }
}
