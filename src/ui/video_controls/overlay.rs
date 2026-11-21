//! Video controls floating over the bottom of the video on a gradient, hidden once the
//! pointer rests
//!
//! The timeline spans the width above one row of buttons, and the thumbnail strip, when
//! shown, floats below them. Everything fades together, and subtitles are lifted clear
//! while the controls are up

use std::time::{Duration, Instant};

use egui::{
    Align, Color32, CursorIcon, Id, Layout, Mesh, Order, Pos2, Rangef, Rect, Response,
    RichText, Stroke, UiBuilder, pos2,
};

use crate::{
    app::Command, settings::OverlayBackdrop, utils::format::media_time,
    video::VideoState, viewer::browser::Browser,
};

use super::{
    super::{
        actions,
        filmstrip::{self, Filmstrip},
        icons::Icon,
        style::{margin, motion, size, space},
        theme::Colors,
        tracks,
    },
    COMPACT_SPEED_BREAKPOINT, INLINE_VOLUME_WIDTH, VideoControls, play_button,
    slider_rail, speed_menu, speed_width, text_width, volume_slider,
};

/// How long the controls stay after the pointer last moved or a playback key was pressed
const IDLE_TIMEOUT: Duration = Duration::from_millis(2500);
/// Hiding is slower than showing, so controls never vanish abruptly
const HIDE_FADE: f32 = 0.25;
/// The timeline's pointer target; the bar painted inside it is thinner
const TIMELINE_HEIGHT: f32 = 18.;
const BAR_THICKNESS: f32 = 3.;
const HOVERED_BAR_THICKNESS: f32 = 5.;
const HANDLE_RADIUS: f32 = 6.5;
/// The volume handle always shows, a little smaller than the timeline's
const VOLUME_HANDLE_RADIUS: f32 = 5.5;
/// How far the gradient reaches above the controls
const BACKDROP_REACH: f32 = 72.;
/// Steps in the gradient's easing curve
const BACKDROP_STEPS: usize = 16;

/// Whether the floating controls are up and when they were last asked for
#[derive(Default)]
pub(super) struct OverlayState {
    /// When the pointer last moved or pressed over the video
    pointer_activity: Option<Instant>,
    /// Where the pointer was last frame, so entering the window counts as a move
    pointer: Option<Pos2>,
    /// When a playback key last asked to show the controls briefly
    flashed: Option<Instant>,
    opacity: f32,
    /// How far up from the canvas bottom subtitles keep clear this frame
    clearance: f32,
    volume_open: bool,
}

/// Everything the floating controls show, and the thumbnail strip below them
pub struct OverlayContent<'a> {
    pub canvas: Rect,
    pub state: &'a VideoState,
    pub colors: Colors,
    pub backdrop: OverlayBackdrop,
    pub fullscreen: bool,
    pub strip: Option<(&'a mut Filmstrip, &'a Browser)>,
}

/// What the row of buttons needs besides the controls' own state
struct RowContent<'a> {
    state: &'a VideoState,
    colors: Colors,
    fullscreen: bool,
}

/// Where each part of the overlay sits inside the canvas
struct OverlayLayout {
    /// Everything that takes the pointer, from the timeline down to the canvas bottom
    controls: Rect,
    timeline: Rect,
    row: Rect,
    strip: Option<Rect>,
}

impl OverlayLayout {
    fn new(canvas: Rect, strip: bool) -> Self {
        let inner = canvas
            .x_range()
            .shrink(f32::from(margin::BOTTOM_PANEL.left));
        let mut bottom = canvas.bottom();

        let strip = strip.then(|| {
            let top = bottom - filmstrip::STRIP_HEIGHT;
            bottom = top;
            Rect::from_x_y_ranges(canvas.x_range(), top..=canvas.bottom())
        });
        if strip.is_none() {
            bottom -= f32::from(margin::BOTTOM_PANEL.bottom);
        }

        let row = Rect::from_x_y_ranges(inner, (bottom - size::CONTROL)..=bottom);

        // The slider insets its rail by the handle radius, so the rail lines up with
        // the row below once the slider is widened by that much
        let timeline_bottom = row.top() - space::XS;
        let timeline = Rect::from_x_y_ranges(
            inner.expand(TIMELINE_HEIGHT / 2.5),
            (timeline_bottom - TIMELINE_HEIGHT)..=timeline_bottom,
        );

        let controls = Rect::from_x_y_ranges(
            canvas.x_range(),
            (timeline.top() - space::SM)..=canvas.bottom(),
        );

        Self {
            controls,
            timeline,
            row,
            strip,
        }
    }
}

