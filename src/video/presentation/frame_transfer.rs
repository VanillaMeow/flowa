//! GPU copies of decoder frames into Vulkan planes that libplacebo can sample

use std::ptr::NonNull;

use ash::{prelude::VkResult, vk};

use crate::video::{
    ffi::{ffmpeg as ffi, native},
    ffmpeg::AvFrame,
    gpu::SharedDevice,
};

/// A sampleable copy of a decoder frame
pub(super) struct TransferredFrame {
    pub frame: AvFrame,
    /// The copy flowa submitted itself, which must complete before its buffer is freed
    command: Option<vk::CommandBuffer>,
}

/// Copies decoder frames that libplacebo cannot sample, without reading them back to
/// the CPU
#[derive(Default)]
pub(super) struct FrameTransfer {
    pool: Option<FramePool>,
    commands: Option<Commands>,
}

impl FrameTransfer {
    /// Whether a Vulkan frame's planes must be copied before libplacebo can sample them
    pub fn needs_copy(frame: &AvFrame) -> bool {
        // SAFETY: A Vulkan frame references the pool that allocated it
        unsafe {
            let pool = &*frames_context(frame);
            let images = &*pool.hwctx.cast::<ffi::AVVulkanFramesContext>();
            let multiplane = ffi::av_pix_fmt_count_planes(pool.sw_format) > 1
                && images.format[1] == vk::Format::UNDEFINED;
            let needs_plane_views = multiplane
                && !images
                    .img_flags
                    .contains(vk::ImageCreateFlags::MUTABLE_FORMAT);

            needs_plane_views || !images.usage.contains(vk::ImageUsageFlags::SAMPLED)
        }
    }

    /// Copies an NVDEC frame into Vulkan images
    pub fn copy_cuda(
        &mut self,
        device: &SharedDevice,
        source: &AvFrame,
    ) -> Option<TransferredFrame> {
        let frame = self.transfer_cuda(device, source);

        if frame.is_none() {
            // SAFETY: The device is alive, and a failed transfer may have submitted work
            unsafe { native::pl_gpu_finish(device.gpu()) };
        }

        Some(TransferredFrame {
            frame: frame?,
            command: None,
        })
    }

    fn transfer_cuda(
        &mut self,
        device: &SharedDevice,
        source: &AvFrame,
    ) -> Option<AvFrame> {
        let pool = self.frame_pool(device, source)?;
        let frame = AvFrame::allocate().ok()?;

        // SAFETY: The new frame is exclusively owned and the source is only read
        unsafe {
            if ffi::av_hwframe_get_buffer(pool, frame.raw(), 0) < 0 {
                return None;
            }

            // FFmpeg performs a device-to-device CUDA copy with external GPU semaphores
            if ffi::av_hwframe_transfer_data(frame.raw(), source.raw(), 0) < 0 {
                return None;
            }

            inherit_properties(&frame, source)?;
        }

        Some(frame)
    }

    /// Copies the planes of a decode-only Vulkan frame into sampleable images
    pub fn copy_vulkan(
        &mut self,
        device: &SharedDevice,
        source: &AvFrame,
    ) -> Option<TransferredFrame> {
        let pool = self.frame_pool(device, source)?;
        let frame = AvFrame::allocate().ok()?;
        let commands = self.commands(device)?;

        // SAFETY: The new frame is exclusively owned and the source is only read
        unsafe {
            if ffi::av_hwframe_get_buffer(pool, frame.raw(), 0) < 0 {
                return None;
            }

            inherit_properties(&frame, source)?;
        }

        let api = device.api();
        let allocation = vk::CommandBufferAllocateInfo::default()
            .command_pool(commands.pool)
            .level(vk::CommandBufferLevel::PRIMARY)
            .command_buffer_count(1);

        // SAFETY: The pool belongs to this device and is only used by the playback owner
        let command = *unsafe { api.allocate_command_buffers(&allocation) }
            .ok()?
            .first()?;

        // SAFETY: Both frames are Vulkan frames of this device
        if unsafe { commands.copy(device, command, source, &frame) }.is_err() {
            // SAFETY: A buffer whose submission failed is not pending
            unsafe { api.free_command_buffers(commands.pool, &[command]) };
            return None;
        }

        Some(TransferredFrame {
            frame,
            command: Some(command),
        })
    }

