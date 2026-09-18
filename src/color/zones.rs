use crate::config::LightZone;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RgbColor {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl RgbColor {
    pub fn new(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b }
    }

    pub fn to_u16(self) -> (u16, u16, u16) {
        (
            ((self.r as u16) << 8) | (self.r as u16),
            ((self.g as u16) << 8) | (self.g as u16),
            ((self.b as u16) << 8) | (self.b as u16),
        )
    }

    pub fn lerp(self, target: RgbColor, alpha: f32) -> Self {
        let r = (self.r as f32 + (target.r as f32 - self.r as f32) * alpha).clamp(0.0, 255.0) as u8;
        let g = (self.g as f32 + (target.g as f32 - self.g as f32) * alpha).clamp(0.0, 255.0) as u8;
        let b = (self.b as f32 + (target.b as f32 - self.b as f32) * alpha).clamp(0.0, 255.0) as u8;
        Self { r, g, b }
    }
}

pub struct ZoneSampler {
    zones: Vec<LightZone>,
    smoothed_colors: Vec<RgbColor>,
    smoothing_factor: f32,
}

impl ZoneSampler {
    pub fn new(zones: Vec<LightZone>, smoothing_factor: f32) -> Self {
        let len = zones.len();
        Self {
            zones,
            smoothed_colors: vec![RgbColor::new(0, 0, 0); len],
            smoothing_factor,
        }
    }

    /// Samples frame buffer (assumed 4 bytes per pixel: BGRA or RGBA).
    pub fn sample_frame(
        &mut self,
        data: &[u8],
        width: u32,
        height: u32,
        is_bgra: bool,
    ) -> Vec<(u8, RgbColor)> {
        let mut results = Vec::with_capacity(self.zones.len());

        for (i, zone) in self.zones.iter().enumerate() {
            let x_start = (zone.x_min * width as f32).clamp(0.0, (width - 1) as f32) as u32;
            let x_end = (zone.x_max * width as f32).clamp(0.0, width as f32) as u32;
            let y_start = (zone.y_min * height as f32).clamp(0.0, (height - 1) as f32) as u32;
            let y_end = (zone.y_max * height as f32).clamp(0.0, height as f32) as u32;

            let mut sum_r: u64 = 0;
            let mut sum_g: u64 = 0;
            let mut sum_b: u64 = 0;
            let mut count: u64 = 0;

            for y in (y_start..y_end).step_by(2) {
                let row_offset = (y * width * 4) as usize;
                for x in (x_start..x_end).step_by(2) {
                    let pixel_offset = row_offset + (x * 4) as usize;
                    if pixel_offset + 3 < data.len() {
                        let (r, g, b) = if is_bgra {
                            (data[pixel_offset + 2], data[pixel_offset + 1], data[pixel_offset])
                        } else {
                            (data[pixel_offset], data[pixel_offset + 1], data[pixel_offset + 2])
                        };

                        sum_r += r as u64;
                        sum_g += g as u64;
                        sum_b += b as u64;
                        count += 1;
                    }
                }
            }

            let raw_color = if count > 0 {
                RgbColor::new(
                    (sum_r / count) as u8,
                    (sum_g / count) as u8,
                    (sum_b / count) as u8,
                )
            } else {
                RgbColor::new(0, 0, 0)
            };

            // Apply smoothing filter
            let current = self.smoothed_colors[i];
            let smoothed = current.lerp(raw_color, self.smoothing_factor);
            self.smoothed_colors[i] = smoothed;

            results.push((zone.channel_id, smoothed));
        }

        results
    }
}
