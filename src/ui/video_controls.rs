//! Compact or two-row video controls above the filmstrip

use std::time::{Duration, Instant};

use egui::{Align, Id, Layout, Response, RichText, style::HandleShape, vec2};

use crate::{
    app::{Command, PlaybackCommand},
    utils::format::media_time,
    video::{MAX_RATE, VideoState},
};

use super::{
    actions,
    icons::{self, Icon},
    style::{margin, size, space},
    theme::Colors,
    tracks,
};

const TIMELINE_HEIGHT: f32 = 22.;
const ROW_SPACING: f32 = space::XS;
const INLINE_VOLUME_WIDTH: f32 = 104.;
const INLINE_VOLUME_BREAKPOINT: f32 = 420.;
const SPEED_WIDTH: f32 = 58.;
const SPEED_SLIDER_WIDTH: f32 = 112.;
/// Speeds one click away in the speed popup
const SPEED_PRESETS: &[f64] = &[0.25, 0.5, 1., 1.5, 2.];
/// The part of the speed slider around its middle that snaps to normal speed
const SPEED_SNAP: f64 = 0.025;
/// Narrower compact rows leave speed to its shortcuts and the context menu
const COMPACT_SPEED_BREAKPOINT: f32 = 400.;
/// The speed popup's slider row and preset row share this width
const SPEED_POPUP_WIDTH: f32 = 250.;
const SEEK_ACKNOWLEDGEMENT_TIMEOUT: Duration = Duration::from_secs(1);

/// Keeps a scrub preview stable while FFmpeg seeks to the new frame
pub struct VideoControls {
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
    speed: Option<Id>,
}

