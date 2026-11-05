//! Shares a libplacebo-created Vulkan device with wgpu and FFmpeg

use std::{
    collections::HashMap,
    ffi::{CStr, c_char, c_int, c_void},
    ptr::NonNull,
    sync::{Arc, Mutex, OnceLock, Weak},
};

use anyhow::{Context, Result};
use ash::vk::{self, Handle};
use eframe::{
    egui_wgpu::{WgpuSetup, WgpuSetupExisting},
    wgpu::{self, hal},
};

use super::native::{self, Api, GpuInfo, GpuInit};

type Registry = Mutex<HashMap<u64, Weak<NativeGpu>>>;
static DEVICES: OnceLock<Registry> = OnceLock::new();

/// Retains the native device until wgpu and every player release them
pub(super) struct NativeGpu {
    pub api: Arc<Api>,
    pub handle: NonNull<c_void>,
    // The adapter retains the Vulkan instance borrowed by the native device
    _adapter: wgpu::Adapter,
}

// SAFETY: Players submit on the UI thread; the final owner may destroy an idle device on any thread
unsafe impl Send for NativeGpu {}
unsafe impl Sync for NativeGpu {}

impl Drop for NativeGpu {
    fn drop(&mut self) {
        // SAFETY: All dependent renderers and frames have released this final device owner
        unsafe { (self.api.gpu_destroy)(self.handle.as_ptr()) };
    }
}

pub(crate) fn setup() -> Result<WgpuSetup> {
    let instance = create_instance()?;
    let adapter =
        pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            ..Default::default()
        }))?;
    let (device, queue) = create_device(&adapter)?;

    Ok(WgpuSetup::Existing(WgpuSetupExisting {
        instance,
        adapter,
        device,
        queue,
    }))
}

fn create_instance() -> Result<wgpu::Instance> {
    let options = wgpu::InstanceDescriptor::new_without_display_handle_from_env();
    let descriptor = hal::InstanceDescriptor {
        name: "flowa",
        flags: options.flags,
        memory_budget_thresholds: options.memory_budget_thresholds,
        backend_options: options.backend_options,
        telemetry: None,
        display: None,
    };

    // SAFETY: The callback only adds advertised extensions and preserves wgpu's requests
    let instance = unsafe {
        hal::vulkan::Instance::init_with_callback(
            &descriptor,
            Some(Box::new(enable_surface_extensions)),
        )
    }?;

    // SAFETY: This is an owned Vulkan instance initialized through wgpu-hal
    Ok(unsafe { wgpu::Instance::from_hal::<hal::api::Vulkan>(instance) })
}

fn enable_surface_extensions(
    args: hal::vulkan::CreateInstanceCallbackArgs<'_, '_, '_>,
) {
    // Libplacebo enables swapchain maintenance when the device advertises it
    let available = unsafe { args.entry.enumerate_instance_extension_properties(None) }
        .unwrap_or_default();
    let extensions = [
        c"VK_KHR_get_surface_capabilities2",
        c"VK_EXT_surface_maintenance1",
        c"VK_KHR_surface_maintenance1",
    ];

    for extension in extensions {
        let supported = available
            .iter()
            .any(|property| property.extension_name_as_c_str() == Ok(extension));

        if supported && !args.extensions.contains(&extension) {
            args.extensions.push(extension);
        }
    }
}

unsafe extern "C" fn log_message(level: c_int, message: *const c_char) {
    // SAFETY: Libplacebo retains the message until this callback returns
    let message = unsafe { CStr::from_ptr(message) }.to_string_lossy();

    match level {
        1 | 2 => log::error!("Video GPU: {message}"),
        3 => log::warn!("Video GPU: {message}"),
        _ => log::debug!("Video GPU: {message}"),
    }
}

