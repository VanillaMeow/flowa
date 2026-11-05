//! Native window setup and graphics backend fallback

use std::{path::PathBuf, sync::Arc};

use eframe::{NativeOptions, egui_wgpu::WgpuSetup, wgpu::Backends};

use crate::settings::{RendererPreference, Settings};

use super::{Flowa, Launch};

struct Startup {
    settings: Settings,
    config_path: Option<PathBuf>,
    launch: Launch,
}

pub(crate) fn run(
    settings: Settings,
    config_path: Option<PathBuf>,
    launch: Launch,
) -> eframe::Result {
    let mut options = native_options(settings.window_size);
    let renderer = settings.renderer;

    configure_video(&mut options, renderer);
    let mut startup = Some(Startup {
        settings,
        config_path,
        launch,
    });

    if std::env::var_os("WGPU_BACKEND").is_some() {
        return run_once(options, &mut startup);
    }

    if let Some(backends) = preferred_backends(renderer) {
        set_backends(&mut options, backends);
        return run_once(options, &mut startup);
    }

    if !cfg!(target_os = "linux") {
        return run_once(options, &mut startup);
    }

    set_backends(&mut options, Backends::VULKAN);
    let result = run_once(options.clone(), &mut startup);

    // Startup options are consumed only after graphics initialization succeeds
    if startup.is_none() {
        return result;
    }

    let Err(eframe::Error::Wgpu(error)) = result else {
        return result;
    };

    log::warn!("Vulkan initialization failed: {error}. Retrying with OpenGL");
    set_backends(&mut options, Backends::GL);

    run_once(options, &mut startup)
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
                video_device,
            )))
        }),
    )
}

fn configure_video(options: &mut NativeOptions, renderer: RendererPreference) {
    #[cfg(all(feature = "video", target_os = "linux"))]
    {
        if !matches!(
            renderer,
            RendererPreference::Auto | RendererPreference::Vulkan
        ) {
            return;
        }

        if let Some(backend) = std::env::var_os("WGPU_BACKEND")
            && backend != "vulkan"
        {
            return;
        }

        match crate::video::gpu::setup() {
            Ok(setup) => options.wgpu_options.wgpu_setup = setup,
            Err(error) => log::warn!("Video device setup: {error:#}"),
        }
    }

    let _ = (options, renderer);
}

fn set_backends(options: &mut NativeOptions, backends: Backends) {
    if backends == Backends::GL
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
