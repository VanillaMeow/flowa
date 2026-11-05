//! Viewer menus, toolbar controls, and the status bar

use egui::RichText;
use strum::VariantArray;

use crate::{
    app::Command,
    settings::{Settings, SortOrder},
    utils::format,
    video::VideoState,
    viewer::{browser::Browser, image_session::ImageSession},
};

use super::{
    actions,
    canvas::ImageCanvas,
    icons::{self, Icon},
    theme::Colors,
};

pub(crate) struct ToolbarState<'a> {
    pub browser: &'a Browser,
    pub image: &'a ImageSession,
    pub video: &'a VideoState,
    pub canvas: &'a ImageCanvas,
    pub settings: &'a Settings,
    pub colors: Colors,
    pub fullscreen: bool,
    pub preferences_open: bool,
    pub busy: Option<&'a str>,
    pub slideshow: bool,
}

pub(crate) fn show(
    root: &mut egui::Ui,
    state: &ToolbarState<'_>,
    commands: &mut Vec<Command>,
) {
    let frame = egui::Frame::new()
        .fill(state.colors.panel)
        .inner_margin(egui::Margin::symmetric(14, 9));

    egui::Panel::top("toolbar")
        .exact_size(58.)
        .frame(frame)
        .show(root, |ui| toolbar_row(ui, state, commands));
}

fn toolbar_row(
    ui: &mut egui::Ui,
    state: &ToolbarState<'_>,
    commands: &mut Vec<Command>,
) {
    ui.horizontal_centered(|ui| toolbar_contents(ui, state, commands));
}

fn toolbar_contents(
    ui: &mut egui::Ui,
    state: &ToolbarState<'_>,
    commands: &mut Vec<Command>,
) {
    let ctx = ui.ctx().clone();
    let compact = ui.available_width() < 600.;

    icons::logo(ui, 26., state.colors.accent);
    ui.add_space(4.);
    ui.menu_button("File", |ui| file_menu(ui, state, commands));

    if actions::OPEN_FILE.button(ui, Icon::Open, false).clicked() {
        commands.push(Command::OpenFile);
    }

    ui.separator();
    ui.add_enabled_ui(state.browser.entries().len() > 1, |ui| {
        navigation_buttons(ui, state, commands);
    });

    let narrow = ui.available_width() < 760.;

    if !narrow {
        image_title(ui, state);
    }

    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        if compact {
            ui.menu_button("View", |ui| compact_view_menu(ui, state, commands));
            return;
        }

        window_buttons(ui, state, commands);
        ui.separator();

        ui.add_enabled_ui(state.image.decoded.is_some() || state.video.ready(), |ui| {
            view_buttons(ui, state, commands, &ctx, narrow);
        });
    });
}

/// Keeps view controls reachable when enlarged UI text leaves little toolbar space
fn compact_view_menu(
    ui: &mut egui::Ui,
    state: &ToolbarState<'_>,
    commands: &mut Vec<Command>,
) {
    let ready = state.image.decoded.is_some() || state.video.ready();

    ui.add_enabled_ui(ready, |ui| compact_zoom_menu(ui, commands));
    ui.separator();

    for (action, selected) in [
        (&actions::FILMSTRIP, state.settings.filmstrip),
        (&actions::INSPECTOR, state.settings.info),
        (&actions::FULLSCREEN, state.fullscreen),
    ] {
        action.menu_label(ui, action.label, true, selected, commands);
    }

    ui.separator();
    actions::PREFERENCES.menu(ui, true, commands);
}

fn compact_zoom_menu(ui: &mut egui::Ui, commands: &mut Vec<Command>) {
    for action in [
        &actions::FIT,
        &actions::ACTUAL_PIXELS,
        &actions::ZOOM_IN,
        &actions::ZOOM_OUT,
        &actions::ROTATE,
    ] {
        action.menu(ui, true, commands);
    }
}

fn navigation_buttons(
    ui: &mut egui::Ui,
    state: &ToolbarState<'_>,
    commands: &mut Vec<Command>,
) {
    for (icon, action, delta) in [
        (Icon::Left, &actions::PREVIOUS, -1),
        (Icon::Right, &actions::NEXT, 1),
    ] {
        let response = action.button(ui, icon, false);

        if response.clicked() {
            commands.push(Command::Navigate(delta));
        } else if response.hovered()
            && let Some(index) =
                state.browser.next(delta, state.settings.wrap_navigation)
        {
            commands.push(Command::PreloadImage(index));
        }
    }
}

