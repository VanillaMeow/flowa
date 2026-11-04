#!/usr/bin/env bash
set -euo pipefail

project_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd)
build_root=${FLOWA_STATIC_BUILD_ROOT:-"$project_dir/.cache/portable-deps/static-media"}
prefix="$build_root/prefix"
downloads="$project_dir/.cache/portable-deps/downloads"
jobs=${CARGO_BUILD_JOBS:-2}
image_prefix=${1:?Usage: build-static-video.sh IMAGE_CODEC_PREFIX}

mkdir -p "$prefix/lib/pkgconfig" "$downloads" "$build_root/sources"
export PKG_CONFIG_PATH="$prefix/lib/pkgconfig:$image_prefix/lib/pkgconfig"
unset PKG_CONFIG_LIBDIR
export PATH="$prefix/bin:$PATH"
export CFLAGS='-O2 -march=x86-64 -mtune=generic -fPIC'
export CXXFLAGS="$CFLAGS"
export CPPFLAGS="-I$prefix/include"
export LDFLAGS="-L$prefix/lib"
export LIBRARY_PATH="$prefix/lib${LIBRARY_PATH:+:$LIBRARY_PATH}"

fetch() {
    local name=$1 archive=$2 checksum=$3 url=$4
    local source_dir="$build_root/sources/$name"

    if [[ ! -f $downloads/$archive ]]; then
        curl --fail --location --retry 3 "$url" -o "$downloads/$archive.part"
        mv "$downloads/$archive.part" "$downloads/$archive"
    fi

    printf '%s  %s\n' "$checksum" "$downloads/$archive" | sha256sum --check

    if [[ ! -d $source_dir ]]; then
        mkdir -p "$source_dir.part"
        tar -xf "$downloads/$archive" -C "$source_dir.part" --strip-components=1 --no-same-owner
        mv "$source_dir.part" "$source_dir"
    fi
}

cmake_library() {
    local name=$1
    shift

    cmake -S "$build_root/sources/$name" -B "$build_root/build/$name" -G Ninja \
        -DCMAKE_BUILD_TYPE=Release -DCMAKE_INSTALL_PREFIX="$prefix" \
        -DCMAKE_INSTALL_LIBDIR=lib -DCMAKE_PREFIX_PATH="$prefix" \
        -DBUILD_SHARED_LIBS=OFF -DCMAKE_POSITION_INDEPENDENT_CODE=ON "$@"
    cmake --build "$build_root/build/$name" --parallel "$jobs"
    cmake --install "$build_root/build/$name"
}

meson_library() {
    local name=$1
    shift

    meson setup --reconfigure "$build_root/build/$name" "$build_root/sources/$name" \
        --prefix "$prefix" --libdir lib --buildtype release --default-library static \
        --prefer-static --auto-features=disabled --wrap-mode=nodownload "$@"
    meson compile -C "$build_root/build/$name" -j "$jobs"
    meson install -C "$build_root/build/$name"
}

copy_if_changed() {
    if cmp -s "$1" "$2"; then
        return;
    fi

    cp "$1" "$2"
}

fetch vulkan-headers vulkan-headers-1.4.366.tar.gz \
    577b90c208e6fd39ec9a1460e48772f63c76f6f5afb441df2a977a0fdb3f618c \
    https://codeload.github.com/KhronosGroup/Vulkan-Headers/tar.gz/refs/tags/v1.4.366
fetch glslang glslang-16.6.0.tar.gz \
    9c09b901149c729df745057dafa815278aaa101b84d2b6e14f16a42de52f97f2 \
    https://codeload.github.com/KhronosGroup/glslang/tar.gz/refs/tags/16.6.0
fetch sdl3 SDL3-3.4.18.tar.gz \
    9c75cf16330322c217dedd2e0609f1124f1b54b8633e763467b4684d0f4334a3 \
    https://www.libsdl.org/release/SDL3-3.4.18.tar.gz
