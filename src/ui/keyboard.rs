//! Translate keyboard input into application commands

use crate::app::Command;

use super::actions;

pub(crate) fn read(
    ctx: &egui::Context,
    modal_open: bool,
    mut preferences_open: bool,
) -> Vec<Command> {
    let mut commands = Vec::new();

    if actions::QUIT.consume(ctx) {
        return vec![Command::Quit];
    }

    for action in [&actions::OPEN_FOLDER, &actions::OPEN_FILE] {
        if action.consume(ctx) {
            commands.push(action.command.clone());
        }
    }

    if actions::PREFERENCES.consume(ctx) {
        preferences_open = !preferences_open;
        commands.push(Command::TogglePreferences);
    }

    // Modal dialogs handle their own Escape and Enter keys
    if modal_open {
        return commands;
    }

    if actions::ESCAPE.consume(ctx) {
        commands.push(Command::Escape);
        return commands;
    }

    // Text fields and modal UI own their keyboard input
    if ctx.text_edit_focused() || preferences_open || egui::Popup::is_any_open(ctx) {
        return commands;
    }

    // Sliders keep arrow keys while playback and window shortcuts stay available
    if !super::video_controls::wants_keyboard_input(ctx) {
        consume_actions(ctx, actions::NAVIGATION, &mut commands);
    }

    consume_actions(ctx, actions::VIEWER, &mut commands);
    commands
}

fn consume_actions(
    ctx: &egui::Context,
    actions: &[actions::Action],
    commands: &mut Vec<Command>,
) {
    for action in actions {
        if action.consume(ctx) {
            commands.push(action.command.clone());
        }
    }
}
