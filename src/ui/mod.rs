//! Egui components, styling, keyboard input, and rendering

pub(crate) mod canvas;
pub(crate) mod dialogs;
pub(crate) mod filmstrip;
pub(crate) mod icons;
pub(crate) mod inspector;
pub(crate) mod keyboard;
pub(crate) mod preferences;
pub(crate) mod rendering;
pub(crate) mod theme;
pub(crate) mod toolbar;
pub(crate) mod video_controls;
mod viewer_ui;

pub(crate) use viewer_ui::ViewerUi;

#[cfg(test)]
mod tests;
