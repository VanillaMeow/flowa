#!/usr/bin/env bash
set -euo pipefail

if [[ $# -lt 2 || $# -gt 3 ]]; then
    printf 'Usage: %s BINARY VIDEO_FIXTURE [unavailable|hardware]\n' "$0" >&2
    exit 1
fi

binary=$(realpath "$1")
fixture=$(realpath "$2")
expected=${3:-unavailable}
if [[ $expected != unavailable && $expected != hardware ]]; then
    printf 'Invalid playback expectation: %s\n' "$expected" >&2
    exit 1
fi
work_dir=$(mktemp -d)
trap 'rm -rf -- "$work_dir"' EXIT

# Copy only the ELF so the test cannot accidentally use companion media libraries
cp "$binary" "$work_dir/flowa"
chmod +x "$work_dir/flowa"
export SDL_AUDIO_DRIVER=dummy
export WGPU_BACKEND=vulkan
export RUST_LOG=flowa=info,wgpu_hal=warn
unset WAYLAND_DISPLAY LD_LIBRARY_PATH LD_PRELOAD

if ! timeout 45s xvfb-run -a "$work_dir/flowa" "$fixture" \
    --smoke-test --screenshot "$work_dir/frame.png" > "$work_dir/playback.log" 2>&1; then
    cat "$work_dir/playback.log" >&2
    exit 1
fi

cat "$work_dir/playback.log"
grep -q 'Audio output: sdl3' "$work_dir/playback.log"
grep -q 'Renderer: .*Vulkan' "$work_dir/playback.log"

# Software Vulkan CI has no video decoder and must show an actionable error
if [[ $expected == unavailable ]]; then
    grep -q 'Hardware decoding is unavailable' "$work_dir/playback.log"
    if grep -q 'Video decoder:' "$work_dir/playback.log"; then
        printf '%s\n' 'A decoder was unexpectedly selected in the software Vulkan runtime.' >&2
        exit 1
    fi
    test -s "$work_dir/frame.png"
    printf '%s\n' 'Single portable executable correctly rejects missing hardware decoding'
    exit 0
fi

grep -q 'Video decoder: .*GPU-resident frames' "$work_dir/playback.log"
if grep -Eq '\bERROR\b|Video backend unavailable|Video device setup:' "$work_dir/playback.log"; then
    exit 1
fi

# Testsrc2 contains saturated colors that cannot come from the default viewer chrome
python3 - "$work_dir/frame.png" <<'PY'
import sys

from PIL import Image

with Image.open(sys.argv[1]) as screenshot:
    pixels = screenshot.convert("RGB").getdata()
    colored_pixels = sum(max(pixel) > 160 and min(pixel) < 70 for pixel in pixels)

if colored_pixels < 10000:
    raise SystemExit("Portable playback did not render the video test pattern")

print("Single portable executable rendered hardware-decoded video through Vulkan")
PY