fetch libdrm libdrm-2.4.128.tar.xz \
    3bb35db8700c2a0b569f2c6729a53f5495786856b310854c8de57782a22bddac \
    https://dri.freedesktop.org/libdrm/libdrm-2.4.128.tar.xz
fetch libva libva-2.24.1.tar.gz \
    0b4a3649ee8d683b9cce2ef094df4fb039d276c0cef7e49337c43d3b297b9f42 \
    https://codeload.github.com/intel/libva/tar.gz/refs/tags/2.24.1
fetch libdisplay-info libdisplay-info-0.3.0.tar.xz \
    6ae77cd937f9cf7d1321d35c116062c4911e8447010a6a713ac4286f7a9d5987 \
    https://gitlab.freedesktop.org/emersion/libdisplay-info/-/releases/0.3.0/downloads/libdisplay-info-0.3.0.tar.xz
fetch harfbuzz harfbuzz-14.5.0.tar.xz \
    b7132e148358a45185c9feafd049dbaf243649d3c44414b3534d9c95d18592b9 \
    https://github.com/harfbuzz/harfbuzz/releases/download/14.5.0/harfbuzz-14.5.0.tar.xz
fetch libunibreak libunibreak-6.1.tar.gz \
    cc4de0099cf7ff05005ceabff4afed4c582a736abc38033e70fdac86335ce93f \
    https://github.com/adah1972/libunibreak/releases/download/libunibreak_6_1/libunibreak-6.1.tar.gz
fetch libplacebo libplacebo-7.360.1.tar.gz \
    d05fdf90bea2f629eaa2d115e909fd356388ac639e54f77b87a018a6d76224bd \
    https://codeload.github.com/haasn/libplacebo/tar.gz/refs/tags/v7.360.1
fetch ffmpeg ffmpeg-9.0.tar.xz \
    7f607a00dd0d28a729d5a4811205812eef01cf6ef6155025febb6f36a9062d52 \
    https://ffmpeg.org/releases/ffmpeg-9.0.tar.xz
fetch mpv mpv-0.41.0.tar.gz \
    ee21092a5ee427353392360929dc64645c54479aefdb5babc5cfbb5fad626209 \
    https://codeload.github.com/mpv-player/mpv/tar.gz/refs/tags/v0.41.0

if [[ ! -f $build_root/dependencies-complete ]]; then
    cmake_library vulkan-headers -DVULKAN_HEADERS_ENABLE_TESTS=OFF
    cmake_library glslang -DGLSLANG_TESTS=OFF -DENABLE_OPT=OFF -DENABLE_HLSL=OFF \
        -DBUILD_EXTERNAL=OFF -DENABLE_PCH=OFF

    # Glslang installs CMake targets but no pkg-config metadata
    cat > "$prefix/lib/pkgconfig/glslang.pc" <<EOF
