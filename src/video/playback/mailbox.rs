//! Coalesces the UI's requests for the playback thread

use std::{
    path::PathBuf,
    sync::{Condvar, Mutex},
};

use crate::video::PlayerCommand;

/// What the UI has asked of the playback thread since it last looked
#[derive(Default)]
pub(super) struct Requests {
    /// A new file (or `None` to stop), tagged with the UI's source generation
    pub source: Option<(u64, Option<PathBuf>)>,
    pub commands: Vec<PlayerCommand>,
    /// A requested playback step, optionally with the target texture size
    pub tick: Option<Option<[u32; 2]>>,
    pub shutdown: bool,
}

/// Requests from the UI, coalesced so a slow playback thread never blocks it
#[derive(Default)]
pub(super) struct Mailbox {
    requests: Mutex<Requests>,
    wake: Condvar,
}

impl Mailbox {
    pub fn update(&self, update: impl FnOnce(&mut Requests)) {
        let mut requests = self.requests.lock().unwrap_or_else(|e| e.into_inner());
        update(&mut requests);
        self.wake.notify_one();
    }

    /// Waits for at least one request and takes everything queued
    pub fn take(&self) -> Requests {
        let mut requests = self.requests.lock().unwrap_or_else(|e| e.into_inner());

        while requests.source.is_none()
            && requests.commands.is_empty()
            && requests.tick.is_none()
            && !requests.shutdown
        {
            requests = self.wake.wait(requests).unwrap_or_else(|e| e.into_inner());
        }

        std::mem::take(&mut *requests)
    }
}

impl Requests {
    pub fn replace_source(&mut self, generation: u64, path: Option<PathBuf>) {
        self.source = Some((generation, path));
        self.tick = self.tick.map(|_| None);
        self.commands.retain(|command| {
            matches!(
                command,
                PlayerCommand::Volume(_) | PlayerCommand::Decoder(_)
            )
        });
    }

    /// Keeps only the latest command of each kind; two queued toggles cancel out
    pub fn command(&mut self, command: PlayerCommand) {
        let previous = self.commands.iter().position(|queued| {
            std::mem::discriminant(queued) == std::mem::discriminant(&command)
        });
        if let Some(index) = previous {
            self.commands.remove(index);
            if matches!(command, PlayerCommand::TogglePause) {
                return;
            }
        }
        self.commands.push(command);
    }
}

#[cfg(test)]
mod tests {
    use super::Requests;
    use crate::{settings::VideoDecoder, video::PlayerCommand};

    #[test]
    fn replacing_a_file_discards_old_controls_and_preserves_preferences() {
        let mut requests = Requests::default();
        requests.command(PlayerCommand::TogglePause);
        requests.command(PlayerCommand::Seek(12.0));
        requests.command(PlayerCommand::Volume(30.0));
        requests.command(PlayerCommand::Decoder(VideoDecoder::Nvdec));
        requests.replace_source(2, Some("other.mp4".into()));

        assert_eq!(requests.commands.len(), 2);
        assert!(matches!(requests.commands[0], PlayerCommand::Volume(30.0)));
        assert!(matches!(
            requests.commands[1],
            PlayerCommand::Decoder(VideoDecoder::Nvdec)
        ));
        requests.command(PlayerCommand::TogglePause);
        requests.command(PlayerCommand::TogglePause);
        assert_eq!(requests.commands.len(), 2);
    }
}
