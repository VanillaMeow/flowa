//! The shared Vulkan device, created with only the queues that flowa uses
//!
//! Libplacebo always creates every queue of every queue family, which costs over
//! 100 ms on some NVIDIA drivers. Creating the device here and importing it keeps
//! startup fast while enabling the same extensions and features

use std::{
    ffi::{CStr, CString, c_char, c_int, c_void},
    ptr,
};

use anyhow::{Context, Result};
use ash::vk;

use super::{
    device_extensions::{self, DeviceExtensions},
    device_features::{self, FeatureChain},
};
use crate::video::{
    ffi::{ffmpeg as ffi, native},
    ffmpeg::HwDevice,
};

const QUEUE_PRIORITIES: [f32; 2] = [1.0; 2];

/// The adapter and the wgpu requirements that the shared device must satisfy
pub(super) struct DeviceRequest<'a> {
    pub instance: &'a ash::Instance,
    pub get_proc_addr: vk::PFN_vkGetInstanceProcAddr,
    pub physical_device: vk::PhysicalDevice,
    pub extensions: &'a [&'static CStr],
    pub features: &'a vk::PhysicalDeviceFeatures2<'a>,
}

/// The Vulkan device with libplacebo's and FFmpeg's views of it
///
/// Nothing changes after creation, so decoder detection can read it during playback
pub(in crate::video) struct SharedDevice {
    // Fields drop in order: FFmpeg's view, then the device, then the log it reports to
    decoding: Option<HwDevice>,
    video_queue: bool,
    render_node: Option<CString>,
    wgpu_queue: u32,
    vulkan: Vulkan,
    log: Log,
}

impl SharedDevice {
    /// Creates the device along with the libplacebo and FFmpeg contexts that share it
    ///
    /// # Safety
    /// The request's instance, physical device, and feature chain must be valid, and the
    /// instance must outlive the returned device
    pub(super) unsafe fn create(request: &DeviceRequest<'_>) -> Result<Self> {
        let log = Log::create();
        let decoding = device_extensions::decoding();

        // SAFETY: The caller provides a valid request
        let selected = unsafe { DeviceExtensions::select(request, &decoding) }.ok();
        let imported = selected.as_ref().and_then(|extensions| unsafe {
            Vulkan::import(&log, request, extensions)
        });
        let vulkan = match imported {
            Some(vulkan) => vulkan,
            None => {
                log::warn!(
                    "Video GPU: Creating the video device with every queue enabled"
                );
                let extensions = selected.unwrap_or_default();

                unsafe { Vulkan::create(&log, request, &decoding, &extensions) }
                    .context("Cannot create the shared Vulkan video device")?
            }
        };

        // SAFETY: The caller provides a valid instance and physical device
        let families = unsafe {
            request
                .instance
                .get_physical_device_queue_family_properties(request.physical_device)
        };
        let graphics = families.get(vulkan.graphics_family() as usize);

        // Playback renders on queue 0; wgpu takes queue 1 when the family has one
        let wgpu_queue =
            u32::from(graphics.is_some_and(|family| family.queue_count > 1));

        // SAFETY: The device and the structures it was created with are alive
        let (decoding, video_queue) = unsafe { vulkan.decoding_device(&families) }
            .map_or((None, false), |(device, video)| (Some(device), video));
        let video_queue = video_queue
            && vulkan
                .extensions
                .contains(ash::khr::video_decode_queue::NAME);

        Ok(Self {
            decoding,
            video_queue,
            // SAFETY: The caller provides a valid instance and physical device
            render_node: unsafe { render_node(request, &vulkan.extensions) },
            wgpu_queue,
            vulkan,
            log,
        })
    }

    pub fn log(&self) -> native::pl_log {
        self.log.0
    }

    pub fn gpu(&self) -> native::pl_gpu {
        // SAFETY: Libplacebo keeps its device description alive until it is destroyed
        unsafe { (*self.vulkan.vk).gpu }
    }

    /// Vulkan functions for the device, whichever path created it
    pub fn api(&self) -> &ash::Device {
        &self.vulkan.api
    }

