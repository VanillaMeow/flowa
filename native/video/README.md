# Video backend

FFmpeg 9 decodes video, libplacebo renders into wgpu textures, and SDL3 plays audio.
Rust owns playback, seeking, queues, FFmpeg contexts, resampling, poster extraction,
and frame lifetimes. Bindings are generated from the linked FFmpeg SDK, and from
`bindings.h` for the shim and SDL3 audio. The C shim handles the shared Vulkan
device, libplacebo rendering, and GPU interoperability.

Linux hardware playback is validated. Windows Vulkan source and build preparation
are present, but native Windows build and hardware validation are still pending.
macOS VideoToolbox/Metal playback remains planned work in [BACKENDS.md](BACKENDS.md).

## Build

Use `just build`. Cargo generates FFmpeg bindings, compiles the GPU shim, and links
the installed media libraries. Binding generation needs Clang/libclang.
`just portable` statically links the media stack into one executable.
See [BUILDING.md](../../BUILDING.md) for dependencies and commands.

## Decoding and fallback

Decoder selection tries NVDEC, Vulkan Video, VA-API on Linux, then FFmpeg software decoding.
Preferences → Performance → Video decoder selects Auto, an available hardware
backend, or Software. A hardware selection falls back to software for unsupported
codecs. Switching decoders preserves position and pause state on seekable videos;
other videos use the saved choice when reopened. After a playback error, selecting
another decoder reopens the stream. File details reports video and audio codecs,
the source audio channel count, sample rate, bitrate, and active video decoder.
Hardware candidates must return hardware frames. Failed candidates replay a
bounded set of initial packets into the next decoder. Software decoding runs on a
dedicated worker with bounded packet and frame channels; FFmpeg sizes its codec
thread pool from the CPU. Poster extraction decodes on a single thread.
Seek generations discard stale CPU frames. Audio decoding also runs on the CPU.
When no audio output device can be opened, playback continues without sound and
the clock follows wall time.

A hardware decode error after playback starts switches to software and seeks to
the current position, preserving pause state and the displayed frame. This
recovery requires a seekable duration. GPU import failures and device loss still
report an error rather than attempting renderer recovery.

Compatible Vulkan and VA-API frames are imported into the renderer. Vulkan output
without sampleable plane views is copied into separate sampleable planes
on the GPU. NVDEC uses its existing GPU-to-GPU copy into Vulkan memory.
Timeline semaphores synchronize transfers and video textures with wgpu.
Seeking keeps the displayed frame until its replacement is ready.
Seeking also clears a latched FFmpeg interruption error so an aborted read from a
superseded seek cannot poison the next playback generation.

Software frames upload into reusable libplacebo plane textures. They use the same
color conversion, SDR output, and tone mapping as hardware frames. CPU fallback
still requires the Vulkan presenter; it does not enable OpenGL or DX12 playback.
CPU frames and their worker no longer retain or require a Vulkan device owner.

Output is SDR RGBA with libplacebo tone mapping. Subtitles, track selection, and
end-to-end HDR display output are not implemented.

## Filmstrip thumbnails

Only visible filmstrip items request thumbnails. A bounded background worker opens
video independently of playback, selects a nearby keyframe within the first three
seconds, and scales a decoded CPU frame directly to at most 192 by 128 pixels.
It creates no audio output or GPU device. Display rotation and sample aspect ratio
are applied before returning pixels to the UI.

Extraction checks folder-generation cancellation and a cooperative three-second
deadline, and limits poster decoding to 512 packets or 32 MiB after stream probing. FFmpeg codec calls
cannot be interrupted midway through decoding a frame. Thumbnail textures use the
existing 128-entry in-memory filmstrip cache, including failed requests; uploads
remain limited to four per UI frame. There is no disk cache.

Thumbnails use libswscale matrix and range conversion. HDR thumbnails do not yet
receive the playback renderer's HDR-to-SDR tone mapping.

See [the backend roadmap](BACKENDS.md) for Windows, Metal, OpenGL, and DX12.

