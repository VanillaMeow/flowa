//! Libplacebo rendering of hardware and software frames into wgpu's textures

use std::{ffi::c_int, fmt, ptr};

use anyhow::{Result, ensure};
use ash::vk;

use super::{
    frame_transfer::{FrameTransfer, TransferredFrame},
    shared_device::SharedDevice,
};
use crate::video::{
    ffmpeg as ffi,
    media::{AvFrame, Stream},
    native_bindings as native,
};

/// How many presentation textures the player cycles through
pub(in crate::video) const TARGET_SLOTS: usize = 3;

/// Submissions the GPU may fall behind by before rendering stops adding more
const MAX_FLIGHTS: usize = 32;

/// A wgpu texture to draw into, lent between two values of a timeline semaphore
pub(in crate::video) struct RenderTarget {
    pub image: vk::Image,
    pub size: [u32; 2],
    pub slot: usize,
    pub semaphore: vk::Semaphore,
    /// Signaled by wgpu once it has stopped using the image
    pub wait_value: u64,
    /// Signaled by the renderer once the frame is drawn
    pub signal_value: u64,
}

/// Why a frame was not drawn
#[derive(Debug)]
pub(in crate::video) enum RenderError {
    /// The renderer cannot import this decoder's frames, so another decoder may work
    UnsupportedFrame(&'static str),
    Failed(&'static str),
}

impl fmt::Display for RenderError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (Self::UnsupportedFrame(message) | Self::Failed(message)) = self;

        formatter.write_str(message)
    }
}

impl std::error::Error for RenderError {}

/// Libplacebo's view of one of the player's textures
#[derive(Clone, Copy)]
struct Target {
    texture: native::pl_tex,
    image: vk::Image,
}

/// Frames the GPU may still be reading until a timeline value is reached
struct Flight {
    semaphore: vk::Semaphore,
    value: u64,
    _source: AvFrame,
    transferred: Option<TransferredFrame>,
}

impl Flight {
    fn landed(&self, api: &ash::Device) -> bool {
        // SAFETY: The player destroys its semaphore only after retiring its targets
        match unsafe { api.get_semaphore_counter_value(self.semaphore) } {
            Ok(value) => value >= self.value,
            // A lost device completes nothing more, so nothing is left to wait for
            Err(error) => error == vk::Result::ERROR_DEVICE_LOST,
        }
    }
}

/// The mutable rendering state of the shared device, used by one playback owner
pub(super) struct FrameRenderer {
    renderer: native::pl_renderer,
    targets: [Target; TARGET_SLOTS],
    /// Textures libplacebo reuses for the planes of software frames
    software_planes: [native::pl_tex; 4],
    transfer: FrameTransfer,
    flights: Vec<Flight>,
}

impl FrameRenderer {
    pub fn new(device: &SharedDevice) -> Result<Self> {
        // SAFETY: The device outlives the renderer, which its owner destroys first
        let renderer =
            unsafe { native::pl_renderer_create(device.log(), device.gpu()) };
        ensure!(!renderer.is_null(), "Cannot create the libplacebo renderer");

        Ok(Self {
            renderer,
            targets: [Target {
                texture: ptr::null(),
                image: vk::Image::null(),
            }; TARGET_SLOTS],
            software_planes: [ptr::null(); 4],
            transfer: FrameTransfer::default(),
            flights: Vec::new(),
        })
    }

    /// Draws a decoded frame into the target and returns the image at its signal value
    pub fn render(
        &mut self,
        device: &SharedDevice,
        frame: &AvFrame,
        stream: &Stream,
        target: &RenderTarget,
    ) -> Result<(), RenderError> {
        self.collect(device, false);
        if self.flights.len() >= MAX_FLIGHTS {
            return Err(RenderError::Failed(
                "The GPU is not completing video submissions",
            ));
        }

        let texture = self.wrap_target(device, target)?;
        let transferred = self.transfer(device, frame)?;
        let mut mapped = self.import(device, frame, stream, target, transferred)?;

        if !draw(device, self.renderer, &mut mapped, texture, target) {
            return Err(RenderError::Failed(
                "The Vulkan video render submission failed",
            ));
        }

        Ok(())
    }