fn image_title(ui: &mut egui::Ui, state: &ToolbarState<'_>) {
    let entry = state.browser.current();
    let title = entry.map(|entry| entry.name.as_str()).unwrap_or("");
    let width = (ui.available_width() - 480.).clamp(40., 260.);
    let label = egui::Label::new(RichText::new(title).size(15.)).truncate();

    let response = ui.add_sized([width, 34.], label);

    if let Some(entry) = entry {
        response.on_hover_text(entry.path.display().to_string());
    }
}

fn file_menu(ui: &mut egui::Ui, state: &ToolbarState<'_>, commands: &mut Vec<Command>) {
    actions::OPEN_FILE.menu(ui, true, commands);
    actions::OPEN_FOLDER.menu(ui, true, commands);
    actions::REFRESH.menu(ui, state.browser.folder().is_some(), commands);

    ui.separator();
    file_actions(ui, state, commands);

    let can_reveal = state.browser.current().is_some() && state.busy.is_none();
    if ui
        .add_enabled(can_reveal, egui::Button::new("Show folder in file manager"))
        .clicked()
        && let Some(entry) = state.browser.current()
    {
        commands.push(Command::Reveal(entry.path.clone()));
        ui.close();
    }

    ui.separator();
    actions::PREFERENCES.menu(ui, true, commands);
    actions::QUIT.menu(ui, true, commands);
}

fn file_actions(
    ui: &mut egui::Ui,
    state: &ToolbarState<'_>,
    commands: &mut Vec<Command>,
) {
    let image_ready = state.image.decoded.is_some() && state.busy.is_none();
    let file_ready = state.browser.current().is_some() && state.busy.is_none();

    actions::COPY_IMAGE.menu(ui, image_ready, commands);
    actions::EXPORT.menu(ui, image_ready, commands);
    actions::RENAME.menu(ui, file_ready, commands);
    actions::TRASH.menu(ui, file_ready, commands);
}

fn window_buttons(
    ui: &mut egui::Ui,
    state: &ToolbarState<'_>,
    commands: &mut Vec<Command>,
) {
    for (action, icon, selected) in [
        (&actions::FULLSCREEN, Icon::Fullscreen, state.fullscreen),
        (
            &actions::PREFERENCES,
            Icon::Settings,
            state.preferences_open,
        ),
        (&actions::INSPECTOR, Icon::Info, state.settings.info),
        (
            &actions::FILMSTRIP,
            Icon::Filmstrip,
            state.settings.filmstrip,
        ),
    ] {
        if action.button(ui, icon, selected).clicked() {
            commands.push(action.command.clone());
        }
    }
}

fn view_buttons(
    ui: &mut egui::Ui,
    state: &ToolbarState<'_>,
    commands: &mut Vec<Command>,
    ctx: &egui::Context,
    narrow: bool,
) {
    let animated = state.image.animated();
    if animated || state.slideshow {
        let playing = if animated {
            state.image.playback.playing
        } else {
            state.slideshow
        };
        if actions::PLAY
            .button(ui, if playing { Icon::Pause } else { Icon::Play }, false)
            .clicked()
        {
            commands.push(Command::TogglePlay);
        }
    }

    if !narrow && actions::ROTATE.button(ui, Icon::Rotate, false).clicked() {
        commands.push(Command::RotateClockwise);
    }

    if actions::FIT
        .button(ui, Icon::Fit, state.canvas.view.fit)
        .clicked()
    {
        commands.push(Command::Fit);
    }

    if !narrow
        && actions::ACTUAL_PIXELS
            .button(
                ui,
                Icon::Actual,
                !state.canvas.view.fit
                    && (state.canvas.view.zoom * ctx.pixels_per_point() - 1.).abs()
                        < 0.001,
            )
            .clicked()
    {
        commands.push(Command::ActualPixels);
    }

    if actions::ZOOM_IN.button(ui, Icon::Plus, false).clicked() {
        commands.push(Command::Zoom(1.25));
    }
    let percentage = format!(
        "{:.0}%",
        state.canvas.view.zoom * ctx.pixels_per_point() * 100.
    );
    if ui
        .add_sized(
            [62., 32.],
            egui::Button::new(RichText::new(percentage).size(13.)).frame(false),
        )
        .on_hover_text("Click for actual pixels")
        .clicked()
    {
        commands.push(Command::ActualPixels);
    }

    if actions::ZOOM_OUT.button(ui, Icon::Minus, false).clicked() {
        commands.push(Command::Zoom(0.8));
    }
}

