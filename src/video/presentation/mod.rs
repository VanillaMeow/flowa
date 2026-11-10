//! Presentation: draws decoded frames into wgpu textures with libplacebo
//!
//! `Presenter` owns its renderer and the texture ring, and keeps the GPU alive until
//! both are destroyed

use std::sync::Arc;

use anyhow::Result;
use eframe::egui_wgpu::RenderState;

use super::{decoding::DecodedFrame, ffmpeg::Stream, gpu::NativeGpu};

mod frame_import;
mod frame_transfer;
mod renderer;
mod targets;

use renderer::FrameRenderer;
pub(super) use renderer::RenderError;
pub(super) use targets::Target;
use targets::TargetRing;

pub(super) struct Presenter {
    renderer: FrameRenderer,
    ring: TargetRing,
    gpu: Arc<NativeGpu>,
}

impl Presenter {
    pub fn new(graphics: &RenderState, gpu: Arc<NativeGpu>) -> Result<Self> {
        Ok(Self {
            renderer: FrameRenderer::new(gpu.device())?,
            ring: TargetRing::new(graphics)?,
            gpu,
        })
    }

    /// Draws the frame when it or the size changed, and returns the texture to show
    ///
    /// Errors from importing the frame are `RenderError`s
    pub fn render(
        &mut self,
        size: [u32; 2],
        frame: Option<(&DecodedFrame, &Stream)>,
        changed: bool,
    ) -> Result<Option<egui::TextureId>> {
        self.ring.check_size(size)?;

        let resized = self.ring.size() != size;

        if resized {
            // Release native views of the textures before wgpu destroys them
            self.renderer.retire_targets(self.gpu.device());
            self.ring.resize(size);
        }

        if !resized && !changed {
            return Ok(self.ring.displayed());
        }

        // Without a frame there is nothing to draw, and the ring stays unchanged
        if let Some((frame, stream)) = frame {
            let target = self.ring.lend()?;
            self.renderer
                .render(self.gpu.device(), frame, stream, &target)?;
            self.ring.reclaim(&target)?;
        }

        Ok(self.ring.displayed())
    }

    pub fn displayed(&self) -> Option<egui::TextureId> {
        self.ring.displayed()
    }

    pub fn displayed_target(&self) -> Option<Target> {
        self.ring.displayed_target()
    }

    /// Stops showing the last frame, as when a new video starts
    pub fn forget_displayed(&mut self) {
        self.ring.forget_displayed();
    }

    /// Textures the UI may free now that a replacement is showing
    pub fn take_retired(&mut self) -> Vec<Target> {
        self.ring.take_retired()
    }

    /// Releases the frames and per-video resources the renderer holds, before the
    /// decoder that produced them is dropped
    pub fn reset_frames(&mut self) {
        self.renderer.reset_frames(self.gpu.device());
    }

    /// Releases every texture, for the UI to free when playback ends
    pub fn release_targets(&mut self) -> Vec<Target> {
        self.renderer.retire_targets(self.gpu.device());
        self.ring.take_all()
    }
}

impl Drop for Presenter {
    fn drop(&mut self) {
        self.ring.detach();

        // The renderer's GPU objects go before the ring destroys its semaphore, and
        // before this owner releases the device
        self.renderer.destroy(self.gpu.device());
    }
}
