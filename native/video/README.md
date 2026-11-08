# Video playback

FFmpeg decodes, libplacebo renders into wgpu textures, and SDL3 plays audio.
Rust owns playback, seeking, FFmpeg contexts, and frame lifetimes. A small C shim
handles the Vulkan device and libplacebo rendering.

Video works on Linux. Windows code exists but is untested, and macOS is planned.
See [BACKENDS.md](BACKENDS.md) for the roadmap.

## Code layout

| Path | Contents |
| --- | --- |
| `src/video/media/` | FFmpeg wrappers for demuxing, decoding, and resampling |
| `src/video/player/` | Playback engine, decoder selection, audio, and clock |
| `src/video/thumbnail.rs` | Filmstrip thumbnails |
| `src/video/gpu.rs` | Shares the Vulkan device with wgpu |
| `native/video/device.c` | Creates the Vulkan device with only the queues flowa uses |
| `native/video/gpu.c` | libplacebo rendering and hardware frame import |
| `native/video/transfer.c` | Copies decoded frames that can't be sampled directly |

Cargo generates the Rust bindings from `ffmpeg.h` and `bindings.h` and compiles
the shim during the build.

## Decoding

Auto tries NVDEC, Vulkan Video, VA-API, then software decoding. Choosing NVDEC or
VA-API in Preferences → Performance falls back to Vulkan Video, then software.
A decoder that fails to decode, or whose frames the renderer can't import, hands
over to the next one, which resumes at the same position. Switching away from the
chosen decoder, or from one that failed during playback, shows a notice.

Hardware frames stay on the GPU. Software frames upload into reusable textures.
Both go through the same libplacebo color conversion and tone mapping to SDR.
Rendering always needs Vulkan, even when decoding in software.

Without an audio device, playback continues silently.

Not implemented yet: subtitles, track selection, and HDR output.

## Thumbnails

A background worker decodes one keyframe from the first three seconds on the CPU
and scales it to fit 192×128. It gives up after three seconds, 512 packets, or
32 MiB of reading. HDR thumbnails aren't tone mapped yet.

## FFmpeg patch

FFmpeg 9.0 creates Vulkan decode images with flags some drivers don't support,
which fails validation and can break Vulkan Video decoding. Portable builds apply
[`ffmpeg-vulkan-frames.patch`](../../tools/portable/ffmpeg-vulkan-frames.patch),
which limits those flags to what the video profile supports and fixes decoder
synchronization. Builds against system FFmpeg need an equivalent fix to use
Vulkan Video on affected drivers.

## Testing

The main hardware test needs FFmpeg's command-line tools with libx264, a hardware
decoder, and the Khronos Vulkan validation layer. Any Vulkan validation error
fails it:

```sh
WGPU_VALIDATION=1 WGPU_DEBUG=1 VK_LAYER_VALIDATE_SYNC=1 SDL_AUDIO_DRIVER=dummy \
    RUSTFLAGS="-C target-cpu=native" cargo test --release \
    native_vulkan_playback_seek_resize_and_reload -- --ignored --nocapture
```

It checks pixels, pausing, seeking, volume, resizing, reloading, and teardown.
`RUSTFLAGS` matches `just build`, so the test reuses its build cache.

| Variable | Effect |
| --- | --- |
| `FLOWA_TEST_VIDEO_DECODER=nvdec` | Require NVDEC |
| `FLOWA_TEST_VIDEO_DECODER=vulkan` | Require Vulkan Video; on NVIDIA also set `CUDA_VISIBLE_DEVICES=-1` |
| `FLOWA_TEST_VIDEO_CODEC=hevc` | Test 10-bit HEVC instead of H.264; needs libx265 |

Other ignored tests:

```sh
cargo test unsupported_hardware_falls_back_to_software -- --ignored --nocapture
cargo test hardware_failure_recovers_with_the_next_decoder -- --ignored --nocapture
cargo test video_posters_handle -- --ignored --nocapture
cargo test seek_recovers_after_an_interrupted_avio_read -- --ignored --nocapture
```
