//! Execute application commands after UI borrows end

use anyhow::Result;

use crate::{
    services::file_actions::{self, Task},
    settings::SortOrder,
};

use super::{
    Command, Flowa,
    command::{
        BrowseCommand, FileCommand, PlaybackCommand, ViewCommand, WindowCommand,
    },
};

impl Flowa {
    pub(super) fn handle_commands(
        &mut self,
        commands: Vec<Command>,
        ctx: &egui::Context,
    ) {
        // Each panel releases its borrows before application state can change
        for command in commands {
            self.handle(command, ctx);
        }
    }

    pub(super) fn handle(&mut self, command: Command, ctx: &egui::Context) {
        match command {
            Command::File(command) => self.handle_file(command, ctx),
            Command::Browse(command) => self.handle_browse(command, ctx),
            Command::View(command) => self.handle_view(command, ctx),
            Command::Playback(command) => self.handle_playback(command),
            Command::Window(command) => self.handle_window(command, ctx),
        }
    }

    fn handle_file(&mut self, command: FileCommand, ctx: &egui::Context) {
        match command {
            FileCommand::OpenFile => self.pick(false),
            FileCommand::OpenFolder => self.pick(true),
            FileCommand::OpenPath(path) => self.open(path, ctx),
            FileCommand::Refresh => self.refresh(ctx),
            FileCommand::CopyImage => self.copy_image(),
            FileCommand::CopyPath => self.copy_path(ctx),
            FileCommand::Export => self.export(),
            FileCommand::Reveal => self.reveal(),
            FileCommand::RequestRename => self.request_rename(),
            FileCommand::RequestTrash => self.request_trash(),
            FileCommand::ConfirmRename { path, name } => {
                let task = Task::Rename { source: path, name };
                self.start_task(task, "Renaming image…");
            }
            FileCommand::ConfirmTrash(path) => {
                self.start_task(Task::Trash(path), "Moving to trash…");
            }
        }
    }

    fn handle_browse(&mut self, command: BrowseCommand, ctx: &egui::Context) {
        match command {
            BrowseCommand::Navigate(delta) => self.navigate(delta, ctx),
            BrowseCommand::Select(index) => self.select(index, ctx),
            BrowseCommand::Preload(index) => self.preload_image(index),
            BrowseCommand::SelectFirst => self.select(0, ctx),
            BrowseCommand::SelectLast => self.select_last(ctx),
            BrowseCommand::Sort { order, reverse } => self.sort(order, reverse),
        }
    }

    fn handle_view(&mut self, command: ViewCommand, ctx: &egui::Context) {
        let canvas = &mut self.ui.canvas;

        match command {
            ViewCommand::Fit => canvas.view.reset(),
            ViewCommand::ActualPixels => {
                canvas.view.show_actual_pixels(ctx.pixels_per_point())
            }
            ViewCommand::Zoom(factor) => canvas.zoom(factor),
            ViewCommand::RotateClockwise => canvas.view.rotate_clockwise(),
            ViewCommand::RotateCounterclockwise => {
                canvas.view.rotate_counterclockwise()
            }
            ViewCommand::FlipHorizontal => canvas.view.flip_horizontal(),
            ViewCommand::FlipVertical => canvas.view.flip_vertical(),
        }
    }

    fn handle_playback(&mut self, command: PlaybackCommand) {
        match command {
            PlaybackCommand::TogglePlay => self.toggle_play(),
            PlaybackCommand::Seek(position) => {
                let result = self.video.seek(position);
                self.report(result);
            }
            PlaybackCommand::SetVolume(volume) => {
                let result = self.video.set_volume(volume);
                self.report(result);
            }
            PlaybackCommand::ToggleSlideshow => self.toggle_slideshow(),
        }
    }

    fn handle_window(&mut self, command: WindowCommand, ctx: &egui::Context) {
        match command {
            WindowCommand::ToggleFullscreen => self.toggle_fullscreen(ctx),
            WindowCommand::ToggleFilmstrip => self.toggle_filmstrip(),
            WindowCommand::ToggleInspector => self.ui.inspector = !self.ui.inspector,
            WindowCommand::TogglePreferences => {
                self.ui.preferences.open = !self.ui.preferences.open;
            }
            WindowCommand::Escape => self.escape(ctx),
            WindowCommand::Quit => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
        }
    }

