//! Execute application commands after UI borrows end

use crate::services::file_actions::{self, Task};

use super::{Command, Flowa};

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
            Command::OpenFile => self.pick(false),
            Command::OpenFolder => self.pick(true),
            Command::OpenPath(path) => self.open(path, ctx),
            Command::Refresh => {
                if let Some(path) = self.browser.refresh_path() {
                    // Refreshing rereads files even when their size and time look unchanged
                    self.loader.invalidate();
                    self.open_path(path, ctx);
                }
            }

            Command::Navigate(delta) => self.navigate(delta, ctx),
            Command::SelectImage(index) => self.select(index, ctx),
            Command::PreloadImage(index) => self.preload_image(index),
            Command::SelectFirst => self.select(0, ctx),
            Command::SelectLast => {
                if let Some(index) = self.browser.entries().len().checked_sub(1) {
                    self.select(index, ctx);
                }
            }
            Command::Sort { order, reverse } => {
                self.settings.values.sort = order;
                self.settings.values.reverse_sort = reverse;
                self.browser.sort(order, reverse);
                self.ui.filmstrip.reveal_selection();
                self.preload_neighbors();
            }

            Command::CopyImage => self.copy_image(),
            Command::CopyPath => {
                if let Some(entry) = self.browser.current() {
                    ctx.copy_text(entry.path.to_string_lossy().into_owned());
                    self.ui.dialogs.notify("Path copied", false);
                }
            }
            Command::Export => self.export(),
            Command::Reveal => self.reveal(),

            Command::RequestRename => {
                if self.busy.is_none()
                    && let Some(entry) = self.browser.current()
                {
                    self.ui
                        .dialogs
                        .rename(entry.path.clone(), entry.name.clone());
                }
            }
            Command::RequestTrash => {
                if self.busy.is_none()
                    && let Some(entry) = self.browser.current()
                {
                    self.ui.dialogs.trash(entry.path.clone());
                }
            }
            Command::ConfirmRename { path, name } => {
                self.start_task(Task::Rename { source: path, name }, "Renaming image…");
            }
            Command::ConfirmTrash(path) => {
                self.start_task(Task::Trash(path), "Moving to trash…");
            }

            Command::Fit => self.ui.canvas.view.reset(),
            Command::ActualPixels => self.ui.canvas.actual_pixels(ctx),
            Command::Zoom(factor) => self.ui.canvas.zoom(factor),
            Command::RotateClockwise => {
                self.ui.canvas.view.rotation = (self.ui.canvas.view.rotation + 1) % 4;
            }
            Command::RotateCounterclockwise => {
                self.ui.canvas.view.rotation = (self.ui.canvas.view.rotation + 3) % 4;
            }
            Command::FlipHorizontal => {
                self.ui.canvas.view.flip_x = !self.ui.canvas.view.flip_x
            }
            Command::FlipVertical => {
                self.ui.canvas.view.flip_y = !self.ui.canvas.view.flip_y
            }

            Command::TogglePlay => {
                if self.video.state.active {
                    if let Err(error) = self.video.toggle_play() {
                        self.notify_error(&error);
                    }
                } else if self.image.animated() {
                    self.image.toggle_play();
                } else {
                    self.slideshow
                        .toggle(self.settings.values.slideshow_seconds);
                }
            }
            Command::SeekVideo(position) => {
                if let Err(error) = self.video.seek(position) {
                    self.notify_error(&error);
                }
            }
            Command::SetVideoVolume(volume) => {
                if let Err(error) = self.video.set_volume(volume) {
                    self.notify_error(&error);
                }
            }
            Command::ToggleSlideshow => self
                .slideshow
                .toggle(self.settings.values.slideshow_seconds),

            Command::ToggleFullscreen => {
                self.ui.fullscreen = !self.ui.fullscreen;
                ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(
                    self.ui.fullscreen,
                ));
            }
            Command::ToggleFilmstrip => {
                self.settings.values.filmstrip = !self.settings.values.filmstrip;
                self.ui.filmstrip.reveal_selection();
            }
            Command::ToggleInspector => self.ui.inspector = !self.ui.inspector,

            Command::TogglePreferences => {
                self.ui.preferences.open = !self.ui.preferences.open
            }

            Command::Escape => {
                if self.ui.preferences.open {
                    self.ui.preferences.open = false;
                } else if self.ui.fullscreen {
                    self.handle(Command::ToggleFullscreen, ctx);
                } else {
                    self.slideshow.active = false;
                }
            }
            Command::Quit => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
        }
    }

    /// Sends one task to the action worker; its result clears the busy status
    fn start_task(&mut self, task: Task, status: &str) {
        if self.busy.is_some() {
            return;
        }

        self.busy = Some(status.into());
        self.actions.request(task);
    }

    fn pick(&mut self, folder: bool) {
        let start = self.browser.folder().map(|folder| folder.to_path_buf());
        self.start_task(Task::Pick { folder, start }, "Opening…");
    }

    fn copy_image(&mut self) {
        let Some(pixels) = self.image.current_pixels() else {
            return;
        };

        let view = self.ui.canvas.view.clone();
        self.start_task(Task::Copy { pixels, view }, "Copying image…");
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

        if let Err(error) = file_actions::reveal(&entry.path) {
            self.notify_error(&error);
        }
    }
}
