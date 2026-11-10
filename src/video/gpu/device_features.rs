//! Vulkan feature structures enabled on the shared device

use std::{ffi::c_void, ptr, ptr::NonNull};

use ash::vk;

use super::{device_extensions::DeviceExtensions, shared_device::DeviceRequest};

/// Extension feature structures wgpu may request that Vulkan 1.1-1.3 core covers
const PROMOTED: &[vk::StructureType] = &[
    vk::StructureType::PHYSICAL_DEVICE_16BIT_STORAGE_FEATURES,
    vk::StructureType::PHYSICAL_DEVICE_MULTIVIEW_FEATURES,
    vk::StructureType::PHYSICAL_DEVICE_VARIABLE_POINTERS_FEATURES,
    vk::StructureType::PHYSICAL_DEVICE_PROTECTED_MEMORY_FEATURES,
    vk::StructureType::PHYSICAL_DEVICE_SAMPLER_YCBCR_CONVERSION_FEATURES,
    vk::StructureType::PHYSICAL_DEVICE_SHADER_DRAW_PARAMETERS_FEATURES,
    vk::StructureType::PHYSICAL_DEVICE_8BIT_STORAGE_FEATURES,
    vk::StructureType::PHYSICAL_DEVICE_SHADER_ATOMIC_INT64_FEATURES,
    vk::StructureType::PHYSICAL_DEVICE_SHADER_FLOAT16_INT8_FEATURES,
    vk::StructureType::PHYSICAL_DEVICE_DESCRIPTOR_INDEXING_FEATURES,
    vk::StructureType::PHYSICAL_DEVICE_SCALAR_BLOCK_LAYOUT_FEATURES,
    vk::StructureType::PHYSICAL_DEVICE_VULKAN_MEMORY_MODEL_FEATURES,
    vk::StructureType::PHYSICAL_DEVICE_IMAGELESS_FRAMEBUFFER_FEATURES,
    vk::StructureType::PHYSICAL_DEVICE_UNIFORM_BUFFER_STANDARD_LAYOUT_FEATURES,
    vk::StructureType::PHYSICAL_DEVICE_SHADER_SUBGROUP_EXTENDED_TYPES_FEATURES,
    vk::StructureType::PHYSICAL_DEVICE_SEPARATE_DEPTH_STENCIL_LAYOUTS_FEATURES,
    vk::StructureType::PHYSICAL_DEVICE_HOST_QUERY_RESET_FEATURES,
    vk::StructureType::PHYSICAL_DEVICE_TIMELINE_SEMAPHORE_FEATURES,
    vk::StructureType::PHYSICAL_DEVICE_BUFFER_DEVICE_ADDRESS_FEATURES,
    vk::StructureType::PHYSICAL_DEVICE_SHADER_TERMINATE_INVOCATION_FEATURES,
    vk::StructureType::PHYSICAL_DEVICE_SHADER_DEMOTE_TO_HELPER_INVOCATION_FEATURES,
    vk::StructureType::PHYSICAL_DEVICE_PRIVATE_DATA_FEATURES,
    vk::StructureType::PHYSICAL_DEVICE_PIPELINE_CREATION_CACHE_CONTROL_FEATURES,
    vk::StructureType::PHYSICAL_DEVICE_SYNCHRONIZATION_2_FEATURES,
    vk::StructureType::PHYSICAL_DEVICE_ZERO_INITIALIZE_WORKGROUP_MEMORY_FEATURES,
    vk::StructureType::PHYSICAL_DEVICE_IMAGE_ROBUSTNESS_FEATURES,
    vk::StructureType::PHYSICAL_DEVICE_SUBGROUP_SIZE_CONTROL_FEATURES,
    vk::StructureType::PHYSICAL_DEVICE_INLINE_UNIFORM_BLOCK_FEATURES,
    vk::StructureType::PHYSICAL_DEVICE_TEXTURE_COMPRESSION_ASTC_HDR_FEATURES,
    vk::StructureType::PHYSICAL_DEVICE_DYNAMIC_RENDERING_FEATURES,
    vk::StructureType::PHYSICAL_DEVICE_SHADER_INTEGER_DOT_PRODUCT_FEATURES,
    vk::StructureType::PHYSICAL_DEVICE_MAINTENANCE_4_FEATURES,
];

