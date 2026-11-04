#!/usr/bin/env bash
set -euo pipefail
export LC_ALL=C

project_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd)
cd "$project_dir"

if [[ $(uname -s) != Linux || $(rustc -vV | sed -n 's/^host: //p') != x86_64-unknown-linux-gnu ]]; then
    printf '%s\n' 'The portable build currently supports native x86_64 GNU/Linux builds only.' >&2
    exit 1
fi

for tool in cc c++ cmake meson ninja nasm pkg-config curl sha256sum tar readelf patch; do
    if ! command -v "$tool" >/dev/null; then
        printf 'Missing build tool: %s\n' "$tool" >&2
        exit 1
    fi
done

export CARGO_TARGET_DIR=${CARGO_TARGET_DIR:-"$project_dir/target"}
mkdir -p "$CARGO_TARGET_DIR"
CARGO_TARGET_DIR=$(cd "$CARGO_TARGET_DIR" && pwd)
export CARGO_TARGET_DIR

jobs=${CARGO_BUILD_JOBS:-$(nproc)}
native_dir="$project_dir/.cache/portable-deps"
download_dir="$native_dir/downloads"
# Include the build recipe and compiler versions so Cargo sees a new library path after changes
build_key=$({
    sha256sum "$project_dir/packaging/linux/"{build-portable.sh,build-static-video.sh,portable.Containerfile} \
        "$project_dir/native/mpv/"{*.c,*.h,*.patch}
    cc --version
    c++ --version
    ldd --version
} | sha256sum | cut -d ' ' -f 1)

if [[ ${1:-} == --print-native-cache-key ]]; then
    printf '%s\n' "$build_key"
    exit 0
fi

build_dir="$native_dir/$build_key"
prefix="$build_dir/prefix"
mkdir -p "$download_dir" "$prefix/lib"

# Keep codec discovery separate from libraries installed on the build host
export PKG_CONFIG_PATH="$prefix/lib/pkgconfig"
export PKG_CONFIG_LIBDIR="$PKG_CONFIG_PATH"
export CC=cc
export CXX=c++
export CFLAGS='-O2 -march=x86-64 -mtune=generic'
export CXXFLAGS="$CFLAGS"

fetch_source() {
    local name=$1 url=$2 checksum=$3
    local archive="$download_dir/${url##*/}"
    local source_dir="$build_dir/sources/$name"

    if [[ ! -f $archive ]]; then
        curl --fail --location --retry 3 "$url" --output "$archive.part"
        mv "$archive.part" "$archive"
    fi

    printf '%s  %s\n' "$checksum" "$archive" | sha256sum --check

    if [[ ! -d $source_dir ]]; then
        mkdir -p "$source_dir.part"
        tar -xf "$archive" -C "$source_dir.part" --strip-components=1 --no-same-owner
        mv "$source_dir.part" "$source_dir"
    fi
}

build_cmake() {
    local name=$1
    shift

    cmake -S "$build_dir/sources/$name" -B "$build_dir/build/$name" -G Ninja \
        -DCMAKE_BUILD_TYPE=Release -DCMAKE_INSTALL_PREFIX="$prefix" \
        -DCMAKE_INSTALL_LIBDIR=lib -DCMAKE_PREFIX_PATH="$prefix" \
        -DCMAKE_POSITION_INDEPENDENT_CODE=ON -DBUILD_SHARED_LIBS=OFF \
        -DCMAKE_C_FLAGS_RELEASE="$CFLAGS -DNDEBUG" \
        -DCMAKE_CXX_FLAGS_RELEASE="$CXXFLAGS -DNDEBUG" "$@"
    cmake --build "$build_dir/build/$name" --parallel "$jobs"
    cmake --install "$build_dir/build/$name"
}

