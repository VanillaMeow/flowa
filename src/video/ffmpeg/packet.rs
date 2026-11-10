//! Owned packet references transferred between the demuxer and codec workers

use std::ptr::NonNull;

use anyhow::{Context, Result};

use super::ffi;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(in crate::video) enum PacketKind {
    Video,
    Audio,
    Other,
}

pub(in crate::video) struct Packet {
    handle: NonNull<ffi::AVPacket>,
    pub kind: PacketKind,
    pub bytes: usize,
}

// SAFETY: Packets transfer exclusive ownership; clones retain atomic FFmpeg buffer references
unsafe impl Send for Packet {}

impl Packet {
    pub(super) fn allocate() -> Result<Self> {
        // SAFETY: The newly allocated packet has one owner and is freed by Drop
        let handle = NonNull::new(unsafe { ffi::av_packet_alloc() })
            .context("Cannot allocate a demux packet")?;

        Ok(Self {
            handle,
            kind: PacketKind::Other,
            bytes: 0,
        })
    }

    pub fn raw(&self) -> *mut ffi::AVPacket {
        self.handle.as_ptr()
    }

    pub fn try_clone(&self) -> Result<Self> {
        // SAFETY: This packet remains alive through cloning its backing references
        let handle = NonNull::new(unsafe { ffi::av_packet_clone(self.raw()) })
            .context("Cannot retain a hardware probe packet")?;

        Ok(Self {
            handle,
            kind: self.kind,
            bytes: self.bytes,
        })
    }
}

impl Drop for Packet {
    fn drop(&mut self) {
        let mut handle = self.raw();

        // SAFETY: This wrapper owns exactly one packet reference
        unsafe { ffi::av_packet_free(&mut handle) };
    }
}
