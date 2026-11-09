//! Settings persistence and change tracking

use std::{
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use anyhow::{Context, Result};

use crate::settings::Settings;

pub struct SettingsStore {
    pub values: Settings,
    path: Option<PathBuf>,
    /// The values last written to disk, so unchanged settings are not rewritten
    saved: Settings,
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
            saved: values.clone(),
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

        if self.values != self.saved
            && let Some(path) = &self.path
        {
            Self::save(&self.values, path)?;
            self.saved = self.values.clone();
        }

        Ok(())
    }
}
