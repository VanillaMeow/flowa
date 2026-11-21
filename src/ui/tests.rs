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
    use egui::{Event, Modifiers, PointerButton, Pos2, Rect, TextureId, vec2};

    use crate::{
        app::{Command, PlaybackCommand},
        settings::{Accent, Settings, Theme},
        ui::{
            canvas::{CanvasContent, ImageCanvas},
            theme,
        },
        video::VideoSession,
        viewer::image_session::ImageSession,
    };

    #[test]
    fn video_margin_applies_only_outside_fullscreen_when_enabled() {
        let ctx = egui::Context::default();
        let mut canvas = ImageCanvas::new(ctx.clone(), false);
        let colors = theme::colors(Theme::Charcoal, Accent::Theme, true);
        let image = ImageSession::new();
        let mut video = VideoSession::default();
        video.state.active = true;
        video.state.size = [3840, 2160];
        video.texture = Some(TextureId::User(7));

        let cases = [
            (false, false),
            (true, false),
            (false, true),
            (true, true),
            (false, false),
        ];

        for (fullscreen, video_margin) in cases {
            let settings = Settings {
                video_margin,
                ..Settings::default()
            };
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
                        subtitle_clearance: 0.,
                    },
                    &mut Vec::new(),
                );
            });
            output.textures_delta.clear();

            let displayed = canvas.view.image_rect(vec2(3840., 2160.), canvas.rect);
            assert_eq!(canvas.rect, viewport);

            if video_margin && !fullscreen {
                assert!(displayed.height() < viewport.height() - 40.);
            } else {
                assert!((displayed.min - viewport.min).length() < 0.01);
                assert!((displayed.max - viewport.max).length() < 0.01);
            }
        }
    }
    #[test]
    fn clicking_a_video_plays_or_pauses_and_a_double_click_cancels_out() {
        let ctx = egui::Context::default();
        let mut canvas = ImageCanvas::new(ctx.clone(), false);
        let colors = theme::colors(Theme::Charcoal, Accent::Theme, true);
        let image = ImageSession::new();
        let settings = Settings::default();
        let mut video = VideoSession::default();
        video.state.active = true;
        video.state.size = [1920, 1080];
        video.texture = Some(TextureId::User(7));

        let viewport = Rect::from_min_size(Pos2::ZERO, vec2(1280., 720.));
        let center = viewport.center();
        let mut run = |time: f64, events: Vec<Event>| {
            let input = egui::RawInput {
                screen_rect: Some(viewport),
                time: Some(time),
                events,
                ..Default::default()
            };
            let mut commands = Vec::new();

            let mut output = ctx.run_ui(input, |ui| {
                let content = CanvasContent {
                    image: &image,
                    video: &video,
                    settings: &settings,
                    colors,
                    can_navigate: false,
                    fullscreen: false,
                    subtitle_clearance: 0.,
                };
                canvas.show(ui, content, &mut commands);
            });
            output.textures_delta.clear();

            commands
                .iter()
                .filter(|command| {
                    **command == Command::Playback(PlaybackCommand::TogglePlay)
                })
                .count()
        };
        let button = |pressed| Event::PointerButton {
            pos: center,
            button: PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        };

        run(0., vec![Event::PointerMoved(center)]);
        run(0.05, vec![button(true)]);
        assert_eq!(run(0.1, vec![button(false)]), 1, "A click toggles at once");

        // The second click of a double-click toggles back
        run(0.15, vec![button(true)]);
        assert_eq!(run(0.2, vec![button(false)]), 1);

        // Dragging pans instead
        run(2., vec![button(true)]);
        run(2.05, vec![Event::PointerMoved(center + vec2(80., 0.))]);
        assert_eq!(run(2.1, vec![button(false)]), 0);
    }

    #[test]
    fn a_video_waiting_for_its_first_frame_shows_the_spinner_only_after_a_delay() {
        let ctx = egui::Context::default();
        let mut canvas = ImageCanvas::new(ctx.clone(), false);
        let colors = theme::colors(Theme::Charcoal, Accent::Theme, true);
        let image = ImageSession::new();
        let settings = Settings::default();
        let mut video = VideoSession::default();
        video.state.active = true;
        video.state.loading = true;

        let viewport = Rect::from_min_size(Pos2::ZERO, vec2(1280., 720.));
        let mut shapes = || {
            let input = egui::RawInput {
                screen_rect: Some(viewport),
                ..Default::default()
            };
            let mut output = ctx.run_ui(input, |ui| {
                let content = CanvasContent {
                    image: &image,
                    video: &video,
                    settings: &settings,
                    colors,
                    can_navigate: false,
                    fullscreen: false,
                    subtitle_clearance: 0.,
                };
                canvas.show(ui, content, &mut Vec::new());
            });
            output.textures_delta.clear();

            output.shapes.len()
        };

        let quick = shapes();
        std::thread::sleep(std::time::Duration::from_millis(300));
        let slow = shapes();

        assert!(
            slow > quick,
            "No spinner after waiting: {quick} -> {slow} shapes"
        );
    }

    #[test]
    fn the_context_menu_stays_open_when_it_reveals_fullscreen_panels() {
        let ctx = egui::Context::default();
        let mut canvas = ImageCanvas::new(ctx.clone(), false);
        let colors = theme::colors(Theme::Charcoal, Accent::Theme, true);
        let image = ImageSession::new();
        let settings = Settings::default();
        let mut video = VideoSession::default();
        video.state.active = true;
        video.state.size = [1920, 1080];
        video.texture = Some(TextureId::User(7));

        let viewport = Rect::from_min_size(Pos2::ZERO, vec2(1280., 720.));
        let center = viewport.center();
        let mut run = |events: Vec<Event>| {
            let input = egui::RawInput {
                screen_rect: Some(viewport),
                events,
                ..Default::default()
            };

            let mut output = ctx.run_ui(input, |ui| {
                // Fullscreen shows the panels while any popup is open
                if egui::Popup::is_any_open(ui.ctx()) {
                    egui::Panel::top("toolbar").exact_size(58.).show(ui, |ui| {
                        ui.label("Toolbar");
                    });
                }

                let content = CanvasContent {
                    image: &image,
                    video: &video,
                    settings: &settings,
                    colors,
                    can_navigate: false,
                    fullscreen: true,
                    subtitle_clearance: 0.,
                };
                if let Some(response) = canvas.show(ui, content, &mut Vec::new()) {
                    response.context_menu(|ui| {
                        ui.label("Menu");
                    });
                }
            });
            output.textures_delta.clear();
        };
        let button = |pressed| Event::PointerButton {
            pos: center,
            button: PointerButton::Secondary,
            pressed,
            modifiers: Modifiers::NONE,
        };

        run(vec![Event::PointerMoved(center)]);
        run(vec![button(true)]);
        run(vec![button(false)]);

        for _ in 0..4 {
            run(Vec::new());
            assert!(egui::Popup::is_any_open(&ctx), "The context menu closed");
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
            formats::MediaKind,
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
                    kind: MediaKind::Video,
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

    /// Where the third thumbnail's caption lands with the strip docked or floating
    fn caption_pos(
        ctx: &egui::Context,
        strip: &mut Filmstrip,
        browser: &Browser,
        floating: bool,
        events: Vec<Event>,
    ) -> Pos2 {
        let colors = theme::colors(Theme::Charcoal, Accent::Theme, true);
        theme::apply(ctx, colors);
        let screen = Rect::from_min_size(Pos2::ZERO, vec2(640., 600.));
        let input = egui::RawInput {
            screen_rect: Some(screen),
            events,
            ..Default::default()
        };

        let mut output = ctx.run_ui(input, |ui| {
            if !floating {
                strip.show(ui, browser, colors, &mut Vec::new());
                return;
            }

            let top = screen.bottom() - crate::ui::filmstrip::STRIP_HEIGHT;
            let rect = Rect::from_x_y_ranges(screen.x_range(), top..=screen.bottom());
            strip.show_floating(ui, rect, browser, colors, &mut Vec::new());
        });
        output.textures_delta.clear();

        output
            .shapes
            .iter()
            .find_map(|shape| {
                let egui::Shape::Text(text) = &shape.shape else {
                    return None;
                };
                (text.galley.job.text == "clip-2.mp4").then_some(text.pos)
            })
            .expect("Third thumbnail should be visible")
    }

    #[test]
    fn thumbnails_stay_put_when_the_strip_starts_floating() {
        let mut browser = Browser::new();
        let entries = (0..20)
            .map(|index| {
                let name = format!("clip-{index}.mp4");
                Entry {
                    path: name.clone().into(),
                    name,
                    kind: MediaKind::Video,
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

        let ctx = egui::Context::default();
        let mut strip =
            Filmstrip::new(ctx.clone(), Settings::default().memory_budget());
        caption_pos(&ctx, &mut strip, &browser, false, Vec::new());

        // A scrolled strip keeps its scroll position too
        let wheel = Event::MouseWheel {
            unit: MouseWheelUnit::Point,
            delta: vec2(-40., 0.),
            phase: egui::TouchPhase::Move,
            source: egui::MouseWheelSource::Trackpad,
            modifiers: egui::Modifiers::NONE,
        };
        let events = vec![Event::PointerMoved(egui::pos2(100., 540.)), wheel];
        caption_pos(&ctx, &mut strip, &browser, false, events);

        // Smooth scrolling spreads the wheel over several frames
        for _ in 0..30 {
            caption_pos(&ctx, &mut strip, &browser, false, Vec::new());
        }
        let docked = caption_pos(&ctx, &mut strip, &browser, false, Vec::new());

        caption_pos(&ctx, &mut strip, &browser, true, Vec::new());
        let floating = caption_pos(&ctx, &mut strip, &browser, true, Vec::new());

        assert_eq!(docked, floating);
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

mod subtitles {
    use egui::{Pos2, Rect, vec2};

    use crate::{
        ui::subtitles::SubtitleOverlay,
        video::subtitles::{Cue, CueContent, Span, SubtitleCues, SubtitleText},
    };

    fn cue(text: &str, top: bool) -> Cue {
        Cue {
            start: 0.,
            end: 10.,
            content: CueContent::Text(SubtitleText {
                spans: vec![Span {
                    text: text.into(),
                    italic: false,
                }],
                top,
            }),
        }
    }

    #[test]
    fn text_cues_sit_inside_the_visible_picture_at_their_edge() {
        let ctx = egui::Context::default();
        let mut overlay = SubtitleOverlay::default();
        let mut cues = SubtitleCues::default();
        cues.insert(cue("Bottom line", false));
        cues.insert(cue("Top sign", true));
        let active = cues.active(1.);

        let screen = Rect::from_min_size(Pos2::ZERO, vec2(1280., 720.));
        let picture = Rect::from_min_size(Pos2::new(140., 0.), vec2(1000., 720.));
        let input = egui::RawInput {
            screen_rect: Some(screen),
            ..Default::default()
        };
        let mut output = ctx.run_ui(input, |ui| {
            overlay.paint(ui.painter(), picture, picture, f32::INFINITY, &active);
        });
        output.textures_delta.clear();

        let text_bounds: Vec<Rect> = output
            .shapes
            .iter()
            .filter(|shape| matches!(shape.shape, egui::Shape::Text(_)))
            .map(|shape| shape.shape.visual_bounding_rect())
            .collect();

        assert!(!text_bounds.is_empty(), "No subtitle text was painted");
        assert!(
            text_bounds
                .iter()
                .all(|bounds| picture.expand(4.).contains_rect(*bounds))
        );
        assert!(
            text_bounds
                .iter()
                .any(|bounds| bounds.top() > picture.center().y)
        );
        assert!(
            text_bounds
                .iter()
                .any(|bounds| bounds.bottom() < picture.center().y)
        );
    }
}

mod adjustments {
    use egui::{Event, Key, Modifiers, Pos2, Rect, vec2};

    use crate::{
        app::{Command, ViewCommand},
        settings::{Accent, Theme},
        ui::{adjustments, theme},
        viewer::adjustments::Adjustments,
    };

    fn key(key: Key) -> Event {
        Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Modifiers::NONE,
        }
    }

    fn frame(
        ctx: &egui::Context,
        current: Adjustments,
        events: Vec<Event>,
    ) -> Vec<Command> {
        let colors = theme::colors(Theme::Charcoal, Accent::Theme, true);
        let input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(400., 400.))),
            events,
            ..Default::default()
        };
        let mut commands = Vec::new();
        let mut output = ctx.run_ui(input, |ui| {
            adjustments::controls(ui, current, colors, &mut commands);
        });
        output.textures_delta.clear();

        commands
    }

    fn click(pos: Pos2) -> Vec<Event> {
        [true, false]
            .map(|pressed| Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: Modifiers::NONE,
            })
            .into_iter()
            .chain([Event::PointerMoved(pos)])
            .collect()
    }

    fn button_frame(ctx: &egui::Context, events: Vec<Event>) -> Rect {
        let colors = theme::colors(Theme::Charcoal, Accent::Theme, true);
        let input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(800., 600.))),
            events,
            ..Default::default()
        };
        let mut rect = Rect::NOTHING;
        let mut output = ctx.run_ui(input, |ui| {
            let current = Adjustments::NEUTRAL;
            rect = adjustments::button(ui, current, colors, &mut Vec::new()).rect;
        });
        output.textures_delta.clear();

        rect
    }

    #[test]
    fn clicks_inside_the_popup_keep_it_open_and_clicks_outside_close_it() {
        let ctx = egui::Context::default();
        let button = button_frame(&ctx, Vec::new());

        button_frame(&ctx, click(button.center()));
        button_frame(&ctx, Vec::new());
        assert!(egui::Popup::is_any_open(&ctx), "The popup did not open");

        // The first slider row sits just below the button
        button_frame(&ctx, click(button.center_bottom() + vec2(60., 20.)));
        button_frame(&ctx, Vec::new());
        assert!(
            egui::Popup::is_any_open(&ctx),
            "A click inside closed the popup"
        );

        button_frame(&ctx, click(Pos2::new(780., 580.)));
        button_frame(&ctx, Vec::new());
        assert!(
            !egui::Popup::is_any_open(&ctx),
            "A click outside left it open"
        );
    }

    /// Screen rects of the sliders, top to bottom, from the accessibility tree
    fn slider_rects(ctx: &egui::Context, current: Adjustments) -> Vec<Rect> {
        ctx.enable_accesskit();
        let colors = theme::colors(Theme::Charcoal, Accent::Theme, true);
        let input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(400., 400.))),
            ..Default::default()
        };
        let mut output = ctx.run_ui(input, |ui| {
            adjustments::controls(ui, current, colors, &mut Vec::new());
        });
        output.textures_delta.clear();

        let update = output
            .platform_output
            .accesskit_update
            .expect("Tree update");
        let mut rects: Vec<Rect> = update
            .nodes
            .iter()
            .filter(|(_, node)| node.role() == egui::accesskit::Role::Slider)
            .filter_map(|(_, node)| node.bounds())
            .map(|bounds| {
                Rect::from_min_max(
                    Pos2::new(bounds.x0 as f32, bounds.y0 as f32),
                    Pos2::new(bounds.x1 as f32, bounds.y1 as f32),
                )
            })
            .collect();
        rects.sort_by(|a, b| a.top().total_cmp(&b.top()));

        rects
    }

    fn double_click(pos: Pos2) -> Vec<Event> {
        let mut events = vec![Event::PointerMoved(pos)];
        events.extend(click(pos));
        events.extend(click(pos));

        events
    }

    #[test]
    fn double_clicking_a_slider_resets_only_that_value() {
        let ctx = egui::Context::default();
        let current = Adjustments {
            contrast: 1.4,
            gamma: 1.5,
            ..Adjustments::NEUTRAL
        };
        let sliders = slider_rects(&ctx, current);
        assert_eq!(sliders.len(), 4);

        // Contrast is the second row; clicking left of center first moves its value
        let commands = frame(
            &ctx,
            current,
            double_click(sliders[1].left_center() + vec2(20., 0.)),
        );

        assert_eq!(
            commands.last(),
            Some(&Command::View(ViewCommand::Adjust(Adjustments {
                contrast: 1.,
                ..current
            })))
        );
    }

    #[test]
    fn delete_on_a_focused_slider_resets_only_that_value() {
        let ctx = egui::Context::default();
        let current = Adjustments {
            brightness: 0.2,
            gamma: 1.5,
            ..Adjustments::NEUTRAL
        };

        assert!(frame(&ctx, current, Vec::new()).is_empty());

        // The brightness slider takes focus first
        frame(&ctx, current, vec![key(Key::Tab)]);
        let commands = frame(&ctx, current, vec![key(Key::Delete)]);

        assert_eq!(
            commands,
            vec![Command::View(ViewCommand::Adjust(Adjustments {
                brightness: 0.,
                ..current
            }))]
        );
    }
}

