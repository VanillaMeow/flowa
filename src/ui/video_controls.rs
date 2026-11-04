//! Responsive video controls with a full-width timeline above the filmstrip

use std::time::{Duration, Instant};

use egui::{Align, Id, Layout, Response, RichText, style::HandleShape, vec2};

use crate::{app::Command, utils::format::media_time, video::VideoState};

use super::{
    icons::{self, Icon},
    theme::Colors,
};

const CONTROL_SIZE: f32 = 34.;
const INLINE_VOLUME_WIDTH: f32 = 104.;
const INLINE_VOLUME_BREAKPOINT: f32 = 420.;
const SEEK_ACKNOWLEDGEMENT_TIMEOUT: Duration = Duration::from_secs(1);

/// Keeps a scrub preview stable while libmpv asynchronously seeks to the new frame
pub(crate) struct VideoControls {
    seek: Option<SeekPreview>,
    audible_volume: f64,
}

impl Default for VideoControls {
    fn default() -> Self {
        Self {
            seek: None,
            audible_volume: 100.,
        }
    }
}

struct SeekPreview {
    seconds: f64,
    submitted: Option<Instant>,
}

/// Identifies sliders whose arrow keys must not trigger folder navigation
#[derive(Clone, Default)]
struct SliderFocus {
    timeline: Option<Id>,
    volume: Option<Id>,
}

impl VideoControls {
    /// Drops a preview when a different file is selected
    pub fn reset(&mut self) {
        self.seek = None;
    }

    /// Reserves only the height required by the current fonts and control layout
    pub fn show(
        &mut self,
        root: &mut egui::Ui,
        state: &VideoState,
        colors: Colors,
        fullscreen: bool,
        commands: &mut Vec<Command>,
    ) {
        self.acknowledge_seek(state.position);

        let frame = egui::Frame::new()
            .fill(colors.panel)
            .inner_margin(egui::Margin::symmetric(18, 10));

        egui::Panel::bottom("video-controls")
            .resizable(false)
            .frame(frame)
            .show(root, |ui| {
                self.contents(ui, state, colors, fullscreen, commands)
            });
    }

    fn contents(
        &mut self,
        ui: &mut egui::Ui,
        state: &VideoState,
        colors: Colors,
        fullscreen: bool,
        commands: &mut Vec<Command>,
    ) {
        ui.spacing_mut().item_spacing = vec2(12., 4.);
        ui.spacing_mut().slider_rail_height = 3.;
        ui.horizontal(|ui| self.timeline(ui, state, colors, commands));
        ui.horizontal(|ui| self.transport(ui, state, colors, fullscreen, commands));
    }

    fn timeline(
        &mut self,
        ui: &mut egui::Ui,
        state: &VideoState,
        colors: Colors,
        commands: &mut Vec<Command>,
    ) {
        ui.spacing_mut().interact_size.y = 22.;
        let position = self
            .seek
            .as_ref()
            .map_or(state.position, |seek| seek.seconds);
        let elapsed = media_time(position);
        let duration = if state.duration > 0. {
            media_time(state.duration)
        } else {
            "--:--".into()
        };
        let time_width = text_width(ui, &elapsed).max(text_width(ui, &duration));
        let spacing = ui.spacing().item_spacing.x;

        time_label(ui, &elapsed, time_width, colors);

        // Measure the labels first so the timeline takes the actual remaining space
        let track_width = ui.available_width() - time_width - spacing;
        ui.scope(|ui| {
            ui.spacing_mut().slider_width = track_width;
            self.seek_slider(ui, state, position, commands);
        });

        time_label(ui, &duration, time_width, colors);
    }

