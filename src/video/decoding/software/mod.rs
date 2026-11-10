//! Software decoding on a CPU thread, without codec work on the playback thread

use std::{
    sync::{Arc, mpsc},
    thread::{self, JoinHandle},
};

use anyhow::{Context, Result, bail};

use super::{DecodedFrame, Decoder};
use crate::video::ffmpeg::{Packet, Receive, Sent, Stream};

mod codec;
mod worker;

pub(in crate::video) use codec::{SoftwareCodec, Threads};
use worker::{Request, Response};

/// Decodes on a CPU thread and hands frames over through bounded queues
pub(super) struct SoftwareDecoder {
    requests: Option<mpsc::SyncSender<Request>>,
    results: Option<mpsc::Receiver<Response>>,
    worker: Option<JoinHandle<()>>,
    generation: u64,
}

impl SoftwareDecoder {
    pub fn new(stream: Arc<Stream>) -> Result<Self> {
        let (requests, requests_rx) = mpsc::sync_channel::<Request>(4);
        let (results_tx, results) = mpsc::sync_channel(2);
        let worker = thread::Builder::new()
            .name("flowa-software-video".into())
            .spawn(move || worker::run(stream, requests_rx, results_tx))?;

        Ok(Self {
            requests: Some(requests),
            results: Some(results),
            worker: Some(worker),
            generation: 0,
        })
    }
}

impl Decoder for SoftwareDecoder {
    fn flush(&mut self) {
        self.generation += 1;
    }

    fn send(&mut self, packet: Option<&Packet>) -> Result<Sent> {
        let request = Request {
            generation: self.generation,
            packet: packet.map(Packet::try_clone).transpose()?,
        };

        match self
            .requests
            .as_ref()
            .context("Software decoder is closed")?
            .try_send(request)
        {
            Ok(()) => Ok(Sent::Accepted),
            Err(mpsc::TrySendError::Full(_)) => Ok(Sent::Again),
            Err(mpsc::TrySendError::Disconnected(_)) => {
                bail!("The software decoder stopped")
            }
        }
    }

    fn receive(&mut self) -> Result<Receive<DecodedFrame>> {
        let results = self
            .results
            .as_ref()
            .context("Software decoder is closed")?;

        loop {
            let response = match results.try_recv() {
                Ok(response) => response,
                Err(mpsc::TryRecvError::Empty) => return Ok(Receive::Again),
                Err(mpsc::TryRecvError::Disconnected) => {
                    bail!("The software decoder stopped")
                }
            };

            if response.generation != self.generation {
                continue;
            }

            return match response.frame? {
                Receive::Item(frame) => Ok(Receive::Item(DecodedFrame::Cpu(frame))),
                Receive::End => Ok(Receive::End),
                Receive::Again => Ok(Receive::Again),
            };
        }
    }
}

impl Drop for SoftwareDecoder {
    fn drop(&mut self) {
        // Closing both channels releases a worker blocked on either input or output
        self.results.take();
        self.requests.take();

        if let Some(worker) = self.worker.take()
            && worker.join().is_err()
        {
            log::warn!("The software video worker panicked");
        }
    }
}