pub(crate) fn status(root: &mut egui::Ui, state: &ToolbarState<'_>) {
    if state.browser.current().is_none() {
        return;
    }

    let frame = egui::Frame::new()
        .fill(state.colors.panel)
        .inner_margin(egui::Margin::symmetric(18, 4));

    egui::Panel::bottom("status")
        .exact_size(32.)
        .frame(frame)
        .show(root, |ui| status_row(ui, state));
}

fn status_row(ui: &mut egui::Ui, state: &ToolbarState<'_>) {
    ui.horizontal_centered(|ui| status_contents(ui, state));
}

fn status_contents(ui: &mut egui::Ui, state: &ToolbarState<'_>) {
    if state.video.ready() {
        let dimensions = format!("{}  ×  {}", state.video.size[0], state.video.size[1]);
        status_label(ui, &dimensions, state.colors);
        ui.separator();
        status_label(ui, "Video", state.colors);
    }

    if let Some(image) = &state.image.decoded {
        let dimensions = format!("{}  ×  {}", image.size[0], image.size[1]);
        let file_size = format::file_size(image.file_size);

        status_label(ui, &dimensions, state.colors);
        ui.separator();
        status_label(ui, &image.format, state.colors);
        ui.separator();
        status_label(ui, &file_size, state.colors);

        if image.frames.len() > 1 {
            ui.separator();
            status_label(ui, &format!("{} frames", image.frames.len()), state.colors);
        }
    }

    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        navigation_status(ui, state)
    });
}

fn navigation_status(ui: &mut egui::Ui, state: &ToolbarState<'_>) {
    if !state.browser.entries().is_empty() {
        let position = format!(
            "{} / {}",
            state.browser.index() + 1,
            state.browser.entries().len(),
        );

        status_label(ui, &position, state.colors);
    }

    if let Some(busy) = state.busy {
        ui.add(egui::Spinner::new().size(16.));
        status_label(ui, busy, state.colors);
    } else if state.slideshow {
        ui.label(
            RichText::new("Slideshow")
                .size(12.)
                .color(state.colors.accent),
        );
    }
}

fn status_label(ui: &mut egui::Ui, text: &str, colors: Colors) {
    ui.label(RichText::new(text).size(12.).color(colors.muted));
}

pub(crate) fn context_menu(
    ui: &mut egui::Ui,
    state: &ToolbarState<'_>,
    commands: &mut Vec<Command>,
) {
    for action in [
        &actions::FIT,
        &actions::ACTUAL_PIXELS,
        &actions::ROTATE,
        &actions::FLIP_HORIZONTAL,
        &actions::FLIP_VERTICAL,
    ] {
        action.menu(ui, true, commands);
    }

    ui.separator();
    playback_menu(ui, state, commands);
    ui.separator();
    file_actions(ui, state, commands);

    ui.separator();
    ui.menu_button("Sort folder", |ui| sort_menu(ui, state, commands));
}

fn sort_menu(ui: &mut egui::Ui, state: &ToolbarState<'_>, commands: &mut Vec<Command>) {
    let old = (state.settings.sort, state.settings.reverse_sort);
    let (mut order, mut reverse) = old;

    for &sort in SortOrder::VARIANTS {
        ui.selectable_value(&mut order, sort, sort.as_ref());
    }

    ui.separator();
    ui.checkbox(&mut reverse, "Reverse order");

    if old != (order, reverse) {
        commands.push(Command::Sort { order, reverse });
    }
}

fn playback_menu(
    ui: &mut egui::Ui,
    state: &ToolbarState<'_>,
    commands: &mut Vec<Command>,
) {
    if state.video.active {
        let label = if state.video.paused { "Play" } else { "Pause" };
        actions::PLAY.menu_label(ui, label, state.video.ready(), false, commands);
        return;
    }

    let label = if state.slideshow {
        "Stop slideshow"
    } else {
        "Start slideshow"
    };
    actions::SLIDESHOW.menu_label(ui, label, true, false, commands);

    if state.image.animated() {
        let label = if state.image.playback.playing {
            "Pause animation"
        } else {
            "Play animation"
        };
        actions::PLAY.menu_label(ui, label, true, false, commands);
    }
}