    /// Maps a frame's planes for libplacebo and keeps the frame until the GPU is done
    fn import(
        &mut self,
        device: &SharedDevice,
        frame: &AvFrame,
        stream: &Stream,
        target: &RenderTarget,
        transferred: Option<TransferredFrame>,
    ) -> Result<native::pl_frame, RenderError> {
        let source = transferred
            .as_ref()
            .map_or(frame, |transferred| &transferred.frame);
        let mut mapped = native::pl_frame::default();

        // SAFETY: The frame, stream, and plane textures outlive the mapping
        let imported = unsafe {
            native::flowa_frame_map(
                device.gpu(),
                &mut mapped,
                source.raw(),
                stream.raw(),
                self.software_planes.as_mut_ptr(),
            )
        };

        if !imported {
            self.abandon(device, None, transferred);

            return Err(RenderError::UnsupportedFrame(
                "Cannot map the video frame into the Vulkan renderer",
            ));
        }

        let Ok(retained) = frame.retain() else {
            self.abandon(device, Some(&mut mapped), transferred);

            return Err(RenderError::Failed(
                "Cannot retain an in-flight video frame",
            ));
        };
        self.flights.push(Flight {
            semaphore: target.semaphore,
            value: target.signal_value,
            _source: retained,
            transferred,
        });

        Ok(mapped)
    }

    /// Releases libplacebo's views of the player's textures before wgpu destroys them
    pub fn retire_targets(&mut self, device: &SharedDevice) {
        // SAFETY: The device is alive, and finishing completes every use of the textures
        unsafe { native::pl_gpu_finish(device.gpu()) };
        self.collect(device, true);

        for target in &mut self.targets {
            // SAFETY: Destroying a wrapper leaves the image to its owner
            unsafe { native::pl_tex_destroy(device.gpu(), &mut target.texture) };
            target.image = vk::Image::null();
        }
    }

    /// Releases everything tied to the video being closed, before its decoder frames
    pub fn reset_frames(&mut self, device: &SharedDevice) {
        // SAFETY: The device is alive, and finishing completes every use of the frames
        unsafe { native::pl_gpu_finish(device.gpu()) };
        self.collect(device, true);
        self.transfer.reset();

        for plane in &mut self.software_planes {
            // SAFETY: Nothing is rendering from the plane textures any more
            unsafe { native::pl_tex_destroy(device.gpu(), plane) };
        }
    }

    /// Releases every GPU object, which must happen before the device is destroyed
    pub fn destroy(&mut self, device: &SharedDevice) {
        self.retire_targets(device);
        self.reset_frames(device);
        self.transfer.destroy(device);

        // SAFETY: Nothing uses the renderer after its owner is dropped
        unsafe { native::pl_renderer_destroy(&mut self.renderer) };
    }

    /// Frees the frames of submissions the GPU has completed, or of all of them once
    /// the GPU has finished
    fn collect(&mut self, device: &SharedDevice, finished: bool) {
        let api = device.api();
        let landed = self
            .flights
            .extract_if(.., |flight| finished || flight.landed(api));

        for flight in landed {
            if let Some(transferred) = flight.transferred {
                self.transfer.release(device, transferred);
            }
        }
    }