    /// Frees a transferred frame, waiting for the copy that wrote it when there is one
    pub fn release(&self, device: &SharedDevice, transferred: TransferredFrame) {
        let (Some(command), Some(commands)) = (transferred.command, self.commands)
        else {
            return;
        };

        // SAFETY: The frame came from the upload pool, and its buffer from this pool
        unsafe {
            let pool = &*frames_context(&transferred.frame);
            let images = &*pool.hwctx.cast::<ffi::AVVulkanFramesContext>();
            let state = &*transferred.frame.data().data[0].cast::<ffi::AVVkFrame>();
            let count = image_count(images);
            let wait = vk::SemaphoreWaitInfo::default()
                .semaphores(&state.sem[..count])
                .values(&state.sem_value[..count]);

            // Rendering has normally completed; error cleanup must also wait for the copy
            let _ = device.api().wait_semaphores(&wait, u64::MAX);
            device.api().free_command_buffers(commands.pool, &[command]);
        }
    }

    /// Drops the upload pool, whose frames belong to the video being closed
    pub fn reset(&mut self) {
        self.pool = None;
    }

    /// Releases the Vulkan objects, after every transferred frame has been released
    pub fn destroy(&mut self, device: &SharedDevice) {
        self.pool = None;

        if let Some(commands) = self.commands.take() {
            // SAFETY: Releasing the transferred frames freed every buffer of the pool
            unsafe { device.api().destroy_command_pool(commands.pool, None) };
        }
    }

    /// A pool of sampleable Vulkan frames matching the source's size and format
    fn frame_pool(
        &mut self,
        device: &SharedDevice,
        source: &AvFrame,
    ) -> Option<*mut ffi::AVBufferRef> {
        // SAFETY: A hardware frame references the pool that allocated it
        let source = unsafe { &*frames_context(source) };

        if let Some(pool) = self.pool.as_ref().filter(|pool| pool.matches(source)) {
            return Some(pool.frames.as_ptr());
        }

        self.pool = None;
        let pool = FramePool::create(device.ffmpeg_device()?, source)?;

        Some(self.pool.insert(pool).frames.as_ptr())
    }

    fn commands(&mut self, device: &SharedDevice) -> Option<Commands> {
        if self.commands.is_none() {
            self.commands = Some(Commands::create(device)?);
        }

        self.commands
    }
}

/// Vulkan frames that both FFmpeg's CUDA transfer and plane copies can write
struct FramePool {
    frames: NonNull<ffi::AVBufferRef>,
    format: ffi::AVPixelFormat,
    width: i32,
    height: i32,
    /// Identifies the decoder device; never dereferenced
    source_device: *const ffi::AVHWDeviceContext,
}

impl FramePool {
    fn create(
        device: *mut ffi::AVBufferRef,
        source: &ffi::AVHWFramesContext,
    ) -> Option<Self> {
        // SAFETY: The device reference is FFmpeg's context for the shared Vulkan device
        let frames = NonNull::new(unsafe { ffi::av_hwframe_ctx_alloc(device) })?;
        let pool = Self {
            frames,
            format: source.sw_format,
            width: source.width,
            height: source.height,
            source_device: source.device_ctx,
        };

        // SAFETY: The new context is exclusively owned until it is initialized
        unsafe {
            let context = &mut *frames.as_ref().data.cast::<ffi::AVHWFramesContext>();
            let vulkan = &mut *context.hwctx.cast::<ffi::AVVulkanFramesContext>();

            context.format = ffi::AV_PIX_FMT_VULKAN;
            context.sw_format = source.sw_format;
            context.width = source.width;
            context.height = source.height;
            vulkan.flags = ffi::AV_VK_FRAME_FLAG_DISABLE_MULTIPLANE;
            vulkan.usage = vk::ImageUsageFlags::SAMPLED
                | vk::ImageUsageFlags::TRANSFER_DST
                | vk::ImageUsageFlags::TRANSFER_SRC;

            if ffi::av_hwframe_ctx_init(frames.as_ptr()) < 0 {
                return None;
            }
        }

        Some(pool)
    }

