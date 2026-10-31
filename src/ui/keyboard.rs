//! Translate keyboard input into application commands

use crate::app::Command;

pub(crate) fn read(
    ctx: &egui::Context,
    modal_open: bool,
    mut preferences_open: bool,
) -> Vec<Command> {
    let mut commands = Vec::new();
    let key = |modifiers, key| ctx.input_mut(|i| i.consume_key(modifiers, key));
    if key(egui::Modifiers::CTRL, egui::Key::Q) {
        commands.push(Command::Quit);
        return commands;
    }

    if key(egui::Modifiers::CTRL | egui::Modifiers::SHIFT, egui::Key::O) {
        commands.push(Command::OpenFolder);
    } else if key(egui::Modifiers::CTRL, egui::Key::O) {
        commands.push(Command::OpenFile);
    }

    if key(egui::Modifiers::CTRL, egui::Key::Comma) {
        preferences_open = !preferences_open;
        commands.push(Command::TogglePreferences);
    }

    // Modal dialogs handle their own Escape and Enter keys
    if modal_open {
        return commands;
    }

    if key(egui::Modifiers::NONE, egui::Key::Escape) {
        commands.push(Command::Escape);
        return commands;
    }

    // Typing a filename must not also navigate or rotate the image
    if ctx.text_edit_focused() || preferences_open || egui::Popup::is_any_open(ctx) {
        return commands;
    }

    if key(egui::Modifiers::NONE, egui::Key::ArrowRight) {
        commands.push(Command::Navigate(1));
    }

    if key(egui::Modifiers::NONE, egui::Key::F5) {
        commands.push(Command::Refresh);
    }

    if key(egui::Modifiers::NONE, egui::Key::ArrowLeft) {
        commands.push(Command::Navigate(-1));
    }

    if key(egui::Modifiers::NONE, egui::Key::Home) {
        commands.push(Command::SelectFirst);
    }

    if key(egui::Modifiers::NONE, egui::Key::End) {
        commands.push(Command::SelectLast);
    }

    if key(egui::Modifiers::NONE, egui::Key::F11)
        || key(egui::Modifiers::NONE, egui::Key::F)
    {
        commands.push(Command::ToggleFullscreen);
    }

    if key(egui::Modifiers::NONE, egui::Key::T) {
        commands.push(Command::ToggleFilmstrip);
    }

    if key(egui::Modifiers::NONE, egui::Key::I) {
        commands.push(Command::ToggleInspector);
    }

    if key(egui::Modifiers::SHIFT, egui::Key::R) {
        commands.push(Command::RotateCounterclockwise);
    } else if key(egui::Modifiers::NONE, egui::Key::R) {
        commands.push(Command::RotateClockwise);
    }

    if key(egui::Modifiers::NONE, egui::Key::H) {
        commands.push(Command::FlipHorizontal);
    }

    if key(egui::Modifiers::NONE, egui::Key::V) {
        commands.push(Command::FlipVertical);
    }

    if key(egui::Modifiers::NONE, egui::Key::Num0) {
        commands.push(Command::Fit);
    }

    if key(egui::Modifiers::NONE, egui::Key::Num1) {
        commands.push(Command::ActualPixels);
    }

    if key(egui::Modifiers::NONE, egui::Key::Plus)
        || key(egui::Modifiers::NONE, egui::Key::Equals)
    {
        commands.push(Command::Zoom(1.25));
    }

    if key(egui::Modifiers::NONE, egui::Key::Minus) {
        commands.push(Command::Zoom(0.8));
    }

    if key(egui::Modifiers::NONE, egui::Key::Space) {
        commands.push(Command::TogglePlay);
    }

    if key(egui::Modifiers::CTRL | egui::Modifiers::SHIFT, egui::Key::C) {
        commands.push(Command::CopyPath);
    } else if key(egui::Modifiers::CTRL, egui::Key::C) {
        commands.push(Command::CopyImage);
    }

    if key(egui::Modifiers::CTRL | egui::Modifiers::SHIFT, egui::Key::S) {
        commands.push(Command::Export);
    }

    if key(egui::Modifiers::NONE, egui::Key::F2) {
        commands.push(Command::RequestRename);
    }

    if key(egui::Modifiers::NONE, egui::Key::Delete) {
        commands.push(Command::RequestTrash);
    }

    commands
}

#[cfg(test)]
mod tests {
    use super::*;

    fn press(
        ctx: &egui::Context,
        key: egui::Key,
        modifiers: egui::Modifiers,
        modal: bool,
        preferences: bool,
    ) -> Vec<Command> {
        let input = egui::RawInput {
            events: vec![egui::Event::Key {
                key,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers,
            }],
            ..Default::default()
        };
        let mut commands = Vec::new();
        let mut output =
            ctx.run_ui(input, |_| commands.extend(read(ctx, modal, preferences)));
        output.textures_delta.clear();
        commands
    }

    #[test]
    fn modal_and_preferences_suppress_navigation_shortcuts() {
        let ctx = egui::Context::default();
        assert!(
            press(
                &ctx,
                egui::Key::ArrowRight,
                egui::Modifiers::NONE,
                true,
                false
            )
            .is_empty()
        );
        assert!(
            press(
                &ctx,
                egui::Key::ArrowRight,
                egui::Modifiers::NONE,
                false,
                true
            )
            .is_empty()
        );
        assert_eq!(
            press(
                &ctx,
                egui::Key::ArrowRight,
                egui::Modifiers::NONE,
                false,
                false
            ),
            vec![Command::Navigate(1)]
        );
    }

    #[test]
    fn preferences_shortcut_remains_available_during_a_modal() {
        let ctx = egui::Context::default();
        assert_eq!(
            press(&ctx, egui::Key::Comma, egui::Modifiers::CTRL, true, false),
            vec![Command::TogglePreferences]
        );
    }
}
