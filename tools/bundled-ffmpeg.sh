#!/usr/bin/env bash
# Builds the patched static FFmpeg that local builds link, then prints its prefix
#
# Build.rs runs this for the bundled-ffmpeg feature. Each recipe is built once into
# .cache/ffmpeg, and a lock stops concurrent Cargo runs from building it twice
set -euo pipefail
export LC_ALL=C

project_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
cd "$project_dir"

# Cargo reads the prefix from standard output, so build logs go to standard error
exec 3>&1 1>&2

for tool in cc make nasm pkg-config curl sha256sum tar patch flock; do
    if ! command -v "$tool" >/dev/null; then
        printf 'Building the bundled FFmpeg needs %s. To link system FFmpeg instead, build with\n' "$tool"
        printf '%s\n' '--no-default-features --features avif,heif,video'
        exit 1
    fi
done

build_key=$({
    sha256sum tools/bundled-ffmpeg.sh tools/portable/{dependencies.sh,*.patch}
    cc --version
} | sha256sum | cut -d ' ' -f 1)

cache_dir="$project_dir/.cache/ffmpeg"
build_dir="$cache_dir/$build_key"
download_dir="$cache_dir/downloads"
prefix="$build_dir/prefix"
jobs=${NUM_JOBS:-$(nproc)}
mkdir -p "$download_dir" "$prefix/lib/pkgconfig"

exec 4>"$cache_dir/lock"
flock 4

if [[ ! -f $build_dir/ffmpeg-complete ]]; then
    source tools/portable/dependencies.sh

    # Cargo's jobserver flags are not meant for this build
    unset MAKEFLAGS MFLAGS CPPFLAGS LDFLAGS

    # FFmpeg picks SIMD code at runtime, so plain flags keep generic builds portable
    export CC=cc
    export CFLAGS='-O2 -fPIC'

    # The pinned NVIDIA headers come first; FFmpeg's other dependencies stay system libraries
    export PKG_CONFIG_PATH="$prefix/lib/pkgconfig${PKG_CONFIG_PATH:+:$PKG_CONFIG_PATH}"

    # Match the headers the portable container uses, which NVIDIA 580 supports
    fetch_source nv-codec-headers \
        https://codeload.github.com/FFmpeg/nv-codec-headers/tar.gz/refs/tags/n12.1.14.0 \
        2fefaa227d2a3b4170797796425a59d1dd2ed5fd231db9b4244468ba327acd0b
    make -C "$build_dir/sources/nv-codec-headers" PREFIX="$prefix" install

    fetch_ffmpeg
    build_ffmpeg

    # Earlier recipes are no longer used by any build
    find "$cache_dir" -mindepth 1 -maxdepth 1 -type d \
        ! -name downloads ! -name "$build_key" -exec rm -rf {} +
fi

printf '%s\n' "$prefix" >&3
