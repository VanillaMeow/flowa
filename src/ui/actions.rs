//! Shared labels, shortcuts, and commands for menus and keyboard input

use egui::{Key, KeyboardShortcut, Modifiers};

use crate::app::Command;

use super::icons::{self, Icon};

pub(crate) struct Action {
    pub label: &'static str,
    pub command: Command,
    shortcuts: &'static [KeyboardShortcut],
}

impl Action {
    const fn new(
        label: &'static str,
        command: Command,
        shortcuts: &'static [KeyboardShortcut],
    ) -> Self {
        Self {
            label,
            command,
            shortcuts,
        }
    }

    pub fn shortcut_text(&self, ctx: &egui::Context) -> String {
        self.shortcuts
            .iter()
            .map(|key| ctx.format_shortcut(key))
            .collect::<Vec<_>>()
            .join(" / ")
    }

    pub fn consume(&self, ctx: &egui::Context) -> bool {
        ctx.input_mut(|input| {
            let mut consumed = false;

            for shortcut in self.shortcuts {
                consumed |= input.consume_shortcut(shortcut);
            }

            consumed
        })
    }

    pub fn button(
        &self,
        ui: &mut egui::Ui,
        icon: Icon,
        selected: bool,
    ) -> egui::Response {
        let label = format!(
            "{} · {}",
            self.label.trim_end_matches('…'),
            self.shortcut_text(ui.ctx())
        );
        icons::button(ui, icon, &label, selected)
    }

    pub fn menu(&self, ui: &mut egui::Ui, enabled: bool, commands: &mut Vec<Command>) {
        self.menu_label(ui, self.label, enabled, false, commands);
    }

    pub fn menu_label(
        &self,
        ui: &mut egui::Ui,
        label: &str,
        enabled: bool,
        selected: bool,
        commands: &mut Vec<Command>,
    ) {
        let button = egui::Button::new(label)
            .shortcut_text(self.shortcut_text(ui.ctx()))
            .selected(selected);

        if ui.add_enabled(enabled, button).clicked() {
            commands.push(self.command.clone());
            ui.close();
        }
    }
}

const fn key(key: Key) -> KeyboardShortcut {
    KeyboardShortcut::new(Modifiers::NONE, key)
}

const fn ctrl(key: Key) -> KeyboardShortcut {
    KeyboardShortcut::new(Modifiers::CTRL, key)
}

const fn ctrl_shift(key: Key) -> KeyboardShortcut {
    KeyboardShortcut::new(
        Modifiers {
            shift: true,
            ..Modifiers::CTRL
        },
        key,
    )
}

pub const OPEN_FILE: Action =
    Action::new("Open file…", Command::OpenFile, &[ctrl(Key::O)]);
pub const OPEN_FOLDER: Action =
    Action::new("Open folder…", Command::OpenFolder, &[ctrl_shift(Key::O)]);
pub const PREFERENCES: Action = Action::new(
    "Preferences…",
    Command::TogglePreferences,
    &[ctrl(Key::Comma)],
);
pub const QUIT: Action = Action::new("Quit", Command::Quit, &[ctrl(Key::Q)]);
pub const ESCAPE: Action = Action::new(
    "Close dialog / leave fullscreen",
    Command::Escape,
    &[key(Key::Escape)],
);
pub const PREVIOUS: Action = Action::new(
    "Previous file",
    Command::Navigate(-1),
    &[key(Key::ArrowLeft)],
);
pub const NEXT: Action =
    Action::new("Next file", Command::Navigate(1), &[key(Key::ArrowRight)]);
pub const FIRST: Action =
    Action::new("First file", Command::SelectFirst, &[key(Key::Home)]);
pub const LAST: Action =
    Action::new("Last file", Command::SelectLast, &[key(Key::End)]);
pub const REFRESH: Action =
    Action::new("Refresh folder", Command::Refresh, &[key(Key::F5)]);
pub const FULLSCREEN: Action = Action::new(
    "Fullscreen",
    Command::ToggleFullscreen,
    &[key(Key::F11), key(Key::F)],
);
pub const FILMSTRIP: Action =
    Action::new("Thumbnail strip", Command::ToggleFilmstrip, &[key(Key::T)]);
pub const INSPECTOR: Action =
    Action::new("File details", Command::ToggleInspector, &[key(Key::I)]);
pub const ROTATE_BACK: Action = Action::new(
    "Rotate counterclockwise",
    Command::RotateCounterclockwise,
    &[KeyboardShortcut::new(Modifiers::SHIFT, Key::R)],
);
pub const ROTATE: Action =
    Action::new("Rotate clockwise", Command::RotateClockwise, &[key(Key::R)]);
pub const FLIP_HORIZONTAL: Action =
    Action::new("Flip horizontally", Command::FlipHorizontal, &[key(Key::H)]);
pub const FLIP_VERTICAL: Action =
    Action::new("Flip vertically", Command::FlipVertical, &[key(Key::V)]);
pub const FIT: Action = Action::new("Fit to window", Command::Fit, &[key(Key::Num0)]);
pub const ACTUAL_PIXELS: Action =
    Action::new("Actual pixels", Command::ActualPixels, &[key(Key::Num1)]);
pub const ZOOM_IN: Action = Action::new(
    "Zoom in",
    Command::Zoom(1.25),
    &[key(Key::Plus), key(Key::Equals)],
);
pub const ZOOM_OUT: Action =
    Action::new("Zoom out", Command::Zoom(0.8), &[key(Key::Minus)]);
pub const PLAY: Action =
    Action::new("Play / pause", Command::TogglePlay, &[key(Key::Space)]);
pub const SLIDESHOW: Action = Action::new("Slideshow", Command::ToggleSlideshow, &[]);
pub const COPY_PATH: Action =
    Action::new("Copy file path", Command::CopyPath, &[ctrl_shift(Key::C)]);
pub const COPY_IMAGE: Action =
    Action::new("Copy image", Command::CopyImage, &[ctrl(Key::C)]);
pub const EXPORT: Action =
    Action::new("Export a copy…", Command::Export, &[ctrl_shift(Key::S)]);
pub const RENAME: Action =
    Action::new("Rename…", Command::RequestRename, &[key(Key::F2)]);
pub const TRASH: Action =
    Action::new("Move to trash…", Command::RequestTrash, &[key(Key::Delete)]);

pub const NAVIGATION: &[Action] = &[PREVIOUS, NEXT, FIRST, LAST];

// More specific modifier combinations must consume input before their plain variants
pub const VIEWER: &[Action] = &[
    REFRESH,
    FULLSCREEN,
    FILMSTRIP,
    INSPECTOR,
    ROTATE_BACK,
    ROTATE,
    FLIP_HORIZONTAL,
    FLIP_VERTICAL,
    FIT,
    ACTUAL_PIXELS,
    ZOOM_IN,
    ZOOM_OUT,
    PLAY,
    COPY_PATH,
    COPY_IMAGE,
    EXPORT,
    RENAME,
    TRASH,
];

pub const GLOBAL: &[Action] = &[OPEN_FOLDER, OPEN_FILE, PREFERENCES, QUIT, ESCAPE];