    fn seek_slider(
        &mut self,
        ui: &mut egui::Ui,
        state: &VideoState,
        mut position: f64,
        commands: &mut Vec<Command>,
    ) {
        let duration = state.duration.max(0.);
        let enabled = state.ready() && duration > 0.;
        position = position.clamp(0., duration);

        let response = ui.add_enabled(
            enabled,
            egui::Slider::new(&mut position, 0.0..=duration)
                .show_value(false)
                .handle_shape(HandleShape::Circle),
        );
        response.widget_info(|| {
            egui::WidgetInfo::slider(enabled, position, "Playback position")
        });
        remember_slider(ui.ctx(), response.id, true);

        if response.changed() {
            self.seek = Some(SeekPreview {
                seconds: position,
                submitted: None,
            });
        }

        // Mouse drags seek once on release; keyboard and track clicks apply immediately
        if response.drag_stopped() || response.changed() && !response.dragged() {
            self.seek = Some(SeekPreview {
                seconds: position,
                submitted: Some(Instant::now()),
            });
            commands.push(Command::SeekVideo(position));
        }

        response.on_hover_text(format!("Seek · {}", media_time(position)));
    }

    fn transport(
        &mut self,
        ui: &mut egui::Ui,
        state: &VideoState,
        colors: Colors,
        fullscreen: bool,
        commands: &mut Vec<Command>,
    ) {
        let compact = ui.available_width() < INLINE_VOLUME_BREAKPOINT;

        ui.add_enabled_ui(state.ready(), |ui| play_button(ui, state.paused, commands));
        playback_status(ui, state, colors);

        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            fullscreen_button(ui, fullscreen, commands);
            self.volume_controls(ui, state, compact, commands);
        });
    }

    fn volume_controls(
        &mut self,
        ui: &mut egui::Ui,
        state: &VideoState,
        compact: bool,
        commands: &mut Vec<Command>,
    ) {
        if state.volume > 0. {
            self.audible_volume = state.volume;
        }

        if compact {
            self.volume_menu(ui, state, commands);
            return;
        }

        // The nested group keeps the volume slider natural in the trailing controls
        let width = CONTROL_SIZE + INLINE_VOLUME_WIDTH + ui.spacing().item_spacing.x;
        ui.allocate_ui_with_layout(
            vec2(width, CONTROL_SIZE),
            Layout::left_to_right(Align::Center),
            |ui| {
                self.mute_button(ui, state, commands);
                volume_slider(ui, state.volume, INLINE_VOLUME_WIDTH, commands);
            },
        );
    }

    fn volume_menu(
        &mut self,
        ui: &mut egui::Ui,
        state: &VideoState,
        commands: &mut Vec<Command>,
    ) {
        let button = egui::Button::new("").min_size(vec2(CONTROL_SIZE, CONTROL_SIZE));
        let (response, _) =
            egui::containers::menu::MenuButton::from_button(button).ui(ui, |ui| {
                ui.label(format!("Volume · {:.0}%", state.volume));
                ui.horizontal(|ui| {
                    self.mute_button(ui, state, commands);
                    volume_slider(ui, state.volume, INLINE_VOLUME_WIDTH, commands);
                });
            });

        paint_button_icon(ui, &response, volume_icon(state.volume));
        response.widget_info(|| {
            egui::WidgetInfo::labeled(egui::Role::Button, true, "Volume")
        });
        response.on_hover_text("Volume");
    }

    fn mute_button(
        &self,
        ui: &mut egui::Ui,
        state: &VideoState,
        commands: &mut Vec<Command>,
    ) {
        let label = if state.volume == 0. { "Unmute" } else { "Mute" };

        if icons::button(ui, volume_icon(state.volume), label, false).clicked() {
            let volume = if state.volume == 0. {
                self.audible_volume.max(1.)
            } else {
                0.
            };
            commands.push(Command::SetVideoVolume(volume));
        }
    }

    fn acknowledge_seek(&mut self, position: f64) {
        let Some(seek) = &self.seek else {
            return;
        };
        let Some(submitted) = seek.submitted else {
            return;
        };

        if (position - seek.seconds).abs() < 0.25
            || submitted.elapsed() >= SEEK_ACKNOWLEDGEMENT_TIMEOUT
        {
            self.seek = None;
        }
    }
}

fn play_button(ui: &mut egui::Ui, paused: bool, commands: &mut Vec<Command>) {
    let icon = if paused { Icon::Play } else { Icon::Pause };
    let label = if paused {
        "Play · Space"
    } else {
        "Pause · Space"
    };

    if icons::button(ui, icon, label, false).clicked() {
        commands.push(Command::TogglePlay);
    }
}

