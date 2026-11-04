//! Vulkan device creation with the features needed by both wgpu and libplacebo

use std::{ffi::CStr, sync::Arc};

use anyhow::{Context, Result, ensure};
use ash::vk;
use eframe::{
    egui_wgpu::{WgpuSetup, WgpuSetupExisting},
    wgpu::{self, hal},
};

use super::mpv;

/// Creates a shared Vulkan device only when the versioned playback library exists
pub(crate) fn setup() -> Result<Option<WgpuSetup>> {
    if mpv::library_path().is_none() {
        return Ok(None);
    }

    let _api = mpv::Api::load()?;
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::VULKAN,
        ..wgpu::InstanceDescriptor::new_without_display_handle_from_env()
    });
    let adapter =
        pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            ..Default::default()
        }))?;
    let (device, queue) = create_device(&adapter)?;

    Ok(Some(WgpuSetup::Existing(WgpuSetupExisting {
        instance,
        adapter,
        device,
        queue,
    })))
}

fn create_device(adapter: &wgpu::Adapter) -> Result<(wgpu::Device, wgpu::Queue)> {
    // SAFETY: Handles stay owned by wgpu and are used only during device creation
    let native = unsafe { adapter.as_hal::<hal::api::Vulkan>() }
        .context("Vulkan adapter unavailable")?;
    let instance = native.shared_instance().raw_instance();
    let physical = native.raw_physical_device();
    let families =
        unsafe { instance.get_physical_device_queue_family_properties(physical) };
    let family = families
        .iter()
        .position(|family| family.queue_flags.contains(vk::QueueFlags::GRAPHICS))
        .context("No Vulkan graphics queue")? as u32;

    let mut supported_reset = vk::PhysicalDeviceHostQueryResetFeatures::default();
    let mut supported_sync = vk::PhysicalDeviceSynchronization2Features::default();
    let mut supported = vk::PhysicalDeviceFeatures2::default()
        .push_next(&mut supported_reset)
        .push_next(&mut supported_sync);
    unsafe { instance.get_physical_device_features2(physical, &mut supported) };
    ensure!(
        supported_reset.host_query_reset != 0,
        "Video requires Vulkan host query reset support"
    );
    ensure!(
        supported_sync.synchronization2 != 0,
        "Video requires Vulkan synchronization2 support"
    );

    let extensions = device_extensions(&native)?;
    let pointers: Vec<_> = extensions
        .iter()
        .map(|extension| extension.as_ptr())
        .collect();
    let descriptor = wgpu::DeviceDescriptor {
        label: Some("flowa-video-device"),
        required_limits: wgpu::Limits::default().using_resolution(adapter.limits()),
        ..Default::default()
    };
    let mut features =
        native.physical_device_features(&extensions, descriptor.required_features);
    let queues = [vk::DeviceQueueCreateInfo::default()
        .queue_family_index(family)
        .queue_priorities(&[1.])];
    let mut reset =
        vk::PhysicalDeviceHostQueryResetFeatures::default().host_query_reset(true);
    let mut sync =
        vk::PhysicalDeviceSynchronization2Features::default().synchronization2(true);
    let info = features
        .add_to_device_create(
            vk::DeviceCreateInfo::default()
                .queue_create_infos(&queues)
                .enabled_extension_names(&pointers),
        )
        .push_next(&mut reset)
        .push_next(&mut sync);

    // SAFETY: The device includes wgpu's required features plus the native rendering features
    let owner = Arc::new(OwnedDevice(unsafe {
        instance.create_device(physical, &info, None)
    }?));
    let retained = Arc::clone(&owner);
    let opened = unsafe {
        native.device_from_raw(
            owner.0.clone(),
            Some(Box::new(move || drop(retained))),
            &extensions,
            descriptor.required_features,
            &descriptor.required_limits,
            &descriptor.memory_hints,
            family,
            0,
        )
    }?;
    drop(native);

    // SAFETY: The native device was built from this adapter with the matching descriptor
    Ok(unsafe {
        adapter.create_device_from_hal::<hal::api::Vulkan>(opened, &descriptor)
    }?)
}

/// Destroys a custom-created device after wgpu releases its final resource
struct OwnedDevice(ash::Device);

impl Drop for OwnedDevice {
    fn drop(&mut self) {
        // SAFETY: The final owner is released only after wgpu has destroyed its device resources
        unsafe { self.0.destroy_device(None) };
    }
}

pub(super) fn device_extensions(
    adapter: &hal::vulkan::Adapter,
) -> Result<Vec<&'static CStr>> {
    let mut extensions = adapter.required_device_extensions(wgpu::Features::empty());
    let instance = adapter.shared_instance().raw_instance();
    let available = unsafe {
        instance.enumerate_device_extension_properties(adapter.raw_physical_device())
    }?;

    for extension in [
        ash::khr::external_memory_fd::NAME,
        ash::khr::external_semaphore_fd::NAME,
        ash::khr::synchronization2::NAME,
    ] {
        let supported = available.iter().any(|property| {
            // SAFETY: Vulkan extension names are terminated within the fixed array
            unsafe { CStr::from_ptr(property.extension_name.as_ptr()) == extension }
        });
        if supported && !extensions.contains(&extension) {
            extensions.push(extension);
        }
    }

    Ok(extensions)
}
