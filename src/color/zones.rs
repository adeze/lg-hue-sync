use crate::color::gamut::{rgb_to_xy_brightness, HueGamut, HueXYBrightness};
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

    /// Convert to 16-bit RGB values (0..65535) for Hue Entertainment protocol
    pub fn to_u16(self) -> (u16, u16, u16) {
        (
            ((self.r as u16) << 8) | (self.r as u16),
            ((self.g as u16) << 8) | (self.g as u16),
            ((self.b as u16) << 8) | (self.b as u16),
        )
    }

    /// Relative perceptual luminance in range [0.0, 1.0] (Rec. 709 coefficients)
    pub fn luminance(self) -> f32 {
        0.2126 * (self.r as f32 / 255.0)
            + 0.7152 * (self.g as f32 / 255.0)
            + 0.0722 * (self.b as f32 / 255.0)
    }

    /// Chroma saturation in range [0.0, 1.0]
    pub fn saturation(self) -> f32 {
        let max = self.r.max(self.g).max(self.b) as f32 / 255.0;
        let min = self.r.min(self.g).min(self.b) as f32 / 255.0;
        if max <= 0.001 {
            0.0
        } else {
            (max - min) / max
        }
    }

    /// Convert to CIE 1931 xy coordinates and brightness, clamped to Hue Gamut C
    pub fn to_xy_brightness(self, gamut: HueGamut) -> HueXYBrightness {
        rgb_to_xy_brightness(self.r, self.g, self.b, gamut)
    }

    /// Converts CIE 1931 xy + brightness to 16-bit integers for HueStream
    pub fn to_xy_u16(self, gamut: HueGamut) -> (u16, u16, u16) {
        let xy = self.to_xy_brightness(gamut);
        (
            (xy.x * 65535.0).clamp(0.0, 65535.0).round() as u16,
            (xy.y * 65535.0).clamp(0.0, 65535.0).round() as u16,
            (xy.brightness * 65535.0).clamp(0.0, 65535.0).round() as u16,
        )
    }

    pub fn lerp(self, target: RgbColor, alpha: f32) -> Self {
        let r = (self.r as f32 + (target.r as f32 - self.r as f32) * alpha).clamp(0.0, 255.0) as u8;
        let g = (self.g as f32 + (target.g as f32 - self.g as f32) * alpha).clamp(0.0, 255.0) as u8;
        let b = (self.b as f32 + (target.b as f32 - self.b as f32) * alpha).clamp(0.0, 255.0) as u8;
        Self { r, g, b }
    }

    /// Applies Reinhard tone mapping to prevent clipped highlights on HDR10/Dolby Vision video
    pub fn tone_map_hdr(self) -> Self {
        let r_f = self.r as f32 / 255.0;
        let g_f = self.g as f32 / 255.0;
        let b_f = self.b as f32 / 255.0;

        let r_tm = (r_f / (r_f + 0.25) * 1.25).clamp(0.0, 1.0);
        let g_tm = (g_f / (g_f + 0.25) * 1.25).clamp(0.0, 1.0);
        let b_tm = (b_f / (b_f + 0.25) * 1.25).clamp(0.0, 1.0);

        Self {
            r: (r_tm * 255.0) as u8,
            g: (g_tm * 255.0) as u8,
            b: (b_tm * 255.0) as u8,
        }
    }

    /// OLED near-black noise gate: clamps low-level compression noise (< 2% luminance) to absolute 0
    pub fn apply_noise_gate(self, threshold: f32) -> Self {
        let lum = self.luminance();
        if lum <= threshold {
            Self::new(0, 0, 0)
        } else {
            // Smoothly taper off between threshold and 2 * threshold
            let ramp = ((lum - threshold) / threshold).clamp(0.0, 1.0);
            Self::new(
                ((self.r as f32) * ramp) as u8,
                ((self.g as f32) * ramp) as u8,
                ((self.b as f32) * ramp) as u8,
            )
        }
    }

    /// Color difference metric (Euclidean distance in normalized RGB, range 0.0 to ~1.73)
    pub fn delta(self, other: RgbColor) -> f32 {
        let dr = (self.r as f32 - other.r as f32) / 255.0;
        let dg = (self.g as f32 - other.g as f32) / 255.0;
        let db = (self.b as f32 - other.b as f32) / 255.0;
        (dr * dr + dg * dg + db * db).sqrt()
    }
}

