//! Owned UI components constructed by the application

use crate::settings::Settings;

use super::{
    canvas::ImageCanvas, dialogs::Dialogs, filmstrip::Filmstrip,
    preferences::PreferencesWindow, video_controls::VideoControls,
};

/// Pointer zones that reveal the hidden toolbar and video controls in fullscreen
const TOOLBAR_REVEAL_HEIGHT: f32 = 60.;
const VIDEO_CONTROLS_REVEAL_HEIGHT: f32 = 240.;

pub(crate) struct ViewerUi {
    pub canvas: ImageCanvas,
    pub filmstrip: Filmstrip,
    pub video_controls: VideoControls,
    pub preferences: PreferencesWindow,
    pub dialogs: Dialogs,
    pub fullscreen: bool,
}

impl ViewerUi {
    pub fn new(
        ctx: egui::Context,
        settings: &Settings,
        preferences_open: bool,
    ) -> Self {
        Self {
            canvas: ImageCanvas::new(ctx.clone(), settings.nearest),
            filmstrip: Filmstrip::new(ctx, settings.memory_budget()),
            video_controls: VideoControls::default(),
            preferences: PreferencesWindow::new(preferences_open),
            dialogs: Dialogs::default(),
            fullscreen: false,
        }
    }

    pub fn controls_visible(&self, ctx: &egui::Context, video_active: bool) -> bool {
        if !self.fullscreen
            || self.preferences.open
            || self.dialogs.is_open()
            || egui::Popup::is_any_open(ctx)
        {
            return true;
        }

        let bottom_edge = ctx.viewport_rect().bottom() - VIDEO_CONTROLS_REVEAL_HEIGHT;
        let Some(pointer) = ctx.input(|input| input.pointer.hover_pos()) else {
            return false;
        };

        pointer.y < TOOLBAR_REVEAL_HEIGHT || video_active && pointer.y > bottom_edge
    }
}