/// Colors of the gradient and of everything drawn on it
#[derive(Clone, Copy)]
struct Palette {
    backdrop: Color32,
    /// Opacity of the gradient at the canvas bottom
    backdrop_strength: f32,
    text: Color32,
    muted: Color32,
    track: Color32,
    /// The timeline between the played part and the pointer
    ahead: Color32,
    accent: Color32,
}

impl Palette {
    fn new(backdrop: OverlayBackdrop, colors: Colors) -> Self {
        let (base, strength, text, muted) = match backdrop {
            OverlayBackdrop::Black => {
                let text = Color32::from_gray(245);
                (Color32::BLACK, 0.78, text, text.gamma_multiply(0.72))
            }
            OverlayBackdrop::Theme => (colors.panel, 0.92, colors.text, colors.muted),
        };

        Self {
            backdrop: base,
            backdrop_strength: strength,
            text,
            muted,
            track: text.gamma_multiply(0.3),
            ahead: text.gamma_multiply(0.5),
            accent: colors.accent,
        }
    }

    /// Frameless buttons in the overlay's colors; sliders paint their own bars, and
    /// popups opened from here keep the theme, since egui styles them from the context
    fn apply(self, ui: &mut egui::Ui) {
        let visuals = ui.visuals_mut();
        visuals.override_text_color = Some(self.text);

        let widgets = &mut visuals.widgets;
        for widget in [
            &mut widgets.inactive,
            &mut widgets.hovered,
            &mut widgets.active,
            &mut widgets.open,
        ] {
            widget.fg_stroke = Stroke::new(1.5, self.text);
            widget.bg_stroke = Stroke::NONE;
        }

        // Buttons fill with the weak color, clear until hovered
        widgets.inactive.weak_bg_fill = Color32::TRANSPARENT;
        widgets.hovered.weak_bg_fill = self.text.gamma_multiply(0.14);
        widgets.active.weak_bg_fill = self.text.gamma_multiply(0.22);
        widgets.open.weak_bg_fill = self.text.gamma_multiply(0.14);
    }
}

impl VideoControls {
    /// Shows the floating controls briefly, so a playback key's effect can be seen
    pub fn flash_overlay(&mut self, now: Instant) {
        self.overlay.flashed = Some(now);
    }

    /// Decides whether the floating controls are up this frame, before the canvas
    /// paints subtitles around them
    ///
    /// `pinned` keeps them up, such as while preferences are open. Returns how far up
    /// from the canvas bottom subtitles must stay
    pub fn update_overlay(
        &mut self,
        ctx: &egui::Context,
        canvas: Rect,
        strip: bool,
        pinned: bool,
        now: Instant,
    ) -> f32 {
        let layout = OverlayLayout::new(canvas, strip);
        let visible = self.overlay_wanted(ctx, canvas, &layout, pinned, now);

        let (target, duration) = if visible {
            (1., motion::FADE_IN)
        } else {
            (0., HIDE_FADE)
        };
        let opacity =
            ctx.animate_value_with_time(overlay_id().with("opacity"), target, duration);

        self.overlay.opacity = opacity;
        self.overlay.clearance = (canvas.bottom() - layout.controls.top()) * opacity;
        self.overlay.clearance
    }

    fn overlay_wanted(
        &mut self,
        ctx: &egui::Context,
        canvas: Rect,
        layout: &OverlayLayout,
        pinned: bool,
        now: Instant,
    ) -> bool {
        let (pointer, pressed) =
            ctx.input(|input| (input.pointer.hover_pos(), input.pointer.any_pressed()));
        let moved = pointer != self.overlay.pointer;
        let over_canvas = pointer.is_some_and(|pos| canvas.contains(pos));
        self.overlay.pointer = pointer;

        if (moved || pressed) && over_canvas {
            self.overlay.pointer_activity = Some(now);
        }

        // Controls already up stay while the pointer rests on them
        let shown = self.overlay.opacity > 0.;
        let over_controls =
            shown && pointer.is_some_and(|pos| layout.controls.contains(pos));

        // Menus, scrubbing, and panning keep them up until they finish
        let busy = egui::Popup::is_any_open(ctx) || shown && ctx.dragged_id().is_some();

        let pointer_left =
            remaining(self.overlay.pointer_activity, now).filter(|_| over_canvas);
        let flash_left = remaining(self.overlay.flashed, now);
        let timer = pointer_left.max(flash_left);

        if let Some(left) = timer {
            ctx.request_repaint_after(left);
        }

        pinned || busy || over_controls || timer.is_some()
    }

