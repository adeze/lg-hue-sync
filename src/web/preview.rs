use super::server::LiveSettings;
use crate::{
    capture::CapturedFrame,
    color::{zones::FloatColor, ColorProcessor},
};
use serde::{Deserialize, Serialize};
use std::{
    sync::{
        atomic::{AtomicU64, Ordering},
        RwLock,
    },
    time::{SystemTime, UNIX_EPOCH},
};
use utoipa::ToSchema;

const MAX_WIDTH: u32 = 128;
const MAX_HEIGHT: u32 = 72;
const LEASE_MS: u64 = 2_000;
const COPY_INTERVAL_MS: u64 = 1_000;

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct PreviewFrame {
    #[schema(minimum = 1, maximum = 128)]
    pub width: u32,
    #[schema(minimum = 1, maximum = 72)]
    pub height: u32,
    #[schema(max_items = 27648)]
    pub rgb: Vec<u8>,
    pub captured_unix_ms: u64,
}

#[derive(Default)]
pub struct PreviewState {
    requested_until: AtomicU64,
    next_copy: AtomicU64,
    frame: RwLock<Option<PreviewFrame>>,
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|duration| duration.as_millis().try_into().ok())
        .unwrap_or_default()
}

impl PreviewState {
    pub fn request(&self) -> Option<PreviewFrame> {
        self.request_at(now_ms())
    }

    fn request_at(&self, now: u64) -> Option<PreviewFrame> {
        self.requested_until
            .store(now.saturating_add(LEASE_MS), Ordering::Relaxed);
        self.frame
            .try_read()
            .ok()?
            .as_ref()
            .filter(|frame| {
                frame.captured_unix_ms <= now && now - frame.captured_unix_ms <= LEASE_MS
            })
            .cloned()
    }

    pub fn publish(&self, frame: &CapturedFrame<'_>) {
        if self.requested_until.load(Ordering::Relaxed) != 0 {
            self.publish_at(frame, now_ms());
        }
    }

    fn publish_at(&self, frame: &CapturedFrame<'_>, now: u64) {
        let requested_until = self.requested_until.load(Ordering::Relaxed);
        if requested_until == 0 || now > requested_until {
            let _ = self.requested_until.compare_exchange(
                requested_until,
                0,
                Ordering::Relaxed,
                Ordering::Relaxed,
            );
            return;
        }
        if now < self.next_copy.load(Ordering::Relaxed) {
            return;
        }
        let Ok(mut stored) = self.frame.try_write() else {
            return;
        };
        // Keep the copy ceiling global even if another producer is added later.
        if now < self.next_copy.load(Ordering::Relaxed) {
            return;
        }
        let Some(copy) = copy_frame(frame, now) else {
            return;
        };
        *stored = Some(copy);
        self.next_copy
            .store(now.saturating_add(COPY_INTERVAL_MS), Ordering::Relaxed);
    }

    pub fn clear(&self) {
        self.requested_until.store(0, Ordering::Relaxed);
        self.next_copy.store(0, Ordering::Relaxed);
        if let Ok(mut frame) = self.frame.write() {
            *frame = None;
        }
    }
}

fn copy_frame(frame: &CapturedFrame<'_>, now: u64) -> Option<PreviewFrame> {
    let source_len = usize::try_from(frame.width)
        .ok()?
        .checked_mul(usize::try_from(frame.height).ok()?)?
        .checked_mul(4)?;
    if frame.width == 0 || frame.height == 0 || frame.data.len() < source_len {
        return None;
    }
    let (width, height) = if frame.width <= MAX_WIDTH && frame.height <= MAX_HEIGHT {
        (frame.width, frame.height)
    } else if u64::from(frame.width) * u64::from(MAX_HEIGHT)
        > u64::from(frame.height) * u64::from(MAX_WIDTH)
    {
        (
            MAX_WIDTH,
            (u64::from(frame.height) * u64::from(MAX_WIDTH) / u64::from(frame.width)).max(1) as u32,
        )
    } else {
        (
            (u64::from(frame.width) * u64::from(MAX_HEIGHT) / u64::from(frame.height)).max(1)
                as u32,
            MAX_HEIGHT,
        )
    };
    let mut rgb = Vec::with_capacity(width as usize * height as usize * 3);
    for y in 0..height {
        let sy =
            ((u64::from(y) * 2 + 1) * u64::from(frame.height) / (u64::from(height) * 2)) as usize;
        for x in 0..width {
            let sx =
                ((u64::from(x) * 2 + 1) * u64::from(frame.width) / (u64::from(width) * 2)) as usize;
            let offset = (sy * frame.width as usize + sx) * 4;
            let pixel = &frame.data[offset..offset + 3];
            if frame.is_bgra {
                rgb.extend_from_slice(&[pixel[2], pixel[1], pixel[0]]);
            } else {
                rgb.extend_from_slice(pixel);
            }
        }
    }
    Some(PreviewFrame {
        width,
        height,
        rgb,
        captured_unix_ms: now,
    })
}

