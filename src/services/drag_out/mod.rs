//! Dragging files from the filmstrip into other applications
//!
//! Winit can receive dropped files but cannot start a drag, so each Linux display
//! protocol gets its own drag source. Wayland attaches a second event queue to the
//! window's connection; X11 takes the pointer grab over on its own connection. Other
//! platforms report dragging as unavailable

#[cfg(target_os = "linux")]
mod wayland;
#[cfg(target_os = "linux")]
mod x11;

use std::{
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

use anyhow::Result;

/// The MIME type file managers and chat apps accept for dropped files
const URI_LIST: &str = "text/uri-list";

/// Starts drags of files out of the window, when the display protocol allows it
pub struct DragOut {
    source: Option<Box<dyn DragSource>>,
    finished: Arc<AtomicBool>,
}

/// One display protocol's way of offering files to whatever is under the pointer
trait DragSource {
    /// Begins a drag while the pointer button is still held
    fn start(&mut self, uris: Vec<u8>) -> Result<()>;
}

impl DragOut {
    /// Prepares the drag source for the window's display protocol
    #[cfg(target_os = "linux")]
    pub fn new(
        window: &impl raw_window_handle::HasWindowHandle,
        display: &impl raw_window_handle::HasDisplayHandle,
        ctx: &egui::Context,
    ) -> Self {
        use raw_window_handle::{RawDisplayHandle, RawWindowHandle};

        let finished = Arc::new(AtomicBool::new(false));
        let handles = window
            .window_handle()
            .ok()
            .zip(display.display_handle().ok())
            .map(|(window, display)| (window.as_raw(), display.as_raw()));

        let source: Result<Box<dyn DragSource>> = match handles {
            Some((
                RawWindowHandle::Wayland(window),
                RawDisplayHandle::Wayland(display),
            )) => wayland::WaylandSource::new(
                display.display,
                window.surface,
                Arc::clone(&finished),
                ctx.clone(),
            )
            .map(|source| Box::new(source) as Box<dyn DragSource>),
            Some((RawWindowHandle::Xlib(window), RawDisplayHandle::Xlib(display))) => {
                x11::X11Source::new(
                    display.display,
                    window.window,
                    Arc::clone(&finished),
                    ctx.clone(),
                )
                .map(|source| Box::new(source) as Box<dyn DragSource>)
            }
            _ => Err(anyhow::anyhow!("This display protocol has no drag source")),
        };

        let source = source
            .inspect_err(|error| {
                log::info!("Dragging files out is unavailable: {error:#}")
            })
            .ok();

        Self { source, finished }
    }

    #[cfg(not(target_os = "linux"))]
    pub fn new(
        _window: &impl raw_window_handle::HasWindowHandle,
        _display: &impl raw_window_handle::HasDisplayHandle,
        _ctx: &egui::Context,
    ) -> Self {
        Self::unavailable()
    }

    /// A drag source that refuses every drag, for tests and unsupported platforms
    pub fn unavailable() -> Self {
        Self {
            source: None,
            finished: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn available(&self) -> bool {
        self.source.is_some()
    }

    /// Offers the files to the application under the pointer until the button is released
    pub fn start(&mut self, paths: &[PathBuf]) -> Result<()> {
        let source = self.source.as_mut().ok_or_else(|| {
            anyhow::anyhow!("Dragging files out is not supported here")
        })?;

        self.finished.store(false, Ordering::SeqCst);
        source.start(uri_list(paths))
    }

    /// Reports once that a drag ended, since the window never saw its button release
    pub fn take_finished(&self) -> bool {
        self.finished.swap(false, Ordering::SeqCst)
    }
}

/// `file://` URIs for absolute paths, one per CRLF-terminated line
pub fn uri_list(paths: &[PathBuf]) -> Vec<u8> {
    let mut list = String::new();

    for path in paths {
        list.push_str(&file_uri(path));
        list.push_str("\r\n");
    }

    list.into_bytes()
}

/// Percent-encodes every byte of the path except unreserved characters and slashes
fn file_uri(path: &Path) -> String {
    #[cfg(unix)]
    let bytes = {
        use std::os::unix::ffi::OsStrExt;
        path.as_os_str().as_bytes().to_vec()
    };
    #[cfg(not(unix))]
    let bytes = path.to_string_lossy().replace('\\', "/").into_bytes();

    let mut uri = String::from("file://");

    for byte in bytes {
        let unreserved = byte.is_ascii_alphanumeric() || b"-._~/".contains(&byte);

        if unreserved {
            uri.push(char::from(byte));
        } else {
            uri.push_str(&format!("%{byte:02X}"));
        }
    }

    uri
}
