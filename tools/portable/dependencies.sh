#!/usr/bin/env bash
# Pinned static dependencies for the portable build

fetch_source() {
    local name=$1 url=$2 checksum=$3
    local source_patch=${4:-}
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

        if [[ -n $source_patch ]]; then
            patch --batch --forward --fuzz=0 --strip=1 --directory="$source_dir.part" < "$source_patch"
        fi

        mv "$source_dir.part" "$source_dir"
    fi
}

cmake_library() {
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

meson_library() {
    local name=$1
    shift

    local reconfigure=()
    if [[ -f $build_dir/build/$name/build.ninja ]]; then
        reconfigure=(--reconfigure)
    fi

    meson setup "${reconfigure[@]}" "$build_dir/build/$name" "$build_dir/sources/$name" \
        --prefix "$prefix" --libdir lib --buildtype release --default-library static \
        --prefer-static --auto-features=disabled --wrap-mode=nodownload "$@"
    meson compile -C "$build_dir/build/$name" -j "$jobs"
    meson install -C "$build_dir/build/$name"
}

build_images() {
    fetch_source dav1d \
        https://download.videolan.org/pub/videolan/dav1d/1.5.4/dav1d-1.5.4.tar.xz \
        686616b7c69eb88d44459391ab25cac13b6647a3b288835c5784e71c1514a5c5
    fetch_source libde265 \
        https://github.com/strukturag/libde265/releases/download/v1.1.3/libde265-1.1.3.tar.gz \
        554228bd17788c99a7e63b37ab5634722190e6e2bf60c1dcb01cef328e133905
    fetch_source libheif \
        https://github.com/strukturag/libheif/releases/download/v1.23.7/libheif-1.23.7.tar.gz \
        f7483e818503ea6224c8863e4a56ac02c7089021a98472c811c12aa19faf2475

    meson_library dav1d -Denable_tools=false -Denable_tests=false

    cmake_library libde265 -DENABLE_DECODER=OFF -DENABLE_ENCODER=OFF -DENABLE_SDL=OFF

    local disabled_codecs=(
        X265 X264 OpenH264_DECODER KVAZAAR UVG266 VVDEC VVENC AOM_DECODER AOM_ENCODER
        SvtEnc RAV1E JPEG_DECODER JPEG_ENCODER OpenJPEG_DECODER OpenJPEG_ENCODER
        FFMPEG_DECODER OPENJPH_ENCODER
    )
    local codec_options=() codec
    for codec in "${disabled_codecs[@]}"; do
        codec_options+=("-DWITH_$codec=OFF")
    done

    cmake_library libheif "${codec_options[@]}" \
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
    touch "$build_dir/image-complete"
}

build_media() {
    fetch_source vulkan-headers \
        https://codeload.github.com/KhronosGroup/Vulkan-Headers/tar.gz/refs/tags/v1.4.366 \
        577b90c208e6fd39ec9a1460e48772f63c76f6f5afb441df2a977a0fdb3f618c
    fetch_source glslang \
        https://codeload.github.com/KhronosGroup/glslang/tar.gz/refs/tags/16.6.0 \
        9c09b901149c729df745057dafa815278aaa101b84d2b6e14f16a42de52f97f2
    fetch_source sdl3 \
        https://www.libsdl.org/release/SDL3-3.4.18.tar.gz \
        9c75cf16330322c217dedd2e0609f1124f1b54b8633e763467b4684d0f4334a3
    fetch_source libdrm \
        https://dri.freedesktop.org/libdrm/libdrm-2.4.128.tar.xz \
        3bb35db8700c2a0b569f2c6729a53f5495786856b310854c8de57782a22bddac
    fetch_source libva \
        https://codeload.github.com/intel/libva/tar.gz/refs/tags/2.24.1 \
        0b4a3649ee8d683b9cce2ef094df4fb039d276c0cef7e49337c43d3b297b9f42
    fetch_source libplacebo \
        https://codeload.github.com/haasn/libplacebo/tar.gz/refs/tags/v7.360.1 \
        d05fdf90bea2f629eaa2d115e909fd356388ac639e54f77b87a018a6d76224bd
    # Keep NVDEC compatible with the validated NVIDIA 580 and CUDA 13.0 driver
    fetch_source ffmpeg \
        https://ffmpeg.org/releases/ffmpeg-9.0.tar.xz \
        7f607a00dd0d28a729d5a4811205812eef01cf6ef6155025febb6f36a9062d52 \
        "$project_dir/tools/portable/ffmpeg-vulkan-frames.patch"


    if [[ ! -f $build_dir/media-complete ]]; then
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
        sed -i 's/shared_library(/library(/g' "$build_dir/sources/libva/va/meson.build"
        meson_library libva -Dwith_x11=no -Dwith_glx=no -Dwith_wayland=no \
            -Ddriverdir=/usr/lib/x86_64-linux-gnu/dri:/usr/lib64/dri:/usr/lib/dri
        meson_library libplacebo -Dvulkan=enabled -Dvk-proc-addr=disabled -Dvulkan-sdk="$prefix" \
            -Dvulkan-registry="$prefix/share/vulkan/registry/vk.xml" \
            -Dglslang=enabled -Ddemos=false -Dtests=false
        touch "$build_dir/media-complete"
    fi

    if [[ ! -f $build_dir/ffmpeg-complete ]]; then
        mkdir -p "$build_dir/build/ffmpeg"
        (cd "$build_dir/build/ffmpeg"
            "$build_dir/sources/ffmpeg/configure" --prefix="$prefix" \
                --disable-shared --enable-static --enable-pic --disable-autodetect \
                --disable-programs --disable-doc --disable-debug --disable-avdevice \
                --disable-encoders --disable-muxers --disable-network \
                --enable-protocol=file,pipe --enable-libdav1d --enable-libdrm \
                --enable-vaapi --enable-vulkan --enable-ffnvcodec --enable-cuda \
                --enable-cuvid --enable-nvdec --pkg-config-flags=--static \
                --extra-cflags="$CFLAGS $CPPFLAGS" --extra-ldflags="$LDFLAGS"
            make -j "$jobs"
            make install)
        touch "$build_dir/ffmpeg-complete"
    fi

}