    pub fn graphics_family(&self) -> u32 {
        self.vulkan.graphics_family()
    }

    /// The graphics queue wgpu submits on, leaving queue zero to playback when it can
    pub fn wgpu_queue(&self) -> u32 {
        self.wgpu_queue
    }

    pub fn supports(&self, version: u32) -> bool {
        // SAFETY: Libplacebo keeps its device description alive until it is destroyed
        unsafe { (*self.vulkan.vk).api_version >= version }
    }

    pub(super) fn extensions(&self) -> &DeviceExtensions {
        &self.vulkan.extensions
    }

    /// FFmpeg's context for this device, when FFmpeg supports it
    pub fn ffmpeg_device(&self) -> Option<*mut ffi::AVBufferRef> {
        self.decoding.as_ref().map(HwDevice::raw)
    }

    /// Whether the device has a Vulkan video decode queue that FFmpeg can use
    pub fn has_video_queue(&self) -> bool {
        self.video_queue
    }

    /// The DRM render node of the GPU, for APIs that open it separately
    pub fn render_node(&self) -> Option<&CStr> {
        self.render_node.as_deref()
    }

    /// Runs a submission while holding libplacebo's lock on playback's graphics queue
    pub fn with_graphics_queue<T>(&self, submit: impl FnOnce() -> T) -> T {
        let vk = self.vulkan.vk;
        let family = self.graphics_family();

        // SAFETY: Libplacebo provides both functions for the lifetime of its device
        unsafe {
            if let Some(lock) = (*vk).lock_queue {
                lock(vk, family, 0);
            }

            let result = submit();

            if let Some(unlock) = (*vk).unlock_queue {
                unlock(vk, family, 0);
            }

            result
        }
    }
}

/// Libplacebo's device and the Vulkan device underneath it
struct Vulkan {
    vk: native::pl_vulkan,
    api: ash::Device,
    /// Whether flowa created the device, which libplacebo then leaves for it to destroy
    owned: bool,
    extensions: DeviceExtensions,
    /// The structures an owned device was created with, which FFmpeg keeps reading
    features: Option<FeatureChain>,
}

impl Vulkan {
    /// Creates a device with only the queues flowa uses and hands it to libplacebo
    ///
    /// # Safety
    /// The request's instance, physical device, and feature chain must be valid
    unsafe fn import(
        log: &Log,
        request: &DeviceRequest<'_>,
        extensions: &DeviceExtensions,
    ) -> Option<Self> {
        // SAFETY: The caller provides a valid request
        let (families, features) = unsafe {
            if !device_features::covers(request.features) {
                return None;
            }

            let families = request
                .instance
                .get_physical_device_queue_family_properties(request.physical_device);

            (families, FeatureChain::query(request, extensions))
        };
        let queues = QueuePlan::new(&families)?;
        let info = vk::DeviceCreateInfo {
            p_next: features.core().cast(),
            queue_create_info_count: queues.families.len() as u32,
            p_queue_create_infos: queues.families.as_ptr(),
            enabled_extension_count: extensions.len() as u32,
            pp_enabled_extension_names: extensions.as_ptr(),
            ..Default::default()
        };

        // SAFETY: The description borrows the queue plan, extensions, and features above
        let api = unsafe {
            request
                .instance
                .create_device(request.physical_device, &info, None)
        }
        .ok()?;
        let params = native::pl_vulkan_import_params {
            instance: request.instance.handle(),
            get_proc_addr: Some(request.get_proc_addr),
            phys_device: request.physical_device,
            device: api.handle(),
            extensions: extensions.as_ptr(),
            num_extensions: extensions.len() as c_int,
            queue_graphics: queues.graphics,
            queue_compute: queues.compute,
            queue_transfer: queues.transfer,
            features: features.core(),
            ..Default::default()
        };

        // SAFETY: Libplacebo copies what it keeps from the parameters
        let vk = unsafe { native::pl_vulkan_import(log.0, &params) };
        if vk.is_null() {
            // SAFETY: Nothing else has used the device yet
            unsafe { api.destroy_device(None) };
            return None;
        }

        Some(Self {
            vk,
            api,
            owned: true,
            extensions: extensions.clone(),
            features: Some(features),
        })
    }

