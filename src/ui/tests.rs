//! Unit tests for the ui area

mod keyboard {
    use crate::{app::Command, ui::keyboard::read};

    fn press(
        ctx: &egui::Context,
        key: egui::Key,
        modifiers: egui::Modifiers,
        modal: bool,
        preferences: bool,
    ) -> Vec<Command> {
        let input = egui::RawInput {
            events: vec![egui::Event::Key {
                key,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers,
            }],
            ..Default::default()
        };
        let mut commands = Vec::new();
        let mut output =
            ctx.run_ui(input, |_| commands.extend(read(ctx, modal, preferences)));
        output.textures_delta.clear();
        commands
    }

    #[test]
    fn modal_and_preferences_suppress_navigation_shortcuts() {
        let ctx = egui::Context::default();
        assert!(
            press(
                &ctx,
                egui::Key::ArrowRight,
                egui::Modifiers::NONE,
                true,
                false
            )
            .is_empty()
        );
        assert!(
            press(
                &ctx,
                egui::Key::ArrowRight,
                egui::Modifiers::NONE,
                false,
                true
            )
            .is_empty()
        );
        assert_eq!(
            press(
                &ctx,
                egui::Key::ArrowRight,
                egui::Modifiers::NONE,
                false,
                false
            ),
            vec![Command::Navigate(1)]
        );
    }
}

mod toolbar {
    use crate::{
        settings::{Accent, Settings, Theme},
        ui::{
            canvas::ImageCanvas,
            theme,
            toolbar::{self, ToolbarState},
        },
        video::VideoState,
        viewer::{browser::Browser, image_session::ImageSession},
    };

    #[test]
    fn toolbar_fits_at_large_ui_scales() {
        let ctx = egui::Context::default();
        let colors = theme::colors(Theme::Charcoal, Accent::Theme, true);
        theme::apply(&ctx, colors);

        let browser = Browser::new();
        let image = ImageSession::new();
        let video = VideoState {
            active: true,
            ..Default::default()
        };
        let canvas = ImageCanvas::new(ctx.clone(), false);
        let settings = Settings::default();
        let state = ToolbarState {
            browser: &browser,
            image: &image,
            video: &video,
            canvas: &canvas,
            settings: &settings,
            colors,
            fullscreen: false,
            preferences_open: false,
            busy: None,
            slideshow: false,
        };

        for width in [320., 548., 640., 1200.] {
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(width, 600.),
                )),
                ..Default::default()
            };
            let mut output =
                ctx.run_ui(input, |root| toolbar::show(root, &state, &mut Vec::new()));
            output.textures_delta.clear();

            let text: Vec<_> = output
                .shapes
                .iter()
                .filter_map(|shape| {
                    if let egui::Shape::Text(text) = &shape.shape
                        && !text.galley.job.text.is_empty()
                    {
                        Some(text)
                    } else {
                        None
                    }
                })
                .collect();
            let view_menu = text.iter().any(|text| text.galley.job.text == "View");
            assert_eq!(view_menu, width < 640.);

            for label in text {
                let bounds = label.visual_bounding_rect();
                assert!(
                    bounds.left() >= 0. && bounds.right() <= width,
                    "Toolbar text overflow at width {width}: {bounds:?}"
                );
            }
        }
    }
}
