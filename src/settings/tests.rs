//! Unit tests for the settings area

mod model {
    use anyhow::Result;

    use crate::settings::{
        Canvas, OverlayBackdrop, RendererPreference, Settings, SortOrder, Theme,
        VideoControlsStyle, VideoDecoder, migrations,
    };

    #[test]
    fn old_preferences_get_new_defaults() -> Result<()> {
        let s: Settings = serde_json::from_str("{\"filmstrip\":true}")?;
        assert!(s.filmstrip);
        assert!(!s.checkerboard);
        assert_eq!(s.video_controls, VideoControlsStyle::Overlay);
        assert_eq!(s.overlay_backdrop, OverlayBackdrop::Black);
        assert_eq!(s.renderer, RendererPreference::Auto);
        assert_eq!(s.video_decoder, VideoDecoder::Auto);
        Ok(())
    }

    #[test]
    fn the_old_compact_checkbox_moves_to_the_new_styles_once() -> Result<()> {
        // The checkbox was on by default, so only turning it off was a choice
        for (compact, expected) in [
            (true, VideoControlsStyle::Overlay),
            (false, VideoControlsStyle::TwoRows),
        ] {
            let json = format!("{{\"compact_video_controls\":{compact}}}");
            let mut s: Settings = serde_json::from_str(&json)?;
            s.validate();

            assert_eq!(s.video_controls, expected);
            assert_eq!(s.legacy_compact_controls, None);

            // Only the new setting is written back
            let saved = serde_json::to_value(&s)?;
            assert!(saved.get("compact_video_controls").is_none());
        }

        Ok(())
    }

    #[test]
    fn older_preferences_migrate_to_the_current_version() -> Result<()> {
        // Files from before versions are version 0; only an untouched old default
        // window moves to the new one
        for (saved, expected) in [
            ([1200., 820.], [1280., 810.]),
            ([1000., 700.], [1000., 700.]),
        ] {
            let json = format!("{{\"window_size\":[{},{}]}}", saved[0], saved[1]);
            let mut s: Settings = serde_json::from_str(&json)?;

            assert_eq!(s.version, 0);
            assert!(migrations::migrate(&mut s));
            assert_eq!(s.version, migrations::CURRENT);
            assert_eq!(s.window_size, expected);
        }

        Ok(())
    }

    #[test]
    fn current_and_newer_preferences_are_left_alone() {
        let mut current = Settings::default();
        assert!(!migrations::migrate(&mut current));

        // A newer build's preferences keep their version for when it runs again
        let mut newer = Settings {
            version: migrations::CURRENT + 1,
            window_size: [1200., 820.],
            ..Default::default()
        };
        assert!(!migrations::migrate(&mut newer));
        assert_eq!(newer.version, migrations::CURRENT + 1);
        assert_eq!(newer.window_size, [1200., 820.]);
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
            video_decoder: VideoDecoder::Nvdec,
            ..Default::default()
        };
        let saved = serde_json::to_value(&settings)?;

        assert_eq!(settings.canvas.as_ref(), "Neutral dark");
        assert_eq!(settings.sort.as_ref(), "Date modified");
        assert_eq!(settings.renderer.as_ref(), "OpenGL");
        assert_eq!(saved["canvas"], "Dark");
        assert_eq!(saved["sort"], "Modified");
        assert_eq!(saved["renderer"], "OpenGl");
        assert_eq!(saved["video_decoder"], "Nvdec");
        Ok(())
    }
}

mod store {
    use std::path::PathBuf;

    use anyhow::Result;

    use crate::settings::{
        Accent, OverlayBackdrop, RendererPreference, Settings, SettingsStore, Theme,
        VideoControlsStyle, VideoDecoder,
    };

    #[test]
    fn loading_writes_migrated_preferences_back_once() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("settings.json");
        std::fs::write(&path, "{\"window_size\":[1200,820]}")?;

        // The store treats what it loads as saved, so the update is written at once
        let loaded = SettingsStore::load(&path)?;
        let written: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path)?)?;
        assert_eq!(written["version"], loaded.version);
        assert_eq!(written["window_size"], serde_json::json!([1280., 810.]));

        // Current preferences are not rewritten
        std::fs::write(&path, serde_json::to_vec(&loaded)?)?;
        let before = std::fs::read(&path)?;
        SettingsStore::load(&path)?;
        assert_eq!(std::fs::read(&path)?, before);

        Ok(())
    }

    #[test]
    fn persisting_changed_settings_preserves_user_preferences() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("settings.json");
        let mut s = Settings {
            theme: Theme::Sweet,
            video_controls: VideoControlsStyle::Compact,
            overlay_backdrop: OverlayBackdrop::Theme,
            checkerboard: true,
            video_decoder: VideoDecoder::Vulkan,
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
        assert_eq!(loaded.video_controls, VideoControlsStyle::Compact);
        assert_eq!(loaded.overlay_backdrop, OverlayBackdrop::Theme);
        assert_eq!(loaded.accent, Accent::Rose);
        assert_eq!(loaded.renderer, store.values.renderer);
        assert_eq!(loaded.video_decoder, VideoDecoder::Vulkan);
        assert!(loaded.checkerboard);
        assert_eq!(loaded.recent, vec![PathBuf::from("picture.png")]);
        Ok(())
    }
}
