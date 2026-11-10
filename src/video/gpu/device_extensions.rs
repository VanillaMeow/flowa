//! Device extensions wanted by wgpu, libplacebo, and FFmpeg's hardware decoders

use std::{
    ffi::{CStr, c_char, c_int},
    slice,
};

use ash::{prelude::VkResult, vk};

use super::shared_device::DeviceRequest;
use crate::video::{ffmpeg as ffi, native_bindings as native};

/// Enabled device extension names as the pointer array that Vulkan, libplacebo, and
/// FFmpeg read
///
/// The names are static strings, or libplacebo's for as long as its device lives
#[derive(Clone, Default)]
pub(super) struct DeviceExtensions(Vec<*const c_char>);

impl DeviceExtensions {
    /// Enables the supported subset of wgpu, libplacebo, and FFmpeg decoding extensions
    ///
    /// # Safety
    /// The request's instance and physical device must be valid
    pub unsafe fn select(
        request: &DeviceRequest<'_>,
        decoding: &[&'static CStr],
    ) -> VkResult<Self> {
        // SAFETY: The caller provides a valid instance and physical device
        let available = unsafe {
            request
                .instance
                .enumerate_device_extension_properties(request.physical_device)
        }?;
        let candidates = request
            .extensions
            .iter()
            .copied()
            // The DRM render node identifies the GPU when opening VA-API
            .chain([ash::ext::physical_device_drm::NAME])
            .chain(recommended())
            .chain(decoding.iter().copied());
        let mut extensions = Self::default();

        for name in candidates {
            extensions.enable(&available, name);
        }

        Ok(extensions)
    }

    /// Adopts the list that libplacebo retains for a device it created
    ///
    /// # Safety
    /// The array must hold `count` NUL-terminated names that outlive this list
    pub unsafe fn adopt(names: *const *const c_char, count: c_int) -> Self {
        if names.is_null() {
            return Self::default();
        }

        // SAFETY: The caller provides `count` readable name pointers
        Self(unsafe { slice::from_raw_parts(names, count.max(0) as usize) }.to_vec())
    }

    fn enable(&mut self, available: &[vk::ExtensionProperties], name: &'static CStr) {
        let supported = available
            .iter()
            .any(|extension| extension.extension_name_as_c_str() == Ok(name));

        if supported && !self.contains(name) {
            self.0.push(name.as_ptr());
        }
    }

    pub fn contains(&self, name: &CStr) -> bool {
        self.names().any(|enabled| enabled == name)
    }

    pub fn names(&self) -> impl Iterator<Item = &CStr> {
        // SAFETY: Every entry is a NUL-terminated name that outlives this list
        self.0.iter().map(|&name| unsafe { CStr::from_ptr(name) })
    }

    pub fn as_ptr(&self) -> *const *const c_char {
        self.0.as_ptr()
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }
}

/// FFmpeg's optional device extensions, without the encoding ones
///
/// Encoding extensions make FFmpeg add incompatible encode usage to decoder images
pub(super) fn decoding() -> Vec<&'static CStr> {
    let mut count = 0;

    // SAFETY: FFmpeg returns an array it allocated, holding `count` static strings
    unsafe {
        let names = ffi::av_vk_get_optional_device_extensions(&mut count);
        if names.is_null() {
            return Vec::new();
        }

        let decoding = static_names(names, count)
            .filter(|name| !name.to_string_lossy().contains("video_encode"))
            .collect();
        ffi::av_free(names.cast());

        decoding
    }
}

/// DeviceExtensions libplacebo can make use of when the device offers them
fn recommended() -> Vec<&'static CStr> {
    let mut count = 0;

    // SAFETY: Libplacebo returns its static array of `count` static strings
    unsafe {
        let names = native::flowa_vulkan_recommended_extensions(&mut count);

        static_names(names, count).collect()
    }
}

/// # Safety
/// The array must hold `count` pointers to static NUL-terminated strings
unsafe fn static_names(
    names: *const *const c_char,
    count: c_int,
) -> impl Iterator<Item = &'static CStr> {
    // SAFETY: The caller provides `count` readable pointers to static strings
    unsafe { slice::from_raw_parts(names, count.max(0) as usize) }
        .iter()
        .map(|&name| unsafe { CStr::from_ptr(name) })
}
