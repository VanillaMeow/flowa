//! Native window setup and graphics backend fallback

use std::{path::PathBuf, sync::Arc};

use eframe::{NativeOptions, egui_wgpu::WgpuSetup, wgpu::Backends};

use crate::{
    services::image_loader::Loader,
    settings::{RendererPreference, Settings, SettingsStore},
    viewer::formats::{self, MediaKind},
};

use super::{Flowa, Launch};

struct Startup {
    settings: Settings,
    config_path: Option<PathBuf>,
    launch: Launch,
    loader: Loader,
}

pub fn run(
    settings: Settings,
    config_path: Option<PathBuf>,
    launch: Launch,
) -> eframe::Result {
    // The launch image decodes while the graphics device is created below
    let loader = Loader::new(settings.memory_budget());
    if let Some(path) = launch.initial_path(&settings)
        && formats::kind(&path) != Some(MediaKind::Video)
    {
        loader.preload(path);
    }

    let window_size = settings.window_size;
    let mut options = native_options(window_size);
    let renderer = settings.renderer;

    configure_video(&mut options, renderer);
    let mut startup = Some(Startup {
        settings,
        config_path,
        launch,
        loader,
    });

    if std::env::var_os("WGPU_BACKEND").is_some() {
        return run_once(options, &mut startup);
    }

    if let Some(backends) = preferred_backends(renderer) {
        set_backends(&mut options, backends);
        let result = run_once(options, &mut startup);
        if startup.is_none() || !matches!(&result, Err(eframe::Error::Wgpu(_))) {
            return result;
        }

        log::warn!(
            "{} initialization failed; retrying with Auto",
            renderer.as_ref()
        );
        reset_renderer(&mut startup);
        options = native_options(window_size);
        configure_video(&mut options, RendererPreference::Auto);
    }

    let Some((fallback, fallback_name)) = auto_fallback() else {
        return run_once(options, &mut startup);
    };

    set_backends(&mut options, Backends::VULKAN);
    let result = run_once(options.clone(), &mut startup);

    // Startup options are consumed only after graphics initialization succeeds
    if startup.is_none() {
        return result;
    }

    let Err(eframe::Error::Wgpu(error)) = result else {
        return result;
    };

    log::warn!("Vulkan initialization failed: {error}. Retrying with {fallback_name}");
    set_backends(&mut options, fallback);

    run_once(options, &mut startup)
}

fn reset_renderer(startup: &mut Option<Startup>) {
    let Some(startup) = startup else {
        return;
    };
    startup.settings.renderer = RendererPreference::Auto;
    if let Some(path) = &startup.config_path
        && let Err(error) = SettingsStore::save(&startup.settings, path)
    {
        log::warn!("Could not save renderer recovery preference: {error:#}");
    }
}

fn auto_fallback() -> Option<(Backends, &'static str)> {
    if cfg!(target_os = "linux") {
        return Some((Backends::GL, "OpenGL"));
    }

    if cfg!(target_os = "windows") {
        return Some((Backends::DX12, "DirectX 12"));
    }

    None
}

fn preferred_backends(renderer: RendererPreference) -> Option<Backends> {
    match renderer {
        RendererPreference::Auto => None,
        RendererPreference::Vulkan => Some(Backends::VULKAN),
        RendererPreference::OpenGl => Some(Backends::GL),
        RendererPreference::Metal => Some(Backends::METAL),
        RendererPreference::DirectX12 => Some(Backends::DX12),
    }
}

fn run_once(options: NativeOptions, startup: &mut Option<Startup>) -> eframe::Result {
    let video_device =
        matches!(options.wgpu_options.wgpu_setup, WgpuSetup::Existing(_));

    eframe::run_native(
        "flowa",
        options,
        Box::new(move |context| {
            let startup = startup
                .take()
                .ok_or("The application has already started")?;

            Ok(Box::new(Flowa::new(
                context,
                startup.settings,
                startup.config_path,
                startup.launch,
                startup.loader,
                video_device,
            )))
        }),
    )
}

/// Creates the shared Vulkan device that video playback renders into
fn configure_video(options: &mut NativeOptions, renderer: RendererPreference) {
    let vulkan_renderer = matches!(
        renderer,
        RendererPreference::Auto | RendererPreference::Vulkan
    );

    if !cfg!(video_playback) || !vulkan_renderer {
        return;
    }

    if let Some(backend) = std::env::var_os("WGPU_BACKEND")
        && !backend.eq_ignore_ascii_case("vulkan")
    {
        return;
    }

    match crate::video::gpu::setup() {
        Ok(setup) => options.wgpu_options.wgpu_setup = setup,
        Err(error) => log::warn!("Video device setup: {error:#}"),
    }
}

fn set_backends(options: &mut NativeOptions, backends: Backends) {
    if backends != Backends::VULKAN
        && matches!(options.wgpu_options.wgpu_setup, WgpuSetup::Existing(_))
    {
        options.wgpu_options.wgpu_setup = WgpuSetup::without_display_handle();
    }

    if let WgpuSetup::CreateNew(setup) = &mut options.wgpu_options.wgpu_setup {
        setup.instance_descriptor.backends = backends;
    }
}

fn native_options(window_size: [f32; 2]) -> NativeOptions {
    let app_id = std::env::var("FLATPAK_ID").unwrap_or_else(|_| "flowa".into());

    NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("flowa")
            .with_app_id(app_id)
            .with_inner_size(window_size)
            .with_min_inner_size([640., 400.])
            .with_drag_and_drop(true)
            .with_icon(Arc::new(app_icon())),
        renderer: eframe::Renderer::Wgpu,
        ..Default::default()
    }
}

fn app_icon() -> egui::IconData {
    let tree = resvg::usvg::Tree::from_data(
        include_bytes!("../../assets/flowa.svg"),
        &resvg::usvg::Options::default(),
    );
    let (Ok(tree), Some(mut pixmap)) = (tree, resvg::tiny_skia::Pixmap::new(128, 128))
    else {
        return egui::IconData::default();
    };

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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failed_renderer_selection_is_reset_and_persisted() -> anyhow::Result<()> {
        let folder = tempfile::tempdir()?;
        let path = folder.path().join("settings.json");
        let settings = Settings {
            renderer: RendererPreference::OpenGl,
            memory_mb: 512,
            ..Default::default()
        };
        SettingsStore::save(&settings, &path)?;
        let mut startup = Some(Startup {
            settings,
            config_path: Some(path.clone()),
            launch: Launch {
                path: None,
                preferences: false,
                filmstrip: false,
                info: false,
                zoom: None,
            },
            loader: Loader::new(Settings::default().memory_budget()),
        });

        reset_renderer(&mut startup);

        let saved = SettingsStore::load(&path)?;
        assert_eq!(saved.renderer, RendererPreference::Auto);
        assert_eq!(saved.memory_mb, 512);
        assert_eq!(
            startup.as_ref().map(|startup| startup.settings.renderer),
            Some(RendererPreference::Auto)
        );
        Ok(())
    }
}
