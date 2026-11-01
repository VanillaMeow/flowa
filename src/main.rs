#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

mod app;
mod cli;
mod services;
mod settings;
mod ui;
mod utils;
mod viewer;

use std::sync::Arc;

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

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("flowa")
            .with_app_id("flowa")
            .with_inner_size(settings.window_size)
            .with_min_inner_size([640., 400.])
            .with_drag_and_drop(true)
            .with_icon(Arc::new(app_icon())),
        renderer: eframe::Renderer::Wgpu,
        ..Default::default()
    };

    eframe::run_native(
        "flowa",
        options,
        Box::new(move |cc| {
            Ok(Box::new(app::Flowa::new(cc, settings, persistence, launch)))
        }),
    )
    .map_err(|error| anyhow::anyhow!("Could not start Flowa: {error}"))
}

fn app_icon() -> egui::IconData {
    let tree = resvg::usvg::Tree::from_data(
        include_bytes!("../assets/flowa.svg"),
        &resvg::usvg::Options::default(),
    );
    if let (Ok(tree), Some(mut pixmap)) =
        (tree, resvg::tiny_skia::Pixmap::new(128, 128))
    {
        resvg::render(
            &tree,
            resvg::tiny_skia::Transform::identity(),
            &mut pixmap.as_mut(),
        );
        egui::IconData {
            rgba: pixmap.take(),
            width: 128,
            height: 128,
        }
    } else {
        egui::IconData::default()
    }
}
