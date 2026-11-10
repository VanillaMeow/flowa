//! Generates FFmpeg bindings and links bundled, system, or portable media dependencies

use std::{
    env,
    error::Error,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

const MEDIA_LIBRARIES: &[(&str, &str)] = &[
    ("libavformat", "63"),
    ("libavcodec", "63"),
    ("libavutil", "61"),
    ("libswresample", "7"),
    ("libswscale", "10"),
    ("libplacebo", "7.360"),
    ("sdl3", "3.2"),
];

/// SDL3 audio functions called by the playback engine
const SDL_FUNCTIONS: &[&str] = &[
    "SDL_InitSubSystem",
    "SDL_QuitSubSystem",
    "SDL_GetError",
    "SDL_GetCurrentAudioDriver",
    "SDL_OpenAudioDeviceStream",
    "SDL_DestroyAudioStream",
    "SDL_GetAudioStreamDevice",
    "SDL_GetAudioDeviceFormat",
    "SDL_SetAudioStreamGain",
    "SDL_PauseAudioStreamDevice",
    "SDL_ResumeAudioStreamDevice",
    "SDL_ClearAudioStream",
    "SDL_FlushAudioStream",
    "SDL_GetAudioStreamQueued",
    "SDL_PutAudioStreamData",
];

/// Libplacebo functions called by the Rust renderer; the shim covers the rest
const PLACEBO_FUNCTIONS: &[&str] = &[
    "pl_log_destroy",
    "pl_vulkan_import",
    "pl_vulkan_create",
    "pl_vulkan_destroy",
    "pl_vulkan_wrap",
    "pl_vulkan_release_ex",
    "pl_vulkan_hold_ex",
    "pl_renderer_create",
    "pl_renderer_destroy",
    "pl_tex_destroy",
    "pl_gpu_finish",
    "pl_gpu_flush",
];

fn main() -> Result<(), Box<dyn Error>> {
    println!("cargo:rerun-if-changed=build.rs");

    // Sources check one cfg instead of repeating the feature and platform condition
    println!("cargo::rustc-check-cfg=cfg(video_playback)");

    let target_os = env::var("CARGO_CFG_TARGET_OS")?;

    if env::var_os("CARGO_FEATURE_VIDEO").is_none()
        || !matches!(target_os.as_str(), "linux" | "windows")
    {
        return Ok(());
    }

    println!("cargo::rustc-cfg=video_playback");

    let static_media = env::var_os("CARGO_FEATURE_STATIC_VIDEO").is_some();

    if static_media && target_os != "linux" {
        return Err(
            "The static-video packaging profile currently supports Linux only".into(),
        );
    }

    // Portable builds bring their own static FFmpeg, and other platforms use their SDK
    let ffmpeg_prefix = if env::var_os("CARGO_FEATURE_BUNDLED_FFMPEG").is_some()
        && target_os == "linux"
        && !static_media
    {
        Some(build_bundled_ffmpeg()?)
    } else {
        None
    };

    // The bundled FFmpeg takes precedence; its dependencies come from the system
    if let Some(prefix) = &ffmpeg_prefix {
        let mut search_paths = vec![prefix.join("lib/pkgconfig")];
        let existing = env::var_os("PKG_CONFIG_PATH");
        search_paths.extend(existing.iter().flat_map(env::split_paths));

        // SAFETY: The build script has not started any other threads
        unsafe { env::set_var("PKG_CONFIG_PATH", env::join_paths(search_paths)?) };
    }

    let mut compiler = cc::Build::new();
    compiler.std("c11").pic(true);

    if target_os == "windows" {
        compiler.define("_CRT_SECURE_NO_WARNINGS", None);
    } else {
        compiler.define("_POSIX_C_SOURCE", "200809L");
    }

    compiler
        .flag_if_supported("-Werror=implicit-function-declaration")
        .flag_if_supported("-Wno-unused-parameter");

    let mut libraries = Vec::new();
    let mut bundled = Vec::new();
    for &(name, version) in MEDIA_LIBRARIES {
        let from_bundle = ffmpeg_prefix.is_some() && is_ffmpeg(name);
        let library = pkg_config::Config::new()
            .atleast_version(version)
            .statik(static_media || from_bundle)
            .cargo_metadata(!static_media && !from_bundle)
            .probe(name)
            .map_err(|error| {
                format!(
                    "Video support requires {name} >= {version}. Configure the media SDK \
                 listed in BUILDING.md.\n{error}"
                )
            })?;

        compiler.includes(&library.include_paths);
        for (name, value) in &library.defines {
            compiler.define(name, value.as_deref());
        }

        if from_bundle {
            bundled.push(library.clone());
        }
        libraries.push(library);
    }

    let out_dir = PathBuf::from(env::var("OUT_DIR")?);
    generate_ffmpeg_bindings(&libraries, &out_dir)?;
    generate_native_bindings(&libraries, &out_dir)?;

    println!("cargo:rerun-if-changed=native/video/placebo.c");
    compiler.file("native/video/placebo.c");

    for header in ["bindings.h", "ffmpeg.h", "placebo.h"] {
        println!("cargo:rerun-if-changed=native/video/{header}");
    }

    compiler.try_compile("flowa-video")?;

    if static_media {
        link_static_media(&libraries);
    }

    if let Some(prefix) = &ffmpeg_prefix {
        link_bundled_ffmpeg(&bundled, &prefix.join("lib"));
    }

    Ok(())
}

fn is_ffmpeg(library: &str) -> bool {
    library.starts_with("libav") || library.starts_with("libsw")
}

/// Builds the patched static FFmpeg on first use and returns its install prefix
fn build_bundled_ffmpeg() -> Result<PathBuf, Box<dyn Error>> {
    let script = "tools/bundled-ffmpeg.sh";

    for path in [
        script,
        "tools/portable/dependencies.sh",
        "tools/portable/ffmpeg-vulkan-frames.patch",
    ] {
        println!("cargo:rerun-if-changed={path}");
    }

    let output = Command::new("bash")
        .arg(script)
        .current_dir(env::var("CARGO_MANIFEST_DIR")?)
        .stderr(Stdio::inherit())
        .output()?;

    if !output.status.success() {
        return Err("Building the bundled FFmpeg failed; its log is above".into());
    }

    Ok(PathBuf::from(String::from_utf8(output.stdout)?.trim()))
}

/// Links the bundled FFmpeg archives and, dynamically, the system libraries they use
fn link_bundled_ffmpeg(libraries: &[pkg_config::Library], archive_dir: &Path) {
    // A group resolves references between the archives regardless of their order
    println!("cargo:rustc-link-arg=-Wl,--start-group");

    for library in libraries {
        for name in &library.libs {
            let archive = archive_dir.join(format!("lib{name}.a"));

            if archive.is_file() {
                println!("cargo:rerun-if-changed={}", archive.display());
                println!("cargo:rustc-link-arg={}", archive.display());
            } else {
                println!("cargo:rustc-link-arg=-l{name}");
            }
        }

        for arguments in &library.ld_args {
            println!("cargo:rustc-link-arg=-Wl,{}", arguments.join(","));
        }
    }

    println!("cargo:rustc-link-arg=-Wl,--end-group");
}

/// Common bindgen settings with the include paths and defines of every media library
fn bindings_builder(
    header: &str,
    libraries: &[pkg_config::Library],
) -> bindgen::Builder {
    let mut builder = bindgen::Builder::default()
        .header(header)
        .parse_callbacks(Box::new(bindgen::CargoCallbacks::new()))
        .prepend_enum_name(false)
        .generate_comments(false)
        .layout_tests(false);

    for library in libraries {
        for include in &library.include_paths {
            builder = builder.clang_arg(format!("-I{}", include.display()));
        }

        for (name, value) in &library.defines {
            let definition = value.as_ref().map_or_else(
                || format!("-D{name}"),
                |value| format!("-D{name}={value}"),
            );
            builder = builder.clang_arg(definition);
        }
    }

    builder
}

fn generate_ffmpeg_bindings(
    libraries: &[pkg_config::Library],
    out_dir: &Path,
) -> Result<(), Box<dyn Error>> {
    bindings_builder("native/video/ffmpeg.h", libraries)
        .allowlist_function("av.*|swr_.*|sws_.*")
        .allowlist_type("AV.*|SwrContext|SwsContext|SwsFlags")
        .allowlist_var("FLOWA_AVERROR_.*|AV_TIME_BASE|AVSEEK_FLAG_BACKWARD|AV_CODEC_HW_CONFIG_METHOD_HW_DEVICE_CTX|FF_THREAD_.*|SWS_.*")
        .allowlist_var("AV_PIX_FMT_FLAG_HWACCEL|AV_LOG_ERROR|AV_LOG_WARNING")
        .blocklist_type("Vk.*|PFN_vk.*")
        .derive_default(true)
        .generate()?
        .write_to_file(out_dir.join("ffmpeg.rs"))?;

    Ok(())
}

/// Libplacebo, shim, and SDL3 declarations
///
/// Rust supplies the Vulkan and FFmpeg types they reference
fn generate_native_bindings(
    libraries: &[pkg_config::Library],
    out_dir: &Path,
) -> Result<(), Box<dyn Error>> {
    bindings_builder("native/video/bindings.h", libraries)
        .allowlist_function("flowa_.*")
        .allowlist_function(PLACEBO_FUNCTIONS.join("|"))
        .allowlist_function(SDL_FUNCTIONS.join("|"))
        .allowlist_type("SDL_AudioFormat")
        .allowlist_var("SDL_INIT_AUDIO")
        .blocklist_type("Vk.*|PFN_vk.*|AV.*")
        .derive_default(true)
        .generate()?
        .write_to_file(out_dir.join("native.rs"))?;

    Ok(())
}

/// Keeps media archives static while retaining host GPU and platform libraries
fn link_static_media(libraries: &[pkg_config::Library]) {
    // A group resolves dependencies between the shader compiler's static archives
    println!("cargo:rustc-link-arg=-Wl,--start-group");

    let search_paths: Vec<_> = libraries
        .iter()
        .flat_map(|library| library.link_paths.iter().cloned())
        .collect();

    for library in libraries {
        for name in &library.libs {
            if is_platform_library(name) {
                println!("cargo:rustc-link-arg=-l{name}");
                continue;
            }

            let archive = find_archive(name, &search_paths);
            println!("cargo:rerun-if-changed={}", archive.display());
            println!("cargo:rustc-link-arg={}", archive.display());
        }

        for archive in &library.link_files {
            assert!(
                archive
                    .extension()
                    .is_some_and(|extension| extension == "a"),
                "Expected a static media archive: {}",
                archive.display()
            );

            println!("cargo:rerun-if-changed={}", archive.display());
            println!("cargo:rustc-link-arg={}", archive.display());
        }

        for arguments in &library.ld_args {
            println!("cargo:rustc-link-arg=-Wl,{}", arguments.join(","));
        }
    }

    // Libplacebo's static pkg-config metadata omits its C++ runtime
    let runtime = find_archive("stdc++", &search_paths);
    println!("cargo:rustc-link-arg={}", runtime.display());
    println!("cargo:rustc-link-arg=-Wl,--end-group");
}

fn is_platform_library(name: &str) -> bool {
    matches!(
        name,
        "m" | "dl" | "pthread" | "rt" | "c" | "gcc_s" | "util" | "resolv" | "vulkan"
    )
}

fn find_archive(name: &str, search_paths: &[PathBuf]) -> PathBuf {
    let filename = format!("lib{name}.a");

    for directory in search_paths {
        let archive = directory.join(&filename);

        if archive.is_file() {
            return archive;
        }
    }

    let output = Command::new("c++")
        .arg(format!("-print-file-name={filename}"))
        .output()
        .expect("Cannot query the native compiler's archive search path");
    let archive = PathBuf::from(
        String::from_utf8(output.stdout)
            .expect("Invalid compiler output")
            .trim(),
    );

    assert!(
        archive.is_file(),
        "Missing static media dependency: {filename}"
    );
    archive
}
