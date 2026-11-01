//! Execute application commands after UI borrows end

use crate::services::file_actions::Task;

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
            Command::OpenPath(path) => self.open_path(path, ctx),
            Command::Refresh => {
                if let Some(path) = self.browser.refresh_path() {
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
            Command::Reveal(path) => self.actions.request(Task::Reveal(path)),

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
                if self.busy.is_none() {
                    self.actions.request(Task::Rename { source: path, name });
                    self.busy = Some("Renaming image…".into());
                }
            }
            Command::ConfirmTrash(path) => {
                if self.busy.is_none() {
                    self.actions.request(Task::Trash(path));
                    self.busy = Some("Moving to trash…".into());
                }
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
                if self.image.animated() {
                    self.image.toggle_play();
                } else {
                    self.slideshow
                        .toggle(self.settings.values.slideshow_seconds);
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
            Command::ToggleInspector => {
                self.settings.values.info = !self.settings.values.info
            }

            Command::OpenPreferences => self.ui.preferences.open = true,
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

    fn pick(&mut self, folder: bool) {
        if self.busy.is_none() {
            self.busy = Some("Opening…".into());
            self.actions.request(Task::Pick {
                folder,
                start: self.browser.folder().map(|folder| folder.to_path_buf()),
            });
        }
    }

    fn copy_image(&mut self) {
        if self.busy.is_some() {
            return;
        }

        if let Some(pixels) = self.image.current_pixels() {
            self.busy = Some("Copying image…".into());
            self.actions.request(Task::Copy {
                pixels,
                view: self.ui.canvas.view.clone(),
            });
        }
    }

    fn export(&mut self) {
        if self.busy.is_some() {
            return;
        }

        if let (Some(pixels), Some(entry)) =
            (self.image.current_pixels(), self.browser.current())
        {
            self.actions.request(Task::Export {
                pixels,
                source: entry.path.clone(),
                view: self.ui.canvas.view.clone(),
            });
            self.busy = Some("Exporting a copy…".into());
        }
    }
}
