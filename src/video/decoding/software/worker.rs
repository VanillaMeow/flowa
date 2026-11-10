//! The software decoder's thread, which tags its output with the generation it decoded

use std::sync::{Arc, mpsc};

use anyhow::Result;

use super::codec::{SoftwareCodec, Threads};
use crate::video::{
    decoding::frame::CpuFrame,
    ffmpeg::{Packet, Receive, Stream},
};

/// A packet to decode, or `None` to drain, from the given flush generation
pub(super) struct Request {
    pub generation: u64,
    pub packet: Option<Packet>,
}

/// A decoded frame or error, from the generation whose packets produced it
pub(super) struct Response {
    pub generation: u64,
    pub frame: Result<Receive<CpuFrame>>,
}

/// Decodes requests until either channel closes, flushing when the generation changes
pub(super) fn run(
    stream: Arc<Stream>,
    requests: mpsc::Receiver<Request>,
    results: mpsc::SyncSender<Response>,
) {
    let mut codec = match SoftwareCodec::open(stream.data(), Threads::Automatic) {
        Ok(codec) => codec,
        Err(error) => {
            let _ = results.send(Response {
                generation: 0,
                frame: Err(error),
            });
            return;
        }
    };
    let mut generation = 0;

    while let Ok(request) = requests.recv() {
        if request.generation != generation {
            codec.flush();
            generation = request.generation;
        }

        let result = codec.decode(request.packet.as_ref(), |frame| {
            results
                .send(Response {
                    generation,
                    frame: Ok(frame),
                })
                .is_ok()
        });

        match result {
            Ok(true) => {}
            Ok(false) => return,
            Err(error) => {
                let _ = results.send(Response {
                    generation,
                    frame: Err(error),
                });
                return;
            }
        }
    }
}
