use crate::{capture_source::CaptureSource, tv_power::PowerObservation};
use serde_json::Value;
use std::{future::pending, time::Duration};
use tokio::sync::watch;
use tokio_util::{sync::CancellationToken, task::TaskTracker};

const FOREGROUND_URI: &str = "luna://com.webos.applicationManager/getForegroundAppInfo";
const POWER_URI: &str = "luna://com.webos.service.tvpower/power/getPowerState";
const VIDEO_URI: &str = "luna://com.webos.service.videooutput/getStatus";
const RETRY_DELAY: Duration = Duration::from_secs(2);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Signal {
    ready: bool,
    port: Option<u64>,
    width: u64,
    height: u64,
    frame_rate_millihz: Option<u64>,
}

pub fn frame_rate(video: Option<&Value>) -> Option<f64> {
    let value = video?;
    if value["returnValue"] != true {
        return None;
    }
    let main = value["video"]
        .as_array()?
        .iter()
        .find(|sink| sink["sink"] == "MAIN")?;
    if main["connected"] != true || main["connectedSource"] != "HDMI" {
        return None;
    }
    let value = &main["frameRate"];
    let rate = value.as_f64().or_else(|| {
        let text = value.as_str()?;
        if let Some((numerator, denominator)) = text.split_once('/') {
            Some(numerator.parse::<f64>().ok()? / denominator.parse::<f64>().ok()?)
        } else {
            text.parse().ok()
        }
    })?;
    (rate.is_finite() && rate > 0.0 && rate <= 1000.0).then_some(rate)
}

fn hdmi_signal(value: &Value) -> Option<Signal> {
    if value["returnValue"] != true {
        return None;
    }
    let main = value["video"]
        .as_array()?
        .iter()
        .find(|sink| sink["sink"] == "MAIN")?;
    let connected = main["connected"].as_bool()?;
    let width = main["width"].as_u64().unwrap_or_default();
    let height = main["height"].as_u64().unwrap_or_default();
    Some(Signal {
        ready: connected
            && main["connectedSource"] == "HDMI"
            && main["muted"] != true
            && width > 0
            && height > 0,
        port: main["connectedSourcePort"].as_u64(),
        width,
        height,
        frame_rate_millihz: frame_rate(Some(value)).map(|rate| (rate * 1000.0).round() as u64),
    })
}

/// Event-triggered output hold, separate from black content and capture recovery.
#[derive(Default)]
pub struct SignalTransitionHold {
    previous: Option<Signal>,
    since: Option<std::time::Instant>,
}

impl SignalTransitionHold {
    pub fn observe(&mut self, video: Option<&Value>, now: std::time::Instant) {
        let Some(signal) = video.and_then(hdmi_signal) else {
            // A disconnected event feed must not freeze stale output indefinitely.
            self.reset();
            return;
        };
        if self.since.is_none()
            && self
                .previous
                .is_some_and(|previous| previous.ready && previous != signal)
        {
            self.since = Some(now);
        }
        self.previous = Some(signal);
    }

    pub fn active(&self, now: std::time::Instant) -> bool {
        self.since
            .is_some_and(|since| now.saturating_duration_since(since) < Duration::from_secs(5))
    }

    pub fn pending(&self) -> bool {
        self.since.is_some()
    }

    pub fn hold_frame(&mut self, black: bool, now: std::time::Instant) -> bool {
        if self.previous.is_some_and(|signal| signal.ready) && !black {
            self.since = None;
            return false;
        }
        self.active(now)
    }

