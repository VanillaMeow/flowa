//! Unit tests for the ui area

mod dialogs {
    use std::path::PathBuf;

    use crate::{
        app::Command,
        settings::{Accent, Theme},
        ui::{dialogs::Dialogs, theme},
    };

    #[test]
    fn rename_enter_returns_the_path_and_name_from_the_dialog() {
        let ctx = egui::Context::default();
        let colors = theme::colors(Theme::Nord, Accent::Theme, true);
        let mut dialogs = Dialogs::default();
        let mut commands = Vec::new();
        let path = PathBuf::from("original.png");
        dialogs.rename(path.clone(), "renamed.png".into());
        let mut output = ctx.run_ui(egui::RawInput::default(), |_| {
            dialogs.show(&ctx, colors, &mut commands)
        });
        output.textures_delta.clear();
        assert!(commands.is_empty());

        let input = egui::RawInput {
            events: vec![egui::Event::Key {
                key: egui::Key::Enter,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }],
            ..Default::default()
        };
        let mut output =
            ctx.run_ui(input, |_| dialogs.show(&ctx, colors, &mut commands));
        output.textures_delta.clear();
        assert_eq!(
            commands,
            vec![Command::ConfirmRename {
                path,
                name: "renamed.png".into()
            }]
        );
        assert!(!dialogs.is_open());
    }
}

mod keyboard {
    use crate::{app::Command, ui::keyboard::read};

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
}
