use crate::web::ControlCommand;
use crate::web::LiveSettings;
use std::time::Duration;
use tokio::{
    sync::{mpsc, oneshot},
    time::Instant,
};

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
    pub save_config: Vec<oneshot::Sender<Result<(), String>>>,
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
                ControlCommand::SaveConfig(reply) => self.save_config.push(reply),
                ControlCommand::ApplySettings(settings) => self.apply_settings = Some(settings),
            }
        }
    }

    pub fn has_management_action(&self) -> bool {
        self.restart
            || self.reconfigure
            || self.sync_bridge
            || !self.save_config.is_empty()
            || self.apply_settings.is_some()
    }

    pub fn take_stop_if_running(&mut self, state: PipelineState) -> bool {
        state == PipelineState::Running && self.desired_running.take() == Some(false)
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
        let (reply, _result) = oneshot::channel();
        sender
            .send(ControlCommand::SaveConfig(reply))
            .await
            .unwrap();
        sender.send(ControlCommand::Stop).await.unwrap();

        let mut pending = PendingCommands::default();
        pending.receive(&mut receiver);

        assert_eq!(pending.desired_running, Some(false));
        assert_eq!(pending.save_config.len(), 1);
        assert!(pending.has_management_action());
    }

    #[tokio::test]
    async fn concurrent_save_requests_keep_each_acknowledgement() {
        let (sender, mut receiver) = mpsc::channel(2);
        let (first_reply, first_result) = oneshot::channel();
        let (second_reply, second_result) = oneshot::channel();
        sender
            .send(ControlCommand::SaveConfig(first_reply))
            .await
            .unwrap();
        sender
            .send(ControlCommand::SaveConfig(second_reply))
            .await
            .unwrap();

        let mut pending = PendingCommands::default();
        pending.receive(&mut receiver);
        assert_eq!(pending.save_config.len(), 2);
        for reply in std::mem::take(&mut pending.save_config) {
            reply.send(Ok(())).unwrap();
        }
        assert_eq!(first_result.await.unwrap(), Ok(()));
        assert_eq!(second_result.await.unwrap(), Ok(()));
    }

    #[test]
    fn start_and_stop_alone_do_not_interrupt_paused_management() {
        let mut pending = PendingCommands {
            desired_running: Some(false),
            ..PendingCommands::default()
        };
        assert!(!pending.has_management_action());
        pending.reconfigure = true;
        assert!(pending.has_management_action());
    }

    #[test]
    fn paused_start_request_survives_outer_command_check() {
        let mut pending = PendingCommands {
            desired_running: Some(true),
            ..PendingCommands::default()
        };
        assert!(!pending.take_stop_if_running(PipelineState::Paused));
        assert_eq!(pending.desired_running, Some(true));
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