const ROBUSTNESS_2: vk::StructureType =
    vk::StructureType::PHYSICAL_DEVICE_ROBUSTNESS_2_FEATURES_EXT;

// Ash's Vulkan 1.3.281 definitions predate these extensions
const VIDEO_MAINTENANCE_2: &std::ffi::CStr = c"VK_KHR_video_maintenance2";
const VIDEO_MAINTENANCE_2_FEATURES: vk::StructureType =
    vk::StructureType::from_raw(1_000_586_000);
const VIDEO_DECODE_VP9: &std::ffi::CStr = c"VK_KHR_video_decode_vp9";
const VIDEO_DECODE_VP9_FEATURES: vk::StructureType =
    vk::StructureType::from_raw(1_000_514_000);
const SWAPCHAIN_MAINTENANCE_1_KHR: &std::ffi::CStr = c"VK_KHR_swapchain_maintenance1";

/// The layout shared by feature structures that hold a single flag
#[repr(C)]
struct SingleFeature {
    s_type: vk::StructureType,
    p_next: *mut c_void,
    enabled: vk::Bool32,
}

impl SingleFeature {
    fn new(s_type: vk::StructureType) -> Self {
        Self {
            s_type,
            p_next: ptr::null_mut(),
            enabled: vk::FALSE,
        }
    }
}

struct Features {
    core: vk::PhysicalDeviceFeatures2<'static>,
    v11: vk::PhysicalDeviceVulkan11Features<'static>,
    v12: vk::PhysicalDeviceVulkan12Features<'static>,
    v13: vk::PhysicalDeviceVulkan13Features<'static>,
    video_maintenance1: vk::PhysicalDeviceVideoMaintenance1FeaturesKHR<'static>,
    video_maintenance2: SingleFeature,
    vp9: SingleFeature,
    swapchain_maintenance1: vk::PhysicalDeviceSwapchainMaintenance1FeaturesEXT<'static>,
    host_image_copy: vk::PhysicalDeviceHostImageCopyFeaturesEXT<'static>,
    robustness2: vk::PhysicalDeviceRobustness2FeaturesEXT<'static>,
}

impl Features {
    fn new() -> Self {
        Self {
            core: Default::default(),
            v11: Default::default(),
            v12: Default::default(),
            v13: Default::default(),
            video_maintenance1: Default::default(),
            video_maintenance2: SingleFeature::new(VIDEO_MAINTENANCE_2_FEATURES),
            vp9: SingleFeature::new(VIDEO_DECODE_VP9_FEATURES),
            swapchain_maintenance1: Default::default(),
            host_image_copy: Default::default(),
            robustness2: Default::default(),
        }
    }
}

/// Linked feature structures for a device that flowa creates, kept alive with it
///
/// The structures link to each other, so they stay behind one raw allocation
pub(super) struct FeatureChain(NonNull<Features>);

impl FeatureChain {
    /// Enables every supported core and extension feature, plus wgpu's robustness request
    ///
    /// # Safety
    /// The request's instance, physical device, and feature chain must be valid
    pub unsafe fn query(
        request: &DeviceRequest<'_>,
        extensions: &DeviceExtensions,
    ) -> Self {
        let features = Box::into_raw(Box::new(Features::new()));
        let swapchain_maintenance = extensions
            .contains(ash::ext::swapchain_maintenance1::NAME)
            || extensions.contains(SWAPCHAIN_MAINTENANCE_1_KHR);

        // SAFETY: The allocation is reached only through this pointer, so the links
        // between its structures stay valid until the chain is dropped
        unsafe {
            let mut tail = (&raw mut (*features).core).cast();
            append(&mut tail, &raw mut (*features).v11);
            append(&mut tail, &raw mut (*features).v12);
            append(&mut tail, &raw mut (*features).v13);

            // Extension structures may only be chained when their extension is enabled
            if extensions.contains(ash::khr::video_maintenance1::NAME) {
                append(&mut tail, &raw mut (*features).video_maintenance1);
            }
            if extensions.contains(VIDEO_MAINTENANCE_2) {
                append(&mut tail, &raw mut (*features).video_maintenance2);
            }
            if extensions.contains(VIDEO_DECODE_VP9) {
                append(&mut tail, &raw mut (*features).vp9);
            }
            if swapchain_maintenance {
                append(&mut tail, &raw mut (*features).swapchain_maintenance1);
            }
            if extensions.contains(ash::ext::host_image_copy::NAME) {
                append(&mut tail, &raw mut (*features).host_image_copy);
            }

            request.instance.get_physical_device_features2(
                request.physical_device,
                &mut (*features).core,
            );

            // Copy wgpu's choice so robust access costs nothing it did not ask for
            let robustness = chained(request.features.p_next)
                .find(|&structure| (*structure).s_type == ROBUSTNESS_2);

            if let Some(requested) = robustness {
                (*features).robustness2 = *requested.cast();
                (*features).robustness2.p_next = ptr::null_mut();
                append(&mut tail, &raw mut (*features).robustness2);
            }

            Self(NonNull::new_unchecked(features))
        }
    }

