#!/usr/bin/env bash
set -euo pipefail
export LC_ALL=C

project_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd)
cd "$project_dir"

if [[ $# -gt 1 || ($# == 1 && $1 != --print-native-cache-key) ]]; then
    printf 'Usage: %s [--print-native-cache-key]\n' "$0" >&2
    exit 2
fi

# Generic executables run on any x86-64 CPU; native ones only on the CPU that built them
case ${FLOWA_PORTABLE_CPU:-generic} in
    generic)
        target_cpu=x86-64
        profile=portable
        ;;
    native)
        target_cpu=native
        profile=portable-native
        ;;
    *)
        printf '%s\n' 'FLOWA_PORTABLE_CPU must be generic or native.' >&2
        exit 2
        ;;
esac

if [[ $(uname -s) != Linux || $(rustc -vV | sed -n 's/^host: //p') != x86_64-unknown-linux-gnu ]]; then
    printf '%s\n' 'Portable builds require native x86_64 GNU/Linux.' >&2
    exit 1
fi

# The source recipes and compiler determine which native cache can be reused
build_key=$({
    sha256sum tools/portable/{build.sh,dependencies.sh,Containerfile,*.patch}
    cc --version
    c++ --version
    ldd --version
} | sha256sum | cut -d ' ' -f 1)

if [[ ${1:-} == --print-native-cache-key ]]; then
    printf '%s\n' "$build_key"
    exit 0
fi

for tool in cc c++ cmake meson ninja nasm pkg-config curl sha256sum tar patch readelf make; do
    if ! command -v "$tool" >/dev/null; then
        printf 'Missing build tool: %s\n' "$tool" >&2
        exit 1
    fi
done

export CARGO_TARGET_DIR=${CARGO_TARGET_DIR:-"$project_dir/target"}
mkdir -p "$CARGO_TARGET_DIR"
CARGO_TARGET_DIR=$(cd "$CARGO_TARGET_DIR" && pwd)
export CARGO_TARGET_DIR

jobs=${CARGO_BUILD_JOBS:-2}
build_dir="$project_dir/.cache/portable-deps/$build_key"
download_dir="$project_dir/.cache/portable-deps/downloads"
prefix="$build_dir/prefix"
mkdir -p "$download_dir" "$prefix/lib/pkgconfig"

export CC=cc CXX=c++
export CFLAGS='-O2 -march=x86-64 -mtune=generic -fPIC'
export CXXFLAGS="$CFLAGS"
export PKG_CONFIG_PATH="$prefix/lib/pkgconfig"
export PKG_CONFIG_LIBDIR="$PKG_CONFIG_PATH"
source "$project_dir/tools/portable/dependencies.sh"

if [[ ! -f $build_dir/image-complete ]]; then
    build_images
fi

# Media dependencies also need the host's Vulkan loader and GPU SDK headers
unset PKG_CONFIG_LIBDIR
export PATH="$prefix/bin:$PATH"
export CPPFLAGS="-I$prefix/include"
export LDFLAGS="-L$prefix/lib"
export LIBRARY_PATH="$prefix/lib${LIBRARY_PATH:+:$LIBRARY_PATH}"
build_media

export SYSTEM_DEPS_DAV1D_LINK=static
export SYSTEM_DEPS_DAV1D_BUILD_INTERNAL=never
export SYSTEM_DEPS_LIBHEIF_LINK=static

# The CPU target replaces local Rust flags; media libraries above stay generic and
# choose SIMD at runtime, so both flavors share one dependency cache
export CARGO_ENCODED_RUSTFLAGS="-C"$'\x1f'"target-cpu=$target_cpu"
cargo build --locked --profile "$profile" --target x86_64-unknown-linux-gnu \
    --no-default-features --features avif,heif,static-video

binary="$CARGO_TARGET_DIR/x86_64-unknown-linux-gnu/$profile/flowa"
"$project_dir/tools/checks/portable.sh" "$binary" "${FLOWA_MAX_GLIBC:-2.39}"
printf 'Portable executable: %s\n' "$binary"