/// Renders UI pieces offscreen to PNG files, to look at layouts without the app
///
/// Run with `FLOWA_SCREENSHOTS=<folder> cargo test screenshots -- --ignored`
#[cfg(video_playback)]
mod screenshots {
    use std::{path::PathBuf, sync::mpsc, time::Duration};

    use anyhow::{Context, Result};
    use eframe::{
        egui_wgpu::{
            RenderState, RendererOptions, ScreenDescriptor, WgpuConfiguration,
            WgpuSetup,
        },
        wgpu,
    };
    use egui::{Pos2, Rect, vec2};

    use crate::{
        settings::{Accent, OverlayBackdrop, Theme},
        ui::{adjustments, theme, video_controls},
        video::VideoState,
        viewer::adjustments::Adjustments,
    };

    const SCALE: f32 = 1.5;

    fn graphics() -> Result<RenderState> {
        let setup = WgpuSetup::without_display_handle();
        let instance = pollster::block_on(setup.new_instance());
        let config = WgpuConfiguration {
            wgpu_setup: setup,
            ..Default::default()
        };

        Ok(pollster::block_on(RenderState::create(
            &config,
            &instance,
            None,
            RendererOptions::default(),
        ))?)
    }

    /// Draws `contents` in a window-sized pass and saves what egui painted
    fn screenshot(
        graphics: &RenderState,
        name: &str,
        size: [u32; 2],
        mut contents: impl FnMut(&mut egui::Ui),
    ) -> Result<()> {
        let Some(folder) = std::env::var_os("FLOWA_SCREENSHOTS") else {
            return Ok(());
        };
        let ctx = egui::Context::default();
        let colors = theme::colors(Theme::Charcoal, Accent::Theme, true);
        theme::apply(&ctx, colors);

        let points = vec2(size[0] as f32, size[1] as f32) / SCALE;
        let mut input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, points)),
            ..Default::default()
        };
        input
            .viewports
            .entry(egui::ViewportId::ROOT)
            .or_default()
            .native_pixels_per_point = Some(SCALE);

        // The second pass lays out with sizes measured in the first
        let mut output = ctx.run_ui(input.clone(), |ui| contents(ui));
        let first = std::mem::take(&mut output.textures_delta);
        output = ctx.run_ui(input, |ui| contents(ui));
        let primitives = ctx.tessellate(output.shapes, SCALE);

        let device = &graphics.device;
        let mut renderer = graphics.renderer.write();
        for (id, deltas) in first.set.iter().chain(&output.textures_delta.set) {
            for delta in deltas {
                renderer.update_texture(device, &graphics.queue, *id, delta);
            }
        }

        let target = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("screenshot"),
            size: wgpu::Extent3d {
                width: size[0],
                height: size[1],
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: graphics.target_format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = target.create_view(&wgpu::TextureViewDescriptor::default());
        let screen = ScreenDescriptor {
            size_in_pixels: size,
            pixels_per_point: SCALE,
        };
        let background = colors.canvas;

        let mut encoder = device.create_command_encoder(&Default::default());
        let mut buffers = renderer.update_buffers(
            device,
            &graphics.queue,
            &mut encoder,
            &primitives,
            &screen,
        );
        {
            let [r, g, b, _] = background.to_normalized_gamma_f32();
            let pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("screenshot"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: f64::from(r),
                            g: f64::from(g),
                            b: f64::from(b),
                            a: 1.,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            renderer.render(&mut pass.forget_lifetime(), &primitives, &screen);
        }
        drop(renderer);

        let mut first = first;
        first.clear();
        output.textures_delta.clear();

        let row_bytes = (size[0] * 4).div_ceil(256) * 256;
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("screenshot-readback"),
            size: u64::from(row_bytes * size[1]),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        encoder.copy_texture_to_buffer(
            target.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(row_bytes),
                    rows_per_image: Some(size[1]),
                },
            },
            target.size(),
        );
        buffers.push(encoder.finish());
        graphics.queue.submit(buffers);

        let (sender, receiver) = mpsc::channel();
        readback
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |result| {
                let _ = sender.send(result);
            });
        device.poll(wgpu::PollType::wait_indefinitely())?;
        receiver.recv_timeout(Duration::from_secs(5))??;

        let bytes = readback.slice(..).get_mapped_range()?;
        let bgra = matches!(
            graphics.target_format,
            wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Bgra8UnormSrgb
        );
        let mut pixels = Vec::with_capacity((size[0] * size[1] * 4) as usize);
        for row in bytes.chunks(row_bytes as usize) {
            for pixel in row[..(size[0] * 4) as usize].chunks(4) {
                let [a, b, c, d] = [pixel[0], pixel[1], pixel[2], pixel[3]];
                pixels.extend(if bgra { [c, b, a, d] } else { [a, b, c, d] });
            }
        }

        let image = image::RgbaImage::from_raw(size[0], size[1], pixels)
            .context("Wrong screenshot size")?;
        let path = PathBuf::from(folder).join(format!("{name}.png"));
        image.save(&path)?;

        Ok(())
    }

    fn popup(ui: &mut egui::Ui, contents: impl FnOnce(&mut egui::Ui)) {
        egui::Area::new(egui::Id::unique("popup"))
            .fixed_pos(Pos2::new(12., 12.))
            .show(ui.ctx(), |ui| {
                egui::Frame::popup(ui.style()).show(ui, contents);
            });
    }

    fn playing() -> VideoState {
        VideoState {
            active: true,
            position: 70.,
            duration: 7200.,
            rate: 1.5,
            ..Default::default()
        }
    }

    #[test]
    #[ignore = "Requires a GPU and FLOWA_SCREENSHOTS"]
    fn screenshots() -> Result<()> {
        let graphics = graphics()?;
        let colors = theme::colors(Theme::Charcoal, Accent::Theme, true);
        let changed = Adjustments {
            brightness: 0.1,
            gamma: 1.4,
            ..Adjustments::NEUTRAL
        };

        screenshot(&graphics, "adjustments", [720, 420], |ui| {
            popup(ui, |ui| {
                adjustments::controls(ui, changed, colors, &mut Vec::new())
            });
        })?;
        screenshot(&graphics, "speed-popup", [480, 280], |ui| {
            popup(ui, |ui| {
                video_controls::speed_options(
                    ui,
                    1.5,
                    crate::settings::SpeedPitch::Lower,
                    &mut Vec::new(),
                )
            });
        })?;

        let subtitles = {
            let mut cues = crate::video::subtitles::SubtitleCues::default();
            for (text, top) in [
                ("A line of dialogue that runs a little long", false),
                ("Sign", true),
            ] {
                cues.insert(crate::video::subtitles::Cue {
                    start: 0.,
                    end: 9.,
                    content: crate::video::subtitles::CueContent::Text(
                        crate::video::subtitles::SubtitleText {
                            spans: vec![crate::video::subtitles::Span {
                                text: text.into(),
                                italic: !top,
                            }],
                            top,
                        },
                    ),
                });
            }
            cues.active(1.)
        };
        let mut overlay = crate::ui::subtitles::SubtitleOverlay::default();
        screenshot(&graphics, "subtitles", [960, 540], |ui| {
            let picture = ui.max_rect();
            ui.painter().rect_filled(
                picture,
                0.,
                egui::Color32::from_rgb(170, 160, 140),
            );
            overlay.paint(ui.painter(), picture, picture, f32::INFINITY, &subtitles);
        })?;

        let tracked = VideoState {
            tracks: crate::video::Tracks {
                audio: ["English · AAC stereo", "Commentary · Japanese · AAC mono"]
                    .into_iter()
                    .enumerate()
                    .map(|(index, label)| crate::video::Track {
                        index: index + 1,
                        label: label.into(),
                        default: index == 0,
                        forced: false,
                        external: false,
                    })
                    .collect(),
                subtitles: vec![crate::video::Track {
                    index: 3,
                    label: "French · SRT · External".into(),
                    default: false,
                    forced: false,
                    external: true,
                }],
            },
            audio_track: Some(1),
            subtitle_track: Some(3),
            ..playing()
        };
        screenshot(&graphics, "tracks", [520, 300], |ui| {
            popup(ui, |ui| {
                crate::ui::tracks::menu(ui, &tracked, colors, &mut Vec::new())
            });
        })?;

        for width in [1200, 760] {
            let browser = crate::viewer::browser::Browser::new();
            let image = crate::viewer::image_session::ImageSession::new();
            let canvas =
                crate::ui::canvas::ImageCanvas::new(egui::Context::default(), false);
            let settings = crate::settings::Settings::default();
            let video = playing();
            screenshot(&graphics, &format!("toolbar-{width}"), [width, 120], |ui| {
                let state = crate::ui::toolbar::ToolbarState {
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
                crate::ui::toolbar::show(ui, &state, &mut Vec::new());
            })?;
        }

        for (name, compact, width) in [
            ("controls-compact", true, 1200),
            ("controls-compact-narrow", true, 640),
            ("controls-expanded", false, 1200),
            ("controls-expanded-narrow", false, 560),
        ] {
            let mut controls = video_controls::VideoControls::default();
            let state = playing();
            screenshot(&graphics, name, [width, 200], |ui| {
                controls.show(ui, &state, colors, compact, &mut Vec::new());
            })?;
        }

        for (name, theme_name, backdrop, width) in [
            (
                "overlay-black",
                Theme::Charcoal,
                OverlayBackdrop::Black,
                1200,
            ),
            (
                "overlay-black-light-video",
                Theme::Paper,
                OverlayBackdrop::Black,
                1200,
            ),
            (
                "overlay-theme",
                Theme::Charcoal,
                OverlayBackdrop::Theme,
                1200,
            ),
            (
                "overlay-theme-paper",
                Theme::Paper,
                OverlayBackdrop::Theme,
                1200,
            ),
            (
                "overlay-narrow",
                Theme::Charcoal,
                OverlayBackdrop::Black,
                480,
            ),
        ] {
            let mut controls = video_controls::VideoControls::default();
            let colors = theme::colors(theme_name, Accent::Theme, true);
            let bright = name.contains("light-video");
            screenshot(&graphics, name, [width, 420], |ui| {
                theme::apply(ui.ctx(), colors);
                let canvas = ui.max_rect();
                paint_fake_video(ui.painter(), canvas, bright);

                let now = std::time::Instant::now();
                controls.update_overlay(ui.ctx(), canvas, false, true, now);

                let content = video_controls::OverlayContent {
                    canvas,
                    state: &tracked,
                    colors,
                    backdrop,
                    fullscreen: false,
                    strip: None,
                };
                controls.show_overlay(ui.ctx(), content, &mut Vec::new());
            })?;
        }

        Ok(())
    }

    /// A sky over a pale beach, so the overlay's gradient is judged over bright and
    /// busy footage rather than a flat backdrop
    fn paint_fake_video(painter: &egui::Painter, rect: Rect, bright: bool) {
        let (sky, ground) = if bright {
            (
                egui::Color32::from_rgb(214, 232, 246),
                egui::Color32::from_rgb(247, 241, 226),
            )
        } else {
            (
                egui::Color32::from_rgb(70, 130, 190),
                egui::Color32::from_rgb(222, 196, 150),
            )
        };
        let horizon = rect.top() + rect.height() * 0.55;

        painter.rect_filled(rect, 0., sky);
        painter.rect_filled(
            Rect::from_min_max(egui::pos2(rect.left(), horizon), rect.max),
            0.,
            ground,
        );
        painter.circle_filled(
            egui::pos2(rect.left() + rect.width() * 0.7, rect.top() + 80.),
            36.,
            egui::Color32::from_rgb(255, 240, 180),
        );
    }
}