    /// Shows a failed action's error as a notice
    fn report(&mut self, result: Result<()>) {
        if let Err(error) = result {
            self.notify_error(&error);
        }
    }

    /// Sends one task to the action worker; its result clears the busy status
    fn start_task(&mut self, task: Task, status: &str) {
        if self.busy.is_some() {
            return;
        }

        if self.actions.request(task) {
            self.busy = Some(status.into());
        } else {
            self.ui.dialogs.notify("File actions are unavailable", true);
        }
    }

    fn pick(&mut self, folder: bool) {
        let start = self.browser.folder().map(|folder| folder.to_path_buf());
        self.start_task(Task::Pick { folder, start }, "Opening…");
    }

    fn refresh(&mut self, ctx: &egui::Context) {
        let Some(path) = self.browser.refresh_path() else {
            return;
        };

        // Refreshing rereads files even when their size and time look unchanged
        self.loader.invalidate();
        self.open_path(path, ctx);
    }

    fn copy_image(&mut self) {
        let Some(pixels) = self.image.current_pixels() else {
            return;
        };

        let view = self.ui.canvas.view.clone();
        self.start_task(Task::Copy { pixels, view }, "Copying image…");
    }

    fn copy_path(&mut self, ctx: &egui::Context) {
        let Some(entry) = self.browser.current() else {
            return;
        };

        ctx.copy_text(entry.path.to_string_lossy().into_owned());
        self.ui.dialogs.notify("Path copied", false);
    }

    fn export(&mut self) {
        let (Some(pixels), Some(entry)) =
            (self.image.current_pixels(), self.browser.current())
        else {
            return;
        };

        let task = Task::Export {
            pixels,
            source: entry.path.clone(),
            view: self.ui.canvas.view.clone(),
        };
        self.start_task(task, "Exporting a copy…");
    }

    fn reveal(&mut self) {
        let Some(entry) = self.browser.current() else {
            return;
        };

        let result = file_actions::reveal(&entry.path);
        self.report(result);
    }

    fn request_rename(&mut self) {
        if self.busy.is_some() {
            return;
        }

        let Some(entry) = self.browser.current() else {
            return;
        };

        let (path, name) = (entry.path.clone(), entry.name.clone());
        self.ui.dialogs.rename(path, name);
    }

    fn request_trash(&mut self) {
        if self.busy.is_some() {
            return;
        }

        let Some(entry) = self.browser.current() else {
            return;
        };

        let path = entry.path.clone();
        self.ui.dialogs.trash(path);
    }

    fn select_last(&mut self, ctx: &egui::Context) {
        if let Some(index) = self.browser.entries().len().checked_sub(1) {
            self.select(index, ctx);
        }
    }

    fn sort(&mut self, order: SortOrder, reverse: bool) {
        self.settings.values.sort = order;
        self.settings.values.reverse_sort = reverse;
        self.browser.sort(order, reverse);
        self.ui.filmstrip.reveal_selection();
        self.preload_neighbors();
    }

    /// Plays the video, else the animation, else the slideshow
    fn toggle_play(&mut self) {
        if self.video.state.active {
            let result = self.video.toggle_play();
            self.report(result);
        } else if self.image.animated() {
            self.image.toggle_play();
        } else {
            self.toggle_slideshow();
        }
    }

    fn toggle_slideshow(&mut self) {
        self.slideshow
            .toggle(self.settings.values.slideshow_seconds);
    }

    fn toggle_fullscreen(&mut self, ctx: &egui::Context) {
        self.ui.fullscreen = !self.ui.fullscreen;
        ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(self.ui.fullscreen));
    }

    fn toggle_filmstrip(&mut self) {
        self.settings.values.filmstrip = !self.settings.values.filmstrip;
        self.ui.filmstrip.reveal_selection();
    }

    /// Closes preferences first, then leaves fullscreen, then stops the slideshow
    fn escape(&mut self, ctx: &egui::Context) {
        if self.ui.preferences.open {
            self.ui.preferences.open = false;
        } else if self.ui.fullscreen {
            self.toggle_fullscreen(ctx);
        } else {
            self.slideshow.active = false;
        }
    }
}
