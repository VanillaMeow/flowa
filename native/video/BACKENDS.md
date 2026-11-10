# Video backends

Video works on Linux through Vulkan. Windows code exists but has never been
compiled or tested there. macOS is planned.

## Goals

- Every supported platform gets hardware decoding with frames that stay on the GPU.
  GPU-to-GPU copies are fine; reading pixels back to the CPU is not
- Software decoding is a fallback only. A CPU-only player doesn't count as
  platform support
- A renderer is only advertised as supporting video once its hardware path is tested
- Keep media logic in Rust, with small C bridges where libraries only offer
  header-implemented helpers

## Architecture

| Platform | Rendering | Hardware decoders |
| --- | --- | --- |
| Linux | wgpu Vulkan + libplacebo | NVDEC, Vulkan Video, VA-API |
| Windows | wgpu Vulkan + libplacebo | NVDEC and Vulkan Video; maybe D3D11VA/D3D12VA |
| macOS | wgpu Metal + native color processing | VideoToolbox |

Using Vulkan on Windows reuses the Linux renderer and color processing without a
bridge to DX12. Decoder and renderer are separate choices: a Vulkan renderer can
show NVDEC frames after a GPU copy, and Vulkan graphics support says nothing about
Vulkan Video codec support. Decoder and renderer must run on the same GPU.

## What ties video to Vulkan

- `src/video/gpu/shared_device.rs` creates one Vulkan device and `src/video/gpu/mod.rs`
  shares it with wgpu. The player can't use an ordinary wgpu device, so video only works
  when startup created this shared device
- The player passes raw `VkImage` handles and timeline semaphore values between
  wgpu and libplacebo
- CUDA and Vulkan Video contexts are derived from the shared device. VA-API uses
  the matching DRM render node
- With a second graphics queue, playback runs on a background worker so the UI
  keeps drawing. Otherwise it runs on the UI thread

Software decoding and thumbnails don't depend on the GPU, and a future Metal
player must keep it that way.

## Remaining work

### Windows

Build it with a Windows FFmpeg/libplacebo/SDL3 SDK and test NVDEC and Vulkan
Video on real hardware, including AMD and Intel. Test non-ASCII paths, which
FFmpeg receives as UTF-8. Package the DLLs. If AMD or Intel coverage needs
D3D11VA or D3D12VA, add GPU import of those frames into the Vulkan renderer first.

### macOS

Decode with VideoToolbox and import its IOSurface-backed frames into Metal
textures without CPU conversion. libplacebo has no Metal renderer, so color
conversion and HDR-to-SDR tone mapping need a new implementation. Compare its
output against libplacebo on reference clips.

### DX12 and OpenGL

These are optional, separate milestones. For DX12, either decode natively with
D3D11VA/D3D12VA or keep libplacebo on Vulkan and share the final RGBA texture
with DX12. For OpenGL, try libplacebo's GL renderer on wgpu's GL context with
VA-API DMA-BUF or CUDA import. Measure both before choosing.

## Release checks

- Run the Linux playback, fallback, recovery, and thumbnail tests from the
  [video README](README.md#testing), plus the portable build's dependency checks
- Run equivalent tests on real Windows and macOS hardware
- Hardware tests must fail if software decoding is silently used instead
- Cover each supported codec and bit depth, HDR-to-SDR output, rotation, aspect
  ratio, audio sync, seeking, end of file and replay, resizing, rapid file
  switching, Unicode paths, and teardown
- Measure throughput, dropped frames, and CPU load before making performance claims

## References

- [libplacebo hardware requirements](https://github.com/haasn/libplacebo/blob/v7.360.1/README.md#hardware-requirements)
- [libplacebo FFmpeg frame API](https://github.com/haasn/libplacebo/blob/v7.360.1/src/include/libplacebo/utils/libav.h)
- [FFmpeg Vulkan hardware context](https://github.com/FFmpeg/FFmpeg/blob/n9.0/libavutil/hwcontext_vulkan.c)
- [NVIDIA FFmpeg hardware acceleration](https://docs.nvidia.com/video-technologies/video-codec-sdk/13.1/ffmpeg-with-nvidia-gpu/)
- [Apple CoreVideo Metal texture cache](https://developer.apple.com/documentation/corevideo/cvmetaltexturecache)
