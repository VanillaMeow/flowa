//! Bounded software decoding without codec work on the UI thread

use std::{
    ffi::c_void,
    ptr::{self, NonNull},
    sync::{Arc, mpsc},
    thread::{self, JoinHandle},
};

use anyhow::{Context, Result, bail};

use super::media::{Frame, Packet, Receive, Sent, Stream};
use crate::video::{
    gpu::NativeGpu,
    native::{self, Api},
};

const SOFTWARE_BACKEND: i32 = 3;

struct Request {
    generation: u64,
    packet: Option<Packet>,
}

struct Response {
    generation: u64,
    frame: Result<Receive<CpuFrame>>,
}

struct CpuFrame {
    api: Arc<Api>,
    handle: Option<NonNull<c_void>>,
}

// SAFETY: Only software AVFrames cross this channel, transferring exclusive ownership
unsafe impl Send for CpuFrame {}

impl Drop for CpuFrame {
    fn drop(&mut self) {
        if let Some(handle) = self.handle {
            // SAFETY: This wrapper owns one CPU frame reference
            unsafe { (self.api.frame_free)(handle.as_ptr()) };
        }
    }
}

pub(super) struct SoftwareDecoder {
    requests: Option<mpsc::SyncSender<Request>>,
    results: Option<mpsc::Receiver<Response>>,
    worker: Option<JoinHandle<()>>,
    gpu: Arc<NativeGpu>,
    generation: u64,
}

impl SoftwareDecoder {
    pub fn new(gpu: Arc<NativeGpu>, stream: Arc<Stream>) -> Result<Self> {
        let (requests, requests_rx) = mpsc::sync_channel::<Request>(4);
        let (results_tx, results) = mpsc::sync_channel(2);
        let api = Arc::clone(&gpu.api);
        let worker = thread::Builder::new()
            .name("flowa-software-video".into())
            .spawn(move || {
                let mut codec = match CpuCodec::new(api, &stream) {
                    Ok(codec) => codec,
                    Err(error) => {
                        let _ = results_tx.send(Response {
                            generation: 0,
                            frame: Err(error),
                        });
                        return;
                    }
                };
                let mut generation = 0;

                while let Ok(request) = requests_rx.recv() {
                    if request.generation != generation {
                        // SAFETY: This worker exclusively owns the codec
                        unsafe { (codec.api.decoder_flush)(codec.handle.as_ptr()) };
                        generation = request.generation;
                    }

                    let result = codec.decode(request.packet.as_ref(), |frame| {
                        results_tx
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
                            let _ = results_tx.send(Response {
                                generation,
                                frame: Err(error),
                            });
                            return;
                        }
                    }
                }
            })?;

        Ok(Self {
            requests: Some(requests),
            results: Some(results),
            worker: Some(worker),
            gpu,
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
                Receive::Item(mut frame) => {
                    let handle = frame.handle.take().context("Empty software frame")?;
                    Ok(Receive::Item(Frame::from_handle(&self.gpu, handle)))
                }
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

struct CpuCodec {
    api: Arc<Api>,
    handle: NonNull<c_void>,
}

impl CpuCodec {
    fn new(api: Arc<Api>, stream: &Stream) -> Result<Self> {
        // SAFETY: Software codecs accept a NULL GPU and copy the borrowed stream parameters
        let handle = native::create(|error, size| unsafe {
            (api.decoder_create)(
                ptr::null_mut(),
                stream.raw(),
                SOFTWARE_BACKEND,
                error,
                size,
            )
        })?;

        Ok(Self { api, handle })
    }

    fn decode(
        &mut self,
        packet: Option<&Packet>,
        mut output: impl FnMut(Receive<CpuFrame>) -> bool,
    ) -> Result<bool> {
        // SAFETY: The codec belongs to this worker and borrows the packet only during the call
        native::check(|error, size| unsafe {
            (self.api.decoder_send)(
                self.handle.as_ptr(),
                packet.map_or(ptr::null_mut(), Packet::raw),
                error,
                size,
            )
        })
        .and_then(|code| {
            anyhow::ensure!(
                code != 1,
                "Software decoder did not consume its previous output"
            );
            Ok(())
        })?;

        loop {
            let mut handle = ptr::null_mut();
            // SAFETY: Output storage is writable and the codec has one owner
            let code = native::check(|error, size| unsafe {
                (self.api.decoder_receive)(
                    self.handle.as_ptr(),
                    &mut handle,
                    error,
                    size,
                )
            })?;
            let frame = match code {
                0 => return Ok(true),
                2 => return Ok(output(Receive::End)),
                _ => Receive::Item(CpuFrame {
                    api: Arc::clone(&self.api),
                    handle: Some(
                        NonNull::new(handle)
                            .context("FFmpeg returned an empty software frame")?,
                    ),
                }),
            };

            if !output(frame) {
                return Ok(false);
            }
        }
    }
}

impl Drop for CpuCodec {
    fn drop(&mut self) {
        // SAFETY: The codec is destroyed on its owning worker after its last operation
        unsafe { (self.api.decoder_free)(self.handle.as_ptr()) };
    }
}