    /// Paints the floating controls and strip over the canvas, after the canvas has
    /// painted the video
    pub fn show_overlay(
        &mut self,
        ctx: &egui::Context,
        content: OverlayContent<'_>,
        commands: &mut Vec<Command>,
    ) {
        let opacity = self.overlay.opacity;

        if opacity <= 0. {
            hide_cursor(ctx, content.canvas);
            return;
        }

        let layout = OverlayLayout::new(content.canvas, content.strip.is_some());
        let palette = Palette::new(content.backdrop, content.colors);

        // Background order keeps the overlay above the canvas but below windows such
        // as preferences, while still taking the pointer from the canvas beneath
        egui::Area::new(overlay_id())
            .order(Order::Background)
            .fixed_pos(layout.controls.min)
            .constrain(false)
            .fade_in(false)
            .show(ctx, |ui| {
                ui.set_opacity(opacity);
                paint_backdrop(ui, content.canvas, &layout, palette);
                ui.expand_to_include_rect(layout.controls);

                self.overlay_contents(ui, &layout, content, palette, commands);
            });
    }

    fn overlay_contents(
        &mut self,
        ui: &mut egui::Ui,
        layout: &OverlayLayout,
        content: OverlayContent<'_>,
        palette: Palette,
        commands: &mut Vec<Command>,
    ) {
        let state = content.state;
        let row = RowContent {
            state,
            colors: content.colors,
            fullscreen: content.fullscreen,
        };

        // The strip keeps the theme's look, so it is drawn before the palette applies
        if let (Some(rect), Some((filmstrip, browser))) = (layout.strip, content.strip)
        {
            filmstrip.show_floating(ui, rect, browser, content.colors, commands);
        }

        overlay_scope(ui, layout.timeline, palette, |ui| {
            self.overlay_timeline(ui, state, palette, commands);
        });
        overlay_scope(ui, layout.row, palette, |ui| {
            self.overlay_row(ui, &row, palette, commands);
        });
    }

    fn overlay_timeline(
        &mut self,
        ui: &mut egui::Ui,
        state: &VideoState,
        palette: Palette,
        commands: &mut Vec<Command>,
    ) {
        ui.spacing_mut().slider_width = ui.available_width();
        ui.spacing_mut().interact_size.y = TIMELINE_HEIGHT;
        let position = self.displayed_position(state);

        // Egui's slider still handles the pointer, keys, and accessibility; only its
        // painting is replaced with the thin bar
        let response = ui
            .scope(|ui| {
                ui.set_opacity(0.);
                self.seek_slider(ui, state, position, commands)
            })
            .inner;

        let position = self.displayed_position(state);
        paint_timeline(ui, &response, position, state.duration, palette);
    }

    /// The scrubbed or pending position while a seek settles, otherwise playback's
    fn displayed_position(&self, state: &VideoState) -> f64 {
        self.seek
            .as_ref()
            .map_or(state.position, |seek| seek.seconds)
    }

