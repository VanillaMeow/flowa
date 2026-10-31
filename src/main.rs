#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

mod actions;
mod app;
mod catalog;
mod icons;
mod loader;
mod preferences;
mod settings;
mod theme;
mod view;

use std::{path::PathBuf, sync::Arc};

fn main() -> anyhow::Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn"))
        .init();

    jxl_oxide::integration::register_image_decoding_hook();

    #[cfg(feature = "heif")]
    {
        libheif_rs::integration::image::register_heif_decoding_hook();
        libheif_rs::integration::image::register_heic_decoding_hook();
    }

    let mut launch = app::Launch {
        path: None,
        screenshot: None,
        smoke_test: false,
        preferences: false,
        filmstrip: false,
        info: false,
        zoom: None,
    };
    let mut theme_override = None;

    let mut args = std::env::args_os().skip(1);

    while let Some(arg) = args.next() {
        match arg.to_str() {
            Some("--help" | "-h") => {
                println!(
                    "flowa {}\n\nUsage: flowa [IMAGE OR FOLDER]\n\n  --preferences     Open preferences\n  --filmstrip       Show the thumbnail strip\n  --info            Show image details\n  --theme NAME      Choose an appearance for this session\n  --version         Print version\n  --help            Print this help",
                    env!("CARGO_PKG_VERSION")
                );
                return Ok(());
            }
            Some("--version" | "-V") => {
                println!("flowa {}", env!("CARGO_PKG_VERSION"));
                return Ok(());
            }
            Some("--preferences") => launch.preferences = true,
            Some("--filmstrip") => launch.filmstrip = true,
            Some("--info") => launch.info = true,
            Some("--smoke-test") => launch.smoke_test = true,
            Some("--zoom") => {
                let value = args
                    .next()
                    .ok_or_else(|| anyhow::anyhow!("--zoom requires a percentage"))?;
                let percent = value.to_string_lossy().parse::<f32>()?;
                anyhow::ensure!(
                    percent.is_finite() && (0.1..=12800.0).contains(&percent),
                    "Zoom must be between 0.1 and 12800 percent"
                );
                launch.zoom = Some(percent);
            }
            Some("--screenshot") => {
                launch.screenshot =
                    Some(PathBuf::from(args.next().ok_or_else(|| {
                        anyhow::anyhow!("--screenshot requires a path")
                    })?))
            }
            Some("--theme") => {
                let value = args
                    .next()
                    .ok_or_else(|| anyhow::anyhow!("--theme requires a theme name"))?;
                let value = value.to_string_lossy().to_ascii_lowercase();
                theme_override = Some(
                    theme::Theme::ALL
                        .into_iter()
                        .find(|t| {
                            t.name().to_ascii_lowercase().replace(' ', "")
                                == value.replace(' ', "")
                        })
                        .ok_or_else(|| anyhow::anyhow!("Unknown theme: {value}"))?,
                );
            }
            Some("--") => {
                launch.path = args.next().map(PathBuf::from);
                break;
            }
            Some(s) if s.starts_with('-') => anyhow::bail!("Unknown option: {s}"),
            _ => {
                anyhow::ensure!(
                    launch.path.is_none(),
                    "Open one image or folder at a time"
                );
                launch.path = Some(PathBuf::from(arg));
            }
        }
    }

    let config_path = if launch.smoke_test {
        None
    } else {
        settings::Settings::config_path()
    };

    let mut settings = if let Some(path) = config_path.as_ref().filter(|p| p.exists()) {
        settings::Settings::load(path).unwrap_or_else(|error| {
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
