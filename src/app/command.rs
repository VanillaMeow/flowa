//! User intentions shared by UI components and the application

use std::path::PathBuf;

use crate::settings::SortOrder;

#[derive(Clone, Debug, PartialEq)]
pub enum Command {
    File(FileCommand),
    Browse(BrowseCommand),
    View(ViewCommand),
    Playback(PlaybackCommand),
    Window(WindowCommand),
}

/// Opening, copying, exporting, and changing files
#[derive(Clone, Debug, PartialEq)]
pub enum FileCommand {
    OpenFile,
    OpenFolder,
    OpenPath(PathBuf),
    Refresh,
    CopyImage,
    CopyPath,
    Export,
    Reveal,
    RequestRename,
    RequestTrash,
    ConfirmRename { path: PathBuf, name: String },
    ConfirmTrash(PathBuf),
}

/// Moving through and ordering the current folder
#[derive(Clone, Debug, PartialEq)]
pub enum BrowseCommand {
    Navigate(isize),
    Select(usize),
    Preload(usize),
    SelectFirst,
    SelectLast,
    Sort { order: SortOrder, reverse: bool },
}

/// Zooming, rotating, and flipping the canvas
#[derive(Clone, Debug, PartialEq)]
pub enum ViewCommand {
    Fit,
    ActualPixels,
    Zoom(f32),
    RotateClockwise,
    RotateCounterclockwise,
    FlipHorizontal,
    FlipVertical,
}

/// Videos, animations, and slideshows
#[derive(Clone, Debug, PartialEq)]
pub enum PlaybackCommand {
    TogglePlay,
    Seek(f64),
    SetVolume(f64),
    ToggleSlideshow,
}

/// Panels, windows, and leaving the app
#[derive(Clone, Debug, PartialEq)]
pub enum WindowCommand {
    ToggleFullscreen,
    ToggleFilmstrip,
    ToggleInspector,
    TogglePreferences,
    Escape,
    Quit,
}

impl From<FileCommand> for Command {
    fn from(command: FileCommand) -> Self {
        Self::File(command)
    }
}

impl From<BrowseCommand> for Command {
    fn from(command: BrowseCommand) -> Self {
        Self::Browse(command)
    }
}

impl From<ViewCommand> for Command {
    fn from(command: ViewCommand) -> Self {
        Self::View(command)
    }
}

impl From<PlaybackCommand> for Command {
    fn from(command: PlaybackCommand) -> Self {
        Self::Playback(command)
    }
}

impl From<WindowCommand> for Command {
    fn from(command: WindowCommand) -> Self {
        Self::Window(command)
    }
}
