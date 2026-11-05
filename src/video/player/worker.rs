//! Demuxing and audio decoding on an owned Rust thread with generation-tagged seeks

use std::{
    path::{Path, PathBuf},
    sync::{Arc, atomic::Ordering},
    thread::{self, JoinHandle},
    time::Instant,
};

use anyhow::{Result, ensure};

use super::{
    audio::{BYTES_PER_SECOND, CHANNELS, RATE},
    media::{AudioBlock, Media, Packet, PacketKind, Receive, Sent},
    shared::{BYTE_LIMIT, Shared},
};
use crate::video::native::Api;

pub(super) fn spawn(
    api: Arc<Api>,
    path: PathBuf,
    shared: Arc<Shared>,
) -> Result<JoinHandle<()>> {
    Ok(thread::Builder::new()
        .name("flowa-demux-audio".into())
        .spawn(move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                run(api, &path, &shared)
            }));
            match result {
                Ok(Err(error)) if !shared.stopped() => {
                    shared.fail(format!("{error:#}"))
                }
                Err(_) => shared.fail("The media worker panicked"),
                _ => {}
            }
        })?)
}

fn run(api: Arc<Api>, path: &Path, shared: &Shared) -> Result<()> {
    let mut media = Media::open(api, path, Arc::clone(&shared.interrupt))?;
    let stream = media.copy_stream()?;
    shared.lock().stream = Some(stream);
    shared.notify();

    while !shared.stopped() {
        if apply_seek(&mut media, shared)? {
            continue;
        }

        let generation = {
            let mut state = shared.lock();
            while !shared.stopped()
                && state.seek.is_none()
                && (state.eof || !state.packets.has_capacity())
            {
                state = shared.wait(state);
            }
            if shared.stopped() || state.seek.is_some() {
                continue;
            }
            state.generation
        };

        let packet = match media.read() {
            Ok(packet) => packet,
            Err(_)
                if shared.interrupt.seeking.load(Ordering::SeqCst)
                    || shared.stopped() =>
            {
                continue;
            }
            Err(error) => return Err(error),
        };
        let Some(packet) = packet else {
            finish_input(&mut media, shared, generation)?;
            continue;
        };

        match packet.kind {
            PacketKind::Video => {
                queue_video(packet, shared, generation)?;
            }
            PacketKind::Audio => {
                feed_audio(&mut media, Some(&packet), shared, generation)?;
            }
            PacketKind::Other => {}
        }
    }
    Ok(())
}

fn queue_video(packet: Packet, shared: &Shared, generation: u64) -> Result<()> {
    ensure!(
        packet.bytes <= BYTE_LIMIT,
        "A video packet exceeds the buffer limit"
    );
    let mut state = shared.lock();

    loop {
        let stale = generation != state.generation || state.seeking;
        if shared.stopped() || stale {
            return Ok(());
        }

        if state.packets.can_push(packet.bytes) {
            state.packets.push(packet)?;
            drop(state);
            shared.notify();
            return Ok(());
        }
        state = shared.wait(state);
    }
}

fn apply_seek(media: &mut Media, shared: &Shared) -> Result<bool> {
    let request = {
        let mut state = shared.lock();
        let Some(request) = state.seek.take() else {
            return Ok(false);
        };

        // Clearing the interrupt under this lock cannot erase a newer UI seek request
        shared.interrupt.seeking.store(false, Ordering::SeqCst);
        request
    };

    if let Err(error) = media.seek(request.target) {
        if shared.interrupt.seeking.load(Ordering::SeqCst) || shared.stopped() {
            return Ok(true);
        }
        return Err(error);
    }

    let mut state = shared.lock();
    if request.generation == state.generation {
        state.eof = false;
        state.flushed_generation = request.generation;
        state.seeking = false;
    }
    drop(state);
    shared.notify();
    Ok(true)
}

fn finish_input(media: &mut Media, shared: &Shared, generation: u64) -> Result<()> {
    if media.info.has_audio != 0 {
        feed_audio(media, None, shared, generation)?;
    }

    let mut state = shared.lock();
    if generation == state.generation && !state.seeking {
        if let Some(audio) = &state.audio {
            audio.flush()?;
        }
        state.eof = true;
    }
    drop(state);
    shared.notify();
    Ok(())
}

fn feed_audio(
    media: &mut Media,
    packet: Option<&Packet>,
    shared: &Shared,
    generation: u64,
) -> Result<()> {
    loop {
        if shared.stopped() || shared.lock().generation != generation {
            return Ok(());
        }

        match media.send_audio(packet)? {
            Sent::Again => {
                if !drain_audio(media, shared, generation)? {
                    return Ok(());
                }
            }
            Sent::Accepted | Sent::End => break,
        }
    }
    drain_audio(media, shared, generation)?;
    Ok(())
}

fn drain_audio(media: &mut Media, shared: &Shared, generation: u64) -> Result<bool> {
    while !shared.stopped() {
        match media.receive_audio()? {
            Receive::Item(block) => {
                if !queue_block(&block, media, shared, generation)? {
                    return Ok(false);
                }
            }
            Receive::Again | Receive::End => return Ok(true),
        }
    }
    Ok(false)
}

fn queue_block(
    block: &AudioBlock,
    media: &Media,
    shared: &Shared,
    generation: u64,
) -> Result<bool> {
    let (floor, mut previous) = {
        let mut state = shared.lock();
        if generation != state.generation {
            return Ok(false);
        }
        let previous = match state.clock.audio_end {
            Some(end) => end,
            None => state.position(Instant::now())?,
        };
        (state.clock.floor, previous)
    };
    let position = block.position(&media.info, previous);
    ensure!(position.is_finite(), "Invalid audio timestamp");
    let frames = block.samples().len() / CHANNELS;
    let skip = ((floor - position) * RATE as f64)
        .ceil()
        .clamp(0.0, frames as f64) as usize;
    let position = position + skip as f64 / RATE as f64;
    let samples = &block.samples()[skip * CHANNELS..];

    // Bound silence insertion while preserving files whose audio starts after video
    let silence = [0.0; 4096 * CHANNELS];
    while !samples.is_empty() && position - previous > 0.01 {
        let gap = ((position - previous) * RATE as f64).min(4096.0) as usize;
        if gap == 0 {
            break;
        }
        if !put_audio(shared, &silence[..gap * CHANNELS], previous, generation)? {
            return Ok(false);
        }
        previous += gap as f64 / RATE as f64;
    }

    put_audio(shared, samples, position, generation)
}

fn put_audio(
    shared: &Shared,
    samples: &[f32],
    mut position: f64,
    generation: u64,
) -> Result<bool> {
    for chunk in samples.chunks(4096 * CHANNELS) {
        let mut state = shared.lock();
        loop {
            if shared.stopped() || generation != state.generation {
                return Ok(false);
            }

            // Startup allows delayed video decoders to receive enough packets to produce a frame
            let limit = if state.started {
                BYTES_PER_SECOND / 3
            } else {
                BYTES_PER_SECOND * 2
            };
            if let Some(audio) = &state.audio
                && audio.queued_bytes()? <= limit
            {
                audio.put(chunk)?;
                break;
            }
            state = shared.wait(state);
        }

        position += (chunk.len() / CHANNELS) as f64 / RATE as f64;
        state.clock.audio_end = Some(position);
        drop(state);
        shared.notify();
    }
    Ok(!shared.stopped() && shared.lock().generation == generation)
}