fn fullscreen_button(ui: &mut egui::Ui, fullscreen: bool, commands: &mut Vec<Command>) {
    if icons::button(ui, Icon::Fullscreen, "Fullscreen · F11", fullscreen).clicked() {
        commands.push(Command::ToggleFullscreen);
    }
}

fn playback_status(ui: &mut egui::Ui, state: &VideoState, colors: Colors) {
    let label = if state.error.is_some() {
        "Playback unavailable"
    } else if state.loading {
        "Opening video…"
    } else if state.paused {
        "Paused"
    } else {
        "Playing"
    };

    ui.label(RichText::new(label).small().color(colors.muted));
}

fn volume_slider(
    ui: &mut egui::Ui,
    mut volume: f64,
    width: f32,
    commands: &mut Vec<Command>,
) {
    ui.spacing_mut().slider_width = width;
    ui.spacing_mut().interact_size.y = 22.;
    let response = ui.add(
        egui::Slider::new(&mut volume, 0.0..=100.0)
            .show_value(false)
            .handle_shape(HandleShape::Circle),
    );
    response.widget_info(|| egui::WidgetInfo::slider(true, volume, "Volume"));
    remember_slider(ui.ctx(), response.id, false);

    if response.changed() {
        commands.push(Command::SetVideoVolume(volume));
    }

    response.on_hover_text(format!("Volume · {volume:.0}%"));
}

fn volume_icon(volume: f64) -> Icon {
    if volume == 0. {
        Icon::Muted
    } else {
        Icon::Volume
    }
}

fn paint_button_icon(ui: &egui::Ui, response: &Response, icon: Icon) {
    icons::paint(
        ui.painter(),
        icon,
        response.rect.center(),
        18.,
        ui.visuals().text_color(),
    );
}

fn text_width(ui: &egui::Ui, text: &str) -> f32 {
    ui.painter()
        .layout_no_wrap(
            text.into(),
            egui::TextStyle::Monospace.resolve(ui.style()),
            ui.visuals().text_color(),
        )
        .size()
        .x
}

fn time_label(ui: &mut egui::Ui, text: &str, width: f32, colors: Colors) {
    ui.add_sized(
        [width, 22.],
        egui::Label::new(RichText::new(text).monospace().color(colors.muted)),
    );
}

fn remember_slider(ctx: &egui::Context, id: Id, timeline: bool) {
    ctx.data_mut(|data| {
        let focus = data
            .get_temp_mut_or_default::<SliderFocus>(Id::unique("video-slider-focus"));
        if timeline {
            focus.timeline = Some(id);
        } else {
            focus.volume = Some(id);
        }
    });
}

/// Lets focused playback sliders consume arrows before global folder shortcuts
pub(crate) fn wants_keyboard_input(ctx: &egui::Context) -> bool {
    let Some(focused) = ctx.memory(|memory| memory.focused()) else {
        return false;
    };

    ctx.data(|data| {
        data.get_temp::<SliderFocus>(Id::unique("video-slider-focus"))
            .is_some_and(|sliders| {
                sliders.timeline == Some(focused) || sliders.volume == Some(focused)
            })
    })
}

#[cfg(test)]
mod tests {
    use super::{SliderFocus, VideoControls};
    use crate::{
        app::Command,
        settings::{Accent, Theme},
        ui::{keyboard, theme},
        video::VideoState,
    };
    use egui::{Event, Id, Modifiers, PointerButton, Pos2, Rect, vec2};

