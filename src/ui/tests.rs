//! Unit tests for the ui area

mod keyboard {
    use crate::{
        app::{BrowseCommand, Command, FileCommand, ViewCommand},
        ui::keyboard::read,
    };

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
            vec![Command::Browse(BrowseCommand::Navigate(1))]
        );
    }

    #[test]
    fn modified_shortcuts_do_not_also_fire_the_plain_action() {
        for (key, modifiers, expected) in [
            (
                egui::Key::O,
                egui::Modifiers::CTRL | egui::Modifiers::SHIFT,
                Command::File(FileCommand::OpenFolder),
            ),
            (
                egui::Key::C,
                egui::Modifiers::CTRL | egui::Modifiers::SHIFT,
                Command::File(FileCommand::CopyPath),
            ),
            (
                egui::Key::R,
                egui::Modifiers::SHIFT,
                Command::View(ViewCommand::RotateCounterclockwise),
            ),
        ] {
            let ctx = egui::Context::default();
            assert_eq!(press(&ctx, key, modifiers, false, false), vec![expected]);
        }
    }
}

mod actions {
    use crate::{
        settings::{Accent, Theme},
        ui::{actions, theme},
    };

    #[test]
    fn menu_shortcuts_share_a_right_edge_including_disabled_actions() {
        let ctx = egui::Context::default();
        theme::apply(&ctx, theme::colors(Theme::Charcoal, Accent::Theme, true));
        let shortcuts = [&actions::OPEN_FILE, &actions::OPEN_FOLDER, &actions::RENAME]
            .map(|action| action.shortcut_text(&ctx));
        let mut output = egui::FullOutput::default();

        for _ in 0..3 {
            output.textures_delta.clear();
            output = ctx.run_ui(egui::RawInput::default(), |ui| {
                let anchor = ui.button("File");
                egui::Popup::menu(&anchor).open(true).show(|ui| {
                    let mut commands = Vec::new();
                    actions::OPEN_FILE.menu(ui, true, &mut commands);
                    actions::OPEN_FOLDER.menu(ui, true, &mut commands);
                    ui.separator();
                    actions::RENAME.menu(ui, false, &mut commands);
                });
            });
        }

        let right_edges: Vec<_> = output
            .shapes
            .iter()
            .filter_map(|shape| {
                let egui::Shape::Text(text) = &shape.shape else {
                    return None;
                };
                shortcuts
                    .contains(&text.galley.job.text)
                    .then_some(text.pos.x + text.galley.size().x)
            })
            .collect();

        output.textures_delta.clear();
        assert_eq!(right_edges.len(), shortcuts.len());
        for right in &right_edges {
            assert!(
                (right - right_edges[0]).abs() < 1.,
                "Misaligned shortcut edges: {right_edges:?}"
            );
        }
    }
}

mod canvas {
    use egui::{Pos2, Rect, TextureId, vec2};

    use crate::{
        settings::{Accent, Settings, Theme},
        ui::{
            canvas::{CanvasContent, ImageCanvas},
            theme,
        },
        video::VideoSession,
        viewer::image_session::ImageSession,
    };

    #[test]
    fn fullscreen_content_reaches_the_viewport_edges() {
        let ctx = egui::Context::default();
        let mut canvas = ImageCanvas::new(ctx.clone(), false);
        let colors = theme::colors(Theme::Charcoal, Accent::Theme, true);
        let image = ImageSession::new();
        let settings = Settings::default();
        let mut video = VideoSession::default();
        video.state.active = true;
        video.state.size = [3840, 2160];
        video.texture = Some(TextureId::User(7));

        for fullscreen in [false, true, false, true] {
            let viewport = Rect::from_min_size(Pos2::ZERO, vec2(1280., 720.));
            let input = egui::RawInput {
                screen_rect: Some(viewport),
                ..Default::default()
            };
            let mut output = ctx.run_ui(input, |ui| {
                canvas.show(
                    ui,
                    CanvasContent {
                        image: &image,
                        video: &video,
                        settings: &settings,
                        colors,
                        can_navigate: false,
                        fullscreen,
                    },
                    &mut Vec::new(),
                );
            });
            output.textures_delta.clear();
            let displayed = canvas.view.image_rect(vec2(3840., 2160.), canvas.rect);

            assert_eq!(canvas.rect, viewport);
            if fullscreen {
                assert!((displayed.min - viewport.min).length() < 0.01);
                assert!((displayed.max - viewport.max).length() < 0.01);
            } else {
                assert!(displayed.height() < viewport.height() - 40.);
            }
        }
    }
}

