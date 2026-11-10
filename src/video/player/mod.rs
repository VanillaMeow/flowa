//! Background FFmpeg playback and shared Vulkan targets

use std::{path::Path, sync::Arc};

use anyhow::{Context, Result};
use ash::vk;
use eframe::{
    egui_wgpu::RenderState,
    wgpu::{self, hal},
};

use super::{
    PlayerCommand,
    gpu::{self, NativeGpu},
};

mod audio_output;
mod clock;
mod decoded_frame;
mod decoder;
mod decoder_detection;
mod decoder_selection;
mod engine;
mod hardware_decoder;
mod rendering;
mod service;
mod shared_state;
mod software_decoder;
mod worker;

use engine::{Engine, Snapshot};
pub(super) use service::Player;

const SLOT_COUNT: usize = gpu::TARGET_SLOTS;

#[derive(Clone)]
struct Target {
    texture: wgpu::Texture,
    id: egui::TextureId,
}

/// Couples the decode engine with its ring of presentation textures
///
/// Native resources are retired before the wgpu targets they reference
struct Pipeline {
    gpu: Arc<NativeGpu>,
    engine: Engine,
    graphics: RenderState,
    native: ash::Device,
    semaphore: vk::Semaphore,
    timeline: u64,
    targets: Vec<Target>,
    size: [u32; 2],
    slot: usize,
    displayed: Option<egui::TextureId>,
    needs_redraw: bool,
    status: Snapshot,
    retired: Vec<Target>,
}

impl Pipeline {
    pub fn new(graphics: &RenderState, ctx: &egui::Context) -> Result<Self> {
        let gpu = gpu::for_device(&graphics.device)?;
        let guard = unsafe { graphics.device.as_hal::<hal::api::Vulkan>() }
            .context("Vulkan device unavailable")?;
        let native = guard.raw_device().clone();
        let mut timeline = vk::SemaphoreTypeCreateInfo::default()
            .semaphore_type(vk::SemaphoreType::TIMELINE);
        let semaphore = unsafe {
            native.create_semaphore(
                &vk::SemaphoreCreateInfo::default().push_next(&mut timeline),
                None,
            )
        }?;
        drop(guard);

        Ok(Self {
            engine: Engine::new(Arc::clone(&gpu), ctx.clone()),
            gpu,
            graphics: graphics.clone(),
            native,
            semaphore,
            timeline: 0,
            targets: Vec::new(),
            size: [0; 2],
            slot: 0,
            displayed: None,
            needs_redraw: false,
            status: Snapshot::default(),
            retired: Vec::new(),
        })
    }

    pub fn load(&mut self, path: &Path) -> Result<()> {
        self.engine.open(path)?;
        self.displayed = None;
        self.status = Snapshot::default();
        self.needs_redraw = false;
        Ok(())
    }

    pub fn stop(&mut self) {
        self.engine.stop();
        self.displayed = None;
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

    fn poll(&mut self) {
        let status = self.engine.poll();
        self.needs_redraw |= status.frame_serial != self.status.frame_serial;
        self.status = status;
    }
}

impl Drop for Pipeline {
    fn drop(&mut self) {
        self.engine.stop();

        if let Some(queue) = unsafe { self.graphics.queue.as_hal::<hal::api::Vulkan>() }
        {
            queue.remove_wait_semaphore(self.semaphore);
            queue.remove_signal_semaphore(self.semaphore);
        }

        // The player is gone, so retire GPU wrappers directly before destroying its textures
        self.gpu.retire_targets();
        if let Err(error) = self
            .graphics
            .device
            .poll(wgpu::PollType::wait_indefinitely())
        {
            log::warn!("Video submission cleanup: {error:#}");
        }
        self.targets.clear();
        unsafe { self.native.destroy_semaphore(self.semaphore, None) };
    }
}