    pub fn reset(&mut self) {
        self.previous = None;
        self.since = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn transitions_hold_only_after_known_signal_and_release_on_picture_timeout_or_reset() {
        let now = std::time::Instant::now();
        let ready = json!({"returnValue":true,"video":[{"sink":"MAIN","connected":true,
            "connectedSource":"HDMI","connectedSourcePort":1,"width":1920,"height":1080,"muted":false,"frameRate":60}]});
        let mut lost = ready.clone();
        lost["video"][0]["connected"] = json!(false);
        let mut hold = SignalTransitionHold::default();
        hold.observe(Some(&lost), now);
        assert!(!hold.hold_frame(true, now));
        hold.observe(Some(&ready), now);
        // Ordinary black content does not arm a signal hold.
        assert!(!hold.hold_frame(true, now));
        hold.observe(Some(&lost), now);
        assert!(hold.hold_frame(true, now + Duration::from_secs(1)));
        hold.observe(Some(&ready), now + Duration::from_secs(1));
        assert!(hold.hold_frame(true, now + Duration::from_secs(2)));
        assert!(!hold.hold_frame(false, now + Duration::from_secs(2)));
        for change in ["width", "connectedSourcePort", "muted", "frameRate"] {
            let mut changed = ready.clone();
            changed["video"][0][change] = if change == "muted" {
                json!(true)
            } else {
                json!(2)
            };
            hold.reset();
            hold.observe(Some(&ready), now);
            hold.observe(Some(&changed), now);
            assert!(hold.hold_frame(true, now));
            changed["video"][0]["frameRate"] = json!(50);
            hold.observe(Some(&changed), now + Duration::from_secs(4));
            assert!(!hold.hold_frame(true, now + Duration::from_secs(5)));
            changed["video"][0]["width"] = json!(3840);
            hold.observe(Some(&changed), now + Duration::from_secs(6));
            assert!(!hold.hold_frame(true, now + Duration::from_secs(6)));
            hold.reset();
            assert!(!hold.active(now));
        }
        hold.observe(Some(&ready), now);
        hold.observe(Some(&lost), now);
        hold.observe(None, now);
        assert!(!hold.active(now));
        let mut fractional = ready;
        fractional["video"][0]["frameRate"] = json!("24000/1001");
        assert!((frame_rate(Some(&fractional)).unwrap() - 23.976023976).abs() < 0.000001);
        fractional["video"][0]["frameRate"] = json!("1/0");
        assert!(frame_rate(Some(&fractional)).is_none());
    }
}

pub struct Snapshot {
    pub source: CaptureSource,
    pub power: PowerObservation,
    pub power_received_at: Option<std::time::Instant>,
    pub video: Option<Value>,
}

#[derive(Clone)]
struct EventValue {
    value: Value,
    received_at: std::time::Instant,
}

pub struct TvEvents {
    source: watch::Receiver<Option<EventValue>>,
    power: watch::Receiver<Option<EventValue>>,
    video: watch::Receiver<Option<EventValue>>,
    shutdown: CancellationToken,
    tasks: TaskTracker,
}

impl TvEvents {
    pub fn start(shutdown: CancellationToken) -> Self {
        let shutdown = shutdown.child_token();
        let tasks = TaskTracker::new();
        let source = start_feed(FOREGROUND_URI, &tasks, &shutdown);
        let power = start_feed(POWER_URI, &tasks, &shutdown);
        let video = start_feed(VIDEO_URI, &tasks, &shutdown);
        Self {
            source,
            power,
            video,
            shutdown,
            tasks,
        }
    }

    pub fn snapshot(&mut self) -> Snapshot {
        let source = self
            .source
            .borrow_and_update()
            .as_ref()
            .map(|event| crate::capture_source::parse_source(&event.value))
            .unwrap_or(CaptureSource::Unknown);
        let power_event = self.power.borrow_and_update();
        let power_received_at = power_event.as_ref().map(|event| event.received_at);
        let power = power_event
            .as_ref()
            .map(|event| crate::tv_power::parse_observation(&event.value))
            .unwrap_or_default();
        let video = self
            .video
            .borrow_and_update()
            .as_ref()
            .map(|event| event.value.clone());
        Snapshot {
            source,
            power,
            power_received_at,
            video,
        }
    }

    pub async fn changed(&mut self) {
        tokio::select! {
            biased;
            _ = self.shutdown.cancelled() => {},
            _ = wait_changed(&mut self.source) => {},
            _ = wait_changed(&mut self.power) => {},
            _ = wait_changed(&mut self.video) => {},
        }
    }

    pub async fn close(self) {
        self.shutdown.cancel();
        self.tasks.close();
        self.tasks.wait().await;
    }
}

fn start_feed(
    uri: &'static str,
    tasks: &TaskTracker,
    shutdown: &CancellationToken,
) -> watch::Receiver<Option<EventValue>> {
    let (sender, receiver) = watch::channel(None);
    tasks.spawn(run_feed(uri, sender, shutdown.clone()));
    receiver
}

fn publish(sender: &watch::Sender<Option<EventValue>>, value: Option<Value>) {
    let received_at = std::time::Instant::now();
    sender.send_if_modified(|current| {
        if current.as_ref().map(|event| &event.value) == value.as_ref() {
            return false;
        }
        *current = value.map(|value| EventValue { value, received_at });
        true
    });
}

async fn wait_changed(receiver: &mut watch::Receiver<Option<EventValue>>) {
    if receiver.changed().await.is_err() {
        pending::<()>().await;
    }
}

async fn run_feed(
    uri: &'static str,
    sender: watch::Sender<Option<EventValue>>,
    shutdown: CancellationToken,
) {
    while !shutdown.is_cancelled() {
        let updates = sender.clone();
        let _ = crate::luna::subscribe(
            uri,
            r#"{"subscribe":true}"#,
            shutdown.clone(),
            move |value| publish(&updates, Some(value)),
        )
        .await;
        if shutdown.is_cancelled() {
            break;
        }
        publish(&sender, None);
        // Let the bounded query finish so its child is always killed and reaped.
        let fallback = crate::luna::query(uri, "{}").await.ok();
        if shutdown.is_cancelled() {
            break;
        }
        publish(&sender, fallback);
        tokio::select! {
            biased;
            _ = shutdown.cancelled() => break,
            _ = tokio::time::sleep(RETRY_DELAY) => {},
        }
    }
}