mod theme {
    use crate::{
        settings::{Accent, Theme},
        ui::theme::colors,
    };
    use egui::Color32;

    #[test]
    fn sweet_is_a_built_in_palette_and_honors_accent_overrides() {
        let palette = colors(Theme::Sweet, Accent::Theme, false);
        assert!(palette.dark);
        assert_eq!(palette.canvas, Color32::from_rgb(22, 25, 37));
        assert_eq!(palette.panel, Color32::from_rgb(24, 27, 40));
        assert_eq!(palette.accent, Color32::from_rgb(197, 14, 210));

        let custom = colors(Theme::Sweet, Accent::Blue, true);
        assert_ne!(custom.accent, palette.accent);
        assert_eq!(custom.panel, palette.panel);
    }
}

mod filmstrip {
    use std::{path::PathBuf, time::SystemTime};

    use egui::{Event, MouseWheelUnit, Pos2, Rect, vec2};

    use crate::{
        settings::{Accent, Settings, SortOrder, Theme},
        ui::{filmstrip::Filmstrip, theme},
        viewer::{
            browser::Browser,
            catalog::{Entry, ScannedFolder},
        },
    };

    fn frame(
        ctx: &egui::Context,
        strip: &mut Filmstrip,
        browser: &Browser,
        events: Vec<Event>,
    ) -> f32 {
        let colors = theme::colors(Theme::Charcoal, Accent::Theme, true);
        let input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(640., 600.))),
            events,
            ..Default::default()
        };
        let mut output =
            ctx.run_ui(input, |ui| strip.show(ui, browser, colors, &mut Vec::new()));

        output.textures_delta.clear();
        output
            .shapes
            .iter()
            .find_map(|shape| {
                let egui::Shape::Text(text) = &shape.shape else {
                    return None;
                };
                (text.galley.job.text == "clip-2.mp4").then_some(text.pos.x)
            })
            .expect("Third thumbnail should remain visible")
    }

    #[test]
    fn wheel_and_trackpad_scroll_the_strip_without_shift() {
        let mut browser = Browser::new();
        let entries = (0..20)
            .map(|index| {
                let name = format!("clip-{index}.mp4");
                Entry {
                    path: name.clone().into(),
                    name,
                    size: 0,
                    modified: SystemTime::UNIX_EPOCH,
                }
            })
            .collect();
        browser.replace(
            ScannedFolder {
                folder: PathBuf::from("."),
                selected: None,
                entries,
            },
            SortOrder::Name,
            false,
        );

        for (delta, source) in [
            (vec2(0., -60.), egui::MouseWheelSource::Wheel),
            (vec2(-60., 0.), egui::MouseWheelSource::Trackpad),
        ] {
            let ctx = egui::Context::default();
            let mut strip =
                Filmstrip::new(ctx.clone(), Settings::default().memory_budget());
            frame(&ctx, &mut strip, &browser, Vec::new());
            let initial = frame(&ctx, &mut strip, &browser, Vec::new());

            frame(
                &ctx,
                &mut strip,
                &browser,
                vec![
                    Event::PointerMoved(egui::pos2(100., 540.)),
                    Event::MouseWheel {
                        unit: MouseWheelUnit::Point,
                        delta,
                        phase: egui::TouchPhase::Move,
                        source,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
            );
            let scrolled = frame(&ctx, &mut strip, &browser, Vec::new());
            assert!(
                scrolled < initial - 1.,
                "Wheel did not move thumbnails: {initial} -> {scrolled}"
            );
        }
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
            inspector_open: false,
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
