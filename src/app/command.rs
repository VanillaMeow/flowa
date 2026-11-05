//! User intentions shared by UI components and the application

use std::path::PathBuf;

use crate::settings::SortOrder;

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Command {
    OpenFile,
    OpenFolder,
    OpenPath(PathBuf),
    Refresh,
    Navigate(isize),
    SelectImage(usize),
    PreloadImage(usize),
    SelectFirst,
    SelectLast,
    Sort { order: SortOrder, reverse: bool },
    CopyImage,
    CopyPath,
    Export,
    Reveal(PathBuf),
    RequestRename,
    RequestTrash,
    ConfirmRename { path: PathBuf, name: String },
    ConfirmTrash(PathBuf),
    Fit,
    ActualPixels,
    Zoom(f32),
    RotateClockwise,
    RotateCounterclockwise,
    FlipHorizontal,
    FlipVertical,
    TogglePlay,
    SeekVideo(f64),
    SetVideoVolume(f64),
    ToggleSlideshow,
    ToggleFullscreen,
    ToggleFilmstrip,
    ToggleInspector,
    TogglePreferences,
    Escape,
    Quit,
}