#[derive(Clone, Copy)]
enum SliderKind {
    Timeline,
    Volume,
    Speed,
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
        compact: bool,
        commands: &mut Vec<Command>,
    ) {
        self.acknowledge_seek(state.position);

        let frame = egui::Frame::new()
            .fill(colors.panel)
            .inner_margin(margin::BOTTOM_PANEL);
        let transport_height = root.spacing().interact_size.y.max(size::CONTROL);
        let timeline_height = root
            .text_style_height(&egui::TextStyle::Body)
            .max(TIMELINE_HEIGHT);
        let content_height = transport_height
            + if compact {
                0.
            } else {
                timeline_height + ROW_SPACING
            };
        let separator_height = root
            .visuals()
            .widgets
            .noninteractive
            .bg_stroke
            .width
            .round();

        // Bottom panels need their final height before laying out the first frame
        egui::Panel::bottom("video-controls")
            .resizable(false)
            .exact_size(
                content_height + frame.total_margin().sum().y + separator_height,
            )
            .frame(frame)
            .show(root, |ui| {
                self.contents(ui, state, colors, compact, commands)
            });
    }

    fn contents(
        &mut self,
        ui: &mut egui::Ui,
        state: &VideoState,
        colors: Colors,
        compact: bool,
        commands: &mut Vec<Command>,
    ) {
        ui.spacing_mut().item_spacing = vec2(space::MD, ROW_SPACING);
        ui.spacing_mut().slider_rail_height = 3.;

        if compact {
            ui.horizontal(|ui| self.compact_row(ui, state, colors, commands));
            return;
        }

        ui.horizontal(|ui| self.timeline(ui, state, colors, commands));
        ui.horizontal(|ui| self.transport(ui, state, colors, commands));
    }

    fn timeline(
        &mut self,
        ui: &mut egui::Ui,
        state: &VideoState,
        colors: Colors,
        commands: &mut Vec<Command>,
    ) {
        ui.spacing_mut().interact_size.y = TIMELINE_HEIGHT;
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
        remember_slider(ui.ctx(), response.id, SliderKind::Timeline);

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
            commands.push(PlaybackCommand::Seek(position).into());
        }

        response.on_hover_text(format!("Seek · {}", media_time(position)));
    }

    fn compact_row(
        &mut self,
        ui: &mut egui::Ui,
        state: &VideoState,
        colors: Colors,
        commands: &mut Vec<Command>,
    ) {
        ui.add_enabled_ui(state.ready(), |ui| play_button(ui, state.paused, commands));

        let volume_menu = ui.available_width() < 640.;
        let show_speed = ui.available_width() >= COMPACT_SPEED_BREAKPOINT;
        let spacing = ui.spacing().item_spacing.x;
        let volume_width = if volume_menu {
            size::CONTROL
        } else {
            size::CONTROL + INLINE_VOLUME_WIDTH + spacing
        };
        let speed_width = if show_speed {
            SPEED_WIDTH + spacing
        } else {
            0.
        };
        let timeline_width =
            ui.available_width() - volume_width - speed_width - spacing;

        ui.allocate_ui_with_layout(
            vec2(timeline_width, size::CONTROL),
            Layout::left_to_right(Align::Center),
            |ui| self.timeline(ui, state, colors, commands),
        );

        if show_speed {
            ui.add_enabled_ui(state.ready(), |ui| speed_menu(ui, state.rate, commands));
        }

        self.volume_controls(ui, state, volume_menu, commands);
    }

    fn transport(
        &mut self,
        ui: &mut egui::Ui,
        state: &VideoState,
        colors: Colors,
        commands: &mut Vec<Command>,
    ) {
        let volume_menu = ui.available_width() < INLINE_VOLUME_BREAKPOINT;
        ui.add_enabled_ui(state.ready(), |ui| play_button(ui, state.paused, commands));

        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            self.volume_controls(ui, state, volume_menu, commands);
            ui.add_enabled_ui(state.ready(), |ui| {
                if volume_menu {
                    speed_menu(ui, state.rate, commands);
                } else {
                    speed_slider(ui, state.rate, commands);
                }
            });

            if tracks::available(state) {
                ui.add_enabled_ui(state.ready(), |ui| {
                    tracks::button(ui, state, colors, commands);
                });
            }
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
        let width = size::CONTROL + INLINE_VOLUME_WIDTH + ui.spacing().item_spacing.x;
        ui.allocate_ui_with_layout(
            vec2(width, size::CONTROL),
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
        let button = egui::Button::new("").min_size(vec2(size::CONTROL, size::CONTROL));
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
            commands.push(PlaybackCommand::SetVolume(volume).into());
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
    let verb = if paused { "Play" } else { "Pause" };

    actions::PLAY.button_label(ui, icon, verb, false, commands);
}

/// A button showing the current speed that opens the list of offered speeds
fn speed_menu(ui: &mut egui::Ui, rate: f64, commands: &mut Vec<Command>) {
    let label = RichText::new(rate_label(rate)).monospace();
    let button = egui::Button::new(label).min_size(vec2(SPEED_WIDTH, size::CONTROL));
    let (response, _) = egui::containers::menu::MenuButton::from_button(button)
        .ui(ui, |ui| speed_options(ui, rate, commands));

    let shortcuts = format!(
        "Playback speed · {} / {}",
        actions::SLOWER.shortcut_text(ui.ctx()),
        actions::FASTER.shortcut_text(ui.ctx())
    );
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::Role::Button, ui.is_enabled(), "Playback speed")
    });
    response.on_hover_text(shortcuts);
}

/// The speed slider and buttons for common speeds, for the speed button and context
/// menus
pub fn speed_options(ui: &mut egui::Ui, rate: f64, commands: &mut Vec<Command>) {
    ui.spacing_mut().slider_rail_height = 3.;
    ui.set_width(SPEED_POPUP_WIDTH);

    let slider_width = SPEED_POPUP_WIDTH - SPEED_WIDTH - ui.spacing().item_spacing.x;
    ui.horizontal(|ui| speed_slider_contents(ui, rate, slider_width, commands));
    ui.horizontal(|ui| speed_presets(ui, rate, commands));
}

fn speed_presets(ui: &mut egui::Ui, rate: f64, commands: &mut Vec<Command>) {
    ui.spacing_mut().item_spacing.x = space::XS;
    let gaps = ui.spacing().item_spacing.x * (SPEED_PRESETS.len() - 1) as f32;
    let width = (SPEED_POPUP_WIDTH - gaps) / SPEED_PRESETS.len() as f32;

    for &option in SPEED_PRESETS {
        let selected = (option - rate).abs() < 1e-6;
        let button = egui::Button::selectable(selected, rate_label(option));

        if ui.add_sized([width, 24.], button).clicked() {
            commands.push(PlaybackCommand::SetRate(option).into());
        }
    }
}

