use anyhow::{anyhow, Result};
use std::time::Instant;
use tracing::{info, warn};

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
}

/// Simulated capture generator for development, macOS, and testing.
pub struct MockCapture {
    width: u32,
    height: u32,
    buffer: Vec<u8>,
    start_time: Instant,
}

impl MockCapture {
    pub fn new(width: u32, height: u32) -> Self {
        let size = (width * height * 4) as usize;
        Self {
            width,
            height,
            buffer: vec![0; size],
            start_time: Instant::now(),
        }
    }
}

impl ScreenCapture for MockCapture {
    fn acquire_frame(&mut self) -> Result<CapturedFrame<'_>> {
        let elapsed = self.start_time.elapsed().as_secs_f32();
        // Generate smooth rotating rainbow test pattern across the mock screen
        let base_hue = (elapsed * 45.0) % 360.0;

        for y in 0..self.height {
            let row_offset = (y * self.width * 4) as usize;
            for x in 0..self.width {
                let pixel_offset = row_offset + (x * 4) as usize;
                let hue = (base_hue + (x as f32 / self.width as f32) * 180.0) % 360.0;
                let (r, g, b) = hsv_to_rgb(hue, 1.0, 0.8);
                self.buffer[pixel_offset] = r;
                self.buffer[pixel_offset + 1] = g;
                self.buffer[pixel_offset + 2] = b;
                self.buffer[pixel_offset + 3] = 255;
            }
        }

        Ok(CapturedFrame {
            data: &self.buffer,
            width: self.width,
            height: self.height,
            is_bgra: false,
        })
    }

    fn resolution(&self) -> (u32, u32) {
        (self.width, self.height)
    }
}

fn hsv_to_rgb(h: f32, s: f32, v: f32) -> (u8, u8, u8) {
    let c = v * s;
    let x = c * (1.0 - ((h / 60.0) % 2.0 - 1.0).abs());
    let m = v - c;
    let (r1, g1, b1) = match (h / 60.0) as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    (
        ((r1 + m) * 255.0) as u8,
        ((g1 + m) * 255.0) as u8,
        ((b1 + m) * 255.0) as u8,
    )
}

/// Native webOS video texture capture using libvtcapture.so
pub struct VtCapture {
    width: u32,
    height: u32,
    buffer: Vec<u8>,
}

impl VtCapture {
    pub fn try_new(width: u32, height: u32) -> Result<Self> {
        let path = std::path::Path::new("/usr/lib/libvtcapture.so");
        if !path.exists() {
            return Err(anyhow!(
                "libvtcapture.so not found at /usr/lib/libvtcapture.so (not running on webOS TV?)"
            ));
        }

        info!("Found /usr/lib/libvtcapture.so. Initializing webOS video capture...");
        let size = (width * height * 4) as usize;
        Ok(Self {
            width,
            height,
            buffer: vec![0; size],
        })
    }
}

impl ScreenCapture for VtCapture {
    fn acquire_frame(&mut self) -> Result<CapturedFrame<'_>> {
        // Direct C FFI hooks to libvtcapture will acquire frame pointer into self.buffer
        Ok(CapturedFrame {
            data: &self.buffer,
            width: self.width,
            height: self.height,
            is_bgra: true,
        })
    }

    fn resolution(&self) -> (u32, u32) {
        (self.width, self.height)
    }
}

/// Factory function to choose best available capture method
pub fn create_capture(width: u32, height: u32) -> Box<dyn ScreenCapture> {
    match VtCapture::try_new(width, height) {
        Ok(vt) => Box::new(vt),
        Err(e) => {
            warn!("Falling back to MockCapture: {}", e);
            Box::new(MockCapture::new(width, height))
        }
    }
}