/// Colour grading only: excludes regional sampling, peak weighting, spatial/temporal response and device trims.
pub fn render(frame: &PreviewFrame, settings: &LiveSettings) -> Result<PreviewFrame, String> {
    if frame.width == 0
        || frame.height == 0
        || frame.width > MAX_WIDTH
        || frame.height > MAX_HEIGHT
        || frame.rgb.len() != frame.width as usize * frame.height as usize * 3
    {
        return Err("preview frame must contain exact RGB pixels within 128 × 72".into());
    }
    for (value, low, high) in [
        (settings.brightness_multiplier, 0.5, 2.5),
        (settings.saturation_boost, 0.5, 2.5),
        (settings.gamma, 0.5, 2.0),
        (settings.noise_gate_threshold, 0.0, 0.06),
    ] {
        if !value.is_finite() || !(low..=high).contains(&value) {
            return Err("invalid preview colour settings".into());
        }
    }
    let mut processor = ColorProcessor::new(
        1.0,
        settings.hdr_tone_mapping,
        settings.saturation_boost,
        settings.noise_gate_threshold,
    );
    processor.set_gamma(settings.gamma);
    let mut output = frame.clone();
    for pixel in output.rgb.as_chunks_mut::<3>().0 {
        let color = FloatColor::new(pixel[0].into(), pixel[1].into(), pixel[2].into());
        let color = processor
            .transform_color(color)
            .to_rgb()
            .scale(settings.brightness_multiplier);
        pixel.copy_from_slice(&[color.r, color.g, color.b]);
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capture_is_opt_in_throttled_and_expires() {
        let state = PreviewState::default();
        let frame = CapturedFrame {
            data: &[10, 20, 30, 255],
            width: 1,
            height: 1,
            is_bgra: false,
        };
        state.publish_at(&frame, 100);
        assert!(state.frame.read().unwrap().is_none());
        assert!(state.request_at(100).is_none());
        state.publish_at(&frame, 101);
        assert_eq!(state.request_at(102).unwrap().captured_unix_ms, 101);
        state.publish_at(&frame, 200);
        assert_eq!(state.request_at(201).unwrap().captured_unix_ms, 101);
        state.publish_at(&frame, 1_101);
        assert_eq!(state.request_at(1_102).unwrap().captured_unix_ms, 1_101);
        assert!(state.request_at(3_103).is_none());
        state.clear();
        assert!(state.frame.read().unwrap().is_none());
        assert_eq!(state.requested_until.load(Ordering::Relaxed), 0);
        state.request_at(4_000);
        state.publish_at(&frame, 6_001);
        assert!(state.frame.read().unwrap().is_none());
        state.request_at(7_000);
        let reader = state.frame.read().unwrap();
        state.publish_at(&frame, 7_001);
        assert!(reader.is_none());
        drop(reader);
        state.publish_at(&frame, 7_002);
        assert_eq!(state.request_at(7_003).unwrap().captured_unix_ms, 7_002);
    }

    #[test]
    fn copies_rgb_and_bgra_with_bounded_dimensions_and_validates_buffers() {
        let pixels = [10, 20, 30, 255].repeat(320 * 180);
        let mut source = CapturedFrame {
            data: &pixels,
            width: 320,
            height: 180,
            is_bgra: false,
        };
        let rgb = copy_frame(&source, 1).unwrap();
        assert_eq!(
            (rgb.width, rgb.height, rgb.rgb.len()),
            (128, 72, 128 * 72 * 3)
        );
        assert_eq!(&rgb.rgb[..3], &[10, 20, 30]);
        source.is_bgra = true;
        assert_eq!(&copy_frame(&source, 1).unwrap().rgb[..3], &[30, 20, 10]);
        source.width = 80;
        source.height = 160;
        let portrait = copy_frame(&source, 1).unwrap();
        assert_eq!((portrait.width, portrait.height), (36, 72));
        source.data = &[];
        assert!(copy_frame(&source, 1).is_none());
        source.width = u32::MAX;
        source.height = u32::MAX;
        assert!(copy_frame(&source, 1).is_none());
        source.width = 0;
        assert!(copy_frame(&source, 1).is_none());
    }

    #[test]
    fn rendering_uses_shared_colour_transform_and_rejects_bad_inputs() {
        let mut settings: LiveSettings = serde_json::from_value(serde_json::json!({
            "brightness_multiplier":1.0,"saturation_boost":1.0,"peak_weight":0.35,
            "gamma":1.0,"noise_gate_threshold":0.0,"use_xy_gamut":false,
            "letterbox_detection":true,"hdr_tone_mapping":false,"hue_sync_enabled":true,
            "nanoleaf_sync_enabled":true,"max_color_step":12
        }))
        .unwrap();
        let mut frame = PreviewFrame {
            width: 2,
            height: 1,
            rgb: vec![180, 120, 120, 1, 1, 1],
            captured_unix_ms: 1,
        };
        assert_eq!(render(&frame, &settings).unwrap().rgb, frame.rgb);
        settings.saturation_boost = 1.5;
        settings.gamma = 2.0;
        settings.brightness_multiplier = 2.0;
        settings.noise_gate_threshold = 0.02;
        assert_eq!(
            render(&frame, &settings).unwrap().rgb,
            vec![254, 64, 64, 0, 0, 0]
        );
        settings.gamma = 0.5;
        settings.saturation_boost = 0.5;
        assert!(render(&frame, &settings).is_ok());
        settings.gamma = f32::NAN;
        assert!(render(&frame, &settings).is_err());
        settings.gamma = 1.0;
        frame.rgb.pop();
        assert!(render(&frame, &settings).is_err());
        frame.width = 129;
        assert!(render(&frame, &settings).is_err());
    }
}
