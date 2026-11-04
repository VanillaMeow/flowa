#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

mod app;
mod cli;
mod services;
mod settings;
mod ui;
mod utils;
mod video;
mod viewer;

use clap::Parser;

fn main() -> anyhow::Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn"))
        .init();

    jxl_oxide::integration::register_image_decoding_hook();

    #[cfg(feature = "heif")]
    {
        libheif_rs::integration::image::register_heif_decoding_hook();
        libheif_rs::integration::image::register_heic_decoding_hook();
    }

    let cli = cli::Cli::parse();
    let theme_override = cli.theme;
    let launch = cli.into_launch();

    let config_path = if launch.smoke_test {
        None
    } else {
        settings::SettingsStore::config_path()
    };

    let mut settings = if let Some(path) = config_path.as_ref().filter(|p| p.exists()) {
        settings::SettingsStore::load(path).unwrap_or_else(|error| {
            eprintln!("Could not load preferences: {error:#}");
            settings::Settings::default()
        })
    } else {
        settings::Settings::default()
    };

    if let Some(theme) = theme_override {
        settings.theme = theme;
    }

    // Session theme overrides must not replace the user's saved appearance
    let persistence = if theme_override.is_some() {
        None
    } else {
        config_path
    };

    app::run(settings, persistence, launch)
        .map_err(|error| anyhow::anyhow!("Could not start Flowa: {error}"))
}
