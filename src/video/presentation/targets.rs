//! A ring of wgpu textures shared with libplacebo, handed back and forth through one
//! timeline semaphore

use anyhow::{Context, Result, ensure};
use ash::vk;
use eframe::{
    egui_wgpu::RenderState,
    wgpu::{self, hal},
};

/// How many presentation textures the player cycles through
pub(in crate::video) const TARGET_SLOTS: usize = 3;

/// A presentation texture and its egui registration
#[derive(Clone)]
pub(in crate::video) struct Target {
    pub texture: wgpu::Texture,
    pub id: egui::TextureId,
}

/// A wgpu texture to draw into, lent between two values of a timeline semaphore
pub(super) struct RenderTarget {
    pub image: vk::Image,
    pub size: [u32; 2],
    pub slot: usize,
    pub semaphore: vk::Semaphore,
    /// Signaled by wgpu once it has stopped using the image
    pub wait_value: u64,
    /// Signaled by the renderer once the frame is drawn
    pub signal_value: u64,
}

pub(super) struct TargetRing {
    graphics: RenderState,
    native: ash::Device,
    semaphore: vk::Semaphore,
    timeline: u64,
    targets: Vec<Target>,
    size: [u32; 2],
    slot: usize,
    displayed: Option<egui::TextureId>,
    /// Targets the UI frees once both renderers have finished with them
    retired: Vec<Target>,
}

impl TargetRing {
    pub fn new(graphics: &RenderState) -> Result<Self> {
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
            graphics: graphics.clone(),
            native,
            semaphore,
            timeline: 0,
            targets: Vec::new(),
            size: [0; 2],
            slot: 0,
            displayed: None,
            retired: Vec::new(),
        })
    }

    pub fn size(&self) -> [u32; 2] {
        self.size
    }

    /// The texture showing the latest drawn frame
    pub fn displayed(&self) -> Option<egui::TextureId> {
        self.displayed
    }

    pub fn displayed_target(&self) -> Option<Target> {
        self.targets
            .iter()
            .find(|target| Some(target.id) == self.displayed)
            .cloned()
    }

    /// Stops showing the last frame, as when a new video starts
    pub fn forget_displayed(&mut self) {
        self.displayed = None;
    }

    pub fn take_retired(&mut self) -> Vec<Target> {
        std::mem::take(&mut self.retired)
    }

    /// Checks that the GPU can allocate textures of this size
    pub fn check_size(&self, size: [u32; 2]) -> Result<()> {
        let limit = self.graphics.device.limits().max_texture_dimension_2d;
        ensure!(
            size[0] <= limit && size[1] <= limit,
            "Video dimensions exceed the GPU texture limit of {limit} pixels"
        );

        Ok(())
    }

    /// Replaces the textures with new ones of another size
    ///
    /// The renderer must have released its views of the current textures
    pub fn resize(&mut self, size: [u32; 2]) {
        self.retire();

        let mut clear = self.graphics.device.create_command_encoder(
            &wgpu::CommandEncoderDescriptor {
                label: Some("initialize-video-targets"),
            },
        );

        for _ in 0..TARGET_SLOTS {
            let texture =
                self.graphics
                    .device
                    .create_texture(&wgpu::TextureDescriptor {
                        label: Some("video-target"),
                        size: wgpu::Extent3d {
                            width: size[0],
                            height: size[1],
                            depth_or_array_layers: 1,
                        },
                        mip_level_count: 1,
                        sample_count: 1,
                        dimension: wgpu::TextureDimension::D2,
                        format: wgpu::TextureFormat::Rgba8Unorm,
                        usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                            | wgpu::TextureUsages::TEXTURE_BINDING
                            | wgpu::TextureUsages::COPY_DST
                            | wgpu::TextureUsages::COPY_SRC,
                        view_formats: &[],
                    });
            let view = texture.create_view(&Default::default());
            {
                // Initialize wgpu's tracking before the native renderer writes its first frame
                let _pass = clear.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("clear-video-target"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    ..Default::default()
                });
            }
            let id = self.graphics.renderer.write().register_native_texture(
                &self.graphics.device,
                &view,
                wgpu::FilterMode::Linear,
            );
            self.targets.push(Target { texture, id });
        }

        self.graphics.queue.submit([clear.finish()]);

        self.size = size;
        self.slot = 0;
        self.displayed = None;
    }

    /// Lends the next texture to the renderer once wgpu has stopped sampling it
    pub fn lend(&mut self) -> Result<RenderTarget> {
        let target = &self.targets[self.slot];

        // End any earlier sampling before the renderer writes this ring slot
        let mut encoder = self.graphics.device.create_command_encoder(
            &wgpu::CommandEncoderDescriptor {
                label: Some("video-handoff"),
            },
        );
        encoder.transition_resources(
            std::iter::empty(),
            std::iter::once(wgpu::TextureTransition {
                texture: &target.texture,
                selector: None,
                state: wgpu::TextureUses::COLOR_TARGET,
            }),
        );

        self.timeline += 2;
        let ready = self.timeline - 1;
        self.graphics.queue.submit([encoder.finish()]);

        // Stage the signal after submitting the transition, since a concurrent UI
        // submission can consume it before the following empty submission
        // SAFETY: Semaphore ownership is retained until all submissions complete
        let queue = unsafe { self.graphics.queue.as_hal::<hal::api::Vulkan>() }
            .context("Vulkan queue unavailable")?;
        queue.add_signal_semaphore(self.semaphore, Some(ready));
        drop(queue);
        self.graphics.queue.submit([]);

        // The renderer borrows the image until it signals the next even value
        let texture = unsafe { target.texture.as_hal::<hal::api::Vulkan>() }
            .context("Vulkan texture unavailable")?;

        Ok(RenderTarget {
            image: unsafe { texture.raw_handle() },
            size: self.size,
            slot: self.slot,
            semaphore: self.semaphore,
            wait_value: ready,
            signal_value: self.timeline,
        })
    }

    /// Takes back a texture the renderer drew into and shows it
    pub fn reclaim(&mut self, lent: &RenderTarget) -> Result<()> {
        // Add the wait only after native rendering has submitted its GPU work
        // UI submissions can continue while the worker prepares shaders and frames
        let queue = unsafe { self.graphics.queue.as_hal::<hal::api::Vulkan>() }
            .context("Vulkan queue unavailable")?;
        queue.add_wait_semaphore(
            self.semaphore,
            Some(lent.signal_value),
            vk::PipelineStageFlags::ALL_COMMANDS,
        );

        self.displayed = Some(self.targets[lent.slot].id);
        self.slot = (lent.slot + 1) % TARGET_SLOTS;

        Ok(())
    }

    /// Moves every texture to the retired list
    ///
    /// The renderer must have released its views of them. Wgpu retains submitted
    /// resources until completion, and the UI frees the egui registrations only after
    /// receiving the replacement frame and these retired targets
    pub fn retire(&mut self) {
        self.retired.append(&mut self.targets);
    }

    /// Takes every texture, for the UI to free when playback ends
    pub fn take_all(&mut self) -> Vec<Target> {
        self.retire();
        self.take_retired()
    }

    /// Stops wgpu from waiting on or signaling the semaphore in later submissions
    pub fn detach(&self) {
        if let Some(queue) = unsafe { self.graphics.queue.as_hal::<hal::api::Vulkan>() }
        {
            queue.remove_wait_semaphore(self.semaphore);
            queue.remove_signal_semaphore(self.semaphore);
        }
    }
}

impl Drop for TargetRing {
    fn drop(&mut self) {
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
