//! Loaded libmpv functions and the private Vulkan embedding ABI

use std::{
    ffi::{CStr, CString, c_char, c_int, c_void},
    sync::Arc,
};

#[cfg(not(feature = "static-video"))]
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail, ensure};
use ash::vk;

// Public enum values are pinned by mpv's stable C client and rendering APIs
const ERROR_OPTION_NOT_FOUND: c_int = -5;
pub(super) const FORMAT_STRING: c_int = 1;
pub(super) const FORMAT_FLAG: c_int = 3;
pub(super) const FORMAT_INT64: c_int = 4;
pub(super) const FORMAT_DOUBLE: c_int = 5;
pub(super) const EVENT_NONE: c_int = 0;
pub(super) const EVENT_END_FILE: c_int = 7;
pub(super) const EVENT_FILE_LOADED: c_int = 8;
pub(super) const EVENT_PROPERTY_CHANGE: c_int = 22;
pub(super) const END_FILE_ERROR: c_int = 4;

pub(super) const RENDER_PARAM_API_TYPE: c_int = 1;
pub(super) const RENDER_PARAM_ADVANCED_CONTROL: c_int = 10;
pub(super) const RENDER_PARAM_BLOCK_FOR_TARGET_TIME: c_int = 12;
pub(super) const RENDER_UPDATE_FRAME: u64 = 1;

// Private values must match native/mpv/flowa_vk.h and its exported ABI version
const VULKAN_ABI_VERSION: u32 = 1;
pub(super) const RENDER_PARAM_VULKAN_INIT: c_int = 1000;
pub(super) const RENDER_PARAM_VULKAN_TARGET: c_int = 1001;

#[repr(C)]
/// One entry in libmpv's sentinel-terminated rendering parameter array
pub(super) struct RenderParam {
    pub kind: c_int,
    pub data: *mut c_void,
}

impl RenderParam {
    pub fn new<T>(kind: c_int, value: &mut T) -> Self {
        Self {
            kind,
            data: (value as *mut T).cast(),
        }
    }

    pub fn end() -> Self {
        Self {
            kind: 0,
            data: std::ptr::null_mut(),
        }
    }
}

#[repr(C)]
/// Borrowed device description copied by libplacebo during context creation
pub(super) struct VulkanInit {
    pub instance: vk::Instance,
    pub physical_device: vk::PhysicalDevice,
    pub device: vk::Device,
    pub get_proc_addr: vk::PFN_vkGetInstanceProcAddr,
    pub queue_family: u32,
    pub extensions: *const *const c_char,
    pub num_extensions: c_int,
    pub features: *const vk::PhysicalDeviceFeatures2<'static>,
}

#[repr(C)]
/// One GPU target and the timeline values that transfer its ownership
pub(super) struct VulkanTarget {
    pub image: vk::Image,
    pub width: u32,
    pub height: u32,
    pub slot: u32,
    pub semaphore: vk::Semaphore,
    pub wait_value: u64,
    pub signal_value: u64,
}

#[repr(C)]
pub(super) struct Event {
    pub id: c_int,
    pub error: c_int,
    pub userdata: u64,
    pub data: *mut c_void,
}

#[repr(C)]
pub(super) struct Property {
    pub name: *const c_char,
    pub format: c_int,
    pub data: *mut c_void,
}

#[repr(C)]
pub(super) struct EndFile {
    pub reason: c_int,
    pub error: c_int,
}

type Handle = *mut c_void;
type Callback = Option<unsafe extern "C" fn(*mut c_void)>;

/// Typed symbols from the pinned libmpv API
///
/// Dynamic builds retain the library for the lifetime of every function pointer
/// Portable builds link the same entry points directly into the executable
pub(super) struct Api {
    pub create: unsafe extern "C" fn() -> Handle,
    pub initialize: unsafe extern "C" fn(Handle) -> c_int,
    pub terminate_destroy: unsafe extern "C" fn(Handle),
    pub set_option_string:
        unsafe extern "C" fn(Handle, *const c_char, *const c_char) -> c_int,
    pub command_async: unsafe extern "C" fn(Handle, u64, *const *const c_char) -> c_int,
    pub observe_property:
        unsafe extern "C" fn(Handle, u64, *const c_char, c_int) -> c_int,
    pub wait_event: unsafe extern "C" fn(Handle, f64) -> *const Event,
    pub set_wakeup_callback: unsafe extern "C" fn(Handle, Callback, *mut c_void),
    pub error_string: unsafe extern "C" fn(c_int) -> *const c_char,
    pub render_context_create:
        unsafe extern "C" fn(*mut Handle, Handle, *mut RenderParam) -> c_int,
    pub render_context_free: unsafe extern "C" fn(Handle),
    pub render_context_set_update_callback:
        unsafe extern "C" fn(Handle, Callback, *mut c_void),
    pub render_context_update: unsafe extern "C" fn(Handle) -> u64,
    pub render_context_render: unsafe extern "C" fn(Handle, *mut RenderParam) -> c_int,
    pub render_context_report_swap: unsafe extern "C" fn(Handle),
    #[cfg(not(feature = "static-video"))]
    _library: libloading::Library,
}

