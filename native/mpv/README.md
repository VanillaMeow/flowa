# Vulkan playback backend

Flowa adds a private Vulkan target interface to upstream mpv v0.41.0. The patch
reuses mpv's existing GPU renderer, hardware decoder interop, subtitles, playback
clock, and audio output. Despite its internal `gl_video` name, that renderer
already supports Vulkan through libplacebo's rendering abstraction.

The build disables OpenGL. Video images stay in GPU memory between mpv's renderer
and egui. Hardware decoding depends on the GPU, driver, and source codec. Unsupported
codecs can still use software decoding, followed by a GPU upload, as in mpv.

Decoder selection tries `nvdec`, `vaapi`, then `vulkan`, followed by software
decoding. Copy-back methods such as `vulkan-copy` are excluded from automatic
selection. The current bridge uses mpv's legacy `gpu` renderer through libplacebo;
it does not provide `gpu-next` feature parity. Direct Vulkan Video decoding also
needs a decode-capable device and queue plus mpv hardware-context integration,
which the current graphics-only imported device does not provide.
The legacy Vulkan mapper additionally requires `ra_vk_ctx_get()` to recognize
the rendering context; it currently rejects our swapchain-free bridge. Selecting
`hwdec=vulkan` alone cannot enable that missing integration.

## Build and run

On Arch Linux, install the native development dependencies:

```sh
sudo pacman -S --needed base-devel meson ninja pkgconf curl patch \
    ffmpeg sdl3 libplacebo libass vulkan-headers vulkan-icd-loader
```

The native build requires FFmpeg 9.0 or newer and SDL3 3.2 or newer.
For NVIDIA hardware decoding, mpv also needs `ffnvcodec-headers` at build time and
a working NVIDIA driver with CUDA/NVDEC libraries at runtime. Other mpv features
and decoder integrations are detected from the installed development packages.

```sh
./packaging/linux/build-video.sh
cargo build --release
./target/release/flowa /path/to/video.mp4
```

The native build downloads a checksum-verified upstream archive, applies the local
patch, and produces `target/native-video/libflowa-mpv.so`. It does not replace the
system mpv installation. Subsequent runs reuse the build directory.

Flowa first checks `FLOWA_MPV_LIBRARY`, then `libflowa-mpv.so` beside its executable,
then the development build location above. For a local installation, copy the
library beside the executable. It still depends on the system FFmpeg, libplacebo,
libass, and the other libraries selected by Meson. This is not a portable build.

For distribution, use `packaging/linux/build-portable.sh`. It builds this backend
with FFmpeg 9.0 and SDL 3.4.18, then statically links the media stack into
the single Flowa executable. The `static-video` feature calls linked symbols
directly and does not search for `libflowa-mpv.so`. See [the portable build guide](../../BUILDING.md#portable-linux-build).

The backend requires Linux and Vulkan. Image viewing remains available without
the library. Use `--no-default-features` to omit video and the native image codecs,
or `--no-default-features --features avif,heif` for the previous image-only build.

The build adds an SDL3 audio-stream backend with `sdl3-audio.patch` and
`ao_sdl3.c`. SDL3 handles audio only; windowing and video rendering stay with
eframe and wgpu. The static SDL3 library dynamically loads the host audio clients
so their plugins and configuration match the running desktop.

The build also applies `cuda-zero-init.patch` to the pinned mpv source. Its NVDEC
Vulkan mapper otherwise leaves the timeline semaphore counter and cleanup handles
uninitialized. The fix uses mpv's zero-initializing allocator and applies to
existing build trees as well as fresh extractions.

## Texture handoff

Both renderers use the same Vulkan device and graphics queue. Device creation
adds `hostQueryReset` and `synchronization2` to wgpu's native feature chain
and enables available external-memory and external-semaphore extensions for
hardware decoder interop.

The UI thread owns all wgpu and libmpv render submissions. Libmpv's control API
uses asynchronous commands and observed properties. Wakeup callbacks only request
an egui repaint. Decoder and audio threads remain managed by mpv.

Render-context updates are serviced while loading, before dimensions or a frame
are available. Advanced control allows decoders to queue GPU initialization work
at that stage, and delaying those updates until `FILE_LOADED` can deadlock startup.

Each video uses three `Rgba8Unorm` textures. Wgpu initializes each texture once so
its initialization tracking cannot clear externally rendered pixels. For each
video frame:

1. Wgpu transitions a target to `COLOR_ATTACHMENT_OPTIMAL` and signals a timeline
   semaphore after earlier UI work has finished using it
2. Libplacebo releases the wrapped target from host ownership, waits on that
   value, and mpv renders video and subtitles
3. Libplacebo holds the target again, restores `COLOR_ATTACHMENT_OPTIMAL`, and
   signals the next timeline value
4. The next wgpu submission waits on that value before transitioning the texture
   for sampling by egui

There is no per-frame CPU readback or GPU completion wait. GPU waits occur during
target replacement and teardown. The render context is destroyed before the mpv
handle, native textures, semaphore, device, and loaded library.

`flowa_vk.h` defines private render parameters 1000 and 1001. The bridge checks
`flowa_mpv_vulkan_abi_version` before calling them, so a stock libmpv is rejected
instead of interpreting incompatible render parameters.

## Validation

Normal tests cover media classification, seek validation, time formatting,
responsive control widths, timeline placement above the filmstrip, and pointer
and keyboard seeking. The ignored native
integration test generates a short H.264/AAC clip with FFmpeg, requires actual
hardware decoding, checks GPU-rendered pixels, exercises pause, seek, volume,
target resizing, reload, software decoding, and cleanup:

```sh
RUSTFLAGS="-C target-cpu=native" cargo test --release \
    native_vulkan_playback_seek_resize_and_reload -- --ignored --nocapture
```

The test reads pixels back only to verify output. Playback does not use that path.
For Vulkan validation, install the Khronos validation layer and set
`WGPU_VALIDATION=1 WGPU_DEBUG=1 VK_LAYER_VALIDATE_SYNC=1` when running the test or
application.

The first implementation displays SDR RGBA output. End-to-end HDR presentation,
video thumbnails, and native Metal/DirectX backends remain separate work.

## Code layout

`src/video/mod.rs` exposes observed playback state and asynchronous commands to the
app. `gpu.rs` creates the compatible wgpu device, and `mpv.rs` loads typed symbols
after checking the private ABI. The player is split into engine ownership and
cleanup in `player/mod.rs`, context creation in `context.rs`, property updates in
`events.rs`, and texture handoff in `rendering.rs`.

`src/ui/video_controls.rs` owns the temporary scrub preview. It submits a seek on
release and keeps the preview until the engine acknowledges it. The timeline uses
the width left by measured time labels. Transport sits on a separate row; narrow
windows move the volume slider into its icon menu. All dimensions are logical
egui points and follow the application's UI scale.
