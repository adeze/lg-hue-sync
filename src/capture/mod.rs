pub mod dile_vt;
pub mod vtcapture;

use std::time::{Duration, Instant};
use tracing::{error, info, warn};

pub struct CapturedFrame<'a> {
    pub data: &'a [u8],
    pub width: u32,
    pub height: u32,
    pub is_bgra: bool,
}

#[derive(Debug, thiserror::Error)]
#[error("{backend} {operation} returned code {code}")]
pub struct NativeCaptureError {
    pub backend: &'static str,
    pub operation: &'static str,
    pub code: i32,
}

impl NativeCaptureError {
    pub fn is_busy(&self) -> bool {
        // libvtcapture uses 11 for an unavailable scaler; retain its original code.
        matches!(self.code, 11 | -11 | 16 | -16)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum CaptureError {
    #[error("frame unavailable: {0}")]
    Unavailable(#[source] NativeCaptureError),
    #[error("{backend} requires restart: {reason}")]
    RestartRequired {
        backend: &'static str,
        reason: &'static str,
    },
    #[error("invalid capture frame: {0}")]
    InvalidFrame(#[from] anyhow::Error),
}

#[derive(Debug, PartialEq, Eq)]
pub enum RecoveryAction {
    Retry,
    Restart(Duration),
    Pause,
}

#[derive(Default)]
pub struct CaptureRecovery {
    unavailable_frames: u32,
    restarts: u32,
    healthy_since: Option<Instant>,
}

impl CaptureRecovery {
    pub fn frame_received(&mut self, now: Instant) {
        self.unavailable_frames = 0;
        let since = *self.healthy_since.get_or_insert(now);
        if now.saturating_duration_since(since) >= Duration::from_secs(30) {
            self.restarts = 0;
        }
    }

    pub fn failed(&mut self, error: &CaptureError) -> RecoveryAction {
        self.healthy_since = None;
        match error {
            CaptureError::InvalidFrame(_) => return RecoveryAction::Pause,
            CaptureError::Unavailable(_) => {
                self.unavailable_frames = self.unavailable_frames.saturating_add(1);
                if self.unavailable_frames < 4 {
                    return RecoveryAction::Retry;
                }
            }
            CaptureError::RestartRequired { .. } => {}
        }
        self.unavailable_frames = 0;
        if self.restarts >= 3 {
            return RecoveryAction::Pause;
        }
        let delay = Duration::from_millis(750 << self.restarts);
        self.restarts += 1;
        RecoveryAction::Restart(delay)
    }
}

pub trait ScreenCapture {
    fn acquire_frame(&mut self) -> std::result::Result<CapturedFrame<'_>, CaptureError>;
    #[allow(dead_code)]
    fn resolution(&self) -> (u32, u32);
    fn is_real_hardware(&self) -> bool {
        false
    }
}

pub use dile_vt::{detect_source_fps, DileVtCapture, MockCapture};
pub use vtcapture::VtCapture;

/// Creates a screen capture instance, prioritizing libvtcapture (webOS 6.x / C1 OLED),
/// falling back to dile_vt, and finally to MockCapture.
pub fn create_capture(width: u32, height: u32) -> Box<dyn ScreenCapture> {
    // 1. Try modern webOS libvtcapture (/usr/lib/libvtcapture.so.1)
    match VtCapture::try_new(width, height) {
        Ok(capture) => {
            info!("Initialized VtCapture driver (/usr/lib/libvtcapture.so.1) successfully.");
            return Box::new(capture);
        }
        Err(e) => {
            if e.downcast_ref::<NativeCaptureError>()
                .is_some_and(NativeCaptureError::is_busy)
            {
                error!(
                    "[-] Capture device is BUSY: /dev/video* hardware scaler is in use by another process (e.g. lg-hue-sync.service). Stop it before starting a manual capture session: systemctl stop lg-hue-sync"
                );
            } else {
                warn!(
                    "VtCapture not available on this platform ({}). Trying DileVtCapture...",
                    e
                );
            }
        }
    }

    // 2. Try legacy libdile_vt (/usr/lib/libdile_vt.so.0)
    match DileVtCapture::try_new(width, height, 0) {
        Ok(capture) => {
            info!("Initialized DileVtCapture driver (/usr/lib/libdile_vt.so.0) successfully.");
            return Box::new(capture);
        }
        Err(e) => {
            if e.downcast_ref::<NativeCaptureError>()
                .is_some_and(NativeCaptureError::is_busy)
            {
                error!(
                    "[-] DileVtCapture device is BUSY: /dev/video* is in use by another process."
                );
            } else {
                warn!(
                    "DileVtCapture not available on this platform ({}). Falling back to MockCapture.",
                    e
                );
            }
        }
    }

    // 3. Fallback to synthetic MockCapture (host macOS, tests, CI)
    Box::new(MockCapture::new(width, height))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unavailable(code: i32) -> CaptureError {
        CaptureError::Unavailable(NativeCaptureError {
            backend: "libvtcapture",
            operation: "get_buffer",
            code,
        })
    }

    #[test]
    fn transient_outages_and_repeated_transitions_have_bounded_recovery() {
        let mut recovery = CaptureRecovery::default();
        for expected_delay in [750, 1500, 3000] {
            for _ in 0..3 {
                assert_eq!(recovery.failed(&unavailable(11)), RecoveryAction::Retry);
            }
            assert_eq!(
                recovery.failed(&unavailable(11)),
                RecoveryAction::Restart(Duration::from_millis(expected_delay))
            );
            // One valid frame must not erase the budget for a flapping source.
            recovery.frame_received(Instant::now());
        }
        let transition = CaptureError::RestartRequired {
            backend: "DILE_VT",
            reason: "format changed",
        };
        assert_eq!(recovery.failed(&transition), RecoveryAction::Pause);
        let healthy = Instant::now();
        recovery.frame_received(healthy);
        recovery.frame_received(healthy + Duration::from_secs(30));
        assert_eq!(
            recovery.failed(&transition),
            RecoveryAction::Restart(Duration::from_millis(750))
        );
    }

    #[test]
    fn successful_frame_clears_a_short_outage_and_invalid_frames_pause_immediately() {
        let mut recovery = CaptureRecovery::default();
        for _ in 0..3 {
            assert_eq!(recovery.failed(&unavailable(-5)), RecoveryAction::Retry);
        }
        recovery.frame_received(Instant::now());
        assert_eq!(recovery.failed(&unavailable(-5)), RecoveryAction::Retry);
        let source = anyhow::Error::new(std::io::Error::from_raw_os_error(22))
            .context("NV12 dimensions invalid");
        let invalid = CaptureError::from(source);
        assert_eq!(recovery.failed(&invalid), RecoveryAction::Pause);
        assert!(std::error::Error::source(&invalid).is_some());
        if let CaptureError::InvalidFrame(source) = invalid {
            assert_eq!(
                source
                    .downcast_ref::<std::io::Error>()
                    .unwrap()
                    .raw_os_error(),
                Some(22)
            );
        }
        let wrapped = anyhow::Error::new(NativeCaptureError {
            backend: "libvtcapture",
            operation: "vtCapture_init",
            code: 11,
        })
        .context("capture initialization");
        assert!(wrapped
            .downcast_ref::<NativeCaptureError>()
            .unwrap()
            .is_busy());
        if let CaptureError::Unavailable(native) = unavailable(-5) {
            assert_eq!(native.code, -5);
            assert!(!native.is_busy());
        }
    }

    #[test]
    fn test_create_capture_fallback_to_mock() {
        let mut capture = create_capture(160, 90);
        let frame = capture.acquire_frame().expect("Acquire mock frame failed");
        assert_eq!(frame.width, 160);
        assert_eq!(frame.height, 90);
        assert_eq!(frame.data.len(), 160 * 90 * 4);
    }
}
