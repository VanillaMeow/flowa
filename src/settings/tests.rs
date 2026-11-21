//! Unit tests for the settings area

mod model {
    use anyhow::Result;

    use crate::settings::{
        Canvas, OverlayBackdrop, RendererPreference, Settings, SortOrder, Theme,
        VideoControlsStyle, VideoDecoder,
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
