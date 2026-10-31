//! Settings persistence and change tracking

use std::{
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use anyhow::{Context, Result};

use crate::settings::Settings;

pub(crate) struct SettingsStore {
    pub values: Settings,
    path: Option<PathBuf>,
    saved: String,
    last_save: Instant,
}

impl SettingsStore {
    pub fn config_path() -> Option<PathBuf> {
        directories::ProjectDirs::from("", "", "flowa")
            .map(|p| p.config_dir().join("settings.json"))
    }

    pub fn load(path: &Path) -> Result<Settings> {
        let contents = std::fs::read(path).context("Could not read preferences")?;
        let mut settings: Settings =
            serde_json::from_slice(&contents).context("Could not parse preferences")?;

        settings.validate();

        Ok(settings)
    }

    pub fn save(settings: &Settings, path: &Path) -> Result<()> {
        let parent = path.parent().context("Preferences path has no parent")?;
        std::fs::create_dir_all(parent)
            .context("Could not create preferences directory")?;

        // Keep the previous file intact if writing the new settings fails
        let temporary = path.with_extension("json.tmp");
        std::fs::write(&temporary, serde_json::to_vec_pretty(settings)?)
            .context("Could not write preferences")?;
        std::fs::rename(temporary, path).context("Could not save preferences")
    }

    pub fn new(values: Settings, path: Option<PathBuf>) -> Self {
        Self {
            saved: serde_json::to_string(&values).unwrap_or_default(),
            values,
            path,
            last_save: Instant::now(),
        }
    }

    pub fn save_due(&self) -> bool {
        self.last_save.elapsed() > Duration::from_secs(2)
    }

    pub fn persist(&mut self) -> Result<()> {
        self.last_save = Instant::now();
        let snapshot = serde_json::to_string(&self.values)?;

        if snapshot != self.saved
            && let Some(path) = &self.path
        {
            Self::save(&self.values, path)?;
            self.saved = snapshot;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::settings::{Accent, Theme};

    use super::*;

    #[test]
    fn settings_round_trip_and_preserve_user_preferences() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("settings.json");
        let mut s = Settings {
            theme: Theme::Sand,
            checkerboard: true,
            ..Default::default()
        };
        s.remember(PathBuf::from("picture.png"));
        SettingsStore::save(&s, &path)?;
        s.accent = Accent::Rose;
        SettingsStore::save(&s, &path)?;
        let loaded = SettingsStore::load(&path)?;
        assert_eq!(loaded.theme, Theme::Sand);
        assert_eq!(loaded.accent, Accent::Rose);
        assert!(loaded.checkerboard);
        assert_eq!(loaded.recent.len(), 1);
        Ok(())
    }
}
