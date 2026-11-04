//! Creates the libmpv render context from the device already owned by wgpu

use std::ptr;

use anyhow::{Context, Result};
use ash::vk;
use eframe::wgpu::hal;

use super::{Player, mpv};
use mpv::{RenderParam, VulkanInit};

impl Player {
    pub(super) fn create_render_context(&mut self) -> Result<()> {
        // SAFETY: These guards borrow live graphics handles for the duration of initialization
        let device = unsafe { self.graphics.device.as_hal::<hal::api::Vulkan>() }
            .context("Vulkan device unavailable")?;
        let adapter = unsafe { self.graphics.adapter.as_hal::<hal::api::Vulkan>() }
            .context("Vulkan adapter unavailable")?;
        let instance = device.shared_instance();
        let extensions = device.enabled_device_extensions();
        let pointers: Vec<_> = extensions
            .iter()
            .map(|extension| extension.as_ptr())
            .collect();
        let mut features = adapter
            .physical_device_features(extensions, self.graphics.device.features());
        let mut reset =
            vk::PhysicalDeviceHostQueryResetFeatures::default().host_query_reset(true);
        let mut sync = vk::PhysicalDeviceSynchronization2Features::default()
            .synchronization2(true);
        let info = features
            .add_to_device_create(vk::DeviceCreateInfo::default())
            .push_next(&mut reset)
            .push_next(&mut sync);
        let description = vk::PhysicalDeviceFeatures2 {
            features: unsafe { *info.p_enabled_features },
            p_next: info.p_next.cast_mut(),
            ..Default::default()
        };

        let mut init = VulkanInit {
            instance: instance.raw_instance().handle(),
            physical_device: device.raw_physical_device(),
            device: device.raw_device().handle(),
            get_proc_addr: instance.entry().static_fn().get_instance_proc_addr,
            queue_family: device.queue_family_index(),
            extensions: pointers.as_ptr(),
            num_extensions: pointers.len() as i32,
            features: ptr::from_ref(&description).cast(),
        };
        let mut api_type = *b"flowa-vulkan\0";
        let mut advanced = 1_i32;
        let mut params = [
            RenderParam::new(mpv::RENDER_PARAM_API_TYPE, &mut api_type),
            RenderParam::new(mpv::RENDER_PARAM_VULKAN_INIT, &mut init),
            RenderParam::new(mpv::RENDER_PARAM_ADVANCED_CONTROL, &mut advanced),
            RenderParam::end(),
        ];

        // SAFETY: The private ABI is checked before loading and libplacebo copies the descriptors
        self.api.check(unsafe {
            (self.api.render_context_create)(
                &mut self.render,
                self.handle,
                params.as_mut_ptr(),
            )
        })
    }
}
