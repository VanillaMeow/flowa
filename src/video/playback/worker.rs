//! Demuxing and audio decoding on an owned Rust thread with generation-tagged seeks

use std::{
    path::{Path, PathBuf},
    sync::{Arc, atomic::Ordering},
    thread::{self, JoinHandle},
    time::Instant,
};

use anyhow::{Result, ensure};

use super::{queues::BYTE_LIMIT, state::Shared};
use crate::video::{
    audio::{AudioBlock, AudioDecoder, BYTES_PER_SECOND, CHANNELS, RATE, Tempo},
    demux::Media,
    ffmpeg::{Packet, PacketKind, Receive, Sent, StreamInfo},
};

/// Largest block of audio or inserted silence queued under one lock
const AUDIO_CHUNK_FRAMES: usize = 4096;

static SILENCE: [f32; AUDIO_CHUNK_FRAMES * CHANNELS] =
    [0.0; AUDIO_CHUNK_FRAMES * CHANNELS];

/// Stretches decoded audio for the playback rate and tracks how far input has reached
#[derive(Default)]
struct AudioFeed {
    generation: u64,
    epoch: u64,
    /// The rate the filter was built for; zero until the first block
    rate: f64,
    tempo: Option<Tempo>,
    /// Media time where the next input sample belongs, which the filter's output lags
    fed_end: Option<f64>,
}

impl AudioFeed {
    /// Starts over after a seek or rate change, with a filter for the new rate
    fn sync(&mut self, generation: u64, epoch: u64, rate: f64) -> Result<()> {
        if self.generation == generation && self.epoch == epoch && self.rate == rate {
            return Ok(());
        }

        self.generation = generation;
        self.epoch = epoch;
        self.rate = rate;
        self.fed_end = None;
        self.tempo = None;

        if (rate - 1.0).abs() > f64::EPSILON {
            self.tempo = Some(Tempo::new(rate)?);
        }

        Ok(())
    }

    /// Forces the next block to rebuild the filter, as after its input ended
    fn invalidate(&mut self) {
        self.rate = 0.0;
        self.tempo = None;
    }
}

/// Where the first output sample of a write belongs in media time
#[derive(Clone, Copy)]
enum Start {
    /// Exactly here, as for audio that is not stretched
    At(f64),
    /// Right after the audio already queued, or here when nothing is
    After(f64),
}