prefix=$prefix
libdir=\${prefix}/lib
includedir=\${prefix}/include
Name: glslang
Description: Khronos GLSL to SPIR-V compiler
Version: 16.6.0
Libs: -L\${libdir} -lglslang -lSPIRV -lglslang-default-resource-limits -lstdc++ -lm -lpthread
Cflags: -I\${includedir}
EOF

    cmake_library sdl3 -DSDL_SHARED=OFF -DSDL_STATIC=ON -DSDL_DEPS_SHARED=ON \
        -DSDL_TEST_LIBRARY=OFF -DSDL_TESTS=OFF -DSDL_EXAMPLES=OFF -DSDL_UNIX_CONSOLE_BUILD=ON \
        -DSDL_VIDEO=OFF -DSDL_GPU=OFF -DSDL_RENDER=OFF -DSDL_CAMERA=OFF \
        -DSDL_JOYSTICK=OFF -DSDL_HAPTIC=OFF -DSDL_SENSOR=OFF \
        -DSDL_ALSA=ON -DSDL_ALSA_SHARED=ON -DSDL_PULSEAUDIO=ON \
        -DSDL_PULSEAUDIO_SHARED=ON -DSDL_PIPEWIRE=ON -DSDL_PIPEWIRE_SHARED=ON \
        -DSDL_JACK=OFF -DSDL_SNDIO=OFF -DSDL_LIBUDEV=OFF
    meson_library libdrm -Dtests=false

    # Upstream libva hardcodes shared_library instead of honoring default-library
    sed -i 's/shared_library(/library(/g' "$build_root/sources/libva/va/meson.build"
    meson_library libva -Dwith_x11=no -Dwith_glx=no -Dwith_wayland=no \
        -Ddriverdir=/usr/lib/x86_64-linux-gnu/dri:/usr/lib64/dri:/usr/lib/dri
    meson_library libdisplay-info
    meson_library harfbuzz -Dfreetype=enabled -Draster=disabled -Dvector=disabled \
        -Dgpu=disabled -Dsubset=disabled -Dutilities=disabled -Dtests=disabled

    mkdir -p "$build_root/build/libunibreak"
    (cd "$build_root/build/libunibreak"
        "$build_root/sources/libunibreak/configure" --prefix="$prefix" \
            --disable-shared --enable-static
        make -j "$jobs"
        make install)

    meson_library libplacebo -Dvulkan=enabled -Dvk-proc-addr=disabled -Dvulkan-sdk="$prefix" \
        -Dvulkan-registry="$prefix/share/vulkan/registry/vk.xml" \
        -Dglslang=enabled -Ddemos=false -Dtests=false
    touch "$build_root/dependencies-complete"
fi

if [[ ! -f $build_root/ffmpeg-complete ]]; then
    mkdir -p "$build_root/build/ffmpeg"
    (cd "$build_root/build/ffmpeg"
        "$build_root/sources/ffmpeg/configure" --prefix="$prefix" \
            --disable-shared --enable-static --enable-pic --disable-autodetect \
            --disable-programs --disable-doc --disable-debug --disable-avdevice \
            --disable-encoders --disable-muxers --disable-network \
            --enable-protocol=file,pipe --enable-libdav1d --enable-libdrm \
            --enable-vaapi --enable-vulkan --enable-ffnvcodec --enable-cuda \
            --enable-cuvid --enable-nvdec --pkg-config-flags=--static \
            --extra-cflags="$CFLAGS $CPPFLAGS" --extra-ldflags="$LDFLAGS"
        make -j "$jobs"
        make install)
    touch "$build_root/ffmpeg-complete"
fi

for patch_file in mpv.patch cuda-zero-init.patch sdl3-audio.patch; do
    if patch -d "$build_root/sources/mpv" -p1 --forward --dry-run \
        < "$project_dir/native/mpv/$patch_file" > /dev/null 2>&1; then
        patch -d "$build_root/sources/mpv" -p1 < "$project_dir/native/mpv/$patch_file"
    elif ! patch -d "$build_root/sources/mpv" -p1 --reverse --dry-run \
        < "$project_dir/native/mpv/$patch_file" > /dev/null 2>&1; then
        printf 'Cannot apply %s\n' "$patch_file" >&2
        exit 1
    fi
done

for source in flowa_vk.h libmpv_vk.c; do
    copy_if_changed "$project_dir/native/mpv/$source" "$build_root/sources/mpv/video/out/vulkan/$source"
done

copy_if_changed "$project_dir/native/mpv/ao_sdl3.c" "$build_root/sources/mpv/audio/out/ao_sdl3.c"
meson_library mpv -Dcplayer=false -Dlibmpv=true -Dtests=false -Dgpl=true \
    -Dvulkan=enabled -Dgl=disabled -Dsdl3-audio=enabled \
    -Dcuda-hwaccel=enabled -Dcuda-interop=enabled -Dvaapi=enabled \
    -Dvaapi-drm=enabled -Ddrm=enabled -Dmanpage-build=disabled

printf 'Static video libraries: %s\n' "$prefix"
