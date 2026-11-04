#!/usr/bin/env bash
set -euo pipefail

project_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd)
build_root="$project_dir/target/native-video"
revision=v0.41.0
archive_hash=ee21092a5ee427353392360929dc64645c54479aefdb5babc5cfbb5fad626209
source_dir="$build_root/mpv-$revision"

mkdir -p "$build_root"

if [[ ! -f "$build_root/upstream.tar.gz" ]]; then
    curl -fL --retry 3 "https://codeload.github.com/mpv-player/mpv/tar.gz/refs/tags/$revision" \
        -o "$build_root/upstream.tar.gz"
fi

printf '%s  %s\n' "$archive_hash" "$build_root/upstream.tar.gz" | sha256sum --check

if [[ ! -d "$source_dir" ]]; then
    temporary_source=$(mktemp -d "$build_root/mpv-source.XXXXXX")
    trap 'rm -rf -- "$temporary_source"' EXIT
    tar -xzf "$build_root/upstream.tar.gz" --strip-components=1 -C "$temporary_source"
    patch --directory="$temporary_source" --strip=1 < "$project_dir/native/mpv/mpv.patch"
    mv "$temporary_source" "$source_dir"
    trap - EXIT
fi

# Apply the decoder fix to both existing and newly extracted build trees
decoder_patch="$project_dir/native/mpv/cuda-zero-init.patch"
if patch --directory="$source_dir" --strip=1 --forward --dry-run \
    < "$decoder_patch" > /dev/null 2>&1; then
    patch --directory="$source_dir" --strip=1 < "$decoder_patch"
elif ! patch --directory="$source_dir" --strip=1 --reverse --dry-run \
    < "$decoder_patch" > /dev/null 2>&1; then
    printf 'Cannot apply the NVDEC initialization fix in %s\n' "$source_dir" >&2
    exit 1
fi

cp "$project_dir/native/mpv/flowa_vk.h" "$project_dir/native/mpv/libmpv_vk.c" \
    "$source_dir/video/out/vulkan/"

meson setup --reconfigure "$build_root/build" "$source_dir" \
    -Dbuildtype=release -Dcplayer=false -Dlibmpv=true -Dtests=false \
    -Dgpl=true -Dvulkan=enabled -Dgl=disabled -Dmanpage-build=disabled
meson compile -C "$build_root/build" -j "${CARGO_BUILD_JOBS:-4}"

cp "$build_root/build/libmpv.so.2.5.0" "$build_root/libflowa-mpv.so.tmp"
mv "$build_root/libflowa-mpv.so.tmp" "$build_root/libflowa-mpv.so"
printf 'Video backend built: %s\n' "$build_root/libflowa-mpv.so"
