//! Design tokens shared by every component: spacing, sizes, radii, text, and motion
//!
//! Components use these names instead of literal numbers, so related elements stay
//! aligned and a change here restyles the whole interface

use std::time::Duration;

use egui::Margin;

/// Gaps between and inside components, in points
pub mod space {
    pub const XS: f32 = 4.;
    pub const SM: f32 = 8.;
    pub const MD: f32 = 12.;
    pub const LG: f32 = 16.;
    pub const XL: f32 = 24.;
}

/// Inner margins of panels and floating surfaces
pub mod margin {
    use super::Margin;

    pub const TOOLBAR: Margin = Margin::symmetric(14, 9);
    pub const STATUS_BAR: Margin = Margin::symmetric(18, 4);
    pub const BOTTOM_PANEL: Margin = Margin::symmetric(14, 10);
    pub const SIDE_PANEL: Margin = Margin::same(20);
    pub const SECTION: Margin = Margin::symmetric(14, 12);
    pub const NOTICE: Margin = Margin::symmetric(12, 6);
}

/// Corner radii, in points
pub mod radius {
    /// Buttons, inputs, and thumbnails
    pub const CONTROL: u8 = 7;
    /// Sections and notices
    pub const CARD: u8 = 10;
    pub const WINDOW: u8 = 12;
}

/// Fixed component sizes, in points
pub mod size {
    /// Icon buttons and the height of control rows
    pub const CONTROL: f32 = 34.;
    pub const ICON: f32 = 18.;
    pub const SMALL_ICON: f32 = 15.;
    pub const TOOLBAR: f32 = 58.;
    pub const STATUS_BAR: f32 = 32.;
}

/// Font sizes, in points
pub mod text {
    /// Uppercase labels above values and sections
    pub const CAPTION: f32 = 11.;
    pub const SMALL: f32 = 12.;
    pub const LABEL: f32 = 13.;
    pub const BUTTON: f32 = 15.;
    pub const BODY: f32 = 16.;
    pub const HEADING: f32 = 23.;
    pub const DISPLAY: f32 = 34.;
}

/// Animation and notification timing
pub mod motion {
    use super::Duration;

    /// Egui's fade-in for windows, menus, and tooltips, in seconds
    pub const FADE_IN: f32 = 0.16;
    /// Closing is quicker than opening so dismissed elements get out of the way
    pub const FADE_OUT: f32 = 0.1;
    pub const NOTICE: Duration = Duration::from_millis(2500);
    pub const ERROR_NOTICE: Duration = Duration::from_secs(6);
}
