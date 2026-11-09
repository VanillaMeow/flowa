//! Named background threads and the generations that tell them which work is current
//!
//! Requests to a stopped worker are logged, since that means the thread panicked.
//! Results whose receiver is gone are dropped, since their owner no longer exists

use std::{
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
        mpsc::{self, TrySendError},
    },
    thread::{self, JoinHandle},
};

const QUEUED_REQUESTS: usize = 24;
const QUEUED_RESULTS: usize = 4;

/// The generation an owner currently wants work for, shared with its workers
///
/// Owners move it forward when they replace their image or folder, and workers skip
/// requests tagged with any other generation
#[derive(Clone, Default)]
pub struct Generation(Arc<AtomicU64>);

impl Generation {
    pub fn set(&self, generation: u64) {
        self.0.store(generation, Ordering::Relaxed);
    }

    pub fn get(&self) -> u64 {
        self.0.load(Ordering::Relaxed)
    }

    pub fn is_current(&self, generation: u64) -> bool {
        self.get() == generation
    }
}

/// Requests carry the generation of the image or folder they were made for
pub trait Tagged {
    fn generation(&self) -> u64;
}

/// Starts a thread named `flowa-{name}`, panicking like `thread::spawn` if the OS refuses
pub fn spawn<T: Send + 'static>(
    name: &str,
    work: impl FnOnce() -> T + Send + 'static,
) -> JoinHandle<T> {
    thread::Builder::new()
        .name(format!("flowa-{name}"))
        .spawn(work)
        .unwrap_or_else(|error| panic!("Could not start the {name} thread: {error}"))
}

/// A named thread that handles requests in order and wakes the UI with each result
///
/// Both queues are bounded, so a busy worker refuses requests instead of blocking the UI
pub struct Worker<Request, Response> {
    name: &'static str,
    requests: mpsc::SyncSender<Request>,
    pub results: mpsc::Receiver<Response>,
}

impl<Request, Response> Worker<Request, Response>
where
    Request: Send + 'static,
    Response: Send + 'static,
{
    /// Runs `handle` for every request; `None` sends nothing back
    pub fn spawn(
        name: &'static str,
        ctx: egui::Context,
        mut handle: impl FnMut(Request) -> Option<Response> + Send + 'static,
    ) -> Self {
        let (requests, pending) = mpsc::sync_channel::<Request>(QUEUED_REQUESTS);
        let (completed, results) = mpsc::sync_channel(QUEUED_RESULTS);

        spawn(name, move || {
            while let Ok(request) = pending.recv() {
                let Some(response) = handle(request) else {
                    continue;
                };

                if completed.send(response).is_err() {
                    break;
                }

                ctx.request_repaint();
            }
        });

        Self {
            name,
            requests,
            results,
        }
    }

    /// Queues a request without blocking, returning whether the worker accepted it
    pub fn request(&self, request: Request) -> bool {
        match self.requests.try_send(request) {
            Ok(()) => true,
            Err(TrySendError::Full(_)) => false,
            Err(TrySendError::Disconnected(_)) => {
                log::error!("The {} worker stopped", self.name);
                false
            }
        }
    }
}

impl<Request, Response> Worker<Request, Response>
where
    Request: Tagged + Send + 'static,
    Response: Send + 'static,
{
    /// Like `spawn`, but skips requests from other generations before and after the work
    pub fn spawn_tagged(
        name: &'static str,
        ctx: egui::Context,
        generation: &Generation,
        mut handle: impl FnMut(Request) -> Option<Response> + Send + 'static,
    ) -> Self {
        let generation = generation.clone();

        Self::spawn(name, ctx, move |request| {
            let tag = request.generation();

            if !generation.is_current(tag) {
                return None;
            }

            let response = handle(request)?;

            // Drop work for an image or folder that was replaced meanwhile
            generation.is_current(tag).then_some(response)
        })
    }
}
