//! Steps that bring older preferences up to date, one version at a time
//!
//! Append a step to change preferences people already have, such as moving them to a
//! better default. Never reorder or remove steps, since each one's position is the
//! version it upgrades from

use super::Settings;

/// Each step moves preferences from the version at its index to the next one
const STEPS: &[fn(&mut Settings)] = &[size_window_for_16_9_video];

/// The version of the preferences this build writes
pub const CURRENT: u32 = STEPS.len() as u32;

/// Runs the steps the preferences have not had yet, returning whether any ran
///
/// Preferences from a newer build are left as they are
pub fn migrate(settings: &mut Settings) -> bool {
    let pending = STEPS.get(settings.version as usize..).unwrap_or_default();

    if pending.is_empty() {
        return false;
    }

    for step in pending {
        step(settings);
    }

    settings.version = CURRENT;
    true
}

/// Version 0: the default window grew from 1200 × 820 to leave a 16:9 canvas, and a
/// window still at the old default was most likely never resized
fn size_window_for_16_9_video(settings: &mut Settings) {
    if settings.window_size == [1200., 820.] {
        settings.window_size = settings.default_window_size();
    }
}