build_native() {
    fetch_source dav1d \
        https://download.videolan.org/pub/videolan/dav1d/1.5.4/dav1d-1.5.4.tar.xz \
        686616b7c69eb88d44459391ab25cac13b6647a3b288835c5784e71c1514a5c5
    fetch_source libde265 \
        https://github.com/strukturag/libde265/releases/download/v1.1.3/libde265-1.1.3.tar.gz \
        554228bd17788c99a7e63b37ab5634722190e6e2bf60c1dcb01cef328e133905
    fetch_source libheif \
        https://github.com/strukturag/libheif/releases/download/v1.23.7/libheif-1.23.7.tar.gz \
        f7483e818503ea6224c8863e4a56ac02c7089021a98472c811c12aa19faf2475

    meson setup "$build_dir/build/dav1d" "$build_dir/sources/dav1d" \
        --prefix "$prefix" --libdir lib --buildtype release --default-library static \
        -Denable_tools=false -Denable_tests=false
    meson compile -C "$build_dir/build/dav1d" -j "$jobs"
    meson install -C "$build_dir/build/dav1d"

    build_cmake libde265 -DENABLE_DECODER=OFF -DENABLE_ENCODER=OFF -DENABLE_SDL=OFF

    local disabled_codecs=(
        X265 X264 OpenH264_DECODER KVAZAAR UVG266 VVDEC VVENC AOM_DECODER AOM_ENCODER
        SvtEnc RAV1E JPEG_DECODER JPEG_ENCODER OpenJPEG_DECODER OpenJPEG_ENCODER
        FFMPEG_DECODER OPENJPH_ENCODER
    )
    local codec_options=() codec
    for codec in "${disabled_codecs[@]}"; do
        codec_options+=("-DWITH_$codec=OFF")
    done

    build_cmake libheif "${codec_options[@]}" \
        -DENABLE_PLUGIN_LOADING=OFF -DWITH_LIBDE265=ON -DWITH_LIBDE265_PLUGIN=OFF \
        -DWITH_DAV1D=ON -DWITH_DAV1D_PLUGIN=OFF -DWITH_LIBSHARPYUV=OFF \
        -DWITH_UNCOMPRESSED_CODEC=OFF -DWITH_HEADER_COMPRESSION=OFF \
        -DWITH_EXAMPLES=OFF -DWITH_GDK_PIXBUF=OFF -DBUILD_TESTING=OFF \
        -DBUILD_DOCUMENTATION=OFF

    local required_codec
    for required_codec in dav1d libde265; do
        if ! pkg-config --print-requires-private libheif | cut -d ' ' -f 1 | grep -qx "$required_codec"; then
            printf 'Libheif was built without its %s decoder.\n' "$required_codec" >&2
            exit 1
        fi
    done

    # System-deps selects static C++ linkage when the archive is in a pkg-config search directory
    local cxx_archive
    cxx_archive=$(c++ -print-file-name=libstdc++.a)
    if [[ ! -f $cxx_archive ]]; then
        printf '%s\n' 'The C++ compiler must provide libstdc++.a.' >&2
        exit 1
    fi

    cp "$cxx_archive" "$prefix/lib/libstdc++.a"
    touch "$build_dir/complete"
}

if [[ ! -f $build_dir/complete ]]; then
    printf 'Building native libraries: %s\n' "$build_key"
    build_native
else
    printf 'Reusing native libraries: %s\n' "$build_key"
fi

export SYSTEM_DEPS_DAV1D_LINK=static
export SYSTEM_DEPS_DAV1D_BUILD_INTERNAL=never
export SYSTEM_DEPS_LIBHEIF_LINK=static

# Build the media stack before Cargo so libmpv can be linked into the executable
export FLOWA_STATIC_BUILD_ROOT=${FLOWA_STATIC_BUILD_ROOT:-"$build_dir/video"}
"$project_dir/packaging/linux/build-static-video.sh" "$prefix"
export FLOWA_STATIC_MPV_PREFIX="$FLOWA_STATIC_BUILD_ROOT/prefix"
export PKG_CONFIG_PATH="$FLOWA_STATIC_MPV_PREFIX/lib/pkgconfig:$prefix/lib/pkgconfig"
unset PKG_CONFIG_LIBDIR

# Pin the target and CPU baseline even when the developer has local Cargo settings
export CARGO_ENCODED_RUSTFLAGS=$'-C\x1ftarget-cpu=x86-64'
cargo build --locked --profile portable --target x86_64-unknown-linux-gnu \
    --no-default-features --features avif,heif,static-video

binary="$CARGO_TARGET_DIR/x86_64-unknown-linux-gnu/portable/flowa"
"$project_dir/packaging/linux/check-portable.sh" "$binary" "${FLOWA_MAX_GLIBC:-}"
"$binary" --version
printf 'Portable executable: %s\n' "$binary"
