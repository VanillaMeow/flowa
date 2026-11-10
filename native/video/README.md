# Video playback

FFmpeg decodes, libplacebo renders into wgpu textures, and SDL3 plays audio.
Rust owns playback, seeking, FFmpeg contexts, frame lifetimes, the Vulkan device,
and rendering. A few lines of C reach the parts of libplacebo that have no symbol
Rust could link.

Video works on Linux. Windows code exists but is untested, and macOS is planned.
See [BACKENDS.md](BACKENDS.md) for the roadmap.

## Code layout

| Path | Contents |
| --- | --- |
| `src/video/media/` | FFmpeg wrappers for demuxing, decoding, and resampling |
| `src/video/player/` | Playback engine, audio, and clock |
| `src/video/player/decoder.rs` | The `Decoder` trait and the backends that implement it |
| `src/video/player/hardware_decoder.rs` | NVDEC, Vulkan Video, and VA-API decoding on the rendering GPU |
| `src/video/player/software_decoder.rs` | FFmpeg's CPU decoders on a worker thread |
| `src/video/player/decoder_selection.rs` | Picks the first decoder that works and falls back to the next |
| `src/video/player/decoder_detection.rs` | Lists the hardware decoders this computer can open |
| `src/video/thumbnail.rs` | Filmstrip thumbnails |
| `src/video/gpu/mod.rs` | Shares the Vulkan device with wgpu |
| `src/video/gpu/shared_device.rs` | Creates the Vulkan device with only the queues flowa uses |
| `src/video/gpu/frame_renderer.rs` | libplacebo rendering into wgpu textures |
| `src/video/gpu/frame_transfer.rs` | Copies decoded frames that can't be sampled directly |
| `native/video/placebo.c` | libplacebo's macros, header-only FFmpeg helpers, and exported data |

Cargo generates the Rust bindings for FFmpeg, libplacebo, and SDL3 from `ffmpeg.h`
and `bindings.h`, and compiles `placebo.c` during the build.

`placebo.c` stays in C because `libplacebo/utils/libav.h` implements frame import
as `static inline` functions, which only a C compiler can turn into code. It also
wraps the log constructor, a macro whose symbol changes with each API version, and
the exported constants, which Rust can't import from a Windows DLL.

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
which fails validation and can break Vulkan Video decoding.
[`ffmpeg-vulkan-frames.patch`](../../tools/portable/ffmpeg-vulkan-frames.patch)
limits those flags to what the video profile supports and fixes decoder
synchronization.

Local builds link a static FFmpeg with this patch, built by
[`tools/bundled-ffmpeg.sh`](../../tools/bundled-ffmpeg.sh) using the same recipe as
portable builds. Builds against system FFmpeg need an equivalent fix to use
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
