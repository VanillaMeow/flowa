//! Configuration values, validation, and persistence

mod model;
mod store;

pub(crate) use model::{
    Accent, Canvas, RendererPreference, Settings, SortOrder, Theme, VideoDecoder,
};
pub(crate) use store::SettingsStore;

#[cfg(test)]
mod tests;
