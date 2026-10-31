//! Configuration values, validation, and persistence

mod model;
mod store;

pub(crate) use model::{Accent, Canvas, Settings, SortOrder, Theme};
pub(crate) use store::SettingsStore;