pub(super) fn spawn(path: PathBuf, shared: Arc<Shared>) -> Result<JoinHandle<()>> {
    Ok(thread::Builder::new()
        .name("flowa-demux-audio".into())
        .spawn(move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                run(&path, &shared)
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

fn run(path: &Path, shared: &Shared) -> Result<()> {
    let mut media = Media::open(path, Arc::clone(&shared.interrupt))?;
    let mut audio = media.audio_stream().map(AudioDecoder::open).transpose()?;
    let mut feed = AudioFeed::default();
    let stream = media.copy_stream()?;
    shared.lock().stream = Some(stream);
    shared.notify();

    while !shared.stopped() {
        if apply_seek(&mut media, audio.as_mut(), shared)? {
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
            finish_input(audio.as_mut(), &mut feed, &media.info, shared, generation)?;
            continue;
        };

        match packet.kind {
            PacketKind::Video => {
                queue_video(packet, shared, generation)?;
            }
            PacketKind::Audio if !shared.lock().audio_disabled => {
                if let Some(audio) = &mut audio {
                    let input = (&media.info, Some(&packet));
                    feed_audio(audio, &mut feed, input, shared, generation)?;
                }
            }
            PacketKind::Audio | PacketKind::Other => {}
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

fn apply_seek(
    media: &mut Media,
    audio: Option<&mut AudioDecoder>,
    shared: &Shared,
) -> Result<bool> {
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

    if let Some(audio) = audio {
        audio.flush()?;
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

fn finish_input(
    audio: Option<&mut AudioDecoder>,
    feed: &mut AudioFeed,
    info: &StreamInfo,
    shared: &Shared,
    generation: u64,
) -> Result<()> {
    if let Some(audio) = audio {
        feed_audio(audio, feed, (info, None), shared, generation)?;
        finish_stretching(feed, shared, generation)?;
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

/// Releases what the tempo filter still holds once the audio stream has ended
fn finish_stretching(
    feed: &mut AudioFeed,
    shared: &Shared,
    generation: u64,
) -> Result<()> {
    let (Some(tempo), Some(fed_end)) = (feed.tempo.as_mut(), feed.fed_end) else {
        return Ok(());
    };
    let mut output = Vec::new();
    tempo.finish(&mut output)?;

    let (epoch, rate) = (feed.epoch, feed.rate);
    feed.invalidate();
    put_audio(
        shared,
        &output,
        Start::After(fed_end),
        rate,
        (generation, epoch),
    )?;

    Ok(())
}

fn feed_audio(
    audio: &mut AudioDecoder,
    feed: &mut AudioFeed,
    (info, packet): (&StreamInfo, Option<&Packet>),
    shared: &Shared,
    generation: u64,
) -> Result<()> {
    loop {
        if shared.stopped() || shared.lock().generation != generation {
            return Ok(());
        }

        match audio.send(packet)? {
            Sent::Again => {
                if !drain_audio(audio, feed, info, shared, generation)? {
                    return Ok(());
                }
            }
            Sent::Accepted | Sent::End => break,
        }
    }
    drain_audio(audio, feed, info, shared, generation)?;
    Ok(())
}

fn drain_audio(
    audio: &mut AudioDecoder,
    feed: &mut AudioFeed,
    info: &StreamInfo,
    shared: &Shared,
    generation: u64,
) -> Result<bool> {
    while !shared.stopped() {
        match audio.receive()? {
            Receive::Item(block) => {
                if !queue_block(&block, feed, info, shared, generation)? {
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
    feed: &mut AudioFeed,
    info: &StreamInfo,
    shared: &Shared,
    generation: u64,
) -> Result<bool> {
    let (floor, queued_end, epoch, rate) = {
        let mut state = shared.lock();
        if generation != state.generation {
            return Ok(false);
        }
        let queued_end = match state.clock.audio_end {
            Some(end) => end,
            None => state.position(Instant::now())?,
        };
        (
            state.clock.floor,
            queued_end,
            state.rate_epoch,
            state.clock.rate(),
        )
    };
    feed.sync(generation, epoch, rate)?;

    let mut previous = feed.fed_end.unwrap_or(queued_end);
    let position = block.position(info, previous);
    ensure!(position.is_finite(), "Invalid audio timestamp");
    let frames = block.samples().len() / CHANNELS;
    let skip = ((floor - position) * RATE as f64)
        .ceil()
        .clamp(0.0, frames as f64) as usize;
    let position = position + skip as f64 / RATE as f64;
    let samples = &block.samples()[skip * CHANNELS..];

    // Bound silence insertion while preserving files whose audio starts after video
    while !samples.is_empty() && position - previous > 0.01 {
        let gap = ((position - previous) * RATE as f64).min(AUDIO_CHUNK_FRAMES as f64)
            as usize;
        if gap == 0 {
            break;
        }
        let silence = &SILENCE[..gap * CHANNELS];
        if !stretch_audio(shared, feed, silence, previous, (generation, epoch))? {
            return Ok(false);
        }
        previous += gap as f64 / RATE as f64;
        feed.fed_end = Some(previous);
    }

    if !stretch_audio(shared, feed, samples, position, (generation, epoch))? {
        return Ok(false);
    }
    feed.fed_end = Some(position + (samples.len() / CHANNELS) as f64 / RATE as f64);

    Ok(true)
}

/// Queues samples that start at `position`, stretched first when the rate is not one
fn stretch_audio(
    shared: &Shared,
    feed: &mut AudioFeed,
    samples: &[f32],
    position: f64,
    tags: (u64, u64),
) -> Result<bool> {
    let Some(tempo) = &mut feed.tempo else {
        return put_audio(shared, samples, Start::At(position), 1.0, tags);
    };

    let mut output = Vec::new();
    tempo.push(samples, &mut output)?;

    put_audio(shared, &output, Start::After(position), feed.rate, tags)
}

/// Queues output samples, each covering `rate` frames of media time
///
/// Returns false once a seek or rate change has made them stale
fn put_audio(
    shared: &Shared,
    samples: &[f32],
    start: Start,
    rate: f64,
    (generation, epoch): (u64, u64),
) -> Result<bool> {
    let mut position = None;

    for chunk in samples.chunks(AUDIO_CHUNK_FRAMES * CHANNELS) {
        let mut state = shared.lock();
        loop {
            let stale = generation != state.generation || epoch != state.rate_epoch;
            if shared.stopped() || stale {
                return Ok(false);
            }

            // Without an output device the clock runs from wall time, so samples are dropped
            // Stepped frames drop them too, so a full queue never stops video packets
            if state.audio_disabled || state.holding_audio {
                return Ok(true);
            }

            // Startup allows delayed video decoders to receive enough packets to produce a frame
            let seconds = if state.started { 1. / 3. } else { 2. };

            // Slow rates keep as much media queued as normal speed, so video reads ahead
            let stretch = (1. / state.clock.rate()).max(1.);
            let limit = (BYTES_PER_SECOND as f64 * seconds * stretch) as usize;
            if let Some(audio) = &state.audio
                && audio.queued_bytes()? <= limit
            {
                audio.put(chunk)?;
                break;
            }
            state = shared.wait(state);
        }

        let begin = *position.get_or_insert(match start {
            Start::At(position) => position,
            Start::After(position) => state.clock.audio_end.unwrap_or(position),
        });
        let end = begin + (chunk.len() / CHANNELS) as f64 * rate / RATE as f64;
        position = Some(end);
        state.clock.audio_end = Some(end);
        drop(state);
        shared.notify();
    }

    let state = shared.lock();
    Ok(
        !shared.stopped()
            && state.generation == generation
            && state.rate_epoch == epoch,
    )
}