    /// The slower path for feature requests that `import` cannot reproduce
    ///
    /// # Safety
    /// The request's instance, physical device, and feature chain must be valid
    unsafe fn create(
        log: &Log,
        request: &DeviceRequest<'_>,
        decoding: &[&'static CStr],
        extensions: &DeviceExtensions,
    ) -> Option<Self> {
        let required = name_pointers(request.extensions);
        let optional = name_pointers(decoding);

        // SAFETY: The caller provides a valid request
        let mut features = unsafe { FeatureChain::query(request, extensions) };

        // Libplacebo merges wgpu's requests with the queried core features
        features.replace_extensions(request.features.p_next);

        let params = native::pl_vulkan_params {
            instance: request.instance.handle(),
            device: request.physical_device,
            get_proc_addr: Some(request.get_proc_addr),
            extensions: required.as_ptr(),
            num_extensions: required.len() as c_int,
            opt_extensions: optional.as_ptr(),
            num_opt_extensions: optional.len() as c_int,
            features: features.core(),
            queue_count: 1,
            extra_queues: vk::QueueFlags::VIDEO_DECODE_KHR,
            // SAFETY: The shim returns libplacebo's static default parameters
            ..unsafe { ptr::read(native::flowa_vulkan_default_params()) }
        };

        // SAFETY: Libplacebo copies what it keeps from the parameters, which borrow the
        // lists above and wgpu's feature chain
        unsafe {
            let vk = native::pl_vulkan_create(log.0, &params);
            if vk.is_null() {
                return None;
            }

            Some(Self {
                vk,
                api: ash::Device::load(request.instance.fp_v1_0(), (*vk).device),
                owned: false,
                // Libplacebo owns this list for the lifetime of the device
                extensions: DeviceExtensions::adopt(
                    (*vk).extensions,
                    (*vk).num_extensions,
                ),
                features: None,
            })
        }
    }

    fn graphics_family(&self) -> u32 {
        // SAFETY: Libplacebo keeps its device description alive until it is destroyed
        unsafe { (*self.vk).queue_graphics.index }
    }

    /// The features enabled on the device, whichever path created it
    fn enabled_features(&self) -> *const vk::PhysicalDeviceFeatures2<'static> {
        match &self.features {
            Some(features) => features.core(),
            // SAFETY: Libplacebo keeps its device description alive until it is destroyed
            None => unsafe { (*self.vk).features },
        }
    }

    /// Gives FFmpeg the device for hardware decoding and frame transfers, and reports
    /// whether one of its queues decodes video
    ///
    /// # Safety
    /// The device must be alive, and must outlive the returned reference
    unsafe fn decoding_device(
        &self,
        families: &[vk::QueueFamilyProperties],
    ) -> Option<(HwDevice, bool)> {
        // SAFETY: A new Vulkan device reference holds the context FFmpeg expects to be
        // filled in before initialization, and the caller keeps the device alive
        unsafe {
            let reference = HwDevice::from_raw(ffi::av_hwdevice_ctx_alloc(
                ffi::AV_HWDEVICE_TYPE_VULKAN,
            ))
            .ok()?;
            let device = (*reference.raw()).data.cast::<ffi::AVHWDeviceContext>();
            let context = &mut *(*device).hwctx.cast::<ffi::AVVulkanDeviceContext>();
            let vk = &*self.vk;

            context.inst = vk.instance;
            context.phys_dev = vk.phys_device;
            context.act_dev = vk.device;
            context.get_proc_addr = vk.get_proc_addr;
            context.device_features = *self.enabled_features();
            context.enabled_dev_extensions = self.extensions.as_ptr();
            context.nb_enabled_dev_extensions = self.extensions.len() as c_int;

            let mut video_queue = false;

            for (index, family) in (0..).zip(families) {
                let dedicated = |queue: native::pl_vulkan_queue| {
                    queue.count != 0 && queue.index == index
                };
                let enabled = index == vk.queue_graphics.index
                    || dedicated(vk.queue_compute)
                    || dedicated(vk.queue_transfer);
                let video = family
                    .queue_flags
                    .contains(vk::QueueFlags::VIDEO_DECODE_KHR);

                if !enabled && !video {
                    continue;
                }

                let Some(entry) = context.qf.get_mut(context.nb_qf as usize) else {
                    break;
                };
                *entry = ffi::AVVulkanDeviceQueueFamily {
                    idx: index as c_int,
                    num: 1,
                    flags: family.queue_flags,
                    ..Default::default()
                };
                context.nb_qf += 1;
                video_queue |= video;
            }

            if ffi::av_hwdevice_ctx_init(reference.raw()) < 0 {
                return None;
            }

            Some((reference, video_queue))
        }
    }
}

