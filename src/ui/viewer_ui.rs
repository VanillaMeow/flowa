//! Owned UI components constructed by the application

use super::{
    canvas::ImageCanvas, dialogs::Dialogs, filmstrip::Filmstrip,
    preferences::PreferencesWindow, video_controls::VideoControls,
};

pub(crate) struct ViewerUi {
    pub canvas: ImageCanvas,
    pub filmstrip: Filmstrip,
    pub video_controls: VideoControls,
    pub preferences: PreferencesWindow,
    pub dialogs: Dialogs,
    pub fullscreen: bool,
}

impl ViewerUi {
    pub fn new(ctx: egui::Context, nearest: bool, preferences_open: bool) -> Self {
        Self {
            canvas: ImageCanvas::new(ctx.clone(), nearest),
            filmstrip: Filmstrip::new(ctx),
            video_controls: VideoControls::default(),
            preferences: PreferencesWindow::new(preferences_open),
            dialogs: Dialogs::default(),
            fullscreen: false,
        }
    }

    pub fn controls_visible(&self, ctx: &egui::Context, video_active: bool) -> bool {
        let bottom_edge = ctx.viewport_rect().bottom() - 240.;

        !self.fullscreen
            || self.preferences.open
            || self.dialogs.is_open()
            || egui::Popup::is_any_open(ctx)
            || ctx.input(|input| {
                input.pointer.hover_pos().is_some_and(|point| {
                    point.y < 60. || video_active && point.y > bottom_edge
                })
            })
    }
}
