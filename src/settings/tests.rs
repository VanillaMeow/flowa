//! Unit tests for the settings area

mod model {
    use anyhow::Result;

    use crate::settings::{Canvas, RendererPreference, Settings, SortOrder, Theme};

    #[test]
    fn old_preferences_get_new_defaults() -> Result<()> {
        let s: Settings = serde_json::from_str("{\"filmstrip\":true}")?;
        assert!(s.filmstrip);
        assert!(!s.checkerboard);
        assert!(s.compact_video_controls);
        assert_eq!(s.renderer, RendererPreference::Auto);
        Ok(())
    }

    #[test]
    fn display_labels_preserve_the_saved_settings_format() -> Result<()> {
        for (theme, label, saved) in [
            (Theme::System, "System / Qt", "System"),
            (Theme::Oled, "OLED", "Oled"),
            (Theme::HighContrast, "High contrast", "HighContrast"),
        ] {
            assert_eq!(theme.as_ref(), label);
            assert_eq!(serde_json::to_value(theme)?, saved);
        }

        let settings = Settings {
            canvas: Canvas::Dark,
            sort: SortOrder::Modified,
            renderer: RendererPreference::OpenGl,
            ..Default::default()
        };
        let saved = serde_json::to_value(&settings)?;

        assert_eq!(settings.canvas.as_ref(), "Neutral dark");
        assert_eq!(settings.sort.as_ref(), "Date modified");
        assert_eq!(settings.renderer.as_ref(), "OpenGL");
        assert_eq!(saved["canvas"], "Dark");
        assert_eq!(saved["sort"], "Modified");
        assert_eq!(saved["renderer"], "OpenGl");
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
            theme: Theme::Sweet,
            compact_video_controls: false,
            checkerboard: true,
            renderer: RendererPreference::SUPPORTED
                .last()
                .copied()
                .unwrap_or_default(),
            ..Default::default()
        };
        s.remember(PathBuf::from("picture.png"));
        SettingsStore::save(&s, &path)?;
        let mut store = SettingsStore::new(s, Some(path.clone()));
        store.values.accent = Accent::Rose;
        store.persist()?;

        let loaded = SettingsStore::load(&path)?;
        assert_eq!(loaded.theme, Theme::Sweet);
        assert!(!loaded.compact_video_controls);
        assert_eq!(loaded.accent, Accent::Rose);
        assert_eq!(loaded.renderer, store.values.renderer);
        assert!(loaded.checkerboard);
        assert_eq!(loaded.recent, vec![PathBuf::from("picture.png")]);
        Ok(())
    }
}
