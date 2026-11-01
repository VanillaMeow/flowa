//! Command-line options and validation

use std::path::PathBuf;

use clap::Parser;

use crate::{app::Launch, settings::Theme};

#[derive(Parser)]
#[command(name = "flowa", version, about, args_override_self = true)]
pub(crate) struct Cli {
    /// Image or folder to open
    #[arg(value_name = "IMAGE_OR_FOLDER")]
    path: Option<PathBuf>,

    /// Open preferences
    #[arg(long)]
    preferences: bool,

    /// Show the thumbnail strip
    #[arg(long)]
    filmstrip: bool,

    /// Show image details
    #[arg(long)]
    info: bool,

    /// Choose an appearance for this session
    #[arg(long, value_name = "NAME", value_parser = parse_theme)]
    pub theme: Option<Theme>,

    #[arg(long, hide = true)]
    smoke_test: bool,

    #[arg(long, hide = true, value_name = "PERCENT", value_parser = parse_zoom)]
    zoom: Option<f32>,

    #[arg(long, hide = true, value_name = "PATH")]
    screenshot: Option<PathBuf>,
}

impl Cli {
    pub fn into_launch(self) -> Launch {
        Launch {
            path: self.path,
            screenshot: self.screenshot,
            smoke_test: self.smoke_test,
            preferences: self.preferences,
            filmstrip: self.filmstrip,
            info: self.info,
            zoom: self.zoom,
        }
    }
}

fn parse_theme(value: &str) -> Result<Theme, String> {
    let normalized = value.to_ascii_lowercase().replace(' ', "");

    Theme::ALL
        .into_iter()
        .find(|theme| theme.name().to_ascii_lowercase().replace(' ', "") == normalized)
        .ok_or_else(|| format!("Unknown theme: {value}"))
}

fn parse_zoom(value: &str) -> Result<f32, String> {
    let percent = value.parse::<f32>().map_err(|error| error.to_string())?;

    if !percent.is_finite() || !(0.1..=12800.).contains(&percent) {
        return Err("Zoom must be between 0.1 and 12800 percent".into());
    }

    Ok(percent)
}
