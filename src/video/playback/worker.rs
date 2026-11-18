//! Demuxing and audio decoding on an owned Rust thread with generation-tagged seeks

use std::{
    path::{Path, PathBuf},
    sync::{Arc, atomic::Ordering},
    thread::{self, JoinHandle},
    time::Instant,
};

use anyhow::{Context, Result, ensure};

use super::{queues::BYTE_LIMIT, state::Shared};
use crate::video::{
    audio::{AudioBlock, AudioDecoder, CHANNELS, RATE},
    demux::Media,
    ffmpeg::{Packet, PacketKind, Receive, Sent, StreamInfo},
    subtitles::{SubtitleDecoder, SubtitleEvent, load_external},
};

/// Media seconds of audio decoded ahead of playback, which also bounds how far video
/// packets are read ahead, since both come from one interleaved file
const AUDIO_LOOKAHEAD: f64 = 1.;
/// How far audio may run ahead while the video decoder has no packets, as decoders
/// with frame threads hold back several frames before releasing one
const HUNGRY_LOOKAHEAD: f64 = 5.;
/// Startup reads further so delayed video decoders receive enough packets for a frame
const STARTUP_LOOKAHEAD: f64 = 2.;

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
    let mut subtitles = None;
    let stream = media.copy_stream()?;
    {
        let mut state = shared.lock();
        state.stream = Some(stream);
        state.tracks = media.tracks.clone();
        state.audio_track = media.audio_index();
        state.audio_info = media.info.audio;
    }
    shared.notify();
    prepare_subtitles(path, &mut media, &mut subtitles, shared);

    while !shared.stopped() {
        if apply_seek(&mut media, &mut audio, &mut subtitles, shared)? {
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
            finish_input(audio.as_mut(), &media.info, shared, generation)?;
            continue;
        };

        match packet.kind {
            PacketKind::Video => {
                queue_video(packet, shared, generation)?;
            }
            PacketKind::Audio if !shared.lock().audio_disabled => {
                if let Some(audio) = &mut audio {
                    let input = (&media.info, Some(&packet));
                    feed_audio(audio, input, shared, generation)?;
                }
            }
            PacketKind::Subtitle => {
                if let Some(decoder) = &mut subtitles {
                    show_subtitles(decoder, &packet, media.subtitle_index(), shared);
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
    audio: &mut Option<AudioDecoder>,
    subtitles: &mut Option<SubtitleDecoder>,
    shared: &Shared,
) -> Result<bool> {
    let (request, track, subtitle_track) = {
        let mut state = shared.lock();
        let Some(request) = state.seek.take() else {
            return Ok(false);
        };

        // Clearing the interrupt under this lock cannot erase a newer UI seek request
        shared.interrupt.seeking.store(false, Ordering::SeqCst);
        state.subtitles.prune(request.target);
        (request, state.audio_track, state.subtitle_track)
    };

    if let Err(error) = media.seek(request.target) {
        if shared.interrupt.seeking.load(Ordering::SeqCst) || shared.stopped() {
            return Ok(true);
        }
        return Err(error);
    }

    if let Some(track) = track.filter(|&track| Some(track) != media.audio_index()) {
        switch_audio(media, audio, track, shared);
    } else if let Some(audio) = audio {
        audio.flush()?;
    }

    switch_subtitles(media, subtitles, subtitle_track);

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

/// Reads subtitle files beside the video and picks the subtitles to show first
fn prepare_subtitles(
    path: &Path,
    media: &mut Media,
    decoder: &mut Option<SubtitleDecoder>,
    shared: &Shared,
) {
    let interrupt = Arc::clone(&shared.interrupt);
    let cancel = move || interrupt.abort.load(Ordering::SeqCst);
    let external =
        load_external(path, media.stream_count(), media.video_size(), cancel);

    let choice = {
        let mut state = shared.lock();
        state
            .tracks
            .subtitles
            .extend(external.iter().map(|subtitles| subtitles.track.clone()));
        state.external_subtitles = external;

        let choice = state
            .tracks
            .preferred_subtitles()
            .filter(|_| state.subtitles_wanted);
        let from_file = state
            .external_subtitles
            .iter()
            .find(|subtitles| Some(subtitles.track.index) == choice)
            .map(|subtitles| subtitles.cues.clone());

        state.subtitle_track = choice;
        state.subtitles = from_file.unwrap_or_default();
        choice
    };
    shared.notify();

    switch_subtitles(media, decoder, choice);
}

/// Decodes the chosen embedded subtitle stream, or none for files and no subtitles
fn switch_subtitles(
    media: &mut Media,
    decoder: &mut Option<SubtitleDecoder>,
    track: Option<usize>,
) {
    let embedded = track.filter(|&index| {
        media
            .tracks
            .subtitles
            .iter()
            .any(|track| track.index == index && !track.external)
    });

    if embedded == media.subtitle_index() {
        if let Some(decoder) = decoder {
            decoder.flush();
        }
        return;
    }

    let opened = media.select_subtitles(embedded).and_then(|()| {
        media
            .subtitle_stream()
            .map(|stream| {
                SubtitleDecoder::open(stream, media.info.origin, media.video_size())
            })
            .transpose()
    });

    *decoder = opened.unwrap_or_else(|error| {
        log::warn!("Cannot show subtitle track {embedded:?}: {error:#}");
        let _ = media.select_subtitles(None);
        None
    });
}

/// Adds a packet's cues to the track being shown; a broken packet only loses its cues
fn show_subtitles(
    decoder: &mut SubtitleDecoder,
    packet: &Packet,
    index: Option<usize>,
    shared: &Shared,
) {
    let event = match decoder.decode(packet) {
        Ok(Some(event)) => event,
        Ok(None) => return,
        Err(error) => {
            log::debug!("Skipping a subtitle packet: {error:#}");
            return;
        }
    };

    let mut state = shared.lock();
    if state.subtitle_track != index {
        return;
    }

    match event {
        SubtitleEvent::Cues(cues) => {
            cues.into_iter().for_each(|cue| state.subtitles.insert(cue))
        }
        SubtitleEvent::Clear(time) => state.subtitles.clear_at(time),
    }
    drop(state);
    shared.notify();
}

/// Decodes another audio stream, or plays silently when its decoder cannot open
fn switch_audio(
    media: &mut Media,
    audio: &mut Option<AudioDecoder>,
    track: usize,
    shared: &Shared,
) {
    let opened = media.select_audio(track).and_then(|()| {
        let stream = media
            .audio_stream()
            .context("The audio track disappeared")?;
        AudioDecoder::open(stream)
    });

    *audio = match opened {
        Ok(decoder) => Some(decoder),
        Err(error) => {
            log::warn!("Cannot play audio track {track}: {error:#}");
            None
        }
    };

    shared.lock().audio_info = audio.as_ref().and(media.info.audio);
}

fn finish_input(
    audio: Option<&mut AudioDecoder>,
    info: &StreamInfo,
    shared: &Shared,
    generation: u64,
) -> Result<()> {
    if let Some(audio) = audio {
        feed_audio(audio, (info, None), shared, generation)?;
    }

    let mut state = shared.lock();
    if generation == state.generation && !state.seeking {
        state.eof = true;
    }
    drop(state);
    shared.notify();
    Ok(())
}

fn feed_audio(
    audio: &mut AudioDecoder,
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
                if !drain_audio(audio, info, shared, generation)? {
                    return Ok(());
                }
            }
            Sent::Accepted | Sent::End => break,
        }
    }
    drain_audio(audio, info, shared, generation)?;
    Ok(())
}

fn drain_audio(
    audio: &mut AudioDecoder,
    info: &StreamInfo,
    shared: &Shared,
    generation: u64,
) -> Result<bool> {
    while !shared.stopped() {
        match audio.receive()? {
            Receive::Item(block) => {
                if !queue_block(&block, info, shared, generation)? {
                    return Ok(false);
                }
            }
            Receive::Again | Receive::End => return Ok(true),
        }
    }
    Ok(false)
}

/// Adds a decoded block to the feed once there is room, skipping samples before the
/// seek floor
///
/// Returns false once a seek has made the block stale
fn queue_block(
    block: &AudioBlock,
    info: &StreamInfo,
    shared: &Shared,
    generation: u64,
) -> Result<bool> {
    let mut state = shared.lock();

    loop {
        if shared.stopped() || generation != state.generation {
            return Ok(false);
        }

        // Without an output device the clock runs from wall time, so samples are dropped
        if state.audio_disabled {
            return Ok(true);
        }

        let limit = if !state.started {
            STARTUP_LOOKAHEAD
        } else if state.packets.is_empty() {
            HUNGRY_LOOKAHEAD
        } else {
            AUDIO_LOOKAHEAD
        };

        if state.feed.ahead() <= limit {
            break;
        }
        state = shared.wait(state);
    }

    let previous = match state.feed.end() {
        Some(end) => end,
        None => state.position(Instant::now())?,
    };
    let position = block.position(info, previous);
    ensure!(position.is_finite(), "Invalid audio timestamp");

    let frames = block.samples().len() / CHANNELS;
    let skip = ((state.clock.floor - position) * RATE as f64)
        .ceil()
        .clamp(0.0, frames as f64) as usize;
    let samples = &block.samples()[skip * CHANNELS..];

    if !samples.is_empty() {
        state
            .feed
            .push(samples, position + skip as f64 / RATE as f64);
    }
    drop(state);
    shared.notify();

    Ok(true)
}
