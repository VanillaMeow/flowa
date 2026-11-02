//! Unit tests for the settings area

mod model {
    use anyhow::Result;

    use crate::settings::{RendererPreference, Settings};

    #[test]
    fn old_preferences_get_new_defaults() -> Result<()> {
        let s: Settings = serde_json::from_str("{\"filmstrip\":true}")?;
        assert!(s.filmstrip);
        assert!(!s.checkerboard);
        assert_eq!(s.renderer, RendererPreference::Auto);
        Ok(())
    }
}

mod store {
    use std::path::PathBuf;

    use anyhow::Result;

    use crate::settings::{Accent, RendererPreference, Settings, SettingsStore, Theme};

    #[test]
    fn persisting_changed_settings_preserves_user_preferences() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("settings.json");
        let mut s = Settings {
            theme: Theme::Sand,
            checkerboard: true,
            renderer: RendererPreference::ALL.last().copied().unwrap_or_default(),
            ..Default::default()
        };
        s.remember(PathBuf::from("picture.png"));
        SettingsStore::save(&s, &path)?;
        let mut store = SettingsStore::new(s, Some(path.clone()));
        store.values.accent = Accent::Rose;
        store.persist()?;

        let loaded = SettingsStore::load(&path)?;
        assert_eq!(loaded.theme, Theme::Sand);
        assert_eq!(loaded.accent, Accent::Rose);
        assert_eq!(loaded.renderer, store.values.renderer);
        assert!(loaded.checkerboard);
        assert_eq!(loaded.recent, vec![PathBuf::from("picture.png")]);
        Ok(())
    }
}
