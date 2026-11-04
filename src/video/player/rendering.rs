//! Shares a three-texture Vulkan ring between mpv and egui without CPU readback

use std::sync::atomic::Ordering;

use anyhow::{Context, Result, ensure};
use ash::vk;
use eframe::wgpu::{self, hal};

use super::{Player, SLOT_COUNT, Target, mpv};
use mpv::{RenderParam, VulkanTarget};

impl Player {
    /// Reuses the last frame unless mpv requests a redraw or the target size changes
    pub fn render(&mut self, size: [u32; 2]) -> Result<Option<egui::TextureId>> {
        if size.contains(&0) {
            return Ok(None);
        }

        let limit = self.graphics.device.limits().max_texture_dimension_2d;
        ensure!(
            size[0] <= limit && size[1] <= limit,
            "Video dimensions exceed the GPU texture limit of {limit} pixels"
        );

        let resized = self.size != size;

        if resized {
            self.resize(size)?;
        }

        let dirty = self.wakeup.dirty.swap(false, Ordering::AcqRel);
        let updated = if dirty {
            // SAFETY: Render functions run serially on the owning UI thread
            unsafe {
                (self.api.render_context_update)(self.render) & mpv::RENDER_UPDATE_FRAME
                    != 0
            }
        } else {
            false
        };

        if !resized && !updated && !self.needs_redraw {
            return Ok(self.displayed);
        }

        self.draw()?;
        self.needs_redraw = false;
        self.swap_pending = true;

        Ok(self.displayed)
    }

    fn draw(&mut self) -> Result<()> {
        let target = &self.targets[self.slot];

        // End any earlier sampling before the engine writes this ring slot
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

        // SAFETY: Semaphore ownership is retained until all submissions complete
        let queue = unsafe { self.graphics.queue.as_hal::<hal::api::Vulkan>() }
            .context("Vulkan queue unavailable")?;
        queue.add_signal_semaphore(self.semaphore, Some(ready));
        drop(queue);
        self.graphics.queue.submit([encoder.finish()]);

        // The native renderer borrows the image until it signals the next even value
        let texture = unsafe { target.texture.as_hal::<hal::api::Vulkan>() }
            .context("Vulkan texture unavailable")?;
        let mut frame = VulkanTarget {
            image: unsafe { texture.raw_handle() },
            width: self.size[0],
            height: self.size[1],
            slot: self.slot as u32,
            semaphore: self.semaphore,
            wait_value: ready,
            signal_value: self.timeline,
        };
        let mut block = 0_i32;
        let mut params = [
            RenderParam::new(mpv::RENDER_PARAM_VULKAN_TARGET, &mut frame),
            RenderParam::new(mpv::RENDER_PARAM_BLOCK_FOR_TARGET_TIME, &mut block),
            RenderParam::end(),
        ];

        // SAFETY: The target is exclusively writable until mpv signals the completion value
        let result = unsafe {
            (self.api.render_context_render)(self.render, params.as_mut_ptr())
        };
        self.api.check(result)?;

        // The next egui submission waits on the GPU, so the UI thread stays unblocked
        let queue = unsafe { self.graphics.queue.as_hal::<hal::api::Vulkan>() }
            .context("Vulkan queue unavailable")?;
        queue.add_wait_semaphore(
            self.semaphore,
            Some(self.timeline),
            vk::PipelineStageFlags::ALL_COMMANDS,
        );

        self.displayed = Some(target.id);
        self.slot = (self.slot + 1) % SLOT_COUNT;

        Ok(())
    }

    fn resize(&mut self, size: [u32; 2]) -> Result<()> {
        self.clear_targets()?;

        let mut clear = self.graphics.device.create_command_encoder(
            &wgpu::CommandEncoderDescriptor {
                label: Some("initialize-video-targets"),
            },
        );

        for _ in 0..SLOT_COUNT {
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

        Ok(())
    }

    /// Retires egui registrations only after both renderers finish using the images
    pub(super) fn clear_targets(&mut self) -> Result<()> {
        if self.targets.is_empty() {
            return Ok(());
        }

        self.graphics
            .device
            .poll(wgpu::PollType::wait_indefinitely())?;

        // SAFETY: Waiting here is restricted to resize and teardown, after ordered submissions
        unsafe { self.native.device_wait_idle() }?;

        let mut renderer = self.graphics.renderer.write();

        for target in self.targets.drain(..) {
            renderer.free_texture(&target.id);
        }

        Ok(())
    }
}