fn create_device(adapter: &wgpu::Adapter) -> Result<(wgpu::Device, wgpu::Queue)> {
    let api = Api::load()?;

    // SAFETY: The adapter guard retains the instance throughout native device creation
    let native = unsafe { adapter.as_hal::<hal::api::Vulkan>() }
        .context("Vulkan adapter unavailable")?;
    let instance = native.shared_instance();
    let extensions = native.required_device_extensions(wgpu::Features::empty());
    let pointers: Vec<_> = extensions.iter().map(|name| name.as_ptr()).collect();
    let mut features =
        native.physical_device_features(&extensions, wgpu::Features::empty());
    let description = features.add_to_device_create(vk::DeviceCreateInfo::default());
    let requested = vk::PhysicalDeviceFeatures2 {
        features: unsafe { *description.p_enabled_features },
        p_next: description.p_next.cast_mut(),
        ..Default::default()
    };
    let init = GpuInit {
        instance: instance.raw_instance().handle(),
        physical_device: native.raw_physical_device(),
        get_proc_addr: instance.entry().static_fn().get_instance_proc_addr,
        extensions: pointers.as_ptr(),
        num_extensions: pointers.len() as c_int,
        features: std::ptr::from_ref(&requested).cast(),
        log: Some(log_message),
    };
    let mut error = [0; 768];

    // SAFETY: The native side copies descriptors and creates an owned Vulkan device
    let handle = unsafe { (api.gpu_create)(&init, error.as_mut_ptr(), error.len()) };
    let handle = NonNull::new(handle).with_context(|| native::string(&error))?;
    let owner = Arc::new(NativeGpu {
        api,
        handle,
        _adapter: adapter.clone(),
    });
    let mut info = std::mem::MaybeUninit::<GpuInfo>::uninit();
    unsafe { (owner.api.gpu_get_info)(owner.handle.as_ptr(), info.as_mut_ptr()) };
    let info = unsafe { info.assume_init() };

    // SAFETY: Extension strings belong to the native owner retained by the device's drop callback
    let enabled: Vec<&'static CStr> = unsafe {
        std::slice::from_raw_parts(info.extensions, info.num_extensions as usize)
            .iter()
            .map(|&name| CStr::from_ptr(name))
            .collect()
    };
    let descriptor = wgpu::DeviceDescriptor {
        label: Some("flowa-ffmpeg-device"),
        required_limits: wgpu::Limits::default().using_resolution(adapter.limits()),
        ..Default::default()
    };
    let raw =
        unsafe { ash::Device::load(instance.raw_instance().fp_v1_0(), info.device) };
    let retained = Arc::clone(&owner);
    let opened = unsafe {
        native.device_from_raw(
            raw,
            Some(Box::new(move || drop(retained))),
            &enabled,
            descriptor.required_features,
            &descriptor.required_limits,
            &descriptor.memory_hints,
            info.graphics_family,
            0,
        )
    }?;
    drop(native);

    DEVICES
        .get_or_init(Registry::default)
        .lock()
        .map_err(|_| anyhow::anyhow!("Video device registry is poisoned"))?
        .insert(info.device.as_raw(), Arc::downgrade(&owner));

    // SAFETY: The raw device satisfies this adapter's descriptor and has a retained native owner
    Ok(unsafe {
        adapter.create_device_from_hal::<hal::api::Vulkan>(opened, &descriptor)
    }?)
}

pub(super) fn for_device(device: &wgpu::Device) -> Result<Arc<NativeGpu>> {
    // SAFETY: The guard retains the raw device while its identity is looked up
    let native = unsafe { device.as_hal::<hal::api::Vulkan>() }
        .context("Video requires the Vulkan renderer")?;
    let key = native.raw_device().handle().as_raw();
    let mut devices = DEVICES
        .get_or_init(Registry::default)
        .lock()
        .map_err(|_| anyhow::anyhow!("Video device registry is poisoned"))?;
    devices.retain(|_, device| device.strong_count() != 0);
    devices
        .get(&key)
        .and_then(Weak::upgrade)
        .context("The renderer was not created with FFmpeg video support")
}
