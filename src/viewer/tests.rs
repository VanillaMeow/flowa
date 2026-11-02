//! Unit tests for the viewer area

mod catalog {
    use crate::viewer::catalog::next_index;

    #[test]
    fn navigation_handles_edges_and_empty_folders() {
        assert_eq!(next_index(0, 0, 1, true), None);
        assert_eq!(next_index(0, 3, -1, true), Some(2));
        assert_eq!(next_index(2, 3, 1, false), Some(2));
    }
}

mod browser {
    use std::{path::PathBuf, time::SystemTime};

    use crate::{
        settings::SortOrder,
        viewer::{
            browser::Browser,
            catalog::{Entry, ScannedFolder},
        },
    };

    #[test]
    fn sorting_keeps_the_selected_path() {
        let selected = PathBuf::from("a.png");
        let entries = ["a.png", "b.png"].map(|name| Entry {
            path: name.into(),
            name: name.into(),
            size: 0,
            modified: SystemTime::UNIX_EPOCH,
        });
        let mut browser = Browser::new();
        browser.replace(
            ScannedFolder {
                folder: PathBuf::from("."),
                selected: Some(selected.clone()),
                entries: entries.into(),
            },
            SortOrder::Name,
            false,
        );

        browser.sort(SortOrder::Name, true);

        assert_eq!(browser.current().map(|entry| &entry.path), Some(&selected));
        assert_eq!(browser.index(), 1);
    }
}

mod image_session {
    use std::{sync::Arc, time::Duration};

    use crate::viewer::image_session::{DecodedFrame, DecodedImage, ImageSession};

    fn decoded() -> DecodedImage {
        DecodedImage {
            frames: vec![DecodedFrame {
                pixels: Arc::new(image::RgbaImage::new(1, 1)),
                preview: egui::ColorImage::filled([1, 1], egui::Color32::TRANSPARENT),
                delay: Duration::from_millis(100),
            }],
            thumbnail: egui::ColorImage::filled([1, 1], egui::Color32::TRANSPARENT),
            size: [1, 1],
            format: "PNG".into(),
            file_size: 0,
            color: "RGBA".into(),
            metadata: Vec::new(),
        }
    }

    #[test]
    fn only_the_current_selection_accepts_decode_results() {
        let mut session = ImageSession::new();
        let old = session.begin_load();
        let current = session.begin_load();

        assert!(!session.accept(old, Ok(decoded())));
        assert!(!session.accept(old, Err(anyhow::anyhow!("Old image failed"))));
        assert!(session.loading);
        assert!(session.error.is_none());
        assert!(session.accept(current, Ok(decoded())));
        assert!(session.current_pixels().is_some());
        assert!(!session.loading);

        session.clear();
        assert!(!session.accept(current, Ok(decoded())));
        assert!(session.decoded.is_none());
        assert!(!session.loading);
    }
}

mod transform {
    use egui::{Pos2, Rect, vec2};

    use crate::viewer::transform::View;

    #[test]
    fn zoom_keeps_the_image_point_under_the_cursor() {
        let mut v = View::default();
        let rect = Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0));
        let size = vec2(1600.0, 1000.0);
        let cursor = egui::pos2(530.0, 380.0);
        let before = v.screen_to_image(cursor, size, rect);
        v.zoom_at(2.7, cursor, rect.center());
        let after = v.screen_to_image(cursor, size, rect);
        assert!((before - after).length() < 0.001);
    }

    #[test]
    fn transforms_round_trip_after_rotation_and_flips() {
        let rect = Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0));
        let size = vec2(100.0, 60.0);

        // Top-left rotates clockwise around the image center after applying flips
        for (rotation, flip_x, flip_y, expected) in [
            (0, false, false, egui::pos2(350.0, 270.0)),
            (1, false, false, egui::pos2(430.0, 250.0)),
            (2, false, false, egui::pos2(450.0, 330.0)),
            (3, false, false, egui::pos2(370.0, 350.0)),
            (1, true, false, egui::pos2(430.0, 350.0)),
            (1, false, true, egui::pos2(370.0, 250.0)),
            (1, true, true, egui::pos2(370.0, 350.0)),
        ] {
            let view = View {
                rotation,
                flip_x,
                flip_y,
                ..Default::default()
            };
            assert_eq!(view.image_to_screen(egui::Vec2::ZERO, size, rect), expected);
            assert_eq!(view.screen_to_image(expected, size, rect), egui::Vec2::ZERO);
        }

        let view = View {
            rotation: 3,
            flip_x: true,
            zoom: 2.7,
            pan: vec2(30.0, -20.0),
            ..Default::default()
        };
        let point = vec2(17.0, 48.0);
        let screen = view.image_to_screen(point, size, rect);
        assert!((point - view.screen_to_image(screen, size, rect)).length() < 0.001);
    }
}
