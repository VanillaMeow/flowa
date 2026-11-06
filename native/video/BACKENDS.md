# Video backends and Rust migration

Video currently has native validation on Linux/Vulkan, including software decoding
fallback. The shared Vulkan code is enabled for Windows, but native Windows build
and hardware validation are pending. The macOS path remains planned work.

## Requirements

- Every target platform must ship with working hardware decoding and GPU frame
  presentation for a documented set of supported GPUs, drivers, and codecs
- Hardware playback must avoid decoded-pixel readback to the CPU and CPU color
  conversion. GPU-to-GPU copies are acceptable, as in the existing NVDEC path
- Preserve the existing Linux hardware paths, playback behavior, and SDR output
  with HDR-to-SDR tone mapping while adding platforms and migrating code
- Software decoding remains a fallback for unsupported inputs, devices, or
  failures. A CPU-only implementation does not complete a platform milestone
- Each additional renderer advertised as supporting hardware video must have a
  tested hardware decode and GPU presentation path
- Move application-owned media logic and resource ownership into Rust where
  practical. Retain FFmpeg and other native libraries, plus small C bridges where
  upstream header implementations make them useful

The UI already offers Vulkan, OpenGL, Metal, and DX12 preferences on applicable
platforms. Still-image support and renderer initialization do not establish video
support. Linux startup retries OpenGL if Vulkan initialization fails. A saved
renderer selection that fails before the app opens is reset to Auto and retried.

## Primary architecture

Use Vulkan for both the GUI and video processing on Linux and Windows. This lets
the Windows port reuse the libplacebo renderer, shared Vulkan device design, and
existing color processing without a Vulkan-to-DX12 presentation bridge. Use Metal
and VideoToolbox as the primary macOS path.

| Platform | GUI and video presentation | Hardware decoding candidates |
| --- | --- | --- |
| Linux | wgpu Vulkan with libplacebo processing | Existing NVDEC, Vulkan Video, and VA-API paths |
| Windows | wgpu Vulkan with libplacebo processing | NVDEC through CUDA/Vulkan interop and Vulkan Video; assess native Windows decoders if needed for the supported hardware matrix |
| macOS | wgpu Metal with native texture import and GPU color processing | FFmpeg VideoToolbox with verified hardware acceleration |

The renderer and decoder are separate choices. A Vulkan GUI can display NVDEC
frames after a GPU-to-GPU transfer into Vulkan images. Selecting Vulkan for
rendering does not require Vulkan Video decoding support. Conversely, Vulkan
graphics support alone does not establish support for a particular video codec.

Select compatible decoder and presenter pairs on the same physical GPU. Validate
frame formats, import capabilities, synchronization, and retained owners together.
FFmpeg exposing a hardware decoder does not automatically make its frames usable
by an arbitrary wgpu backend.

## Implementation progress

The shared FFmpeg media operations now live in Rust under `src/video/media/`.
Demuxers retain their interruption callbacks, codecs retain stable hardware-format
callback storage, and frames, packets, stream snapshots, resamplers, and audio
buffers have Rust owners. CPU frames and the software worker no longer retain a
Vulkan device. The C implementation now contains the Vulkan/libplacebo GPU path.

Windows source preparation includes native build selection, Vulkan-first startup,
UTF-8 media paths, and skipping VA-API. These changes do not establish native
Windows support until they are compiled and tested with a Windows SDK and GPU.
macOS Metal presentation, native platform prototypes, and the DX12/OpenGL video
paths remain incomplete. Only a Linux NVIDIA machine is currently available for
runtime validation.

Validation on 2026-11-06 used a GTX 1050 Ti with NVIDIA 580.178.04. All 62 tests,
including the native tests, passed. H.264 playback passed through NVDEC and through
Vulkan Video with CUDA disabled. NVDEC also passed with Khronos validation and
synchronization checks enabled. The software fallback seek test passed twelve
additional runs after adding recovery for a latched FFmpeg AVIO interruption error.

The portable executable rebuilt against the pinned FFmpeg 9.0 SDK, passed the
glibc 2.39/dependency checks, rendered video in the clean Ubuntu software-Vulkan
runtime, and passed a native NVDEC playback smoke test. These results cover this
Linux hardware and the tested fixtures. They do not establish Windows, macOS,
AMD/Intel hardware, or additional codec/profile support.

