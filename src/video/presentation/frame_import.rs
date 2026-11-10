//! Turns decoded frames into planes libplacebo can sample
//!
//! CPU frames and most hardware frames map directly. The match in `transfer` is the
//! one place that decides which hardware storage needs a GPU copy first

use std::ptr;

use super::{
    frame_transfer::{FrameTransfer, TransferredFrame},
    renderer::RenderError,
};
use crate::video::{
    decoding::{DecodedFrame, HardwareStorage},
    ffi::native,
    ffmpeg::Stream,
    gpu::SharedDevice,
};

/// A frame mapped for libplacebo, with the copy it was mapped from when it needed one
pub(super) struct ImportedFrame {
    pub mapped: native::pl_frame,
    pub transferred: Option<TransferredFrame>,
}

pub(super) struct FrameImport {
    /// Textures libplacebo reuses for the planes of software frames
    software_planes: [native::pl_tex; 4],
    transfer: FrameTransfer,
}

impl Default for FrameImport {
    fn default() -> Self {
        Self {
            software_planes: [ptr::null(); 4],
            transfer: FrameTransfer::default(),
        }
    }
}

impl FrameImport {
    /// Maps a frame's planes, copying them first when libplacebo cannot sample them
    pub fn import(
        &mut self,
        device: &SharedDevice,
        frame: &DecodedFrame,
        stream: &Stream,
    ) -> Result<ImportedFrame, RenderError> {
        let transferred = self.transfer(device, frame)?;
        let source = transferred
            .as_ref()
            .map_or(frame.av_frame(), |transferred| &transferred.frame);
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

        Ok(ImportedFrame {
            mapped,
            transferred,
        })
    }

    /// Copies a frame that libplacebo cannot sample; other frames need no transfer
    fn transfer(
        &mut self,
        device: &SharedDevice,
        frame: &DecodedFrame,
    ) -> Result<Option<TransferredFrame>, RenderError> {
        let DecodedFrame::Hardware(hardware) = frame else {
            return Ok(None);
        };
        let source = hardware.av_frame();

        // Frames from the transfer's own pool are sampleable, so one step is enough
        match hardware.storage() {
            HardwareStorage::Cuda => self
                .transfer
                .copy_cuda(device, source)
                .map(Some)
                .ok_or(RenderError::UnsupportedFrame(
                    "Cannot transfer the NVDEC frame to Vulkan without CPU readback",
                )),
            HardwareStorage::Vulkan if FrameTransfer::needs_copy(source) => {
                self.transfer.copy_vulkan(device, source).map(Some).ok_or(
                    RenderError::UnsupportedFrame(
                        "Cannot copy Vulkan decode planes without CPU readback",
                    ),
                )
            }
            HardwareStorage::Vulkan | HardwareStorage::Vaapi => Ok(None),
        }
    }

    /// Frees a transferred frame once the GPU no longer reads it
    pub fn release(&self, device: &SharedDevice, transferred: TransferredFrame) {
        self.transfer.release(device, transferred);
    }

    /// Cleans up after a frame that will not be drawn
    pub fn abandon(
        &mut self,
        device: &SharedDevice,
        mapped: Option<&mut native::pl_frame>,
        transferred: Option<TransferredFrame>,
    ) {
        // SAFETY: The device is alive, and the mapping was made by this importer
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

    /// Releases the textures and copy pool sized for the video being closed
    pub fn reset(&mut self, device: &SharedDevice) {
        self.transfer.reset();

        for plane in &mut self.software_planes {
            // SAFETY: Nothing is rendering from the plane textures any more
            unsafe { native::pl_tex_destroy(device.gpu(), plane) };
        }
    }

    /// Releases the Vulkan objects, after every transferred frame has been released
    pub fn destroy(&mut self, device: &SharedDevice) {
        self.reset(device);
        self.transfer.destroy(device);
    }
}
