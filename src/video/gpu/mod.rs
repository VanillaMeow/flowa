//! The Vulkan device shared by wgpu, libplacebo, and FFmpeg's hardware decoders

use std::{
    collections::HashMap,
    ffi::CStr,
    sync::{Arc, Mutex, OnceLock, Weak, atomic::AtomicBool},
};

use anyhow::{Context, Result};
use ash::vk::{self, Handle};
use eframe::{
    egui_wgpu::{WgpuSetup, WgpuSetupExisting},
    wgpu::{self, hal},
};

mod device_extensions;
mod device_features;
mod shared_device;

use shared_device::DeviceRequest;
pub(super) use shared_device::SharedDevice;

type Registry = Mutex<HashMap<u64, Weak<NativeGpu>>>;
static DEVICES: OnceLock<Registry> = OnceLock::new();

/// Retains the native device until wgpu and every player release them
pub(super) struct NativeGpu {
    pub background_playback: bool,
    pub playback_claimed: AtomicBool,
    device: SharedDevice,
    // The adapter retains the Vulkan instance borrowed by the native device
    _adapter: wgpu::Adapter,
}

// SAFETY: The device does not change after creation. One playback owner uses queue
// zero; wgpu uses a separate queue for background playback
unsafe impl Send for NativeGpu {}
unsafe impl Sync for NativeGpu {}

impl NativeGpu {
    /// The Vulkan device with libplacebo's and FFmpeg's views of it
    pub fn device(&self) -> &SharedDevice {
        &self.device
    }
}

pub fn setup() -> Result<WgpuSetup> {
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

fn create_device(adapter: &wgpu::Adapter) -> Result<(wgpu::Device, wgpu::Queue)> {
    // SAFETY: The adapter guard retains the instance throughout native device creation
    let native = unsafe { adapter.as_hal::<hal::api::Vulkan>() }
        .context("Vulkan adapter unavailable")?;
    let instance = native.shared_instance();
    let extensions = native.required_device_extensions(wgpu::Features::empty());
    let mut features =
        native.physical_device_features(&extensions, wgpu::Features::empty());
    let description = features.add_to_device_create(vk::DeviceCreateInfo::default());
    let requested = vk::PhysicalDeviceFeatures2 {
        features: unsafe { *description.p_enabled_features },
        p_next: description.p_next.cast_mut(),
        ..Default::default()
    };
    let request = DeviceRequest {
        instance: instance.raw_instance(),
        get_proc_addr: instance.entry().static_fn().get_instance_proc_addr,
        physical_device: native.raw_physical_device(),
        extensions: &extensions,
        features: &requested,
    };

    // SAFETY: The request describes wgpu's live adapter, which the owner below retains
    let device = unsafe { SharedDevice::create(&request) }?;
    let raw = device.api().clone();
    let graphics_family = device.graphics_family();
    let graphics_queue = device.wgpu_queue();

    // SAFETY: Extension strings belong to the native owner retained by the device's drop callback
    let enabled: Vec<&'static CStr> = device
        .extensions()
        .names()
        .map(|name| unsafe { CStr::from_ptr(name.as_ptr()) })
        .collect();
    let owner = Arc::new(NativeGpu {
        background_playback: graphics_queue != 0,
        playback_claimed: AtomicBool::new(false),
        device,
        _adapter: adapter.clone(),
    });
    let descriptor = wgpu::DeviceDescriptor {
        label: Some("flowa-ffmpeg-device"),
        required_limits: wgpu::Limits::default().using_resolution(adapter.limits()),
        ..Default::default()
    };
    let key = raw.handle().as_raw();
    let retained = Arc::clone(&owner);
    let opened = unsafe {
        native.device_from_raw(
            raw,
            Some(Box::new(move || drop(retained))),
            &enabled,
            descriptor.required_features,
            &descriptor.required_limits,
            &descriptor.memory_hints,
            graphics_family,
            graphics_queue,
        )
    }?;
    drop(native);

    DEVICES
        .get_or_init(Registry::default)
        .lock()
        .map_err(|_| anyhow::anyhow!("Video device registry is poisoned"))?
        .insert(key, Arc::downgrade(&owner));

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
