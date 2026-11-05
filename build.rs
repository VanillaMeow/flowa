//! Compiles the C shim and links system or portable media dependencies

use std::{env, error::Error, path::PathBuf, process::Command};

const MEDIA_LIBRARIES: &[(&str, &str)] = &[
    ("libavformat", "63"),
    ("libavcodec", "63"),
    ("libavutil", "61"),
    ("libswresample", "7"),
    ("libplacebo", "7.360"),
    ("sdl3", "3.2"),
];

fn main() -> Result<(), Box<dyn Error>> {
    println!("cargo:rerun-if-changed=build.rs");

    if env::var_os("CARGO_FEATURE_VIDEO").is_none()
        || env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("linux")
    {
        return Ok(());
    }

    let static_media = env::var_os("CARGO_FEATURE_STATIC_VIDEO").is_some();
    let mut compiler = cc::Build::new();
    compiler
        .std("c11")
        .define("_POSIX_C_SOURCE", "200809L")
        .pic(true)
        .flag("-Werror=implicit-function-declaration")
        .flag("-Wno-unused-parameter");

    let mut libraries = Vec::new();
    for &(name, version) in MEDIA_LIBRARIES {
        let library = pkg_config::Config::new()
            .atleast_version(version)
            .statik(static_media)
            .cargo_metadata(!static_media)
            .probe(name)
            .map_err(|error| {
                format!(
                    "Video support requires {name} >= {version}. Install the media SDK \
                 listed in BUILDING.md, or use just portable.\n{error}"
                )
            })?;

        compiler.includes(&library.include_paths);
        for (name, value) in &library.defines {
            compiler.define(name, value.as_deref());
        }
        libraries.push(library);
    }

    for source in ["gpu.c", "decode.c", "media.c"] {
        let path = format!("native/video/{source}");
        println!("cargo:rerun-if-changed={path}");
        compiler.file(path);
    }

    for header in ["flowa_video.h", "internal.h"] {
        println!("cargo:rerun-if-changed=native/video/{header}");
    }

    compiler.try_compile("flowa-video")?;

    if static_media {
        link_static_media(&libraries);
    }

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