/// Active viewport bounds after detecting letterbox/pillarbox bars (normalized 0.0 to 1.0)
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ActiveRect {
    pub x_min: f32,
    pub x_max: f32,
    pub y_min: f32,
    pub y_max: f32,
}

impl Default for ActiveRect {
    fn default() -> Self {
        Self {
            x_min: 0.0,
            x_max: 1.0,
            y_min: 0.0,
            y_max: 1.0,
        }
    }
}

pub struct ZoneSampler {
    zones: Vec<LightZone>,
    smoothed_colors: Vec<RgbColor>,
    smoothing_factor: f32,
    hdr_tone_mapping: bool,
    letterbox_detection: bool,
    saturation_boost: f32,
    noise_gate_threshold: f32,
    active_rect: ActiveRect,
    frame_count: u64,
    last_global_color: RgbColor,
}

impl ZoneSampler {
    pub fn new(
        zones: Vec<LightZone>,
        smoothing_factor: f32,
        hdr_tone_mapping: bool,
        letterbox_detection: bool,
        saturation_boost: f32,
        noise_gate_threshold: f32,
    ) -> Self {
        let len = zones.len();
        Self {
            zones,
            smoothed_colors: vec![RgbColor::new(0, 0, 0); len],
            smoothing_factor,
            hdr_tone_mapping,
            letterbox_detection,
            saturation_boost,
            noise_gate_threshold,
            active_rect: ActiveRect::default(),
            frame_count: 0,
            last_global_color: RgbColor::new(0, 0, 0),
        }
    }

    /// Fast letterbox / pillarbox detector. Evaluates top/bottom row luminance to find black bars.
    pub fn detect_active_rect(data: &[u8], width: u32, height: u32, is_bgra: bool) -> ActiveRect {
        let luma_threshold = 12u32; // Below this is considered black letterbox bar
        let sample_step_x = 4usize;

        // Top bar detection
        let mut top_bar_height = 0u32;
        let max_bar_y = height / 3; // Never crop more than 33% of screen
        for y in 0..max_bar_y {
            let row_offset = (y * width * 4) as usize;
            let mut row_luma_sum = 0u32;
            let mut sample_count = 0u32;

            for x in (0..width as usize).step_by(sample_step_x) {
                let pixel_offset = row_offset + (x * 4);
                if pixel_offset + 2 < data.len() {
                    let (r, g, b) = if is_bgra {
                        (data[pixel_offset + 2] as u32, data[pixel_offset + 1] as u32, data[pixel_offset] as u32)
                    } else {
                        (data[pixel_offset] as u32, data[pixel_offset + 1] as u32, data[pixel_offset + 2] as u32)
                    };
                    let luma = (r * 299 + g * 587 + b * 114) / 1000;
                    row_luma_sum += luma;
                    sample_count += 1;
                }
            }

            if sample_count > 0 && (row_luma_sum / sample_count) <= luma_threshold {
                top_bar_height = y + 1;
            } else {
                break;
            }
        }

        // Bottom bar detection
        let mut bottom_bar_height = 0u32;
        for y in 0..max_bar_y {
            let actual_y = height - 1 - y;
            let row_offset = (actual_y * width * 4) as usize;
            let mut row_luma_sum = 0u32;
            let mut sample_count = 0u32;

            for x in (0..width as usize).step_by(sample_step_x) {
                let pixel_offset = row_offset + (x * 4);
                if pixel_offset + 2 < data.len() {
                    let (r, g, b) = if is_bgra {
                        (data[pixel_offset + 2] as u32, data[pixel_offset + 1] as u32, data[pixel_offset] as u32)
                    } else {
                        (data[pixel_offset] as u32, data[pixel_offset + 1] as u32, data[pixel_offset + 2] as u32)
                    };
                    let luma = (r * 299 + g * 587 + b * 114) / 1000;
                    row_luma_sum += luma;
                    sample_count += 1;
                }
            }

            if sample_count > 0 && (row_luma_sum / sample_count) <= luma_threshold {
                bottom_bar_height = y + 1;
            } else {
                break;
            }
        }

        // Require symmetry within 6 pixels to avoid false positives from dark shadows
        let y_min = if top_bar_height >= 4 && (top_bar_height as i32 - bottom_bar_height as i32).abs() <= 6 {
            top_bar_height as f32 / height as f32
        } else {
            0.0
        };

        let y_max = if bottom_bar_height >= 4 && (top_bar_height as i32 - bottom_bar_height as i32).abs() <= 6 {
            (height - bottom_bar_height) as f32 / height as f32
        } else {
            1.0
        };

        ActiveRect {
            x_min: 0.0,
            x_max: 1.0,
            y_min,
            y_max,
        }
    }

