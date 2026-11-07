//! Bounded software decoding without codec work on the UI thread

use std::{
    sync::{Arc, mpsc},
    thread::{self, JoinHandle},
};

use anyhow::{Context, Result, bail, ensure};

use super::frame::Frame;
use crate::video::media::{Codec, CpuFrame, Packet, Receive, Sent, Stream};

struct Request {
    generation: u64,
    packet: Option<Packet>,
}

struct Response {
    generation: u64,
    frame: Result<Receive<CpuFrame>>,
}

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
            .spawn(move || run(stream, requests_rx, results_tx))?;

        Ok(Self {
            requests: Some(requests),
            results: Some(results),
            worker: Some(worker),
            generation: 0,
        })
    }

    pub fn flush(&mut self) {
        self.generation += 1;
    }

    pub fn send(&mut self, packet: Option<&Packet>) -> Result<Sent> {
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

    pub fn receive(&mut self) -> Result<Receive<Frame>> {
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
                Receive::Item(frame) => Ok(Receive::Item(Frame::software(frame))),
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

fn run(
    stream: Arc<Stream>,
    requests: mpsc::Receiver<Request>,
    results: mpsc::SyncSender<Response>,
) {
    let mut codec = match Codec::software(&stream) {
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

        let result = decode(&mut codec, request.packet.as_ref(), |frame| {
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

fn decode(
    codec: &mut Codec,
    packet: Option<&Packet>,
    mut output: impl FnMut(Receive<CpuFrame>) -> bool,
) -> Result<bool> {
    ensure!(
        !matches!(codec.send(packet)?, Sent::Again),
        "Software decoder did not consume its previous output"
    );

    loop {
        let frame = match codec.receive()? {
            Receive::Again => return Ok(true),
            Receive::End => return Ok(output(Receive::End)),
            Receive::Item(frame) => Receive::Item(CpuFrame::new(frame)?),
        };

        if !output(frame) {
            return Ok(false);
        }
    }
}
