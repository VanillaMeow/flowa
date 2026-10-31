//! Viewer menus, toolbar controls, and the status bar

use egui::RichText;

use crate::{
    app::Command,
    settings::{Settings, SortOrder},
    utils::format,
    viewer::{browser::Browser, image_session::ImageSession},
};

use super::{
    canvas::ImageCanvas,
    icons::{self, Icon},
    theme::Colors,
};

pub(crate) struct ToolbarState<'a> {
    pub browser: &'a Browser,
    pub image: &'a ImageSession,
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

    icons::logo(ui, 26., state.colors.accent);
    ui.add_space(4.);
    ui.menu_button("File", |ui| file_menu(ui, state, commands));

    if icons::button(ui, Icon::Open, "Open image · Ctrl+O", false).clicked() {
        commands.push(Command::OpenFile);
    }

    ui.separator();
    ui.add_enabled_ui(state.browser.entries().len() > 1, |ui| {
        navigation_buttons(ui, commands);
    });

    let narrow = ui.available_width() < 760.;

    if !narrow {
        image_title(ui, state);
    }

    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        window_buttons(ui, state, commands);
        ui.separator();

        ui.add_enabled_ui(state.image.decoded.is_some(), |ui| {
            view_buttons(ui, state, commands, &ctx, narrow);
        });
    });
}

fn navigation_buttons(ui: &mut egui::Ui, commands: &mut Vec<Command>) {
    if icons::button(ui, Icon::Left, "Previous image · Left arrow", false).clicked() {
        commands.push(Command::Navigate(-1));
    }

    if icons::button(ui, Icon::Right, "Next image · Right arrow", false).clicked() {
        commands.push(Command::Navigate(1));
    }
}

fn image_title(ui: &mut egui::Ui, state: &ToolbarState<'_>) {
    let entry = state.browser.current();
    let title = entry.map(|entry| entry.name.as_str()).unwrap_or("flowa");
    let tooltip = entry
        .map(|entry| entry.path.display().to_string())
        .unwrap_or_else(|| "An open space for your images".into());
    let width = (ui.available_width() - 480.).clamp(40., 260.);
    let label = egui::Label::new(RichText::new(title).size(15.)).truncate();

    ui.add_sized([width, 34.], label).on_hover_text(tooltip);
}

fn file_menu(ui: &mut egui::Ui, state: &ToolbarState<'_>, commands: &mut Vec<Command>) {
    if ui.button("Open image…                    Ctrl+O").clicked() {
        ui.close();
        commands.push(Command::OpenFile);
    }

    if ui.button("Open folder…             Ctrl+Shift+O").clicked() {
        ui.close();
        commands.push(Command::OpenFolder);
    }

    if ui
        .add_enabled(
            state.browser.folder().is_some(),
            egui::Button::new("Refresh folder                              F5"),
        )
        .clicked()
    {
        commands.push(Command::Refresh);
        ui.close();
    }

    ui.separator();
    ui.add_enabled_ui(
        state.image.decoded.is_some() && state.busy.is_none(),
        |ui| {
            if ui
                .button("Copy image                         Ctrl+C")
                .clicked()
            {
                ui.close();
                commands.push(Command::CopyImage);
            }
            if ui
                .button("Export a copy…             Ctrl+Shift+S")
                .clicked()
            {
                ui.close();
                commands.push(Command::Export);
            }
        },
    );
    ui.add_enabled_ui(
        state.browser.current().is_some() && state.busy.is_none(),
        |ui| {
            if ui
                .button("Rename…                                F2")
                .clicked()
            {
                ui.close();
                commands.push(Command::RequestRename);
            }
            if ui
                .button("Move to trash…                     Delete")
                .clicked()
            {
                ui.close();
                commands.push(Command::RequestTrash);
            }
            if ui.button("Show folder in file manager").clicked()
                && let Some(entry) = state.browser.current()
            {
                commands.push(Command::Reveal(entry.path.clone()));
                ui.close();
            }
        },
    );
    ui.separator();
    if ui
        .button("Preferences…                     Ctrl+,")
        .clicked()
    {
        commands.push(Command::OpenPreferences);
        ui.close();
    }

    if ui
        .button("Quit                                  Ctrl+Q")
        .clicked()
    {
        commands.push(Command::Quit);
    }
}