    fn matches(&self, source: &ffi::AVHWFramesContext) -> bool {
        self.format == source.sw_format
            && self.width == source.width
            && self.height == source.height
            && self.source_device == source.device_ctx.cast_const()
    }
}

impl Drop for FramePool {
    fn drop(&mut self) {
        let mut frames = self.frames.as_ptr();

        // SAFETY: This owner holds one reference; allocated frames hold their own
        unsafe { ffi::av_buffer_unref(&mut frames) };
    }
}

/// The command pool and queue for plane copies, used only by the playback owner
#[derive(Clone, Copy)]
struct Commands {
    pool: vk::CommandPool,
    queue: vk::Queue,
}

impl Commands {
    fn create(device: &SharedDevice) -> Option<Self> {
        // The copies use synchronization commands that Vulkan 1.3 made core
        if !device.supports(vk::API_VERSION_1_3) {
            return None;
        }

        let family = device.graphics_family();
        let info = vk::CommandPoolCreateInfo::default()
            .flags(vk::CommandPoolCreateFlags::TRANSIENT)
            .queue_family_index(family);

        // SAFETY: The device is alive and playback owns queue zero of its graphics family
        unsafe {
            Some(Self {
                pool: device.api().create_command_pool(&info, None).ok()?,
                queue: device.api().get_device_queue(family, 0),
            })
        }
    }

    /// Records and submits the copy, then advances the state FFmpeg tracks per frame
    ///
    /// # Safety
    /// Both frames must be Vulkan frames of this device, and the buffer must be new
    unsafe fn copy(
        self,
        device: &SharedDevice,
        command: vk::CommandBuffer,
        source: &AvFrame,
        destination: &AvFrame,
    ) -> VkResult<()> {
        let api = device.api();
        let family = device.graphics_family();
        let begin = vk::CommandBufferBeginInfo::default()
            .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT);

        // SAFETY: The caller provides a new buffer and two Vulkan frames of this device
        unsafe {
            api.begin_command_buffer(command, &begin)?;

            let mut source = LockedFrame::lock(source, Role::Source);
            let mut destination = LockedFrame::lock(destination, Role::Destination);
            let mut handoff = Handoff::default();
            let ready =
                handoff.add(&source, family) && handoff.add(&destination, family);

            if !ready {
                return Err(vk::Result::ERROR_INITIALIZATION_FAILED);
            }

            let dependency =
                vk::DependencyInfo::default().image_memory_barriers(&handoff.barriers);
            api.cmd_pipeline_barrier2(command, &dependency);
            copy_planes(api, command, &source, &destination);
            api.end_command_buffer(command)?;

            let buffers =
                [vk::CommandBufferSubmitInfo::default().command_buffer(command)];
            let submission = vk::SubmitInfo2::default()
                .wait_semaphore_infos(&handoff.waits)
                .command_buffer_infos(&buffers)
                .signal_semaphore_infos(&handoff.signals);
            device.with_graphics_queue(|| {
                api.queue_submit2(self.queue, &[submission], vk::Fence::null())
            })?;

            source.advance(family);
            destination.advance(family);
        }

        Ok(())
    }
}

/// Which side of a plane copy a frame is on
#[derive(Clone, Copy)]
enum Role {
    Source,
    Destination,
}

impl Role {
    fn usage(self) -> vk::ImageUsageFlags {
        match self {
            Self::Source => vk::ImageUsageFlags::TRANSFER_SRC,
            Self::Destination => vk::ImageUsageFlags::TRANSFER_DST,
        }
    }

    fn access(self) -> vk::AccessFlags2 {
        match self {
            Self::Source => vk::AccessFlags2::TRANSFER_READ,
            Self::Destination => vk::AccessFlags2::TRANSFER_WRITE,
        }
    }