    /// Play, volume, and the time on the left; speed, tracks, and fullscreen on the
    /// right
    fn overlay_row(
        &mut self,
        ui: &mut egui::Ui,
        row: &RowContent<'_>,
        palette: Palette,
        commands: &mut Vec<Command>,
    ) {
        let state = row.state;
        ui.spacing_mut().item_spacing.x = space::XS;

        let show_speed = ui.available_width() >= COMPACT_SPEED_BREAKPOINT;
        let show_tracks = tracks::available(state);
        let time = time_text(self.displayed_position(state), state.duration);

        // The volume slider opens only as far as the other controls leave room for
        let fixed = fixed_row_width(ui, &time, show_speed, show_tracks);
        let volume_room = (ui.available_width() - fixed).clamp(0., INLINE_VOLUME_WIDTH);

        ui.add_enabled_ui(state.ready(), |ui| play_button(ui, state.paused, commands));
        self.overlay_volume(ui, state, volume_room, palette, commands);
        time_readout(ui, &time, palette);

        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            fullscreen_button(ui, row.fullscreen, commands);

            ui.add_enabled_ui(state.ready(), |ui| {
                // The track menu opens as a themed popup, so it takes the theme colors
                if show_tracks {
                    tracks::button(ui, state, row.colors, commands);
                }

                if show_speed {
                    speed_menu(ui, state.rate, state.pitch, commands);
                }
            });
        });
    }

    /// The mute button, with the volume slider sliding open beside it while either is
    /// hovered or the slider is in use
    fn overlay_volume(
        &mut self,
        ui: &mut egui::Ui,
        state: &VideoState,
        room: f32,
        palette: Palette,
        commands: &mut Vec<Command>,
    ) {
        if state.volume > 0. {
            self.audible_volume = state.volume;
        }

        let openness = ui.ctx().animate_bool_with_time(
            overlay_id().with("volume"),
            self.overlay.volume_open,
            motion::FADE_IN,
        );
        let width = room * openness;

        let group = ui.horizontal(|ui| {
            self.mute_button(ui, state, commands);

            if width < 1. {
                return false;
            }

            ui.scope(|ui| {
                ui.multiply_opacity(openness);

                // As on the timeline, egui's slider takes input under a painted bar
                let slider = ui
                    .scope(|ui| {
                        ui.set_opacity(0.);
                        volume_slider(ui, state.volume, width, commands)
                    })
                    .inner;
                paint_volume(ui, &slider, state.volume, palette);

                slider.dragged() || slider.has_focus()
            })
            .inner
        });

        let hovered = ui.rect_contains_pointer(group.response.rect.expand(space::XS));
        self.overlay.volume_open = room > 0. && (hovered || group.inner);
    }
}

/// A child of the overlay at a fixed rect, in the overlay's colors
fn overlay_scope(
    ui: &mut egui::Ui,
    rect: Rect,
    palette: Palette,
    add_contents: impl FnOnce(&mut egui::Ui),
) {
    let builder = UiBuilder::new()
        .max_rect(rect)
        .layout(Layout::left_to_right(Align::Center));

    ui.scope_builder(builder, |ui| {
        palette.apply(ui);
        add_contents(ui);
    });
}

/// The elapsed time and the duration, which shows dashes until it is known
fn time_text(position: f64, duration: f64) -> [String; 2] {
    let total = if duration > 0. {
        media_time(duration)
    } else {
        "--:--".into()
    };

    [media_time(position), format!("/ {total}")]
}

fn time_readout(ui: &mut egui::Ui, time: &[String; 2], palette: Palette) {
    ui.add_space(space::XS);

    let [elapsed, total] = time;
    let elapsed = RichText::new(elapsed).monospace().color(palette.text);
    let total = RichText::new(total).monospace().color(palette.muted);

    ui.add(egui::Label::new(elapsed).selectable(false));
    ui.add(egui::Label::new(total).selectable(false));
}

/// The width of everything on the row except the volume slider, including the gaps
fn fixed_row_width(
    ui: &egui::Ui,
    time: &[String; 2],
    show_speed: bool,
    show_tracks: bool,
) -> f32 {
    let spacing = ui.spacing().item_spacing.x;

    // Play, mute, and fullscreen, the time with its leading space, and the gaps
    let mut width = size::CONTROL * 3.
        + text_width(ui, &time[0])
        + text_width(ui, &time[1])
        + space::XS
        + spacing * 6.;

    if show_speed {
        width += speed_width(ui) + spacing;
    }

    if show_tracks {
        width += size::CONTROL + spacing;
    }

    width
}

fn fullscreen_button(ui: &mut egui::Ui, fullscreen: bool, commands: &mut Vec<Command>) {
    let label = if fullscreen {
        "Leave fullscreen"
    } else {
        "Fullscreen"
    };

    actions::FULLSCREEN.button_label(ui, Icon::Fullscreen, label, false, commands);
}

/// A thin rounded bar that thickens and grows a handle while hovered or dragged, with
/// the stretch up to the pointer lightened
fn paint_timeline(
    ui: &egui::Ui,
    response: &Response,
    position: f64,
    duration: f64,
    palette: Palette,
) {
    let active = response.hovered() || response.dragged() || response.has_focus();
    let emphasis = ui.ctx().animate_bool_with_time(
        response.id.with("emphasis"),
        active,
        motion::FADE_IN,
    );

    let bar = Bar {
        rail: slider_rail(response.rect),
        center: response.rect.center().y,
        thickness: egui::lerp(BAR_THICKNESS..=HOVERED_BAR_THICKNESS, emphasis),
        handle: HANDLE_RADIUS * emphasis,
    };
    let fraction = if duration > 0. {
        (position / duration).clamp(0., 1.) as f32
    } else {
        0.
    };
    let pointer = response.hover_pos().map(|pos| pos.x);

    paint_bar(ui.painter(), &bar, fraction, pointer, palette);
}