impl Api {
    /// Checks the private Vulkan ABI before loading any engine entry points
    #[cfg(not(feature = "static-video"))]
    pub fn load() -> Result<Arc<Self>> {
        let path = library_path()
            .context("Build the video backend with packaging/linux/build-video.sh")?;

        // SAFETY: The library is retained for the lifetime of every loaded function
        let library = unsafe { libloading::Library::new(&path) }
            .with_context(|| format!("Cannot load {}", path.display()))?;

        // SAFETY: Only our versioned backend exports this zero-argument ABI query
        let version = unsafe {
            library
                .get::<unsafe extern "C" fn() -> u32>(b"flowa_mpv_vulkan_abi_version\0")
        }
        .context("This libmpv does not include Flowa's Vulkan backend")?;
        ensure!(
            unsafe { version() } == VULKAN_ABI_VERSION,
            "Unsupported Flowa Vulkan backend ABI"
        );

        // SAFETY: These signatures match the public API of the pinned mpv version
        let api = unsafe {
            Self {
                create: *library.get(b"mpv_create\0")?,
                initialize: *library.get(b"mpv_initialize\0")?,
                terminate_destroy: *library.get(b"mpv_terminate_destroy\0")?,
                set_option_string: *library.get(b"mpv_set_option_string\0")?,
                command_async: *library.get(b"mpv_command_async\0")?,
                observe_property: *library.get(b"mpv_observe_property\0")?,
                wait_event: *library.get(b"mpv_wait_event\0")?,
                set_wakeup_callback: *library.get(b"mpv_set_wakeup_callback\0")?,
                error_string: *library.get(b"mpv_error_string\0")?,
                render_context_create: *library.get(b"mpv_render_context_create\0")?,
                render_context_free: *library.get(b"mpv_render_context_free\0")?,
                render_context_set_update_callback: *library
                    .get(b"mpv_render_context_set_update_callback\0")?,
                render_context_update: *library.get(b"mpv_render_context_update\0")?,
                render_context_render: *library.get(b"mpv_render_context_render\0")?,
                render_context_report_swap: *library
                    .get(b"mpv_render_context_report_swap\0")?,
                _library: library,
            }
        };

        Ok(Arc::new(api))
    }
}

#[cfg(feature = "static-video")]
mod linked {
    use super::*;

    // These signatures match the same pinned client API as the dynamic loader
    unsafe extern "C" {
        pub fn flowa_mpv_vulkan_abi_version() -> u32;
        pub fn mpv_create() -> Handle;
        pub fn mpv_initialize(handle: Handle) -> c_int;
        pub fn mpv_terminate_destroy(handle: Handle);
        pub fn mpv_set_option_string(
            handle: Handle,
            name: *const c_char,
            value: *const c_char,
        ) -> c_int;
        pub fn mpv_command_async(
            handle: Handle,
            userdata: u64,
            arguments: *const *const c_char,
        ) -> c_int;
        pub fn mpv_observe_property(
            handle: Handle,
            userdata: u64,
            name: *const c_char,
            format: c_int,
        ) -> c_int;
        pub fn mpv_wait_event(handle: Handle, timeout: f64) -> *const Event;
        pub fn mpv_set_wakeup_callback(
            handle: Handle,
            callback: Callback,
            data: *mut c_void,
        );
        pub fn mpv_error_string(error: c_int) -> *const c_char;
        pub fn mpv_render_context_create(
            context: *mut Handle,
            handle: Handle,
            parameters: *mut RenderParam,
        ) -> c_int;
        pub fn mpv_render_context_free(context: Handle);
        pub fn mpv_render_context_set_update_callback(
            context: Handle,
            callback: Callback,
            data: *mut c_void,
        );
        pub fn mpv_render_context_update(context: Handle) -> u64;
        pub fn mpv_render_context_render(
            context: Handle,
            parameters: *mut RenderParam,
        ) -> c_int;
        pub fn mpv_render_context_report_swap(context: Handle);
        #[cfg(test)]
        pub fn mpv_get_property_string(
            handle: Handle,
            name: *const c_char,
        ) -> *mut c_char;
        #[cfg(test)]
        pub fn mpv_free(data: *mut c_void);
    }
}

