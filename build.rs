//! Links the portable build's media stack directly into the executable

use std::{env, path::PathBuf, process::Command};

fn main() {
    println!("cargo:rerun-if-env-changed=FLOWA_STATIC_MPV_PREFIX");

    if env::var_os("CARGO_FEATURE_STATIC_VIDEO").is_none()
        || env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("linux")
    {
        return;
    }

    let prefix = env::var_os("FLOWA_STATIC_MPV_PREFIX")
        .map(PathBuf::from)
        .expect("Set FLOWA_STATIC_MPV_PREFIX to the build-static-video.sh prefix");
    let current_path = env::var_os("PKG_CONFIG_PATH").unwrap_or_default();
    let pkg_path = env::join_paths(
        [prefix.join("lib/pkgconfig")]
            .into_iter()
            .chain(env::split_paths(&current_path)),
    )
    .expect("Invalid pkg-config search path");

    // This process only runs during compilation before any threads are created
    unsafe { env::set_var("PKG_CONFIG_PATH", pkg_path) };

    let libraries = pkg_config::Config::new()
        .statik(true)
        .cargo_metadata(false)
        .probe("mpv")
        .expect("Cannot find the static mpv dependency graph");

    // A group resolves cycles between FreeType, HarfBuzz and the shader compiler
    println!("cargo:rustc-link-arg=-Wl,--start-group");

    for library in &libraries.libs {
        if is_platform_library(library) {
            println!("cargo:rustc-link-arg=-l{library}");
            continue;
        }

        let archive = find_archive(library, &libraries.link_paths);
        println!("cargo:rerun-if-changed={}", archive.display());
        println!("cargo:rustc-link-arg={}", archive.display());
    }

    for archive in &libraries.link_files {
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

    // Libplacebo's static pkg-config metadata omits its C++ runtime
    let runtime = find_archive("stdc++", &libraries.link_paths);
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
