//! Owned FFmpeg handles with exclusive demuxer access and immutable stream snapshots

use std::{
    ffi::{CString, c_void},
    os::unix::ffi::OsStrExt,
    path::Path,
    ptr::{self, NonNull},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

use anyhow::{Context, Result, ensure};

use crate::video::{
    gpu::NativeGpu,
    native::{self, Api, Handle, StreamInfo},
};

#[derive(Default)]
pub(super) struct InterruptFlags {
    pub abort: AtomicBool,
    pub seeking: AtomicBool,
}

unsafe extern "C" fn interrupted(data: *mut c_void) -> i32 {
    // SAFETY: Media retains this allocation until avformat_close_input has returned
    let flags = unsafe { &*data.cast::<InterruptFlags>() };
    i32::from(
        flags.abort.load(Ordering::SeqCst) || flags.seeking.load(Ordering::SeqCst),
    )
}

pub(super) enum Receive<T> {
    Again,
    Item(T),
    End,
}

pub(super) enum Sent {
    Accepted,
    Again,
    End,
}

impl Sent {
    fn from_code(code: i32) -> Self {
        match code {
            1 => Self::Again,
            2 => Self::End,
            _ => Self::Accepted,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum PacketKind {
    Video,
    Audio,
    Other,
}

/// Owns a demuxer and audio decoder, used exclusively on the media worker
pub(super) struct Media {
    api: Arc<Api>,
    handle: NonNull<c_void>,
    pub info: StreamInfo,
    _interrupt: Arc<InterruptFlags>,
}

impl Media {
    pub fn open(
        api: Arc<Api>,
        path: &Path,
        interrupt: Arc<InterruptFlags>,
    ) -> Result<Self> {
        let path = CString::new(path.as_os_str().as_bytes())?;
        let mut info = std::mem::MaybeUninit::uninit();

        // SAFETY: The path lasts through open and the interrupt allocation is retained below
        let handle = native::create(|error, size| unsafe {
            (api.media_open)(
                path.as_ptr(),
                Some(interrupted),
                Arc::as_ptr(&interrupt).cast_mut().cast(),
                info.as_mut_ptr(),
                error,
                size,
            )
        })?;

        Ok(Self {
            api,
            handle,
            // SAFETY: A successful open initializes every stream-info field
            info: unsafe { info.assume_init() },
            _interrupt: interrupt,
        })
    }

    pub fn copy_stream(&self) -> Result<Arc<Stream>> {
        // SAFETY: The worker exclusively owns the source; the shim makes an independent copy
        let handle =
            NonNull::new(unsafe { (self.api.media_copy_stream)(self.handle.as_ptr()) })
                .context("Cannot copy the video stream parameters")?;

        Ok(Arc::new(Stream {
            api: Arc::clone(&self.api),
            handle,
            info: self.info,
        }))
    }

    pub fn read(&mut self) -> Result<Option<Packet>> {
        let mut handle = ptr::null_mut();
        let mut kind = 0;
        let mut bytes = 0;

        // SAFETY: Output storage is writable and this media handle has one worker owner
        let code = native::check(|error, size| unsafe {
            (self.api.media_read)(
                self.handle.as_ptr(),
                &mut handle,
                &mut kind,
                &mut bytes,
                error,
                size,
            )
        })?;
        if code == 2 {
            return Ok(None);
        }

        let mut packet = Packet {
            api: Arc::clone(&self.api),
            handle: NonNull::new(handle)
                .context("FFmpeg returned an empty packet handle")?,
            kind: match kind {
                1 => PacketKind::Video,
                2 => PacketKind::Audio,
                _ => PacketKind::Other,
            },
            bytes: 0,
        };
        packet.bytes = usize::try_from(bytes).context("Invalid packet size")?;
        Ok(Some(packet))
    }

    pub fn seek(&mut self, seconds: f64) -> Result<()> {
        let timestamp = ((seconds + self.info.origin) * 1_000_000.0).round() as i64;

        // SAFETY: Seeking and audio flushing stay on the exclusive demux worker
        native::check(|error, size| unsafe {
            (self.api.media_seek)(self.handle.as_ptr(), timestamp, error, size)
        })?;
        Ok(())
    }

    pub fn send_audio(&mut self, packet: Option<&Packet>) -> Result<Sent> {
        // SAFETY: A packet is borrowed through the call; NULL drains the decoder at EOF
        let code = native::check(|error, size| unsafe {
            (self.api.media_audio_send)(
                self.handle.as_ptr(),
                packet.map_or(ptr::null_mut(), Packet::raw),
                error,
                size,
            )
        })?;
        Ok(Sent::from_code(code))
    }

    pub fn receive_audio(&mut self) -> Result<Receive<AudioBlock>> {
        let mut block = std::mem::MaybeUninit::uninit();

        // SAFETY: Successful receives initialize the block and transfer its PCM allocation
        let code = native::check(|error, size| unsafe {
            (self.api.media_audio_receive)(
                self.handle.as_ptr(),
                block.as_mut_ptr(),
                error,
                size,
            )
        })?;
        match code {
            0 => Ok(Receive::Again),
            2 => Ok(Receive::End),
            _ => {
                let block = AudioBlock {
                    api: Arc::clone(&self.api),
                    raw: unsafe { block.assume_init() },
                };
                ensure!(
                    (0..=480_000).contains(&block.raw.frames),
                    "Invalid resampled audio size"
                );
                ensure!(
                    block.raw.frames == 0 || !block.raw.samples.is_null(),
                    "Empty audio allocation"
                );
                Ok(Receive::Item(block))
            }
        }
    }
}

impl Drop for Media {
    fn drop(&mut self) {
        // SAFETY: The worker still owns the context and the interrupt allocation remains alive
        unsafe { (self.api.media_close)(self.handle.as_ptr()) };
    }
}

pub(super) struct Stream {
    api: Arc<Api>,
    handle: NonNull<c_void>,
    pub info: StreamInfo,
}

// SAFETY: The independent snapshot is immutable and no I/O ever uses its context
unsafe impl Send for Stream {}
unsafe impl Sync for Stream {}

impl Stream {
    pub fn raw(&self) -> Handle {
        self.handle.as_ptr()
    }
}

impl Drop for Stream {
    fn drop(&mut self) {
        // SAFETY: Arc ownership keeps the immutable snapshot alive for every reader
        unsafe { (self.api.stream_free)(self.handle.as_ptr()) };
    }
}

pub(super) struct Packet {
    api: Arc<Api>,
    handle: NonNull<c_void>,
    pub kind: PacketKind,
    pub bytes: usize,
}

// SAFETY: Packets transfer ownership through a mutex; references use FFmpeg's atomic buffer counts
unsafe impl Send for Packet {}

impl Packet {
    pub fn raw(&self) -> Handle {
        self.handle.as_ptr()
    }

    pub fn try_clone(&self) -> Result<Self> {
        // SAFETY: Cloning retains the packet's backing buffers through FFmpeg
        let handle = NonNull::new(unsafe { (self.api.packet_clone)(self.raw()) })
            .context("Cannot retain a hardware probe packet")?;
        Ok(Self {
            api: Arc::clone(&self.api),
            handle,
            kind: self.kind,
            bytes: self.bytes,
        })
    }
}

impl Drop for Packet {
    fn drop(&mut self) {
        // SAFETY: This wrapper owns one packet reference
        unsafe { (self.api.packet_free)(self.raw()) };
    }
}

/// Owns one hardware codec on the UI thread where Vulkan submissions are serialized
pub(super) struct HardwareCodec {
    api: Arc<Api>,
    handle: NonNull<c_void>,
    gpu: Arc<NativeGpu>,
}

impl HardwareCodec {
    pub fn new(gpu: &Arc<NativeGpu>, stream: &Stream, backend: i32) -> Result<Self> {
        // SAFETY: Both borrowed contexts outlive creation; the codec retains its device reference
        let handle = native::create(|error, size| unsafe {
            (gpu.api.decoder_create)(
                gpu.handle.as_ptr(),
                stream.raw(),
                backend,
                error,
                size,
            )
        })?;
        Ok(Self {
            api: Arc::clone(&gpu.api),
            handle,
            gpu: Arc::clone(gpu),
        })
    }

    pub fn flush(&mut self) {
        // SAFETY: Only the owning UI thread touches this codec
        unsafe { (self.api.decoder_flush)(self.handle.as_ptr()) };
    }

    pub fn send(&mut self, packet: Option<&Packet>) -> Result<Sent> {
        // SAFETY: The codec borrows the packet through the call; NULL requests draining
        let code = native::check(|error, size| unsafe {
            (self.api.decoder_send)(
                self.handle.as_ptr(),
                packet.map_or(ptr::null_mut(), Packet::raw),
                error,
                size,
            )
        })?;
        Ok(Sent::from_code(code))
    }

    pub fn receive(&mut self) -> Result<Receive<Frame>> {
        let mut handle = ptr::null_mut();

        // SAFETY: The shim only returns frames with the selected hardware format
        let code = native::check(|error, size| unsafe {
            (self.api.decoder_receive)(self.handle.as_ptr(), &mut handle, error, size)
        })?;
        match code {
            0 => Ok(Receive::Again),
            2 => Ok(Receive::End),
            _ => {
                let handle = NonNull::new(handle)
                    .context("FFmpeg returned an empty video frame")?;
                let mut info = std::mem::MaybeUninit::uninit();
                unsafe {
                    (self.api.frame_get_info)(handle.as_ptr(), info.as_mut_ptr())
                };
                Ok(Receive::Item(Frame {
                    api: Arc::clone(&self.api),
                    handle,
                    info: unsafe { info.assume_init() },
                    _gpu: Arc::clone(&self.gpu),
                }))
            }
        }
    }
}

impl Drop for HardwareCodec {
    fn drop(&mut self) {
        // SAFETY: The engine stops GPU access before releasing the decoder and its frame pools
        unsafe { (self.api.decoder_free)(self.handle.as_ptr()) };
    }
}

pub(super) struct Frame {
    api: Arc<Api>,
    handle: NonNull<c_void>,
    pub info: native::FrameInfo,
    // FFmpeg borrows the Vulkan device, so each frame also retains its Rust owner
    _gpu: Arc<NativeGpu>,
}

impl Frame {
    pub fn raw(&self) -> Handle {
        self.handle.as_ptr()
    }

    pub fn display_size(&self, stream_rotation: f64) -> [u32; 2] {
        let aspect = if self.info.aspect_num > 0 && self.info.aspect_den > 0 {
            f64::from(self.info.aspect_num) / f64::from(self.info.aspect_den)
        } else {
            1.0
        };
        let mut size = [
            (f64::from(self.info.width.max(0)) * aspect).round() as u32,
            self.info.height.max(0) as u32,
        ];
        let rotation = if self.info.rotation.is_finite() {
            self.info.rotation
        } else {
            stream_rotation
        };

        let quarter_turns = (rotation.abs() / 90.0).round() as u32;
        if rotation.is_finite() && !quarter_turns.is_multiple_of(2) {
            size.swap(0, 1);
        }
        size
    }
}

impl Drop for Frame {
    fn drop(&mut self) {
        // SAFETY: GPU submissions retain independent references until their timeline completes
        unsafe { (self.api.frame_free)(self.raw()) };
    }
}

pub(super) struct AudioBlock {
    api: Arc<Api>,
    raw: native::AudioBlock,
}

impl AudioBlock {
    pub fn samples(&self) -> &[f32] {
        if self.raw.frames == 0 {
            return &[];
        }

        // SAFETY: Receive validated the sample count and this wrapper owns the PCM allocation
        unsafe {
            std::slice::from_raw_parts(self.raw.samples, self.raw.frames as usize * 2)
        }
    }

    pub fn position(&self, info: &StreamInfo, previous: f64) -> f64 {
        timestamp(self.raw.timestamp, info.audio_time_base, info.origin)
            .unwrap_or(previous)
            - self.raw.resample_delay
    }
}

impl Drop for AudioBlock {
    fn drop(&mut self) {
        // SAFETY: The allocation came from the matching FFmpeg instance retained by this API
        unsafe { (self.api.audio_block_free)(self.raw.samples) };
    }
}

pub(super) fn timestamp(value: i64, time_base: f64, origin: f64) -> Option<f64> {
    (value != i64::MIN).then_some(value as f64 * time_base - origin)
}