fn window_buttons(
    ui: &mut egui::Ui,
    state: &ToolbarState<'_>,
    commands: &mut Vec<Command>,
) {
    if icons::button(ui, Icon::Fullscreen, "Fullscreen · F11", state.fullscreen)
        .clicked()
    {
        commands.push(Command::ToggleFullscreen);
    }

    if icons::button(
        ui,
        Icon::Settings,
        "Preferences · Ctrl+,",
        state.preferences_open,
    )
    .clicked()
    {
        commands.push(Command::TogglePreferences);
    }

    if icons::button(ui, Icon::Info, "Image details · I", state.settings.info).clicked()
    {
        commands.push(Command::ToggleInspector);
    }

    if icons::button(
        ui,
        Icon::Filmstrip,
        "Thumbnail strip · T",
        state.settings.filmstrip,
    )
    .clicked()
    {
        commands.push(Command::ToggleFilmstrip);
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
        if icons::button(
            ui,
            if playing { Icon::Pause } else { Icon::Play },
            if playing {
                "Pause · Space"
            } else {
                "Play · Space"
            },
            false,
        )
        .clicked()
        {
            commands.push(Command::TogglePlay);
        }
    }

    if !narrow
        && icons::button(ui, Icon::Rotate, "Rotate clockwise · R", false).clicked()
    {
        commands.push(Command::RotateClockwise);
    }

    if icons::button(ui, Icon::Fit, "Fit image · 0", state.canvas.view.fit).clicked() {
        commands.push(Command::Fit);
    }

    if !narrow
        && icons::button(
            ui,
            Icon::Actual,
            "Actual pixels · 1",
            !state.canvas.view.fit
                && (state.canvas.view.zoom * ctx.pixels_per_point() - 1.).abs() < 0.001,
        )
        .clicked()
    {
        commands.push(Command::ActualPixels);
    }

    if icons::button(ui, Icon::Plus, "Zoom in · +", false).clicked() {
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

    if icons::button(ui, Icon::Minus, "Zoom out · -", false).clicked() {
        commands.push(Command::Zoom(0.8));
    }
}

pub(crate) fn status(root: &mut egui::Ui, state: &ToolbarState<'_>) {
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
    } else {
        status_label(ui, "Drop an image or folder to begin", state.colors);
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
        ui.spinner();
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
    if ui.button("Fit image                         0").clicked() {
        commands.push(Command::Fit);
        ui.close();
    }

    if ui.button("Actual pixels                    1").clicked() {
        commands.push(Command::ActualPixels);
        ui.close();
    }

    if ui.button("Rotate clockwise              R").clicked() {
        commands.push(Command::RotateClockwise);
        ui.close();
    }

    if ui.button("Flip horizontally               H").clicked() {
        commands.push(Command::FlipHorizontal);
        ui.close();
    }

    if ui.button("Flip vertically                   V").clicked() {
        commands.push(Command::FlipVertical);
        ui.close();
    }

    ui.separator();
    if ui
        .button(if state.slideshow {
            "Stop slideshow"
        } else {
            "Start slideshow"
        })
        .clicked()
    {
        commands.push(Command::ToggleSlideshow);
        ui.close();
    }

    if state.image.animated()
        && ui
            .button(if state.image.playback.playing {
                "Pause animation"
            } else {
                "Play animation"
            })
            .clicked()
    {
        commands.push(Command::TogglePlay);
        ui.close();
    }

    ui.separator();
    if ui.button("Copy image                  Ctrl+C").clicked() {
        commands.push(Command::CopyImage);
        ui.close();
    }

    if ui.button("Export a copy…").clicked() {
        commands.push(Command::Export);
        ui.close();
    }

    if ui.button("Rename…").clicked() {
        commands.push(Command::RequestRename);
        ui.close();
    }

    if ui.button("Move to trash…").clicked() {
        commands.push(Command::RequestTrash);
        ui.close();
    }

    ui.separator();
    ui.menu_button("Sort folder", |ui| {
        let old = (state.settings.sort, state.settings.reverse_sort);
        let (mut order, mut reverse) = old;
        for sort in [SortOrder::Name, SortOrder::Modified, SortOrder::Size] {
            ui.selectable_value(&mut order, sort, sort.name());
        }
        ui.separator();
        ui.checkbox(&mut reverse, "Reverse order");
        if old != (order, reverse) {
            commands.push(Command::Sort { order, reverse });
        }
    });
}