    fn layout(self) -> vk::ImageLayout {
        match self {
            Self::Source => vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
            Self::Destination => vk::ImageLayout::TRANSFER_DST_OPTIMAL,
        }
    }
}

/// A Vulkan frame whose image state is locked against FFmpeg's other threads
struct LockedFrame {
    pool: *mut ffi::AVHWFramesContext,
    frame: *mut ffi::AVVkFrame,
    role: Role,
}

impl LockedFrame {
    /// # Safety
    /// The frame must be a Vulkan frame that outlives the lock
    unsafe fn lock(frame: &AvFrame, role: Role) -> Self {
        // SAFETY: A Vulkan frame stores its image state in the first data pointer
        let locked = unsafe {
            Self {
                pool: frames_context(frame),
                frame: frame.data().data[0].cast(),
                role,
            }
        };

        if let Some(lock) = locked.images().lock_frame {
            // SAFETY: FFmpeg provides the lock for frames of this pool
            unsafe { lock(locked.pool, locked.frame) };
        }

        locked
    }

    fn pool(&self) -> &ffi::AVHWFramesContext {
        // SAFETY: The locked frame keeps its pool alive
        unsafe { &*self.pool }
    }

    fn images(&self) -> &ffi::AVVulkanFramesContext {
        // SAFETY: A Vulkan frame pool stores this context
        unsafe { &*self.pool().hwctx.cast() }
    }

    fn state(&self) -> &ffi::AVVkFrame {
        // SAFETY: The lock gives this owner exclusive access to the image state
        unsafe { &*self.frame }
    }

    /// Whether the pool stores every plane in one multiplanar image
    fn multiplane(&self) -> bool {
        // SAFETY: The format belongs to an initialized pool
        let planes = unsafe { ffi::av_pix_fmt_count_planes(self.pool().sw_format) };

        planes > 1 && self.images().format[1] == vk::Format::UNDEFINED
    }

    /// Records the layout, access, and semaphore values the submitted copy leaves behind
    fn advance(&mut self, family: u32) {
        let count = image_count(self.images());

        // SAFETY: The lock gives this owner exclusive access to the image state
        let state = unsafe { &mut *self.frame };

        for image in 0..count {
            state.layout[image] = self.role.layout();
            state.access[image] = self.role.access();
            state.sem_value[image] += 1;

            if state.queue_family[image] != vk::QUEUE_FAMILY_IGNORED {
                state.queue_family[image] = family;
            }
        }
    }
}

impl Drop for LockedFrame {
    fn drop(&mut self) {
        if let Some(unlock) = self.images().unlock_frame {
            // SAFETY: This owner took the lock when it was created
            unsafe { unlock(self.pool, self.frame) };
        }
    }
}

/// Layout transitions and semaphore values that hand images to a copy and back
#[derive(Default)]
struct Handoff {
    barriers: Vec<vk::ImageMemoryBarrier2<'static>>,
    waits: Vec<vk::SemaphoreSubmitInfo<'static>>,
    signals: Vec<vk::SemaphoreSubmitInfo<'static>>,
}

impl Handoff {
    /// Adds every image of a frame, or reports that the copy cannot use them
    fn add(&mut self, frame: &LockedFrame, family: u32) -> bool {
        let images = frame.images();
        let state = frame.state();
        let role = frame.role;

        if !images.usage.contains(role.usage()) {
            return false;
        }

        for image in 0..image_count(images) {
            let owner = state.queue_family[image];
            let foreign = owner != vk::QUEUE_FAMILY_IGNORED
                && owner != family
                && state.layout[image] != vk::ImageLayout::UNDEFINED;

            if foreign {
                return false;
            }

            let range = vk::ImageSubresourceRange::default()
                .aspect_mask(vk::ImageAspectFlags::COLOR)
                .level_count(1)
                .layer_count(1);
            let barrier = vk::ImageMemoryBarrier2::default()
                // Timeline waits make the previous queue's writes available
                .src_stage_mask(vk::PipelineStageFlags2::NONE)
                .src_access_mask(vk::AccessFlags2::NONE)
                .dst_stage_mask(vk::PipelineStageFlags2::TRANSFER)
                .dst_access_mask(role.access())
                .old_layout(state.layout[image])
                .new_layout(role.layout())
                .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                .image(state.img[image])
                .subresource_range(range);
            let wait = vk::SemaphoreSubmitInfo::default()
                .semaphore(state.sem[image])
                .value(state.sem_value[image])
                .stage_mask(vk::PipelineStageFlags2::ALL_COMMANDS);

            self.barriers.push(barrier);
            self.waits.push(wait);
            self.signals.push(wait.value(state.sem_value[image] + 1));
        }

        true
    }
}

