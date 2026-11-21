//! Configuration values, validation, and persistence

mod model;
mod store;

pub(crate) use model::{
    Accent, CHECKER_CONTRAST, CHECKER_SIZE, Canvas, MEMORY_MB, OverlayBackdrop,
    RendererPreference, SLIDESHOW_SECONDS, Settings, SortOrder, SpeedPitch, Theme,
    UI_SCALE, VideoControlsStyle, VideoDecoder,
};
pub(crate) use store::SettingsStore;

#[cfg(test)]
mod tests;
