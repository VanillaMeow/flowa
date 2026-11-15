//! User intentions shared by UI components and the application

use std::path::PathBuf;

use crate::{settings::SortOrder, viewer::adjustments::Adjustments};

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
    ConfirmRename {
        path: PathBuf,
        name: String,
    },
    ConfirmTrash(PathBuf),
    /// Offers the file to the application under the pointer while the button is held
    DragOut(PathBuf),
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

/// Zooming, rotating, flipping, and adjusting the colors of the canvas
#[derive(Clone, Debug, PartialEq)]
pub enum ViewCommand {
    Fit,
    ActualPixels,
    Zoom(f32),
    RotateClockwise,
    RotateCounterclockwise,
    FlipHorizontal,
    FlipVertical,
    /// Brightness, contrast, saturation, and gamma for images and videos
    Adjust(Adjustments),
}

/// Videos, animations, and slideshows
#[derive(Clone, Debug, PartialEq)]
pub enum PlaybackCommand {
    TogglePlay,
    /// Frames to move a video by, pausing it first
    StepFrames(i32),
    Seek(f64),
    SetVolume(f64),
    /// A speed multiplier for videos, kept for later ones
    SetRate(f64),
    /// Moves to the next offered speed, faster for positive steps
    StepRate(i32),
    /// Plays the audio stream with this container index
    SelectAudio(usize),
    /// Shows this subtitle track, or none
    SelectSubtitles(Option<usize>),
    /// Hides subtitles, or shows the preferred or first track
    ToggleSubtitles,
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