/// # Safety
/// The buffer must be recording, with both frames' images in their transfer layouts
unsafe fn copy_planes(
    api: &ash::Device,
    command: vk::CommandBuffer,
    source: &LockedFrame,
    destination: &LockedFrame,
) {
    let pool = source.pool();

    // SAFETY: The format belongs to an initialized pool, which has a static descriptor
    let (planes, format) = unsafe {
        (
            ffi::av_pix_fmt_count_planes(pool.sw_format).max(0) as u32,
            &*ffi::av_pix_fmt_desc_get(pool.sw_format),
        )
    };
    let layers = |frame: &LockedFrame, plane: u32| {
        let aspect = if frame.multiplane() {
            vk::ImageAspectFlags::from_raw(
                vk::ImageAspectFlags::PLANE_0.as_raw() << plane,
            )
        } else {
            vk::ImageAspectFlags::COLOR
        };

        vk::ImageSubresourceLayers::default()
            .aspect_mask(aspect)
            .layer_count(1)
    };
    let image = |frame: &LockedFrame, plane: u32| {
        let index = if frame.multiplane() { 0 } else { plane };

        frame.state().img[index as usize]
    };

    for plane in 0..planes {
        let (shift_width, shift_height) = if plane == 0 {
            (0, 0)
        } else {
            (format.log2_chroma_w, format.log2_chroma_h)
        };
        let region = vk::ImageCopy::default()
            .src_subresource(layers(source, plane))
            .dst_subresource(layers(destination, plane))
            .extent(vk::Extent3D {
                width: plane_extent(pool.width, shift_width),
                height: plane_extent(pool.height, shift_height),
                depth: 1,
            });

        // SAFETY: The caller provides a recording buffer and images in these layouts
        unsafe {
            api.cmd_copy_image(
                command,
                image(source, plane),
                vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
                image(destination, plane),
                vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                &[region],
            );
        }
    }
}

/// A plane's size along one axis, rounding the subsampled size up
fn plane_extent(size: i32, shift: u8) -> u32 {
    let size = size.max(0) as u32;

    (size + (1 << shift) - 1) >> shift
}

/// How many separate images a pool allocates for each frame
fn image_count(images: &ffi::AVVulkanFramesContext) -> usize {
    images
        .format
        .iter()
        .take_while(|&&format| format != vk::Format::UNDEFINED)
        .count()
}

/// # Safety
/// The frame must be a hardware frame, which references the pool that allocated it
unsafe fn frames_context(frame: &AvFrame) -> *mut ffi::AVHWFramesContext {
    // SAFETY: The caller provides a frame with a pool reference
    unsafe { (*frame.data().hw_frames_ctx).data.cast() }
}

/// Gives a transferred frame its source's timing, color, and displayed size
///
/// # Safety
/// The transferred frame must be exclusively owned
unsafe fn inherit_properties(transferred: &AvFrame, source: &AvFrame) -> Option<()> {
    // SAFETY: The caller owns the transferred frame, and the source is only read
    unsafe {
        if ffi::av_frame_copy_props(transferred.raw(), source.raw()) < 0 {
            return None;
        }

        (*transferred.raw()).width = source.data().width;
        (*transferred.raw()).height = source.data().height;
    }

    Some(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subsampled_planes_round_odd_sizes_up() {
        assert_eq!(plane_extent(1920, 0), 1920);
        assert_eq!(plane_extent(1920, 1), 960);
        assert_eq!(plane_extent(1921, 1), 961);
        assert_eq!(plane_extent(5, 2), 2);
    }
}
