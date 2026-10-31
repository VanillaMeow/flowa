//! Startup options passed from the native entry point

use std::path::PathBuf;

pub struct Launch {
    pub path: Option<PathBuf>,
    pub screenshot: Option<PathBuf>,
    pub smoke_test: bool,
    pub preferences: bool,
    pub filmstrip: bool,
    pub info: bool,
    pub zoom: Option<f32>,
}
