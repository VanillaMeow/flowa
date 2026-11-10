//! Routes FFmpeg's warnings and errors through the `log` crate, and lets a caller
//! collect the first error FFmpeg reports on its thread during one call
//!
//! FFmpeg often explains a failure only in its log, such as a hardware decoder's
//! size limit, while the call itself returns a generic error code

use std::{
    cell::RefCell,
    ffi::{CStr, c_char, c_int, c_void},
    sync::Once,
};

use super::ffi;

#[cfg(windows)]
type VaList = ffi::va_list;
#[cfg(not(windows))]
type VaList = *mut ffi::__va_list_tag;

thread_local! {
    /// While a capture runs on this thread, the first error FFmpeg logged during it
    static FIRST_ERROR: RefCell<Option<Option<String>>> = const { RefCell::new(None) };
}

/// Replaces FFmpeg's default stderr logging, once per process
pub(in crate::video) fn install() {
    static INSTALL: Once = Once::new();

    // SAFETY: The callback only formats the message and may run on any thread
    INSTALL.call_once(|| unsafe { ffi::av_log_set_callback(Some(forward)) });
}

/// Runs a call and returns the first error FFmpeg logged on this thread during it
pub(in crate::video) fn first_error<T>(
    call: impl FnOnce() -> T,
) -> (T, Option<String>) {
    let previous = FIRST_ERROR.replace(Some(None));
    let result = call();
    let captured = FIRST_ERROR.replace(previous).flatten();

    (result, captured)
}

unsafe extern "C" fn forward(
    context: *mut c_void,
    level: c_int,
    format: *const c_char,
    arguments: VaList,
) {
    if level > ffi::AV_LOG_WARNING as c_int {
        return;
    }

    let mut line: [c_char; 1024] = [0; 1024];
    // Without the prefix, the line holds only the message itself
    let mut print_prefix = 0;

    // SAFETY: FFmpeg passes a valid format with its arguments, and the call writes a
    // NUL-terminated line of at most the buffer's length
    let written = unsafe {
        ffi::av_log_format_line2(
            context,
            level,
            format,
            arguments,
            line.as_mut_ptr(),
            line.len() as c_int,
            &mut print_prefix,
        )
    };

    if written < 0 {
        return;
    }

    // SAFETY: The buffer was initialized and FFmpeg terminated what it wrote
    let message = unsafe { CStr::from_ptr(line.as_ptr()) }.to_string_lossy();
    let message = message.trim();

    if message.is_empty() {
        return;
    }

    if level <= ffi::AV_LOG_ERROR as c_int {
        record(message);
    }

    log::debug!(target: "ffmpeg", "{message}");
}

/// Keeps the message when a capture on this thread has not seen an error yet
fn record(message: &str) {
    // A thread that is shutting down or already borrowing the slot just skips it
    let _ = FIRST_ERROR.try_with(|first| {
        if let Ok(mut first) = first.try_borrow_mut()
            && let Some(slot @ None) = first.as_mut()
        {
            *slot = Some(message.to_owned());
        }
    });
}
