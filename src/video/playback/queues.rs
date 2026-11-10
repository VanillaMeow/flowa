//! Bounded queues of demuxed video packets waiting for the decoder

use std::collections::VecDeque;

use anyhow::{Result, ensure};

use crate::{utils::units::MIB, video::ffmpeg::Packet};

pub(super) const PACKET_LIMIT: usize = 96;
pub(super) const BYTE_LIMIT: usize = 32 * MIB as usize;

#[derive(Default)]
pub(super) struct PacketQueue {
    packets: VecDeque<Packet>,
    bytes: usize,
}

impl PacketQueue {
    pub fn has_capacity(&self) -> bool {
        self.packets.len() < PACKET_LIMIT && self.bytes < BYTE_LIMIT
    }

    pub fn can_push(&self, bytes: usize) -> bool {
        self.packets.len() < PACKET_LIMIT
            && bytes <= BYTE_LIMIT.saturating_sub(self.bytes)
    }

    pub fn push(&mut self, packet: Packet) -> Result<()> {
        ensure!(
            self.can_push(packet.bytes),
            "Video packet buffer limit exceeded"
        );
        self.bytes += packet.bytes;
        self.packets.push_back(packet);
        Ok(())
    }

    pub fn pop(&mut self) -> Option<Packet> {
        let packet = self.packets.pop_front()?;
        self.bytes -= packet.bytes;
        Some(packet)
    }

    pub fn clear(&mut self) {
        self.packets.clear();
        self.bytes = 0;
    }
}
