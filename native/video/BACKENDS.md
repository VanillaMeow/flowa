# Rendering backend work

The UI already has renderer preferences for Vulkan, OpenGL, Metal, and DX12.
`src/app/native.rs` maps these to wgpu backends and retries Linux startup with
OpenGL when Vulkan initialization fails. Still-image rendering uses ordinary
egui textures and does not require Vulkan interoperability.

Video is currently Linux/Vulkan-only, including the new CPU decoding fallback.
This document describes follow-up work, not backend support added by this change.

## Where video depends on Vulkan

- `build.rs` builds the media shim only on Linux and requires libplacebo and Vulkan
  alongside FFmpeg and SDL3. Cargo's `ash` and `pollster` dependencies and Rust
  video modules are Linux-only.
- `flowa_video.h` includes Vulkan types in its shared ABI. `internal.h` mixes
  demuxing, software codecs, and GPU contexts in one header.
- `src/video/gpu.rs` creates a libplacebo Vulkan device, imports it into wgpu-hal,
  and keeps a registry keyed by the raw Vulkan device. An ordinary wgpu device
  cannot currently initialize the player, even when its backend is Vulkan.
- `Flowa::new` initializes video only when startup supplied that special device.
- `player/mod.rs` owns an `ash::Device` and Vulkan timeline semaphore.
  `player/rendering.rs` passes raw `VkImage` handles and semaphore values between
  wgpu and libplacebo. Texture lifetime and resize retirement depend on both APIs.
- Hardware codec creation derives CUDA and Vulkan contexts from that device;
  VA-API selects its DRM render node. The UI thread serializes hardware submissions.
- `player/media.rs` and `thumbnail.rs` use Unix path bytes. Windows needs a defined
  UTF-8 path contract for FFmpeg and tests covering non-ASCII paths.

The software worker's codec needs no GPU, and thumbnail extraction already runs
without a playback device. Their native declarations still share the Vulkan ABI,
and the Rust frame wrapper retains `NativeGpu`. These are coupling points to remove.

## Recommended first step

Add CPU-decoded playback through normal wgpu textures before adding another native
hardware interop implementation. This is the shortest route to usable video on
OpenGL, Metal, and DX12 and provides a fallback for each later hardware path.

1. Split the media ABI and headers from the Vulkan ABI. Build FFmpeg, libswscale,
   libswresample, and SDL3 independently of libplacebo, Vulkan headers, and `ash`.
   Compile that media path on Linux, macOS, and Windows.
2. Separate frame ownership and decoder construction from `NativeGpu`. Let a
   software player use the API, stream, worker, and audio clock with an ordinary
   wgpu device. Keep the existing Vulkan fast path as a separate renderer.
3. Add a portable frame presenter. An initial implementation can convert frames
   into reusable RGBA buffers on a worker and upload using `Queue::write_texture`.
   Keep frame queues bounded by bytes as well as count and preserve seek generations.
   At 3840 by 2160 and 60 fps, RGBA8 uploads alone carry about 1.99 GB/s, so this
   should be a compatibility path, with measurements before promising 4K60.
4. For better throughput, upload YUV planes and convert in a wgpu fragment shader.
   Account for subsampling, chroma siting, matrix coefficients, range, transfer
   functions, primaries, rotation, and display aspect. HDR-to-SDR tone mapping needs
   its own implementation or an explicitly documented quality limitation. Matching
   libplacebo's current output is a substantial part of this step.
5. Remove the startup `video_device` gate for the portable player. Select the
   presenter based on the actual adapter/backend and available interop capabilities.
   A Vulkan setup failure should still permit the CPU player on ordinary wgpu.
6. Add native SDK packaging and build/runtime CI for each OS. Exercise audio,
   pause, consecutive seeks, EOF/replay, rapid file replacement, resize, Unicode
   paths, HDR input, and teardown. Keep thumbnail tests independent of graphics.

This sequence reuses the current demuxer, audio clock, scheduling, and seek behavior.
It requires reorganizing the player rather than replacing the application renderer.

## Backend-specific work after portable playback

| Backend | Work for hardware frames and current color quality |
| --- | --- |
| OpenGL on Linux | Libplacebo has an OpenGL backend. Enable it in portable builds, arrange a compatible current GL context, wrap the wgpu GL target, and define GL synchronization and state restoration. A separate libplacebo context needs explicit resource sharing. Hardware decode through VA-API/DRM or CUDA needs import-capability checks; CPU decode plus upload is the first fallback |
| Metal on macOS | Add FFmpeg VideoToolbox decoding. A GPU path needs CVPixelBuffer/IOSurface to Metal texture interop, compatible wgpu device ownership, command ordering, and resource lifetime tracking. Libplacebo 7.360.1 has no native Metal renderer; choose wgpu color processing or evaluate a separate Vulkan/MoltenVK bridge with its extra sharing and synchronization work |
| DX12 on Windows | Add FFmpeg D3D11VA or D3D12VA decoding. D3D11 decode plus DX12 presentation needs shared resources and cross-API fences; D3D12 decode still needs compatible device/adapter ownership, resource transitions, formats, and fences. Libplacebo 7.360.1 exposes D3D11 rather than DX12, so it cannot simply replace the current Vulkan device in the wgpu DX12 path |

wgpu already supplies these graphics backends, with OpenGL classified as secondary
support. The missing work is frame presentation, media builds, and platform interop.
The exact cost of each native path needs a small import/synchronization prototype
on target hardware. CPU playback should land before those prototypes become release
requirements.

## Sources checked

- [wgpu 30.0.1 backends](https://docs.rs/wgpu/30.0.1/wgpu/struct.Backends.html)
- [Libplacebo 7.360.1 backend support](https://github.com/haasn/libplacebo/blob/v7.360.1/README.md#hardware-requirements)
- [Libplacebo FFmpeg frame upload API](https://github.com/haasn/libplacebo/blob/v7.360.1/src/include/libplacebo/utils/libav.h)
- [FFmpeg hardware device types](https://www.ffmpeg.org/doxygen/trunk/hwcontext_8c.html)

The proposed implementation order and platform designs above are engineering
recommendations based on the repository and these APIs, not verified platform ports.