    /// Samples frame buffer (assumed 4 bytes per pixel: BGRA or RGBA).
    /// Returns sampled channel colors and a boolean indicating whether a hard scene cut occurred.
    pub fn sample_frame(
        &mut self,
        data: &[u8],
        width: u32,
        height: u32,
        is_bgra: bool,
    ) -> (Vec<(u8, RgbColor)>, bool) {
        self.frame_count += 1;

        // Run letterbox detector periodically (every 15 frames) or on first frame
        if self.letterbox_detection && (self.frame_count == 1 || self.frame_count % 15 == 0) {
            self.active_rect = Self::detect_active_rect(data, width, height, is_bgra);
        }

        // 1. Calculate overall global average color to detect sudden scene cuts
        let mut global_r: u64 = 0;
        let mut global_g: u64 = 0;
        let mut global_b: u64 = 0;
        let mut global_count: u64 = 0;

        // Sample center 50% grid for fast global scene change detection
        let g_ystart = (height / 4) as usize;
        let g_yend = (3 * height / 4) as usize;
        let g_xstart = (width / 4) as usize;
        let g_xend = (3 * width / 4) as usize;

        for y in (g_ystart..g_yend).step_by(4) {
            let row_offset = y * (width as usize) * 4;
            for x in (g_xstart..g_xend).step_by(4) {
                let pixel_offset = row_offset + (x * 4);
                if pixel_offset + 2 < data.len() {
                    let (r, g, b) = if is_bgra {
                        (data[pixel_offset + 2] as u64, data[pixel_offset + 1] as u64, data[pixel_offset] as u64)
                    } else {
                        (data[pixel_offset] as u64, data[pixel_offset + 1] as u64, data[pixel_offset + 2] as u64)
                    };
                    global_r += r;
                    global_g += g;
                    global_b += b;
                    global_count += 1;
                }
            }
        }

        let current_global_color = if global_count > 0 {
            RgbColor::new(
                (global_r / global_count) as u8,
                (global_g / global_count) as u8,
                (global_b / global_count) as u8,
            )
        } else {
            RgbColor::new(0, 0, 0)
        };

        // If global frame difference exceeds 0.35, classify as a sudden scene cut
        let is_scene_cut = self.frame_count > 1 && current_global_color.delta(self.last_global_color) > 0.35;
        self.last_global_color = current_global_color;

        // Effective smoothing factor: snap to 1.0 (zero latency) on scene cuts, else smooth EMA
        let effective_alpha = if is_scene_cut { 1.0 } else { self.smoothing_factor };

        let mut results = Vec::with_capacity(self.zones.len());

        let act_x_min = self.active_rect.x_min;
        let act_x_range = (self.active_rect.x_max - act_x_min).max(0.1);
        let act_y_min = self.active_rect.y_min;
        let act_y_range = (self.active_rect.y_max - act_y_min).max(0.1);

        for (i, zone) in self.zones.iter().enumerate() {
            // Remap normalized coordinates to the active viewport
            let mapped_x_min = act_x_min + zone.x_min * act_x_range;
            let mapped_x_max = act_x_min + zone.x_max * act_x_range;
            let mapped_y_min = act_y_min + zone.y_min * act_y_range;
            let mapped_y_max = act_y_min + zone.y_max * act_y_range;

            let x_start = (mapped_x_min * width as f32).clamp(0.0, (width - 1) as f32) as u32;
            let x_end = (mapped_x_max * width as f32).clamp(0.0, width as f32) as u32;
            let y_start = (mapped_y_min * height as f32).clamp(0.0, (height - 1) as f32) as u32;
            let y_end = (mapped_y_max * height as f32).clamp(0.0, height as f32) as u32;

            let mut weighted_r: f32 = 0.0;
            let mut weighted_g: f32 = 0.0;
            let mut weighted_b: f32 = 0.0;
            let mut total_weight: f32 = 0.0;

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

                        let pix = RgbColor::new(r, g, b);
                        // Saturation weighting: vivid accents get higher weight so they aren't diluted by grey
                        let sat = pix.saturation();
                        let weight = 1.0 + self.saturation_boost * (sat * sat);

                        weighted_r += (r as f32) * weight;
                        weighted_g += (g as f32) * weight;
                        weighted_b += (b as f32) * weight;
                        total_weight += weight;
                    }
                }
            }

            let mut raw_color = if total_weight > 0.0 {
                RgbColor::new(
                    (weighted_r / total_weight).clamp(0.0, 255.0) as u8,
                    (weighted_g / total_weight).clamp(0.0, 255.0) as u8,
                    (weighted_b / total_weight).clamp(0.0, 255.0) as u8,
                )
            } else {
                RgbColor::new(0, 0, 0)
            };

            // Apply OLED near-black noise gate
            if self.noise_gate_threshold > 0.0 {
                raw_color = raw_color.apply_noise_gate(self.noise_gate_threshold);
            }

            // Apply Reinhard HDR tone mapping
            if self.hdr_tone_mapping {
                raw_color = raw_color.tone_map_hdr();
            }

            // Apply Adaptive EMA smoothing
            let current = self.smoothed_colors[i];
            let smoothed = current.lerp(raw_color, effective_alpha);
            self.smoothed_colors[i] = smoothed;

            results.push((zone.channel_id, smoothed));
        }

        (results, is_scene_cut)
    }

    #[allow(dead_code)]
    pub fn active_rect(&self) -> ActiveRect {
        self.active_rect
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_saturation_weighting_favors_vivid_colors() {
        let zone = LightZone {
            channel_id: 0,
            name: "Test".to_string(),
            x_min: 0.0,
            x_max: 1.0,
            y_min: 0.0,
            y_max: 1.0,
        };
        let mut sampler = ZoneSampler::new(vec![zone], 1.0, false, false, 2.0, 0.0);

        // Frame with half dull grey (100, 100, 100) and half bright red (255, 0, 0)
        let width = 4u32;
        let height = 2u32;
        let mut data = vec![0u8; (width * height * 4) as usize];
        for y in 0..height {
            for x in 0..width {
                let idx = ((y * width + x) * 4) as usize;
                if x < 2 {
                    data[idx] = 100;
                    data[idx + 1] = 100;
                    data[idx + 2] = 100;
                } else {
                    data[idx] = 255; // Red
                    data[idx + 1] = 0;
                    data[idx + 2] = 0;
                }
                data[idx + 3] = 255;
            }
        }

        let (results, _) = sampler.sample_frame(&data, width, height, false);
        let color = results[0].1;
        assert!(color.r > 190);
        assert!(color.g < 80);
    }

    #[test]
    fn test_letterbox_detection_detects_black_bars() {
        let width = 16u32;
        let height = 16u32;
        let mut data = vec![0u8; (width * height * 4) as usize];

        // Fill center rows 4..12 with bright white (y=0..3 and y=12..15 are black)
        for y in 4..12 {
            for x in 0..width {
                let idx = ((y * width + x) * 4) as usize;
                data[idx] = 200;
                data[idx + 1] = 200;
                data[idx + 2] = 200;
                data[idx + 3] = 255;
            }
        }

        let rect = ZoneSampler::detect_active_rect(&data, width, height, false);
        assert_eq!(rect.y_min, 0.25); // 4 / 16 = 0.25
        assert_eq!(rect.y_max, 0.75); // 12 / 16 = 0.75
    }

    #[test]
    fn test_noise_gate_suppresses_faint_artifacts() {
        let faint_noise = RgbColor::new(4, 4, 4);
        let gated = faint_noise.apply_noise_gate(0.02);
        assert_eq!(gated, RgbColor::new(0, 0, 0));

        let bright = RgbColor::new(100, 100, 100);
        let not_gated = bright.apply_noise_gate(0.02);
        assert_eq!(not_gated, bright);
    }
}