    /// Returns libplacebo's wrapper for the target, replacing one for another image
    fn wrap_target(
        &mut self,
        device: &SharedDevice,
        target: &RenderTarget,
    ) -> Result<native::pl_tex, RenderError> {
        let gpu = device.gpu();
        let Some(wrapped) = self.targets.get_mut(target.slot) else {
            return Err(RenderError::Failed("The video target slot does not exist"));
        };

        // SAFETY: A wrapper stays readable until it is destroyed
        let current = wrapped.image == target.image
            && unsafe { wrapped.texture.as_ref() }.is_some_and(|texture| {
                [texture.params.w, texture.params.h]
                    == target.size.map(|side| side as c_int)
            });

        if current {
            return Ok(wrapped.texture);
        }

        let params = native::pl_vulkan_wrap_params {
            image: target.image,
            width: target.size[0] as c_int,
            height: target.size[1] as c_int,
            format: vk::Format::R8G8B8A8_UNORM,
            usage: vk::ImageUsageFlags::COLOR_ATTACHMENT
                | vk::ImageUsageFlags::SAMPLED
                | vk::ImageUsageFlags::TRANSFER_SRC
                | vk::ImageUsageFlags::TRANSFER_DST,
            ..Default::default()
        };

        // SAFETY: The player keeps the image alive until it retires its targets
        unsafe {
            if !wrapped.texture.is_null() {
                native::pl_gpu_finish(gpu);
                native::pl_tex_destroy(gpu, &mut wrapped.texture);
            }

            wrapped.texture = native::pl_vulkan_wrap(gpu, &params);
        }
        wrapped.image = target.image;

        if wrapped.texture.is_null() {
            return Err(RenderError::Failed(
                "Cannot share the video target with the renderer",
            ));
        }

        Ok(wrapped.texture)
    }

    /// Copies a frame that libplacebo cannot sample; other frames need no transfer
    fn transfer(
        &mut self,
        device: &SharedDevice,
        frame: &AvFrame,
    ) -> Result<Option<TransferredFrame>, RenderError> {
        // Frames from the transfer's own pool are sampleable, so one step is enough
        match frame.data().format {
            ffi::AV_PIX_FMT_CUDA => self
                .transfer
                .copy_cuda(device, frame)
                .map(Some)
                .ok_or(RenderError::UnsupportedFrame(
                    "Cannot transfer the NVDEC frame to Vulkan without CPU readback",
                )),
            ffi::AV_PIX_FMT_VULKAN if FrameTransfer::needs_copy(frame) => {
                self.transfer.copy_vulkan(device, frame).map(Some).ok_or(
                    RenderError::UnsupportedFrame(
                        "Cannot copy Vulkan decode planes without CPU readback",
                    ),
                )
            }
            _ => Ok(None),
        }
    }

    /// Cleans up after a frame that will not be drawn
    fn abandon(
        &mut self,
        device: &SharedDevice,
        mapped: Option<&mut native::pl_frame>,
        transferred: Option<TransferredFrame>,
    ) {
        // SAFETY: The device is alive, and the mapping was made by this renderer
        unsafe {
            native::pl_gpu_finish(device.gpu());

            if let Some(mapped) = mapped {
                native::flowa_frame_unmap(device.gpu(), mapped);
            }
        }

        if let Some(transferred) = transferred {
            self.transfer.release(device, transferred);
        }
    }
}

/// Renders a mapped frame while the player lends the target, and reports whether the
/// frame was drawn and the target handed back
fn draw(
    device: &SharedDevice,
    renderer: native::pl_renderer,
    mapped: &mut native::pl_frame,
    texture: native::pl_tex,
    target: &RenderTarget,
) -> bool {
    let gpu = device.gpu();
    let lend = native::pl_vulkan_release_params {
        tex: texture,
        layout: vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL,
        qf: vk::QUEUE_FAMILY_IGNORED,
        semaphore: native::pl_vulkan_sem {
            sem: target.semaphore,
            value: target.wait_value,
        },
    };
    let reclaim = native::pl_vulkan_hold_params {
        tex: texture,
        layout: vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL,
        qf: vk::QUEUE_FAMILY_IGNORED,
        semaphore: native::pl_vulkan_sem {
            sem: target.semaphore,
            value: target.signal_value,
        },
        ..Default::default()
    };

    // SAFETY: The texture wraps the target's image, which the player lends until the
    // signal value, and a flight keeps the mapped frame's images alive
    unsafe {
        native::pl_vulkan_release_ex(gpu, &lend);
        let rendered = native::flowa_frame_render(renderer, mapped, texture);
        native::flowa_frame_unmap(gpu, mapped);

        // Always return ownership, even when a shader or hardware import fails
        let held = native::pl_vulkan_hold_ex(gpu, &reclaim);
        native::pl_gpu_flush(gpu);

        rendered && held
    }
}