#[cfg(feature = "static-video")]
impl Api {
    pub fn load() -> Result<Arc<Self>> {
        // SAFETY: The statically linked backend implements this versioned ABI
        ensure!(
            unsafe { linked::flowa_mpv_vulkan_abi_version() } == VULKAN_ABI_VERSION,
            "Unsupported Flowa Vulkan backend ABI"
        );

        Ok(Arc::new(Self {
            create: linked::mpv_create,
            initialize: linked::mpv_initialize,
            terminate_destroy: linked::mpv_terminate_destroy,
            set_option_string: linked::mpv_set_option_string,
            command_async: linked::mpv_command_async,
            observe_property: linked::mpv_observe_property,
            wait_event: linked::mpv_wait_event,
            set_wakeup_callback: linked::mpv_set_wakeup_callback,
            error_string: linked::mpv_error_string,
            render_context_create: linked::mpv_render_context_create,
            render_context_free: linked::mpv_render_context_free,
            render_context_set_update_callback:
                linked::mpv_render_context_set_update_callback,
            render_context_update: linked::mpv_render_context_update,
            render_context_render: linked::mpv_render_context_render,
            render_context_report_swap: linked::mpv_render_context_report_swap,
        }))
    }
}

impl Api {
    #[cfg(test)]
    pub(super) fn audio_output(&self, handle: Handle) -> Result<String> {
        #[cfg(not(feature = "static-video"))]
        type GetPropertyString =
            unsafe extern "C" fn(Handle, *const c_char) -> *mut c_char;

        #[cfg(feature = "static-video")]
        let (get_property, free) = (linked::mpv_get_property_string, linked::mpv_free);

        // SAFETY: The owning Api retains the library and these are stable client functions
        #[cfg(not(feature = "static-video"))]
        let (get_property, free) = unsafe {
            (
                *self
                    ._library
                    .get::<GetPropertyString>(b"mpv_get_property_string\0")?,
                *self
                    ._library
                    .get::<unsafe extern "C" fn(*mut c_void)>(b"mpv_free\0")?,
            )
        };

        // SAFETY: The property string is copied before being returned to mpv's allocator
        unsafe {
            let value = get_property(handle, c"current-ao".as_ptr());
            ensure!(!value.is_null(), "No audio output was initialized");

            let output = CStr::from_ptr(value).to_string_lossy().into_owned();
            free(value.cast());
            Ok(output)
        }
    }

    pub fn check(&self, code: c_int) -> Result<()> {
        if code >= 0 {
            return Ok(());
        }

        // SAFETY: Libmpv returns a permanent NUL-terminated error string
        let error = unsafe { CStr::from_ptr((self.error_string)(code)) };
        bail!("libmpv: {}", error.to_string_lossy())
    }

    pub fn option(&self, handle: Handle, name: &CStr, value: &CStr) -> Result<()> {
        // SAFETY: Options are set before initialization on the owning thread
        self.check(unsafe {
            (self.set_option_string)(handle, name.as_ptr(), value.as_ptr())
        })
        .with_context(|| format!("Cannot set mpv option {}", name.to_string_lossy()))
    }

    /// Handles options provided by optional mpv features such as its Lua UI
    pub fn optional_option(
        &self,
        handle: Handle,
        name: &CStr,
        value: &CStr,
    ) -> Result<()> {
        // SAFETY: Options are set before initialization on the owning thread
        let result =
            unsafe { (self.set_option_string)(handle, name.as_ptr(), value.as_ptr()) };

        if result == ERROR_OPTION_NOT_FOUND {
            return Ok(());
        }

        self.check(result).with_context(|| {
            format!("Cannot set mpv option {}", name.to_string_lossy())
        })
    }

    pub fn command(&self, handle: Handle, args: &[&str]) -> Result<()> {
        let strings = args
            .iter()
            .map(|arg| CString::new(*arg))
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let mut pointers: Vec<_> = strings.iter().map(|arg| arg.as_ptr()).collect();
        pointers.push(std::ptr::null());

        // SAFETY: The async API copies arguments before returning
        self.check(unsafe { (self.command_async)(handle, 0, pointers.as_ptr()) })
    }
}

#[cfg(not(feature = "static-video"))]
pub(super) fn library_path() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("FLOWA_MPV_LIBRARY") {
        return Some(path.into());
    }

    let executable = std::env::current_exe().ok();
    let bundled = executable
        .as_deref()
        .and_then(Path::parent)
        .map(|parent| parent.join("libflowa-mpv.so"));
    if let Some(path) = bundled.filter(|path| path.is_file()) {
        return Some(path);
    }

    let development = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("target/native-video/libflowa-mpv.so");
    development.is_file().then_some(development)
}
