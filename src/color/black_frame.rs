use std::time::{Duration, Instant};

use super::zones::FloatColor;

/// Preserve the last colours across short, whole-screen black interruptions.
#[derive(Default)]
pub(crate) struct BlackFrameGuard {
    hold: Duration,
    fade: Duration,
    since: Option<Instant>,
    held: Vec<FloatColor>,
}

impl BlackFrameGuard {
    pub fn configure(&mut self, hold_ms: u32, fade_ms: u32) {
        let hold = Duration::from_millis(hold_ms.min(5000).into());
        let fade = Duration::from_millis(fade_ms.min(10000).into());
        if (hold, fade) != (self.hold, self.fade) {
            self.reset();
            self.hold = hold;
            self.fade = fade;
        }
    }

    pub fn reset(&mut self) {
        self.since = None;
        self.held.clear();
    }

    /// Fade an output copy; callers must preserve their normal sampling history.
    /// RGB/BGRA order is immaterial: every colour component must be near zero.
    pub fn apply(
        &mut self,
        data: &[u8],
        width: u32,
        height: u32,
        colors: &mut [FloatColor],
        now: Instant,
    ) -> bool {
        let length = (width as usize)
            .checked_mul(height as usize)
            .and_then(|pixels| pixels.checked_mul(4));
        let black = length
            .and_then(|length| data.get(..length))
            .filter(|pixels| !pixels.is_empty())
            .is_some_and(|pixels| {
                pixels
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .all(|pixel| pixel[..3].iter().all(|component| *component <= 3))
            });
        if (self.hold.is_zero() && self.fade.is_zero()) || !black {
            self.reset();
            return false;
        }
        if self.since.is_none() || self.held.len() != colors.len() {
            self.held = colors.to_vec();
            self.since = Some(now);
        }
        let elapsed = now.saturating_duration_since(self.since.unwrap());
        let gain = if elapsed < self.hold {
            1.0
        } else if self.fade.is_zero() {
            0.0
        } else {
            1.0 - (elapsed.saturating_sub(self.hold).as_secs_f32() / self.fade.as_secs_f32())
                .clamp(0.0, 1.0)
        };
        for (color, held) in colors.iter_mut().zip(&self.held) {
            *color = held.scale(gain);
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hold_fade_and_recovery_use_wall_clock_and_restart_for_each_interruption() {
        let mut guard = BlackFrameGuard::default();
        guard.configure(750, 2000);
        let now = Instant::now();
        let original = FloatColor::new(200.0, 100.0, 40.0);
        let mut colors = [original];
        for (millis, gain) in [(0, 1.0), (749, 1.0), (1750, 0.5), (2750, 0.0), (9000, 0.0)] {
            assert!(guard.apply(
                &[0, 0, 0, 255],
                1,
                1,
                &mut colors,
                now + Duration::from_millis(millis)
            ));
            assert_eq!(colors[0], original.scale(gain));
        }
        assert!(!guard.apply(
            &[20, 0, 0, 255],
            1,
            1,
            &mut colors,
            now + Duration::from_secs(10)
        ));
        colors[0] = original;
        assert!(guard.apply(&[0; 4], 1, 1, &mut colors, now + Duration::from_secs(11)));
        assert_eq!(colors[0], original);
    }

    #[test]
    fn detection_requires_the_entire_valid_frame_to_be_near_black() {
        let mut guard = BlackFrameGuard::default();
        guard.configure(750, 2000);
        let now = Instant::now();
        let mut colors = [FloatColor::new(80.0, 40.0, 20.0)];
        for data in [[4, 0, 0, 255], [0, 4, 0, 255], [0, 0, 4, 255]] {
            assert!(!guard.apply(&data, 1, 1, &mut colors, now));
        }
        assert!(guard.apply(&[3, 3, 3, 255], 1, 1, &mut colors, now));
        assert!(!guard.apply(&[0; 4], 2, 1, &mut colors, now));
        assert!(!guard.apply(&[], 0, 0, &mut colors, now));
        assert!(!guard.apply(&[0; 4], u32::MAX, u32::MAX, &mut colors, now));
        let mut data = [0; 32];
        data[28] = 255;
        assert!(!guard.apply(&data, 4, 2, &mut colors, now));
    }

    #[test]
    fn disabled_fade_only_hold_only_and_reset_are_explicit() {
        let mut guard = BlackFrameGuard::default();
        let now = Instant::now();
        let original = FloatColor::new(80.0, 40.0, 20.0);
        let mut colors = [original];
        assert!(!guard.apply(&[0; 4], 1, 1, &mut colors, now));
        guard.configure(0, 1000);
        assert!(guard.apply(&[0; 4], 1, 1, &mut colors, now));
        guard.apply(&[0; 4], 1, 1, &mut colors, now + Duration::from_millis(500));
        assert_eq!(colors[0], original.scale(0.5));
        guard.configure(1000, 0);
        guard.apply(&[0; 4], 1, 1, &mut colors, now);
        guard.apply(&[0; 4], 1, 1, &mut colors, now + Duration::from_secs(1));
        assert_eq!(colors[0], FloatColor::default());
        guard.reset();
        guard.apply(&[0; 4], 1, 1, &mut colors, now + Duration::from_secs(2));
        assert_eq!(colors[0], FloatColor::default());
    }
}