/// A slider from a standstill to the fastest speed, with normal speed in the middle,
/// and the speed beside it; double-clicking returns to normal speed
fn speed_slider(ui: &mut egui::Ui, rate: f64, commands: &mut Vec<Command>) {
    let spacing = ui.spacing().item_spacing.x;
    let width = size::ICON + SPEED_SLIDER_WIDTH + SPEED_WIDTH + spacing * 2.;

    // The right-to-left parent would reverse the slider and its reading
    ui.allocate_ui_with_layout(
        vec2(width, size::CONTROL),
        Layout::left_to_right(Align::Center),
        |ui| {
            speed_icon(ui);
            speed_slider_contents(ui, rate, SPEED_SLIDER_WIDTH, commands);
        },
    );
}

/// Marks the inline slider as speed, beside the similar volume slider
fn speed_icon(ui: &mut egui::Ui) {
    let (rect, response) =
        ui.allocate_exact_size(vec2(size::ICON, size::ICON), egui::Sense::hover());
    let color = ui.visuals().weak_text_color();
    icons::paint(ui.painter(), Icon::Speed, rect.center(), size::ICON, color);
    response.on_hover_text("Playback speed");
}

fn speed_slider_contents(
    ui: &mut egui::Ui,
    rate: f64,
    width: f32,
    commands: &mut Vec<Command>,
) {
    ui.spacing_mut().slider_width = width;
    ui.spacing_mut().interact_size.y = 22.;

    let mut position = rate_to_slider(rate);
    let response = ui.add(
        egui::Slider::new(&mut position, 0.0..=1.0)
            .show_value(false)
            .handle_shape(HandleShape::Circle),
    );
    response.widget_info(|| egui::WidgetInfo::slider(true, rate, "Playback speed"));
    remember_slider(ui.ctx(), response.id, SliderKind::Speed);

    let reading = RichText::new(format!("{rate:.2}×")).monospace();
    ui.add_sized([SPEED_WIDTH, 22.], egui::Label::new(reading));

    let shortcuts = format!(
        "Playback speed · {} / {} · double-click for normal speed",
        actions::SLOWER.shortcut_text(ui.ctx()),
        actions::FASTER.shortcut_text(ui.ctx())
    );
    let response = response.on_hover_text(shortcuts);

    if response.double_clicked() {
        commands.push(PlaybackCommand::SetRate(1.).into());
    } else if response.changed() {
        commands.push(PlaybackCommand::SetRate(slider_to_rate(position)).into());
    }
}

/// The left half of the slider covers a standstill to normal speed, the right half
/// normal to the fastest, so normal speed sits in the middle
fn slider_to_rate(position: f64) -> f64 {
    if (position - 0.5).abs() < SPEED_SNAP {
        return 1.;
    }

    let rate = if position <= 0.5 {
        position * 2.
    } else {
        1. + (position - 0.5) * 2. * (MAX_RATE - 1.)
    };

    (rate * 20.).round() / 20.
}

fn rate_to_slider(rate: f64) -> f64 {
    if rate <= 1. {
        rate / 2.
    } else {
        0.5 + (rate - 1.) / (MAX_RATE - 1.) / 2.
    }
}

