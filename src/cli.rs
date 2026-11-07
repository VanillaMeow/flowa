//! Command-line options and validation

use std::path::PathBuf;

use clap::Parser;

use crate::{app::Launch, settings::Theme};

#[derive(Parser)]
#[command(name = "flowa", version, about, args_override_self = true)]
pub(crate) struct Cli {
    /// Image, video, or folder to open
    #[arg(value_name = "FILE_OR_FOLDER")]
    path: Option<PathBuf>,

    /// Open preferences
    #[arg(long)]
    preferences: bool,

    /// Show the thumbnail strip
    #[arg(long)]
    filmstrip: bool,

    /// Show file details
    #[arg(long)]
    info: bool,

    /// Choose an appearance for this session
    #[arg(long, value_name = "NAME", value_parser = parse_theme)]
    pub theme: Option<Theme>,

    #[arg(long, hide = true, value_name = "PERCENT", value_parser = parse_zoom)]
    zoom: Option<f32>,
}

impl Cli {
    pub fn into_launch(self) -> Launch {
        Launch {
            path: self.path,
            preferences: self.preferences,
            filmstrip: self.filmstrip,
            info: self.info,
            zoom: self.zoom,
        }
    }
}

fn parse_theme(value: &str) -> Result<Theme, String> {
    value
        .replace(' ', "")
        .parse()
        .map_err(|_| format!("Unknown theme: {value}"))
}

fn parse_zoom(value: &str) -> Result<f32, String> {
    let percent = value.parse::<f32>().map_err(|error| error.to_string())?;

    if !percent.is_finite() || !(0.1..=12800.).contains(&percent) {
        return Err("Zoom must be between 0.1 and 12800 percent".into());
    }

    Ok(percent)
}

#[cfg(test)]
mod tests {
    use super::parse_theme;
    use crate::settings::Theme;

    #[test]
    fn theme_names_ignore_case_and_spacing() {
        for (input, expected) in [
            ("Sweet", Theme::Sweet),
            ("sWeEt", Theme::Sweet),
            ("OLED", Theme::Oled),
            ("high contrast", Theme::HighContrast),
            ("HighContrast", Theme::HighContrast),
            ("System / Qt", Theme::System),
            ("SYSTEM/QT", Theme::System),
            ("system", Theme::System),
        ] {
            assert_eq!(parse_theme(input), Ok(expected));
        }

        assert_eq!(parse_theme("missing"), Err("Unknown theme: missing".into()));
    }
}
