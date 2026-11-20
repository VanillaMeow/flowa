//! Compact or two-row video controls above the filmstrip

use std::time::{Duration, Instant};

use strum::VariantArray;

use egui::{
    Align, Align2, Id, Layout, Order, Pos2, Rect, Response, RichText,
    style::HandleShape, vec2,
};

use crate::{
    app::{Command, PlaybackCommand},
    settings::SpeedPitch,
    utils::format::{media_time, precise_media_time},
    video::{MAX_RATE, VideoState},
};

use super::{
    actions,
    icons::{self, Icon},
    style::{margin, size, space, text},
    theme::Colors,
    tracks, widgets,
};

const TIMELINE_HEIGHT: f32 = 22.;
const ROW_SPACING: f32 = space::XS;
const INLINE_VOLUME_WIDTH: f32 = 104.;
const INLINE_VOLUME_BREAKPOINT: f32 = 420.;
/// Every speed reading is at most this wide in the monospace font, so speed controls
/// keep one width and nothing beside them moves while the speed changes
const WIDEST_RATE: &str = "0.00×";
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
/// Space between the timeline and the time shown above the pointer
const SEEK_HINT_GAP: f32 = space::XS;

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
                .handle_shape(HandleShape::Circle)
                // Clicks land exactly on the time shown above the pointer
                .smart_aim(false),
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

        if enabled {
            seek_hint(ui.ctx(), &response, duration);
        }
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
            speed_width(ui) + spacing
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
            ui.add_enabled_ui(state.ready(), |ui| {
                speed_menu(ui, state.rate, state.pitch, commands)
            });
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
                    speed_menu(ui, state.rate, state.pitch, commands);
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
        let (response, _) = egui::containers::menu::MenuButton::from_button(button)
            .config(widgets::adjusting_menu())
            .ui(ui, |ui| {
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
fn speed_menu(
    ui: &mut egui::Ui,
    rate: f64,
    pitch: SpeedPitch,
    commands: &mut Vec<Command>,
) {
    let label = RichText::new(rate_label(rate)).monospace();
    let button =
        egui::Button::new(label).min_size(vec2(speed_width(ui), size::CONTROL));
    let (response, _) = egui::containers::menu::MenuButton::from_button(button)
        .config(widgets::adjusting_menu())
        .ui(ui, |ui| speed_options(ui, rate, pitch, commands));

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

/// The pitch choice, buttons for common speeds, and the speed slider, for the speed
/// button and context menus; the slider ends up nearest the speed button below
pub fn speed_options(
    ui: &mut egui::Ui,
    rate: f64,
    pitch: SpeedPitch,
    commands: &mut Vec<Command>,
) {
    ui.spacing_mut().slider_rail_height = 3.;
    ui.set_width(SPEED_POPUP_WIDTH);

    let slider_width =
        SPEED_POPUP_WIDTH - speed_width(ui) - ui.spacing().item_spacing.x;
    ui.horizontal(|ui| pitch_options(ui, pitch, commands));
    ui.horizontal(|ui| speed_presets(ui, rate, commands));
    ui.horizontal(|ui| speed_slider_contents(ui, rate, slider_width, commands));
}

/// How speed affects pitch, as a compact row of the choices in Preferences
fn pitch_options(ui: &mut egui::Ui, pitch: SpeedPitch, commands: &mut Vec<Command>) {
    ui.spacing_mut().item_spacing.x = space::XS;

    // The label centers on the buttons only once the row is their height
    ui.set_min_height(24.);

    let label = ui.label(RichText::new("Pitch").size(text::LABEL).weak());
    let gaps = ui.spacing().item_spacing.x * SpeedPitch::VARIANTS.len() as f32;
    let width = (SPEED_POPUP_WIDTH - label.rect.width() - gaps)
        / SpeedPitch::VARIANTS.len() as f32;

    for &option in SpeedPitch::VARIANTS {
        let button = egui::Button::selectable(option == pitch, option.short_label())
            .min_size(vec2(width, 24.));
        let response = ui.add(button).on_hover_text(option.description());

        if response.clicked() && option != pitch {
            commands.push(PlaybackCommand::SetPitch(option).into());
        }
    }
}

fn speed_presets(ui: &mut egui::Ui, rate: f64, commands: &mut Vec<Command>) {
    ui.spacing_mut().item_spacing.x = space::XS;
    let gaps = ui.spacing().item_spacing.x * (SPEED_PRESETS.len() - 1) as f32;
    let width = (SPEED_POPUP_WIDTH - gaps) / SPEED_PRESETS.len() as f32;

    for &option in SPEED_PRESETS {
        let selected = (option - rate).abs() < 1e-6;
        let button = egui::Button::selectable(selected, rate_label(option))
            .min_size(vec2(width, 24.));

        if ui.add(button).clicked() {
            commands.push(PlaybackCommand::SetRate(option).into());
        }
    }
}

/// A slider from a standstill to the fastest speed, with normal speed in the middle,
/// and the speed beside it; double-clicking returns to normal speed
fn speed_slider(ui: &mut egui::Ui, rate: f64, commands: &mut Vec<Command>) {
    let spacing = ui.spacing().item_spacing.x;
    let width = size::ICON + SPEED_SLIDER_WIDTH + speed_width(ui) + spacing * 2.;

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

    speed_reading(ui, rate);

    let shortcuts = format!(
        "Playback speed · {} / {} · double-click for normal speed",
        actions::SLOWER.shortcut_text(ui.ctx()),
        actions::FASTER.shortcut_text(ui.ctx())
    );
    let response = response.on_hover_text(shortcuts);

    if widgets::double_clicked(ui, &response) {
        commands.push(PlaybackCommand::SetRate(1.).into());
    } else if response.changed() {
        commands.push(PlaybackCommand::SetRate(slider_to_rate(position)).into());
    }
}

/// The speed beside its slider, painted into a fixed width so dragging the slider never
/// moves it
fn speed_reading(ui: &mut egui::Ui, rate: f64) {
    let (rect, _) =
        ui.allocate_exact_size(vec2(speed_width(ui), 22.), egui::Sense::hover());
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        format!("{rate:.2}×"),
        egui::TextStyle::Monospace.resolve(ui.style()),
        ui.visuals().text_color(),
    );
}

/// The width of the speed button and the reading beside the speed slider
fn speed_width(ui: &egui::Ui) -> f32 {
    let padding = ui.spacing().button_padding.x * 2.;

    (text_width(ui, WIDEST_RATE) + padding).ceil()
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

/// Shows the time under the pointer centered above it while the timeline is hovered
/// or dragged
fn seek_hint(ctx: &egui::Context, response: &Response, duration: f64) {
    let Some(pointer) = response.hover_pos().or(response.interact_pointer_pos()) else {
        return;
    };

    let (x, seconds) = seek_target(response.rect, pointer.x, duration);
    let time = precise_media_time(seconds);

    let anchor = Pos2::new(x, response.rect.top() - SEEK_HINT_GAP);
    egui::Area::new(response.id.with("seek-hint"))
        .order(Order::Tooltip)
        .fixed_pos(anchor)
        .pivot(Align2::CENTER_BOTTOM)
        .interactable(false)
        .show(ctx, |ui| {
            egui::Frame::popup(ui.style())
                .show(ui, |ui| ui.label(RichText::new(time).monospace()));
        });
}

/// Where the timeline handle would sit for a pointer at `pointer_x`, and the time
/// clicking there seeks to
fn seek_target(rect: Rect, pointer_x: f32, duration: f64) -> (f32, f64) {
    // Egui's slider insets the handle's center from the ends by its radius
    let handle_radius = rect.height() / 2.5;
    let rail = rect.x_range().shrink(handle_radius);

    let x = pointer_x.clamp(rail.min, rail.max);
    let fraction = f64::from((x - rail.min) / rail.span());

    (x, fraction * duration)
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
    use super::{SliderFocus, VideoControls, seek_target, slider_focus_id};
    use crate::{
        app::{BrowseCommand, Command, PlaybackCommand},
        settings::{Accent, Theme},
        ui::style::{size, space},
        ui::{keyboard, theme},
        video::MAX_RATE,
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
        let state = VideoState {
            active: true,
            position: 70.,
            duration: 7200.,
            ..Default::default()
        };

        frame_with_state(ctx, controls, width, compact, &state, events)
    }

    fn frame_with_state(
        ctx: &egui::Context,
        controls: &mut VideoControls,
        width: f32,
        compact: bool,
        state: &VideoState,
        events: Vec<Event>,
    ) -> (Vec<Command>, egui::Response) {
        let input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(width, 600.))),
            events,
            ..Default::default()
        };
        let colors = theme::colors(Theme::Charcoal, Accent::Theme, true);
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

            controls.show(root, state, colors, compact, &mut commands);
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

    /// Rects of the timeline, speed, and volume sliders, whichever are shown
    fn slider_rects(ctx: &egui::Context) -> Vec<Option<Rect>> {
        let sliders = ctx
            .data(|data| data.get_temp::<SliderFocus>(slider_focus_id()))
            .unwrap_or_default();

        [sliders.timeline, sliders.speed, sliders.volume]
            .into_iter()
            .map(|id| {
                id.and_then(|id| ctx.read_response(id))
                    .map(|response| response.rect)
            })
            .collect()
    }

    #[test]
    fn changing_speed_never_moves_the_controls() {
        let ctx = egui::Context::default();
        let mut controls = VideoControls::default();

        for (width, compact) in [
            (420., true),
            (800., true),
            (1200., true),
            (600., false),
            (1200., false),
        ] {
            let mut expected = None;

            for rate in [1., 0., 0.05, 0.25, 0.5, 0.95, 1.35, 2., 2.55, 4.] {
                let state = VideoState {
                    active: true,
                    position: 70.,
                    duration: 7200.,
                    rate,
                    ..Default::default()
                };
                ctx.data_mut(|data| data.remove_temp::<SliderFocus>(slider_focus_id()));
                frame_with_state(
                    &ctx,
                    &mut controls,
                    width,
                    compact,
                    &state,
                    Vec::new(),
                );
                frame_with_state(
                    &ctx,
                    &mut controls,
                    width,
                    compact,
                    &state,
                    Vec::new(),
                );
                let rects = slider_rects(&ctx);

                let expected = expected.get_or_insert_with(|| rects.clone());
                assert_eq!(
                    *expected, rects,
                    "Width {width}, compact {compact}, rate {rate}"
                );
            }
        }
    }

    #[test]
    fn dragging_the_speed_popup_slider_keeps_it_under_the_pointer() {
        let ctx = egui::Context::default();
        let mut controls = VideoControls::default();
        let mut state = VideoState {
            active: true,
            position: 70.,
            duration: 7200.,
            ..Default::default()
        };
        let pointer = |pos, pressed| Event::PointerButton {
            pos,
            button: PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        };
        let show = |controls: &mut VideoControls, state: &mut VideoState, events| {
            let (commands, _) =
                frame_with_state(&ctx, controls, 800., true, state, events);

            for command in commands {
                if let Command::Playback(PlaybackCommand::SetRate(rate)) = command {
                    state.rate = rate;
                }
            }

            slider_rects(&ctx)
        };

        // The speed button sits just before the mute button and volume slider
        let volume = show(&mut controls, &mut state, Vec::new())[2].expect("Volume");
        let button =
            volume.left_center() - vec2(space::MD * 2. + size::CONTROL + 10., 0.);
        show(
            &mut controls,
            &mut state,
            vec![Event::PointerMoved(button), pointer(button, true)],
        );
        show(&mut controls, &mut state, vec![pointer(button, false)]);

        // Popups settle where they fit after measuring themselves for a few frames
        for _ in 0..3 {
            show(&mut controls, &mut state, Vec::new());
        }
        let slider =
            show(&mut controls, &mut state, Vec::new())[1].expect("Popup slider");

        let start = slider.left_center();
        show(
            &mut controls,
            &mut state,
            vec![Event::PointerMoved(start), pointer(start, true)],
        );

        let mut previous = state.rate;
        for step in 1..=40 {
            let x = start.x + slider.width() * step as f32 / 40.;
            let rects = show(
                &mut controls,
                &mut state,
                vec![Event::PointerMoved(Pos2::new(x, start.y))],
            );

            assert_eq!(
                rects[1],
                Some(slider),
                "Slider moved at speed {}",
                state.rate
            );
            assert!(state.rate >= previous, "Speed fell to {}", state.rate);
            previous = state.rate;
        }

        assert_eq!(state.rate, MAX_RATE);
    }

    #[test]
    fn double_clicking_the_speed_slider_returns_to_normal_speed() {
        let ctx = egui::Context::default();
        let mut controls = VideoControls::default();
        let state = VideoState {
            active: true,
            position: 70.,
            duration: 7200.,
            rate: 2.,
            ..Default::default()
        };
        frame_with_state(&ctx, &mut controls, 1200., false, &state, Vec::new());
        let slider = slider_rects(&ctx)[1].expect("Speed slider");

        let pos = slider.left_center() + vec2(10., 0.);
        let mut events = vec![Event::PointerMoved(pos)];
        for pressed in [true, false, true, false] {
            events.push(Event::PointerButton {
                pos,
                button: PointerButton::Primary,
                pressed,
                modifiers: Modifiers::NONE,
            });
        }
        let (commands, _) =
            frame_with_state(&ctx, &mut controls, 1200., false, &state, events);

        assert_eq!(
            commands.last(),
            Some(&Command::Playback(PlaybackCommand::SetRate(1.)))
        );
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
    fn seek_hint_names_the_time_a_click_seeks_to() {
        let ctx = egui::Context::default();
        let mut controls = VideoControls::default();
        let (_, timeline) = frame(&ctx, &mut controls, 800., Vec::new());

        let left = timeline.rect.left_center() + vec2(2., 0.);
        let middle = timeline.rect.center() + vec2(37.3, 0.);

        for pos in [left, middle] {
            let mut controls = VideoControls::default();
            let button = |pressed| Event::PointerButton {
                pos,
                button: PointerButton::Primary,
                pressed,
                modifiers: Modifiers::NONE,
            };

            // Mouse presses seek on release
            let press = vec![Event::PointerMoved(pos), button(true)];
            frame(&ctx, &mut controls, 800., press);
            let (commands, _) = frame(&ctx, &mut controls, 800., vec![button(false)]);
            let (_, expected) = seek_target(timeline.rect, pos.x, 7200.);

            let Some(Command::Playback(PlaybackCommand::Seek(seconds))) =
                commands.last()
            else {
                panic!("No seek in {commands:?}");
            };
            assert!((seconds - expected).abs() < 1e-3, "{seconds} != {expected}");
        }
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