    /// The head of the chain, valid for as long as this owner
    pub fn core(&self) -> *const vk::PhysicalDeviceFeatures2<'static> {
        // SAFETY: The allocation lives until this owner is dropped
        unsafe { &raw const (*self.0.as_ptr()).core }
    }

    /// Replaces the extension structures with a caller's chain, which must outlive
    /// every later read of this one
    pub fn replace_extensions(&mut self, chain: *mut c_void) {
        // SAFETY: The allocation lives until this owner is dropped
        unsafe { (*self.0.as_ptr()).v13.p_next = chain };
    }
}

impl Drop for FeatureChain {
    fn drop(&mut self) {
        // SAFETY: The pointer came from a box that only this owner frees
        drop(unsafe { Box::from_raw(self.0.as_ptr()) });
    }
}

/// Whether every structure wgpu chains can be expressed by the features enabled here
///
/// # Safety
/// Every link of the chain must point to a live Vulkan structure
pub(super) unsafe fn covers(requested: &vk::PhysicalDeviceFeatures2<'_>) -> bool {
    // SAFETY: The caller provides a valid chain
    unsafe { chained(requested.p_next) }.all(|structure| {
        let kind = unsafe { (*structure).s_type };

        PROMOTED.contains(&kind) || kind == ROBUSTNESS_2
    })
}

/// # Safety
/// The tail and the structure must be live Vulkan structures
unsafe fn append<T>(tail: &mut *mut vk::BaseOutStructure<'static>, structure: *mut T) {
    // SAFETY: Every Vulkan structure begins with the base structure's fields
    unsafe { (**tail).p_next = structure.cast() };
    *tail = structure.cast();
}

/// Walks the structures linked after a Vulkan structure
///
/// # Safety
/// Every link must point to a live Vulkan structure for as long as the iterator is used
unsafe fn chained(
    first: *const c_void,
) -> impl Iterator<Item = *const vk::BaseInStructure<'static>> {
    let mut next = first.cast::<vk::BaseInStructure<'static>>();

    std::iter::from_fn(move || {
        let current = next;
        if current.is_null() {
            return None;
        }

        // SAFETY: The caller keeps every linked structure alive
        next = unsafe { (*current).p_next };

        Some(current)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn promoted_and_robustness_requests_are_covered() {
        let mut robustness = vk::PhysicalDeviceRobustness2FeaturesEXT::default();
        let mut timeline = vk::PhysicalDeviceTimelineSemaphoreFeatures::default();
        let requested = vk::PhysicalDeviceFeatures2::default()
            .push_next(&mut robustness)
            .push_next(&mut timeline);

        // SAFETY: The chain borrows the structures above
        assert!(unsafe { covers(&requested) });
        assert!(unsafe { covers(&vk::PhysicalDeviceFeatures2::default()) });
    }

    #[test]
    fn unknown_extension_requests_are_not_covered() {
        let mut timeline = vk::PhysicalDeviceTimelineSemaphoreFeatures::default();
        let mut mesh = vk::PhysicalDeviceMeshShaderFeaturesEXT::default();
        let requested = vk::PhysicalDeviceFeatures2::default()
            .push_next(&mut timeline)
            .push_next(&mut mesh);

        // SAFETY: The chain borrows the structures above
        assert!(!unsafe { covers(&requested) });
    }
}