The portable FFmpeg 9.0 SDK now carries a patch that restricts decode-image usage
and flags to the selected video profile's capabilities. Vulkan images without
sampleable plane views use a synchronized GPU copy into sampleable planes. NVDEC's
existing transfer path is preserved. System builds need the same FFmpeg allocation
fix or an upstream equivalent; the local 9.0.2-100 package's CUDA 13.0 compatibility
patch alone does not fix Vulkan image flags. See the
[video guide](README.md#validation) for decoder-specific validation commands.

The patched SDK passes forced Vulkan H.264 and 10-bit HEVC playback, NVDEC H.264,
and the existing fallback/recovery tests on this NVIDIA machine with Khronos
validation and synchronization checks enabled. The patch also fixes 64-bit
access-mask bookkeeping and synchronization when decoder images are reused.

## Where video depends on Vulkan

- `build.rs` builds the GPU shim on Linux and Windows and requires libplacebo and
  Vulkan alongside FFmpeg and SDL3. Cargo's `ash` and `pollster` dependencies and
  the current playback modules are enabled on Linux and Windows.
- `native/video/transfer.c` copies non-mutable Vulkan decode planes into images
  libplacebo can sample, with timeline synchronization and command-buffer retirement
- `flowa_video.h` now contains only the GPU ABI, with borrowed FFmpeg frame and
  stream pointers. The Rust media bindings come from a separate FFmpeg-only header.
- `src/video/gpu.rs` creates a libplacebo Vulkan device, imports it into wgpu-hal,
  and keeps a registry keyed by the raw Vulkan device. An ordinary wgpu device
  cannot currently initialize the player, even when its backend is Vulkan.
- `Flowa::new` initializes video only when startup supplied that special device.
- `player/mod.rs` owns an `ash::Device` and Vulkan timeline semaphore.
  `player/rendering.rs` passes raw `VkImage` handles and semaphore values between
  wgpu and libplacebo. Texture lifetime and resize retirement depend on both APIs.
- Hardware codec creation derives CUDA and Vulkan contexts from that device;
  VA-API selects its DRM render node. The UI thread serializes hardware submissions.
- `media/` preserves Unix path bytes and requires valid Unicode encoded as UTF-8
  on Windows. Native Windows tests must still exercise non-ASCII paths.

The software worker and thumbnail extractor now use GPU-independent Rust owners.
The player and its selected presenter still require Vulkan. A future Metal player
must supply its own device ownership and frame presentation without reintroducing
a Vulkan dependency into CPU media operations.

## Implementation order

### 1. Establish baselines and prove native frame sharing

Record the current Linux results and define the GPU, driver, codec, bit-depth,
and pixel-format combinations each platform will support. Include representative
SDR and HDR input, and keep unsupported-input fallback tests separate.

Before committing to the full abstraction, prove Windows NVDEC-to-Vulkan transfer
and Vulkan Video import on supported hardware. Prove VideoToolbox-to-Metal texture
sharing on macOS. Check that actual hardware decoding is active and that frames
reach the GUI without CPU readback. Use these prototypes to determine the frame
ownership and synchronization contracts, and expose coverage gaps early.

### 2. Separate the media core and migrate it incrementally to Rust

Keep the existing Rust playback engine, audio clock, packet replay, bounded
queues, seek generations, and worker shutdown behavior. Split FFmpeg demuxing,
audio, software codecs, metadata, and thumbnails from platform GPU contexts.

Choose or generate FFmpeg bindings against the same SDK used for local and
packaged builds. Move `media.c`, `thumbnail.c`, and decoder operations from
`decode.c` into focused Rust owners with `Drop` and typed results. Keep working C
operations during the transition so a complete rewrite does not block the ports.

CPU frames must not require `NativeGpu`. Hardware frames must retain the backend
resources they actually borrow until both decoding and presentation finish.
Keep platform-specific device setup and GPU imports behind focused implementations.
Select these through validated capabilities rather than an unrestricted mix of
decoder and renderer names. Test Linux after each migration step.

### 3. Ship Windows hardware playback through Vulkan

Extend native builds, Cargo dependencies, module gates, startup, and packaging to
Windows. Define and test the FFmpeg path encoding contract, including Unicode
paths. Exclude Linux-only VA-API/DRM dependencies from the Windows build.

Create the shared libplacebo/wgpu Vulkan device on Windows and preserve GPU frame
ownership, timeline synchronization, and color processing. Try NVDEC and Vulkan
Video according to actual device and codec support. FFmpeg contains Windows
CUDA/Vulkan memory and semaphore sharing code; Flowa must validate its use with
the packaged SDK and target drivers.

Measure AMD and Intel coverage as well as NVIDIA. If the declared support matrix
needs D3D11VA, D3D12VA, or another decoder, implement its GPU import into the Vulkan
presenter before declaring those combinations supported. Do not assume those
frames can already pass through the current libplacebo helper.

### 4. Ship macOS hardware playback through Metal

Add FFmpeg VideoToolbox decoding, require hardware acceleration for hardware-path
validation, and import compatible CVPixelBuffer/IOSurface-backed planes into Metal
textures. Retain buffers through GPU completion and present through wgpu's Metal
device without CPU pixel conversion or readback.

Implement GPU color processing for the supported formats. Cover NV12/P010 where
supported, subsampling, chroma siting, matrix coefficients, range, transfer
functions, primaries, rotation, aspect ratio, and HDR-to-SDR tone mapping. Compare
reference clips with the existing libplacebo output using defined tolerances.
Libplacebo 7.360.1 has no native Metal renderer, so basic YUV-to-RGB conversion
alone does not establish equivalent output quality.

Package the native media dependencies and validate audio, paths, and teardown on
macOS. Metal GUI startup alone does not complete this milestone.

### 5. Add DX12 and OpenGL hardware video as additional renderers

Keep these as separate renderer milestones after the primary platform paths.
Each must pass the same hardware playback requirements before being advertised
as supporting hardware video.

For DX12, prototype native D3D11VA/D3D12VA decoding and GPU presentation, and compare
it with retaining Vulkan/libplacebo processing and sharing a final RGBA texture
with DX12. The latter could reuse tone mapping but requires compatible shared
allocations, resource transitions, device matching, and cross-API fences. Evaluate
actual coverage and synchronization cost before selecting the implementation.

For OpenGL, prototype libplacebo GL rendering with the wgpu GL context, target
wrapping, state restoration, and synchronization. Validate VA-API/DRM DMA-BUF import
or CUDA interop on the supported platform and driver combinations. A separate GL
context needs explicit resource sharing. Platform-specific GL support must be
measured rather than inferred from GUI support.

### 6. Reduce the remaining C and finish regression validation

Move remaining application-owned GPU bookkeeping into Rust where validated
bindings and backend contracts make that useful. Retain a small C bridge for
libplacebo's header-implemented FFmpeg helpers. Those functions are not ordinary
exported symbols that Rust declarations alone can link against.

The aim is maintainable Rust ownership and reusable platform code. Removing every
C source line is not a release requirement, and rewriting working GPU glue must
not delay required hardware playback. Native codec dependencies remain.

## Release validation

- Run the existing Linux playback, fallback, recovery, poster, and portable dependency
  checks throughout the work
- Add equivalent Windows and macOS build, packaging, and runtime checks, including
  execution on real target hardware
- Hardware-path tests must fail if software decoding is silently selected. Verify
  decoder choice and resource transfers with suitable GPU diagnostics
- Exercise supported codecs and bit depths, SDR and HDR-to-SDR output, rotation,
  aspect ratio, audio synchronization, pause, consecutive seeks, EOF/replay,
  resize, rapid file replacement, Unicode paths, and teardown
- Measure playback throughput, dropped frames, CPU load, and transfer behavior
  before making performance claims. Include representative high-resolution clips
- Verify unsupported-input and failure fallback separately. GPU import or device
  recovery behavior must be explicit; the current renderer does not automatically
  recover from every hardware failure

CPU-decoded fallback and GPU-independent thumbnails remain useful. Neither
substitutes for the required hardware playback path on any target platform.

## Sources checked

- [wgpu 30.0.1 backends](https://docs.rs/wgpu/30.0.1/wgpu/struct.Backends.html)
- [Libplacebo 7.360.1 backend support](https://github.com/haasn/libplacebo/blob/v7.360.1/README.md#hardware-requirements)
- [Libplacebo FFmpeg frame upload API](https://github.com/haasn/libplacebo/blob/v7.360.1/src/include/libplacebo/utils/libav.h)
- [FFmpeg hardware device types](https://www.ffmpeg.org/doxygen/trunk/hwcontext_8c.html)
- [NVIDIA FFmpeg hardware acceleration on Windows and Linux](https://docs.nvidia.com/video-technologies/video-codec-sdk/13.1/ffmpeg-with-nvidia-gpu/)
- [FFmpeg 9 CUDA/Vulkan interoperability, including Windows](https://github.com/FFmpeg/FFmpeg/blob/n9.0/libavutil/hwcontext_vulkan.c)
- [Apple CoreVideo Metal texture cache](https://developer.apple.com/documentation/corevideo/cvmetaltexturecache)
- [Vulkan external memory handle types](https://docs.vulkan.org/refpages/latest/refpages/source/VkExternalMemoryHandleTypeFlagBits.html)
- [Vulkan external semaphore handle types](https://docs.vulkan.org/refpages/latest/refpages/source/VkExternalSemaphoreHandleTypeFlagBits.html)

The implementation order and platform designs are engineering recommendations
based on the repository and these APIs. Prototype results and target hardware
validation must establish actual platform support before release.
