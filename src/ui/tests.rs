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
            overlay.paint(ui.painter(), picture, picture, &active);
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

    #[test]
    fn delete_on_a_focused_slider_resets_only_that_value() {
        let ctx = egui::Context::default();
        let current = Adjustments {
            brightness: 0.2,
            gamma: 1.5,
            ..Adjustments::NEUTRAL
        };

        assert!(frame(&ctx, current, Vec::new()).is_empty());

        // The brightness reset button comes first, then its slider
        frame(&ctx, current, vec![key(Key::Tab)]);
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
        settings::{Accent, Theme},
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
        screenshot(&graphics, "speed-popup", [480, 220], |ui| {
            popup(ui, |ui| {
                video_controls::speed_options(ui, 1.5, &mut Vec::new())
            });
        })?;

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

        Ok(())
    }
}
