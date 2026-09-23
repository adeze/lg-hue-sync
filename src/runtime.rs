use crate::web::ControlCommand;
use crate::web::LiveSettings;
use std::time::Duration;
use tokio::{sync::mpsc, time::Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PipelineState {
    Running,
    Paused,
}

#[derive(Default)]
pub struct PendingCommands {
    pub desired_running: Option<bool>,
    pub restart: bool,
    pub reconfigure: bool,
    pub sync_bridge: bool,
    pub save_config: bool,
    pub apply_settings: Option<LiveSettings>,
}

impl PendingCommands {
    pub fn receive(&mut self, receiver: &mut mpsc::Receiver<ControlCommand>) {
        while let Ok(command) = receiver.try_recv() {
            match command {
                ControlCommand::Start => self.desired_running = Some(true),
                ControlCommand::Stop => self.desired_running = Some(false),
                ControlCommand::Restart => self.restart = true,
                ControlCommand::Reconfigure => self.reconfigure = true,
                ControlCommand::SyncBridge => self.sync_bridge = true,
                ControlCommand::SaveConfig => self.save_config = true,
                ControlCommand::ApplySettings(settings) => self.apply_settings = Some(settings),
            }
        }
    }
}

pub struct RetryState {
    failures: u32,
    next_attempt: Instant,
}

impl RetryState {
    pub fn new() -> Self {
        Self {
            failures: 0,
            next_attempt: Instant::now(),
        }
    }

    pub fn ready(&self) -> bool {
        Instant::now() >= self.next_attempt
    }

    pub fn success(&mut self) {
        self.failures = 0;
        self.next_attempt = Instant::now();
    }

    pub fn failure(&mut self) -> Duration {
        self.failures = self.failures.saturating_add(1);
        let delay = Duration::from_millis(250 * (1u64 << self.failures.min(6)));
        self.next_attempt = Instant::now() + delay;
        delay
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn latest_start_stop_command_wins_without_losing_other_actions() {
        let (sender, mut receiver) = mpsc::channel(4);
        sender.send(ControlCommand::Start).await.unwrap();
        sender.send(ControlCommand::SaveConfig).await.unwrap();
        sender.send(ControlCommand::Stop).await.unwrap();

        let mut pending = PendingCommands::default();
        pending.receive(&mut receiver);

        assert_eq!(pending.desired_running, Some(false));
        assert!(pending.save_config);
    }

    #[test]
    fn retry_delay_is_bounded_and_resets() {
        let mut retry = RetryState::new();
        assert_eq!(retry.failure(), Duration::from_millis(500));
        for _ in 0..20 {
            retry.failure();
        }
        assert_eq!(retry.failure(), Duration::from_secs(16));
        retry.success();
        assert!(retry.ready());
        assert_eq!(retry.failure(), Duration::from_millis(500));
    }
}