/// The volume as a solid bar with its handle always showing, matching the timeline
fn paint_volume(ui: &egui::Ui, response: &Response, volume: f64, palette: Palette) {
    let bar = Bar {
        rail: slider_rail(response.rect),
        center: response.rect.center().y,
        thickness: BAR_THICKNESS,
        handle: VOLUME_HANDLE_RADIUS,
    };
    let fraction = (volume / 100.).clamp(0., 1.) as f32;

    paint_bar(ui.painter(), &bar, fraction, None, palette);
}

/// Where a slider's bar runs and how big it is drawn
struct Bar {
    rail: Rangef,
    center: f32,
    thickness: f32,
    /// The handle's radius, or zero to leave it out
    handle: f32,
}

/// A rounded track filled with the accent up to `fraction`, lightened from there up to
/// `pointer` when given, with a solid handle at the end of the fill
fn paint_bar(
    painter: &egui::Painter,
    bar: &Bar,
    fraction: f32,
    pointer: Option<f32>,
    palette: Palette,
) {
    let rail = bar.rail;
    let band = Rangef::new(
        bar.center - bar.thickness / 2.,
        bar.center + bar.thickness / 2.,
    );
    let rounding = bar.thickness / 2.;
    let filled = rail.min + rail.span() * fraction;

    painter.rect_filled(Rect::from_x_y_ranges(rail, band), rounding, palette.track);

    if let Some(pointer) = pointer.map(|x| x.clamp(rail.min, rail.max))
        && pointer > filled
    {
        let ahead = Rect::from_x_y_ranges(filled..=pointer, band);
        painter.rect_filled(ahead, rounding, palette.ahead);
    }

    let fill = Rect::from_x_y_ranges(rail.min..=filled, band);
    painter.rect_filled(fill, rounding, palette.accent);

    if bar.handle > 0. {
        let handle = pos2(filled, bar.center);
        painter.circle_filled(handle, bar.handle, palette.accent);
    }
}

/// A gradient from clear above the controls to the backdrop color at the canvas
/// bottom, eased so it darkens mostly behind the controls
fn paint_backdrop(
    ui: &egui::Ui,
    canvas: Rect,
    layout: &OverlayLayout,
    palette: Palette,
) {
    let top = (layout.controls.top() - BACKDROP_REACH).max(canvas.top());
    let mut mesh = Mesh::default();

    for step in 0..=BACKDROP_STEPS {
        let t = step as f32 / BACKDROP_STEPS as f32;
        let y = egui::lerp(top..=canvas.bottom(), t);
        let eased = t * t * (3. - 2. * t);
        let color = palette
            .backdrop
            .gamma_multiply(eased * palette.backdrop_strength);

        mesh.colored_vertex(Pos2::new(canvas.left(), y), color);
        mesh.colored_vertex(Pos2::new(canvas.right(), y), color);

        if step > 0 {
            let row = (step as u32 - 1) * 2;
            mesh.add_triangle(row, row + 1, row + 2);
            mesh.add_triangle(row + 1, row + 2, row + 3);
        }
    }

    ui.painter().with_clip_rect(canvas).add(mesh);
}

/// Hides the pointer over the video while the controls are hidden, as players do
fn hide_cursor(ctx: &egui::Context, canvas: Rect) {
    let over_canvas = ctx
        .pointer_hover_pos()
        .is_some_and(|pos| canvas.contains(pos));

    if over_canvas && ctx.dragged_id().is_none() {
        ctx.set_cursor_icon(CursorIcon::None);
    }
}

/// How much of the idle timeout is left since `since`, if any
fn remaining(since: Option<Instant>, now: Instant) -> Option<Duration> {
    let elapsed = now.saturating_duration_since(since?);

    IDLE_TIMEOUT
        .checked_sub(elapsed)
        .filter(|left| !left.is_zero())
}

