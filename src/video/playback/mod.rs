//! Playback: the engine that schedules frames, the demux and audio worker, the clock,
//! and the service that runs them off the UI thread

mod clock;
mod engine;
mod frame_history;
mod mailbox;
mod pipeline;
mod playback_thread;
mod prefetch;
mod queues;
mod service;
mod state;
mod stepping;
#[cfg(test)]
mod test_support;
mod worker;

pub(super) use service::Player;