impl Drop for Vulkan {
    fn drop(&mut self) {
        // SAFETY: Every user of the device is gone, and libplacebo leaves an imported
        // device for its creator to destroy
        unsafe {
            native::pl_vulkan_destroy(&mut self.vk);

            if self.owned {
                self.api.destroy_device(None);
            }
        }
    }
}

/// The queue families to create, and which of them libplacebo uses for what
#[derive(Default)]
struct QueuePlan {
    families: Vec<vk::DeviceQueueCreateInfo<'static>>,
    graphics: native::pl_vulkan_queue,
    compute: native::pl_vulkan_queue,
    transfer: native::pl_vulkan_queue,
}

impl QueuePlan {
    /// Picks the graphics, async compute, async transfer, and video decode families
    fn new(families: &[vk::QueueFamilyProperties]) -> Option<Self> {
        let first = |wanted: vk::QueueFlags, unwanted: vk::QueueFlags| {
            families.iter().position(|family| {
                family.queue_flags.contains(wanted)
                    && !family.queue_flags.intersects(unwanted)
            })
        };
        let graphics = first(vk::QueueFlags::GRAPHICS, vk::QueueFlags::empty())?;
        let compute = first(vk::QueueFlags::COMPUTE, vk::QueueFlags::GRAPHICS);
        let transfer = first(
            vk::QueueFlags::TRANSFER,
            vk::QueueFlags::GRAPHICS | vk::QueueFlags::COMPUTE,
        );
        let mut plan = Self::default();

        // Playback renders on queue 0; wgpu takes queue 1 when the family has one
        let graphics_queues = if families[graphics].queue_count > 1 {
            2
        } else {
            1
        };
        plan.graphics = plan.add(graphics, graphics_queues);

        if let Some(compute) = compute {
            plan.compute = plan.add(compute, 1);
        }

        if let Some(transfer) = transfer {
            plan.transfer = plan.add(transfer, 1);
        }

        for (index, family) in families.iter().enumerate() {
            if family
                .queue_flags
                .contains(vk::QueueFlags::VIDEO_DECODE_KHR)
            {
                plan.add(index, 1);
            }
        }

        Some(plan)
    }

    /// Creates queues for a family unless it is already planned, and returns the one
    /// libplacebo may use
    fn add(&mut self, family: usize, queues: usize) -> native::pl_vulkan_queue {
        let family = family as u32;
        let planned = self
            .families
            .iter()
            .any(|planned| planned.queue_family_index == family);

        if !planned {
            self.families.push(
                vk::DeviceQueueCreateInfo::default()
                    .queue_family_index(family)
                    .queue_priorities(&QUEUE_PRIORITIES[..queues]),
            );
        }

        native::pl_vulkan_queue {
            index: family,
            count: 1,
        }
    }
}

/// Forwards libplacebo's messages to the application log
struct Log(native::pl_log);

impl Log {
    fn create() -> Self {
        let params = native::pl_log_params {
            log_cb: Some(log_message),
            log_level: native::PL_LOG_WARN,
            ..Default::default()
        };

        // SAFETY: Libplacebo copies the parameters, and a missing log is valid
        Self(unsafe { native::flowa_log_create(&params) })
    }
}

