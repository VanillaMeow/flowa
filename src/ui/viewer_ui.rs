//! Owned UI components constructed by the application

use super::{
    canvas::ImageCanvas, dialogs::Dialogs, filmstrip::Filmstrip,
    preferences::PreferencesWindow,
};

pub(crate) struct ViewerUi {
    pub canvas: ImageCanvas,
    pub filmstrip: Filmstrip,
    pub preferences: PreferencesWindow,
    pub dialogs: Dialogs,
    pub fullscreen: bool,
}

impl ViewerUi {
    pub fn new(ctx: egui::Context, nearest: bool, preferences_open: bool) -> Self {
        Self {
            canvas: ImageCanvas::new(ctx.clone(), nearest),
            filmstrip: Filmstrip::new(ctx),
            preferences: PreferencesWindow::new(preferences_open),
            dialogs: Dialogs::default(),
            fullscreen: false,
        }
    }

    pub fn controls_visible(&self, ctx: &egui::Context) -> bool {
        !self.fullscreen
            || self.preferences.open
            || self.dialogs.is_open()
            || ctx.input(|input| {
                input.pointer.hover_pos().is_some_and(|point| point.y < 60.)
            })
    }
}
