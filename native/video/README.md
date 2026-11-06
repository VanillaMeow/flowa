# Video backend

FFmpeg 9 decodes video, libplacebo renders into wgpu textures, and SDL3 plays audio.
Rust owns playback, seeking, queues, FFmpeg contexts, resampling, poster extraction,
and frame lifetimes. Bindings are generated from the linked FFmpeg SDK. The C shim
handles the shared Vulkan device, libplacebo rendering, and GPU interoperability.

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
Hardware candidates must return hardware frames. Failed candidates replay a
bounded set of initial packets into the next decoder. Software decoding runs on a
dedicated worker with bounded packet and frame channels and two codec threads.
Seek generations discard stale CPU frames. Audio decoding also runs on the CPU.

A hardware decode error after playback starts switches to software and seeks to
the current position, preserving pause state and the displayed frame. This
recovery requires a seekable duration. GPU import failures and device loss still
report an error rather than attempting renderer recovery.

Vulkan and VA-API frames are imported into the renderer. NVDEC uses a GPU-to-GPU
copy into Vulkan memory. Timeline semaphores synchronize video textures with wgpu.
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
RUSTFLAGS="-C target-cpu=native" SDL_AUDIO_DRIVER=dummy cargo test --release \
    native_vulkan_playback_seek_resize_and_reload -- --ignored --nocapture
```

Requires FFmpeg's command-line tools, Vulkan, and a hardware decoder. It checks
pixels, pause, seeking, volume, resize, reload, and teardown.
On NVIDIA, `CUDA_VISIBLE_DEVICES=-1` exercises Vulkan Video if supported.

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
On GTX 1050 Ti / NVIDIA 580.178.04, NVDEC passes validation. Vulkan H.264 playback
works but FFmpeg's frame pool triggers `VUID-VkImageCreateInfo-pNext-06811` for
unsupported mutable-image flags. This remains unresolved.

`just portable` checks the executable's shared dependencies, RPATH, and glibc
baseline during the build. Use the Rust tests above for playback validation.