impl Drop for Log {
    fn drop(&mut self) {
        // SAFETY: Everything that reported to this log has been destroyed
        unsafe { native::pl_log_destroy(&mut self.0) };
    }
}

unsafe extern "C" fn log_message(
    _: *mut c_void,
    level: native::pl_log_level,
    message: *const c_char,
) {
    // SAFETY: Libplacebo retains the message until this callback returns
    let message = unsafe { CStr::from_ptr(message) }.to_string_lossy();

    match level {
        native::PL_LOG_FATAL | native::PL_LOG_ERR => {
            log::error!("Video GPU: {message}")
        }
        native::PL_LOG_WARN => log::warn!("Video GPU: {message}"),
        _ => log::debug!("Video GPU: {message}"),
    }
}

/// The DRM render node of the GPU, which is how VA-API finds the same device
///
/// # Safety
/// The request's instance and physical device must be valid
unsafe fn render_node(
    request: &DeviceRequest<'_>,
    extensions: &DeviceExtensions,
) -> Option<CString> {
    if !extensions.contains(ash::ext::physical_device_drm::NAME) {
        return None;
    }

    let mut drm = vk::PhysicalDeviceDrmPropertiesEXT::default();
    let mut properties = vk::PhysicalDeviceProperties2::default().push_next(&mut drm);

    // SAFETY: The caller provides a valid instance and physical device
    unsafe {
        request
            .instance
            .get_physical_device_properties2(request.physical_device, &mut properties);
    }

    if drm.has_render == vk::FALSE {
        return None;
    }

    CString::new(format!("/dev/dri/renderD{}", drm.render_minor)).ok()
}

fn name_pointers(names: &[&'static CStr]) -> Vec<*const c_char> {
    names.iter().map(|name| name.as_ptr()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn family(flags: vk::QueueFlags, queue_count: u32) -> vk::QueueFamilyProperties {
        vk::QueueFamilyProperties {
            queue_flags: flags,
            queue_count,
            ..Default::default()
        }
    }

    fn planned(plan: &QueuePlan) -> Vec<(u32, u32)> {
        plan.families
            .iter()
            .map(|family| (family.queue_family_index, family.queue_count))
            .collect()
    }

    #[test]
    fn dedicated_families_get_one_queue_and_graphics_gets_two() {
        let general = vk::QueueFlags::GRAPHICS
            | vk::QueueFlags::COMPUTE
            | vk::QueueFlags::TRANSFER;
        let families = [
            family(general, 16),
            family(vk::QueueFlags::TRANSFER, 2),
            family(vk::QueueFlags::COMPUTE | vk::QueueFlags::TRANSFER, 8),
            family(vk::QueueFlags::VIDEO_DECODE_KHR, 1),
        ];
        let plan = QueuePlan::new(&families).expect("a graphics family exists");

        assert_eq!(planned(&plan), [(0, 2), (2, 1), (1, 1), (3, 1)]);
        assert_eq!((plan.graphics.index, plan.graphics.count), (0, 1));
        assert_eq!((plan.compute.index, plan.compute.count), (2, 1));
        assert_eq!((plan.transfer.index, plan.transfer.count), (1, 1));
    }

    #[test]
    fn a_single_general_queue_is_shared_with_wgpu() {
        let general = vk::QueueFlags::GRAPHICS
            | vk::QueueFlags::COMPUTE
            | vk::QueueFlags::TRANSFER
            | vk::QueueFlags::VIDEO_DECODE_KHR;
        let plan =
            QueuePlan::new(&[family(general, 1)]).expect("a graphics family exists");

        assert_eq!(planned(&plan), [(0, 1)]);
        assert_eq!(plan.compute.count, 0);
        assert_eq!(plan.transfer.count, 0);
    }

    #[test]
    fn a_device_without_graphics_has_no_plan() {
        assert!(QueuePlan::new(&[family(vk::QueueFlags::COMPUTE, 4)]).is_none());
    }
}