fn overlay_id() -> Id {
    Id::unique("video-overlay")
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use egui::{CursorIcon, Event, Modifiers, PointerButton, Pos2, Rect, vec2};

    use super::{OverlayContent, OverlayLayout, TIMELINE_HEIGHT};
    use crate::{
        app::{Command, PlaybackCommand},
        settings::{Accent, OverlayBackdrop, Theme},
        ui::{
            filmstrip,
            style::{margin, size},
            theme,
            video_controls::{
                SliderFocus, VideoControls, seek_target, slider_focus_id,
            },
        },
        video::VideoState,
    };

    const HEIGHT: f32 = 600.;

    /// Runs one frame at `seconds` after `start` with the canvas filling the window
    struct Frame {
        commands: Vec<Command>,
        clearance: f32,
        cursor: CursorIcon,
        canvas: Rect,
    }

    fn frame(
        ctx: &egui::Context,
        controls: &mut VideoControls,
        start: Instant,
        seconds: f64,
        width: f32,
        events: Vec<Event>,
    ) -> Frame {
        let canvas = Rect::from_min_size(Pos2::ZERO, vec2(width, HEIGHT));
        let state = VideoState {
            active: true,
            position: 70.,
            duration: 7200.,
            ..Default::default()
        };
        let input = egui::RawInput {
            screen_rect: Some(canvas),
            time: Some(seconds),
            events,
            ..Default::default()
        };
        let colors = theme::colors(Theme::Charcoal, Accent::Theme, true);
        theme::apply(ctx, colors);

        let now = start + Duration::from_secs_f64(seconds);
        let mut commands = Vec::new();
        let mut clearance = 0.;
        let mut output = ctx.run_ui(input, |ui| {
            clearance = controls.update_overlay(ui.ctx(), canvas, false, false, now);

            let content = OverlayContent {
                canvas,
                state: &state,
                colors,
                backdrop: OverlayBackdrop::Black,
                fullscreen: false,
                strip: None,
            };
            controls.show_overlay(ui.ctx(), content, &mut commands);
        });
        output.textures_delta.clear();

        for shape in &output.shapes {
            let bounds = shape.shape.visual_bounding_rect();

            if bounds.is_finite() && bounds.is_positive() {
                assert!(
                    bounds.left() >= -1. && bounds.right() <= width + 1.,
                    "Paint overflow at width {width}: {bounds:?}"
                );
            }
        }

        Frame {
            commands,
            clearance,
            cursor: output.platform_output.cursor_icon,
            canvas,
        }
    }

    fn timeline_rect(ctx: &egui::Context) -> Rect {
        let sliders = ctx
            .data(|data| data.get_temp::<SliderFocus>(slider_focus_id()))
            .expect("Sliders should be registered");

        ctx.read_response(sliders.timeline.expect("Timeline ID"))
            .expect("Timeline response")
            .rect
    }

    fn press(pos: Pos2, pressed: bool) -> Event {
        Event::PointerButton {
            pos,
            button: PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        }
    }

    #[test]
    fn controls_hide_once_the_pointer_rests_and_return_when_it_moves() {
        let ctx = egui::Context::default();
        let mut controls = VideoControls::default();
        let start = Instant::now();
        let over_video = Pos2::new(400., 200.);

        let moved = vec![Event::PointerMoved(over_video)];
        frame(&ctx, &mut controls, start, 0., 800., moved);
        let shown = frame(&ctx, &mut controls, start, 0.5, 800., Vec::new());
        assert!(shown.clearance > 60., "Clearance {}", shown.clearance);
        assert_ne!(shown.cursor, CursorIcon::None);

        let resting = frame(&ctx, &mut controls, start, 2., 800., Vec::new());
        assert_eq!(resting.clearance, shown.clearance);

        // Paused or playing, resting past the timeout hides the controls and pointer
        frame(&ctx, &mut controls, start, 3., 800., Vec::new());
        let hidden = frame(&ctx, &mut controls, start, 3.5, 800., Vec::new());
        assert_eq!(hidden.clearance, 0.);
        assert_eq!(hidden.cursor, CursorIcon::None);

        let moved = vec![Event::PointerMoved(over_video + vec2(5., 0.))];
        frame(&ctx, &mut controls, start, 4., 800., moved);
        let back = frame(&ctx, &mut controls, start, 4.5, 800., Vec::new());
        assert_eq!(back.clearance, shown.clearance);
    }

    #[test]
    fn controls_stay_while_the_pointer_rests_on_them() {
        let ctx = egui::Context::default();
        let mut controls = VideoControls::default();
        let start = Instant::now();
        let on_row = Pos2::new(400., HEIGHT - 20.);

        frame(
            &ctx,
            &mut controls,
            start,
            0.,
            800.,
            vec![Event::PointerMoved(on_row)],
        );
        let later = frame(&ctx, &mut controls, start, 10., 800., Vec::new());

        assert!(later.clearance > 0.);
    }

    #[test]
    fn playback_keys_flash_the_controls_without_the_pointer() {
        let ctx = egui::Context::default();
        let mut controls = VideoControls::default();
        let start = Instant::now();

        frame(&ctx, &mut controls, start, 0., 800., Vec::new());
        assert_eq!(
            frame(&ctx, &mut controls, start, 0.5, 800., Vec::new()).clearance,
            0.
        );

        controls.flash_overlay(start + Duration::from_secs(1));
        frame(&ctx, &mut controls, start, 1., 800., Vec::new());
        let flashed = frame(&ctx, &mut controls, start, 1.5, 800., Vec::new());
        assert!(flashed.clearance > 0.);

        frame(&ctx, &mut controls, start, 4., 800., Vec::new());
        let faded = frame(&ctx, &mut controls, start, 4.5, 800., Vec::new());
        assert_eq!(faded.clearance, 0.);
    }

    #[test]
    fn the_timeline_spans_the_row_and_fits_every_supported_width() {
        let ctx = egui::Context::default();
        let start = Instant::now();

        // 320 points also covers a 640-pixel window at 200 percent UI scale
        for width in [320., 400., 640., 1200., 1920.] {
            let mut controls = VideoControls::default();
            controls.flash_overlay(start);

            frame(&ctx, &mut controls, start, 0., width, Vec::new());
            let shown = frame(&ctx, &mut controls, start, 0.5, width, Vec::new());
            let timeline = timeline_rect(&ctx);

            let inset = f32::from(margin::BOTTOM_PANEL.left);
            let rail = super::slider_rail(timeline);
            assert!((rail.min - inset).abs() < 0.5, "Rail at {width}: {rail:?}");
            assert!((rail.max - (width - inset)).abs() < 0.5);
            assert!(timeline.bottom() < shown.canvas.bottom() - size::CONTROL);
        }
    }

    #[test]
    fn clicking_the_timeline_seeks_to_the_time_under_the_pointer() {
        let ctx = egui::Context::default();
        let mut controls = VideoControls::default();
        let start = Instant::now();
        controls.flash_overlay(start);

        frame(&ctx, &mut controls, start, 0., 800., Vec::new());
        frame(&ctx, &mut controls, start, 0.5, 800., Vec::new());
        let timeline = timeline_rect(&ctx);
        let pos = timeline.center() + vec2(91.7, 0.);

        let events = vec![Event::PointerMoved(pos), press(pos, true)];
        frame(&ctx, &mut controls, start, 0.6, 800., events);
        let released = frame(
            &ctx,
            &mut controls,
            start,
            0.7,
            800.,
            vec![press(pos, false)],
        );

        let (_, expected) = seek_target(timeline, pos.x, 7200.);
        let Some(Command::Playback(PlaybackCommand::Seek(seconds))) =
            released.commands.last()
        else {
            panic!("No seek in {:?}", released.commands);
        };
        assert!((seconds - expected).abs() < 1e-3, "{seconds} != {expected}");
    }

    #[test]
    fn the_strip_floats_at_the_bottom_with_the_controls_above_it() {
        let canvas = Rect::from_min_size(Pos2::new(0., 58.), vec2(1000., 700.));

        let docked = OverlayLayout::new(canvas, false);
        assert!(docked.strip.is_none());
        assert_eq!(
            docked.row.bottom(),
            canvas.bottom() - f32::from(margin::BOTTOM_PANEL.bottom)
        );

        let layout = OverlayLayout::new(canvas, true);
        let strip = layout.strip.expect("Strip");
        assert_eq!(strip.bottom(), canvas.bottom());
        assert_eq!(strip.height(), filmstrip::STRIP_HEIGHT);
        assert_eq!(layout.row.bottom(), strip.top());
        assert!(layout.timeline.bottom() <= layout.row.top());
        assert_eq!(layout.timeline.height(), TIMELINE_HEIGHT);
        assert!(layout.controls.contains_rect(layout.timeline));
        assert_eq!(layout.controls.bottom(), canvas.bottom());
    }
}
