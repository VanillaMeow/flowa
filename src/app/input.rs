//! Keyboard commands and physical-pixel zoom

use super::*;

impl Flowa {
    pub(super) fn keyboard(&mut self, ctx: &egui::Context) {
        let key = |modifiers, key| ctx.input_mut(|i| i.consume_key(modifiers, key));
        if key(egui::Modifiers::CTRL, egui::Key::Q) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }
        if key(egui::Modifiers::CTRL | egui::Modifiers::SHIFT, egui::Key::O) {
            self.pick(true);
        } else if key(egui::Modifiers::CTRL, egui::Key::O) {
            self.pick(false);
        }
        if key(egui::Modifiers::CTRL, egui::Key::Comma) {
            self.preferences_open = !self.preferences_open;
        }

        // Modal dialogs handle their own Escape and Enter keys
        if self.dialog.is_some() {
            return;
        }
        if key(egui::Modifiers::NONE, egui::Key::Escape) {
            if self.preferences_open {
                self.preferences_open = false;
            } else if self.fullscreen {
                self.toggle_fullscreen(ctx);
            } else {
                self.slideshow = false;
            }
            return;
        }

        // Typing a filename must not also navigate or rotate the image
        if ctx.text_edit_focused()
            || self.preferences_open
            || egui::Popup::is_any_open(ctx)
        {
            return;
        }

        if key(egui::Modifiers::NONE, egui::Key::ArrowRight) {
            self.navigate(1, ctx);
        }
        if key(egui::Modifiers::NONE, egui::Key::F5)
            && let Some(path) = self
                .current()
                .map(|e| e.path.clone())
                .or_else(|| self.folder.clone())
        {
            self.open_path(path, ctx);
        }
        if key(egui::Modifiers::NONE, egui::Key::ArrowLeft) {
            self.navigate(-1, ctx);
        }
        if key(egui::Modifiers::NONE, egui::Key::Home) && !self.files.is_empty() {
            self.select(0, ctx);
        }
        if key(egui::Modifiers::NONE, egui::Key::End) && !self.files.is_empty() {
            self.select(self.files.len() - 1, ctx);
        }

        if key(egui::Modifiers::NONE, egui::Key::F11)
            || key(egui::Modifiers::NONE, egui::Key::F)
        {
            self.toggle_fullscreen(ctx);
        }
        if key(egui::Modifiers::NONE, egui::Key::T) {
            self.settings.filmstrip = !self.settings.filmstrip;
            self.scroll_to_current = true;
        }
        if key(egui::Modifiers::NONE, egui::Key::I) {
            self.settings.info = !self.settings.info;
        }

        if key(egui::Modifiers::SHIFT, egui::Key::R) {
            self.view.rotation = (self.view.rotation + 3) % 4;
        } else if key(egui::Modifiers::NONE, egui::Key::R) {
            self.view.rotation = (self.view.rotation + 1) % 4;
        }
        if key(egui::Modifiers::NONE, egui::Key::H) {
            self.view.flip_x = !self.view.flip_x;
        }
        if key(egui::Modifiers::NONE, egui::Key::V) {
            self.view.flip_y = !self.view.flip_y;
        }

        if key(egui::Modifiers::NONE, egui::Key::Num0) {
            self.view.reset();
        }
        if key(egui::Modifiers::NONE, egui::Key::Num1) {
            self.actual_pixels(ctx);
        }
        if key(egui::Modifiers::NONE, egui::Key::Plus)
            || key(egui::Modifiers::NONE, egui::Key::Equals)
        {
            self.view.zoom_at(
                1.25,
                self.canvas_rect.center(),
                self.canvas_rect.center(),
            );
        }
        if key(egui::Modifiers::NONE, egui::Key::Minus) {
            self.view.zoom_at(
                0.8,
                self.canvas_rect.center(),
                self.canvas_rect.center(),
            );
        }

        if key(egui::Modifiers::NONE, egui::Key::Space) {
            self.toggle_play();
        }

        if key(egui::Modifiers::CTRL | egui::Modifiers::SHIFT, egui::Key::C) {
            if let Some(entry) = self.current() {
                ctx.copy_text(entry.path.to_string_lossy().into_owned());
                self.notify("Path copied", false);
            }
        } else if key(egui::Modifiers::CTRL, egui::Key::C) {
            self.copy_image();
        }
        if key(egui::Modifiers::CTRL | egui::Modifiers::SHIFT, egui::Key::S) {
            self.export();
        }
        if key(egui::Modifiers::NONE, egui::Key::F2) {
            self.rename_dialog();
        }
        if key(egui::Modifiers::NONE, egui::Key::Delete) {
            self.trash_dialog();
        }
    }

    pub(super) fn actual_pixels(&mut self, ctx: &egui::Context) {
        self.view.fit = false;
        // Egui measures in logical points, but 100% means one physical pixel
        self.view.zoom = 1.0 / ctx.pixels_per_point();
        self.view.pan = Vec2::ZERO;
    }
}
