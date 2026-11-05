# Video backend

FFmpeg 9 decodes video, libplacebo renders into wgpu textures, and SDL3 plays audio.
Rust owns playback, seeking, queues, and frame lifetimes. The C shim handles native
contexts, FFmpeg structure access, and GPU interoperability.

## Build

Use `just build`. Cargo compiles the shim and links the installed media libraries.
`just portable` statically links the media stack into one executable.
See [BUILDING.md](../../BUILDING.md) for dependencies and commands.

## Hardware decoding

Decoder selection tries NVDEC, Vulkan Video, then VA-API. Each backend must return
hardware frames. Unsupported codecs or drivers produce an error; software video
decoding and CPU copy-back are disabled. Audio decoding runs on the CPU.

Vulkan and VA-API frames are imported into the renderer. NVDEC uses a GPU-to-GPU
copy into Vulkan memory. Timeline semaphores synchronize video textures with wgpu.
Seeking keeps the displayed frame until its replacement is ready.

Output is SDR RGBA with libplacebo tone mapping. Subtitles, track selection, and
end-to-end HDR display output are not implemented.

## Validation

Run the hardware playback test:

```sh
RUSTFLAGS="-C target-cpu=native" SDL_AUDIO_DRIVER=dummy cargo test --release \
    native_vulkan_playback_seek_resize_and_reload -- --ignored --nocapture
```

Requires FFmpeg's command-line tools, Vulkan, and a hardware decoder. It checks
pixels, pause, seeking, volume, resize, reload, and teardown.
On NVIDIA, `CUDA_VISIBLE_DEVICES=-1` exercises Vulkan Video if supported.

For Vulkan validation, install the Khronos layer and set `WGPU_VALIDATION=1`,
`WGPU_DEBUG=1`, and `VK_LAYER_VALIDATE_SYNC=1`.
On GTX 1050 Ti / NVIDIA 580.178.04, NVDEC passes validation. Vulkan H.264 playback
works but FFmpeg's frame pool triggers `VUID-VkImageCreateInfo-pNext-06811` for
unsupported mutable-image flags. This remains unresolved.

`just test-portable` checks the single executable in a clean Ubuntu runtime with
no hardware decoder. It verifies startup and an explicit missing-decoder error;
actual hardware playback is covered by the test above.
