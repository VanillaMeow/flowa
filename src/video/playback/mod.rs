//! Playback: the engine that schedules frames, the demux and audio worker, the clock,
//! and the service that runs them off the UI thread

mod clock;
mod engine;
mod pipeline;
mod queues;
mod service;
mod state;
mod worker;

pub(super) use service::Player;