## Validation

Run the hardware playback test:

```sh
WGPU_VALIDATION=1 WGPU_DEBUG=1 VK_LAYER_VALIDATE_SYNC=1 \
    RUSTFLAGS="-C target-cpu=native" SDL_AUDIO_DRIVER=dummy cargo test --release \
    native_vulkan_playback_seek_resize_and_reload -- --ignored --nocapture
```

Requires FFmpeg's command-line tools, Vulkan, the Khronos validation layer, and a
hardware decoder. It checks pixels, pause, seeking, volume, resize, reload, and
teardown.
The test verifies that the validation layer is active. Vulkan errors reported
through wgpu fail the test, including errors during device teardown.
Set `FLOWA_TEST_VIDEO_DECODER=nvdec` to require NVDEC specifically.
On NVIDIA, set `CUDA_VISIBLE_DEVICES=-1` and `FLOWA_TEST_VIDEO_DECODER=vulkan`
to require Vulkan Video instead of accepting another hardware candidate.
Set `FLOWA_TEST_VIDEO_CODEC=hevc` to run the same checks with 10-bit HEVC instead
of 8-bit H.264. This requires FFmpeg's `libx265` encoder.

Additional focused tests:

```sh
cargo test unsupported_hardware_falls_back_to_software -- --ignored --nocapture
cargo test hardware_failure_recovers_with_software_frames -- --ignored --nocapture
cargo test video_posters_handle -- --ignored --nocapture
cargo test seek_recovers_after_an_interrupted_avio_read -- --ignored --nocapture
```

The fallback test uses raw video with no hardware decoder configuration and checks
rendered pixels, pause, consecutive seeks, resize, EOF, replay, reload, and teardown.
The recovery test starts with hardware H.264 and injects a decode failure. The
poster test requires ffmpeg with libx264 but needs no graphics or audio device.

For Vulkan validation, install the Khronos layer and set `WGPU_VALIDATION=1`,
`WGPU_DEBUG=1`, and `VK_LAYER_VALIDATE_SYNC=1`.

The portable FFmpeg 9.0 SDK applies
[`ffmpeg-vulkan-frames.patch`](../../tools/portable/ffmpeg-vulkan-frames.patch).
It removes storage usage from decode output, requires decode and GPU-copy support,
and restricts image usage and creation flags to the selected video profile's
capabilities. This follows the
[Vulkan image creation requirements](https://docs.vulkan.org/refpages/latest/refpages/source/VkImageCreateInfo.html).
When mutable views or sampling are unsupported, `native/video/transfer.c` copies
the decoded planes into ordinary sampleable images, preserving frame metadata.
The copy waits for decoder timeline values, signals updated source and destination
values, and retains its command buffer until GPU completion.
Dedicated reference-only images and DRM modifier allocations keep their existing
paths. Decoded pixels remain on the GPU throughout playback.
The patch also preserves 64-bit access masks and synchronizes decoder layout
transitions with prior reads and writes, including uses on other queues.

On GTX 1050 Ti / NVIDIA 580.178.04, the patched SDK passes forced Vulkan H.264 and
10-bit HEVC playback, NVDEC H.264 playback, and the fallback/recovery tests with
Khronos validation and synchronization checks enabled. This does not establish
other GPU, driver, codec, or HDR output coverage.

Unpatched FFmpeg pools can still trigger `VUID-VkImageCreateInfo-pNext-06811` on
GTX 1050 Ti / NVIDIA 580.178.04. The local 9.0.2-100 package's CUDA 13.0 compatibility
patch allows NVDEC but does not fix Vulkan image allocation. Rebuild FFmpeg with
equivalent Vulkan fixes or use the patched portable SDK before expecting the
forced Vulkan validation test to pass. FFmpeg 9.0 remains the portable pin;
newer CUDA requirements are a separate
compatibility concern.

`just portable` checks the executable's shared dependencies, RPATH, and glibc
baseline during the build. Use the Rust tests above for playback validation.
