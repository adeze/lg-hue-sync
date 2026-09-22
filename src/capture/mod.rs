pub mod dile_vt;
pub mod vtcapture;

use anyhow::Result;
use tracing::{error, info, warn};

pub struct CapturedFrame<'a> {
    pub data: &'a [u8],
    pub width: u32,
    pub height: u32,
    pub is_bgra: bool,
}

pub trait ScreenCapture: Send {
    fn acquire_frame(&mut self) -> Result<CapturedFrame<'_>>;
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
            let err_str = e.to_string();
            if err_str.contains("error code 11") || err_str.contains("EBUSY") {
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
            let err_str = e.to_string();
            if err_str.contains("error code 11") || err_str.contains("EBUSY") {
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

    #[test]
    fn test_create_capture_fallback_to_mock() {
        let mut capture = create_capture(160, 90);
        let frame = capture.acquire_frame().expect("Acquire mock frame failed");
        assert_eq!(frame.width, 160);
        assert_eq!(frame.height, 90);
        assert_eq!(frame.data.len(), 160 * 90 * 4);
    }
}