    fn frame(
        ctx: &egui::Context,
        controls: &mut VideoControls,
        width: f32,
        events: Vec<Event>,
    ) -> (Vec<Command>, egui::Response) {
        let input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(width, 600.))),
            events,
            ..Default::default()
        };
        let colors = theme::colors(Theme::Charcoal, Accent::Theme, true);
        let state = VideoState {
            active: true,
            position: 70.,
            duration: 7200.,
            ..Default::default()
        };
        theme::apply(ctx, colors);

        let mut commands = Vec::new();
        let mut strip_top = 0.;
        let mut controls_top = 0.;
        let mut output = ctx.run_ui(input, |root| {
            commands.extend(keyboard::read(ctx, false, false));

            let strip = egui::Panel::bottom("test-filmstrip").exact_size(116.).show(
                root,
                |ui| {
                    ui.label("Filmstrip");
                },
            );
            strip_top = strip.response.rect.top();

            controls.show(root, &state, colors, false, &mut commands);
            controls_top = root.available_rect_before_wrap().bottom();
        });
        output.textures_delta.clear();

        let sliders = ctx
            .data(|data| data.get_temp::<SliderFocus>(Id::unique("video-slider-focus")))
            .expect("Timeline should be registered");
        let timeline = ctx
            .read_response(sliders.timeline.expect("Timeline ID"))
            .expect("Timeline response");

        assert!(timeline.rect.top() >= controls_top);
        assert!(
            timeline.rect.bottom() < strip_top,
            "Transport must sit above the filmstrip"
        );

        for shape in output.shapes {
            let bounds = shape.shape.visual_bounding_rect();

            if bounds.is_finite() && bounds.is_positive() {
                assert!(
                    bounds.left() >= -1. && bounds.right() <= width + 1.,
                    "Paint overflow at width {width}: {bounds:?}"
                );
            }
        }

        (commands, timeline)
    }

    #[test]
    fn controls_fit_narrow_and_wide_windows_and_timeline_uses_remaining_width() {
        let ctx = egui::Context::default();
        let mut controls = VideoControls::default();
        let mut previous_width = 0.;

        // 320 points also covers a 640-pixel window at 200 percent UI scale
        for width in [320., 400., 640., 1200., 1920.] {
            frame(&ctx, &mut controls, width, Vec::new());
            let (_, timeline) = frame(&ctx, &mut controls, width, Vec::new());
            assert!(timeline.rect.width() > previous_width);
            assert!(timeline.rect.width() > width - 200.);
            previous_width = timeline.rect.width();
        }
    }

    #[test]
    fn scrubbing_keeps_preview_and_seeks_once_on_release() {
        let ctx = egui::Context::default();
        let mut controls = VideoControls::default();
        let (_, timeline) = frame(&ctx, &mut controls, 800., Vec::new());
        let start = timeline.rect.center();
        let end = start + vec2(70., 0.);
        let pointer = |pos, pressed| Event::PointerButton {
            pos,
            button: PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        };

        let (commands, _) = frame(
            &ctx,
            &mut controls,
            800.,
            vec![Event::PointerMoved(start), pointer(start, true)],
        );
        assert!(commands.is_empty());

        let (commands, _) =
            frame(&ctx, &mut controls, 800., vec![Event::PointerMoved(end)]);
        assert!(commands.is_empty());
        let preview = controls.seek.as_ref().expect("Scrub preview").seconds;
        assert!(preview > 3600.);

        let (commands, _) = frame(&ctx, &mut controls, 800., vec![pointer(end, false)]);
        assert_eq!(commands, vec![Command::SeekVideo(preview)]);

        frame(&ctx, &mut controls, 800., Vec::new());
        assert_eq!(
            controls.seek.as_ref().expect("Pending seek").seconds,
            preview
        );

        controls.acknowledge_seek(preview);
        assert!(controls.seek.is_none());
    }

    #[test]
    fn focused_timeline_arrows_seek_without_navigating_the_folder() {
        let ctx = egui::Context::default();
        let mut controls = VideoControls::default();
        let (_, timeline) = frame(&ctx, &mut controls, 800., Vec::new());
        ctx.memory_mut(|memory| memory.request_focus(timeline.id));

        let (commands, _) = frame(
            &ctx,
            &mut controls,
            800.,
            vec![Event::Key {
                key: egui::Key::ArrowRight,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: Modifiers::NONE,
            }],
        );

        assert!(
            commands
                .iter()
                .any(|command| matches!(command, Command::SeekVideo(_)))
        );
        assert!(
            !commands
                .iter()
                .any(|command| matches!(command, Command::Navigate(_)))
        );

        let (commands, _) = frame(
            &ctx,
            &mut controls,
            800.,
            vec![Event::Key {
                key: egui::Key::Space,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: Modifiers::NONE,
            }],
        );
        assert!(commands.contains(&Command::TogglePlay));
    }
}