/// Reads as `1×`, `1.5×`, or `0.25×`
fn rate_label(rate: f64) -> String {
    let rounded = format!("{rate:.2}");
    let trimmed = rounded.trim_end_matches('0').trim_end_matches('.');

    format!("{trimmed}×")
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
    remember_slider(ui.ctx(), response.id, SliderKind::Volume);

    if response.changed() {
        commands.push(PlaybackCommand::SetVolume(volume).into());
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
        size::ICON,
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

fn slider_focus_id() -> Id {
    Id::unique("video-slider-focus")
}

fn remember_slider(ctx: &egui::Context, id: Id, kind: SliderKind) {
    ctx.data_mut(|data| {
        let focus = data.get_temp_mut_or_default::<SliderFocus>(slider_focus_id());
        let slot = match kind {
            SliderKind::Timeline => &mut focus.timeline,
            SliderKind::Volume => &mut focus.volume,
            SliderKind::Speed => &mut focus.speed,
        };
        *slot = Some(id);
    });
}

/// Lets focused playback sliders consume arrows before global folder shortcuts
pub fn wants_keyboard_input(ctx: &egui::Context) -> bool {
    let Some(focused) = ctx.memory(|memory| memory.focused()) else {
        return false;
    };

    ctx.data(|data| {
        data.get_temp::<SliderFocus>(slider_focus_id())
            .is_some_and(|sliders| {
                [sliders.timeline, sliders.volume, sliders.speed]
                    .contains(&Some(focused))
            })
    })
}

#[cfg(test)]
mod tests {
    use super::{SliderFocus, VideoControls, slider_focus_id};
    use crate::{
        app::{BrowseCommand, Command, PlaybackCommand},
        settings::{Accent, Theme},
        ui::{keyboard, theme},
        video::VideoState,
    };
    use egui::{Event, Modifiers, PointerButton, Pos2, Rect, vec2};

    fn frame(
        ctx: &egui::Context,
        controls: &mut VideoControls,
        width: f32,
        events: Vec<Event>,
    ) -> (Vec<Command>, egui::Response) {
        frame_with_layout(ctx, controls, width, false, events)
    }

    fn frame_with_layout(
        ctx: &egui::Context,
        controls: &mut VideoControls,
        width: f32,
        compact: bool,
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

            controls.show(root, &state, colors, compact, &mut commands);
            controls_top = root.available_rect_before_wrap().bottom();
        });
        output.textures_delta.clear();

        let sliders = ctx
            .data(|data| data.get_temp::<SliderFocus>(slider_focus_id()))
            .expect("Timeline should be registered");
        let timeline = ctx
            .read_response(sliders.timeline.expect("Timeline ID"))
            .expect("Timeline response");

        assert!(timeline.rect.top() >= controls_top);
        assert!(
            timeline.rect.bottom() < strip_top,
            "Transport must sit above the filmstrip"
        );

        let height = strip_top - controls_top;
        if compact {
            assert!(height <= 60., "Compact controls took {height} points");
        } else {
            assert!(height > 70., "Expanded controls took {height} points");
        }

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
    fn compact_controls_fit_on_one_row_at_every_supported_width() {
        let ctx = egui::Context::default();
        let mut controls = VideoControls::default();

        for width in [320., 400., 640., 800., 1200., 1920.] {
            frame_with_layout(&ctx, &mut controls, width, true, Vec::new());
            let (_, timeline) =
                frame_with_layout(&ctx, &mut controls, width, true, Vec::new());
            assert!(
                timeline.rect.width() > 48.,
                "Timeline at {width}: {:?}",
                timeline.rect
            );
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
        assert_eq!(
            commands,
            vec![Command::Playback(PlaybackCommand::Seek(preview))]
        );

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

        assert!(commands.iter().any(|command| matches!(
            command,
            Command::Playback(PlaybackCommand::Seek(_))
        )));
        assert!(!commands.iter().any(|command| matches!(
            command,
            Command::Browse(BrowseCommand::Navigate(_))
        )));

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
        assert!(commands.contains(&Command::Playback(PlaybackCommand::TogglePlay)));
    }
}

#[cfg(test)]
mod speed_tests {
    use super::{rate_label, rate_to_slider, slider_to_rate};

    #[test]
    fn the_speed_slider_puts_normal_speed_in_the_middle_and_snaps_to_it() {
        assert_eq!(slider_to_rate(0.), 0.);
        assert_eq!(slider_to_rate(0.5), 1.);
        assert_eq!(slider_to_rate(1.), 4.);
        assert_eq!(slider_to_rate(0.51), 1.);
        assert_eq!(slider_to_rate(0.25), 0.5);

        for rate in [0., 0.25, 1., 1.5, 4.] {
            assert!((slider_to_rate(rate_to_slider(rate)) - rate).abs() < 1e-9);
        }
    }

    #[test]
    fn speed_labels_stay_short() {
        assert_eq!(rate_label(1.), "1×");
        assert_eq!(rate_label(1.5), "1.5×");
        assert_eq!(rate_label(0.25), "0.25×");
        assert_eq!(rate_label(1.350_000_000_1), "1.35×");
    }
}
