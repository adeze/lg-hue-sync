use anyhow::{Context, Result};
use byteorder::{BigEndian, ByteOrder};
use std::net::{SocketAddr, UdpSocket};
use tracing::error;

use crate::{
    color::{ActiveRect, RgbColor},
    config::{NanoleafAlignment, NanoleafStartCorner},
};

/// Manages high-speed binary UDP streaming to Nanoleaf 4D on port 60222 (extControl v2 protocol)
pub struct NanoleafUdpStreamer {
    socket: UdpSocket,
    target_addr: SocketAddr,
    panel_ids: Vec<u16>,
    packet_buf: Vec<u8>,
}

impl NanoleafUdpStreamer {
    pub fn new(target_ip: &str, target_port: u16, panel_ids: Vec<u16>) -> Result<Self> {
        let socket = UdpSocket::bind("0.0.0.0:0")
            .context("Failed to bind UDP socket for Nanoleaf streaming")?;
        socket.set_nonblocking(true)?;

        let target_addr: SocketAddr = format!("{}:{}", target_ip, target_port)
            .parse()
            .with_context(|| {
                format!(
                    "Invalid Nanoleaf target address: {}:{}",
                    target_ip, target_port
                )
            })?;

        socket.connect(target_addr)?;

        let n_panels = panel_ids.len();
        // v2 packet: 2 bytes header (nPanels) + 8 bytes per panel (id:2, r:1, g:1, b:1, w:1, trans:2)
        let packet_size = 2 + n_panels * 8;
        let mut packet_buf = vec![0u8; packet_size];
        BigEndian::write_u16(&mut packet_buf[0..2], n_panels as u16);

        // Pre-populate panel IDs in packet buffer
        for (i, &pid) in panel_ids.iter().enumerate() {
            let offset = 2 + i * 8;
            BigEndian::write_u16(&mut packet_buf[offset..offset + 2], pid);
        }

        Ok(Self {
            socket,
            target_addr,
            panel_ids,
            packet_buf,
        })
    }

    pub fn panel_count(&self) -> usize {
        self.panel_ids.len()
    }

    pub fn panel_ids(&self) -> &[u16] {
        &self.panel_ids
    }

    /// Packs and transmits an RGB frame to the Nanoleaf controller.
    /// `transition_time` is in units of 100ms (0 = instant update).
    pub fn send_frame(&mut self, colors: &[RgbColor], transition_time: u16) -> Result<()> {
        let count = colors.len().min(self.panel_ids.len());

        for (i, c) in colors.iter().enumerate().take(count) {
            let offset = 2 + i * 8;
            self.packet_buf[offset + 2] = c.r;
            self.packet_buf[offset + 3] = c.g;
            self.packet_buf[offset + 4] = c.b;
            self.packet_buf[offset + 5] = 0; // White channel
            BigEndian::write_u16(
                &mut self.packet_buf[offset + 6..offset + 8],
                transition_time,
            );
        }

        match self.socket.send(&self.packet_buf) {
            Ok(_) => Ok(()),
            Err(e)
                if e.kind() == std::io::ErrorKind::WouldBlock
                    || e.kind() == std::io::ErrorKind::Interrupted =>
            {
                // Drop frame under transient socket buffer pressure; not a session error
                Ok(())
            }
            Err(e) => {
                error!(
                    "Failed to send Nanoleaf UDP frame to {}: {}",
                    self.target_addr, e
                );
                Err(e.into())
            }
        }
    }
}

/// Normalized sampling rectangle along the TV perimeter
#[derive(Debug, Clone)]
pub struct PerimeterZone {
    pub panel_id: u16,
    pub x_min: f32,
    pub x_max: f32,
    pub y_min: f32,
    pub y_max: f32,
}

/// Computes sampling coordinates and smooths colors around the TV border for Nanoleaf 4D lightstrip
pub struct NanoleafPerimeterSampler {
    base_zones: Vec<PerimeterZone>,
    zones: Vec<PerimeterZone>,
    smoothed_colors: Vec<RgbColor>,
    active_rect: Option<ActiveRect>,
    rise_smoothing_factor: f32,
    fall_smoothing_factor: f32,
    hdr_tone_mapping: bool,
    saturation_boost: f32,
    noise_gate_threshold: f32,
    brightness_multiplier: f32,
    peak_weight: f32,
    gamma: f32,
    max_color_step: u8,
    strict_blackout: bool,
}

fn limit_color_step(current: RgbColor, target: RgbColor, max_step: u8) -> RgbColor {
    let limit = max_step as i16;
    let cap = |from: u8, to: u8| (to as i16 - from as i16).clamp(-limit, limit) + from as i16;
    RgbColor::new(
        cap(current.r, target.r) as u8,
        cap(current.g, target.g) as u8,
        cap(current.b, target.b) as u8,
    )
}

impl NanoleafPerimeterSampler {
    /// Generates perimeter sampling zones around the 16:9 TV screen.
    /// Uses exact hardware coordinates from Nanoleaf 4D (NL69) controller:
    /// Starts at bottom-center (panel 0) and routes counter-clockwise:
    /// Bottom-Left (0..7) -> Up Left (8..15) -> Across Top (16..28) -> Down Right (29..35) -> Across Bottom (36..39).
    #[cfg(test)]
    pub fn new(
        segment_count: u16,
        panel_ids: &[u16],
        hdr_tone_mapping: bool,
        saturation_boost: f32,
        noise_gate_threshold: f32,
        brightness_multiplier: f32,
    ) -> Self {
        Self::new_aligned(
            segment_count,
            panel_ids,
            hdr_tone_mapping,
            saturation_boost,
            noise_gate_threshold,
            brightness_multiplier,
            NanoleafAlignment::default(),
        )
    }

    pub fn new_aligned(
        segment_count: u16,
        panel_ids: &[u16],
        hdr_tone_mapping: bool,
        saturation_boost: f32,
        noise_gate_threshold: f32,
        brightness_multiplier: f32,
        alignment: NanoleafAlignment,
    ) -> Self {
        let border_depth = 0.08f32; // Sample outer 8% edge of screen
        let base_zones = if segment_count == 40 {
            Self::build_nanoleaf_4d_40_zones(panel_ids, border_depth)
        } else {
            Self::build_generic_perimeter_zones(segment_count, panel_ids, border_depth)
        };

        let mut sampler = Self {
            base_zones,
            zones: Vec::new(),
            smoothed_colors: Vec::new(),
            active_rect: None,
            rise_smoothing_factor: 0.35,
            fall_smoothing_factor: 0.35,
            hdr_tone_mapping,
            saturation_boost,
            noise_gate_threshold,
            brightness_multiplier,
            peak_weight: 0.35,
            gamma: 1.0,
            max_color_step: 12,
            strict_blackout: false,
        };
        sampler.set_alignment(alignment);
        sampler
    }

    pub fn set_alignment(&mut self, alignment: NanoleafAlignment) {
        let len = self.base_zones.len();
        if len == 0 {
            return;
        }
        let corner_index = match alignment.start_corner {
            NanoleafStartCorner::BottomCenter => 0,
            NanoleafStartCorner::BottomLeft => 7,
            NanoleafStartCorner::TopLeft => 15,
            NanoleafStartCorner::TopRight => 28,
            NanoleafStartCorner::BottomRight => 35,
        };
        let start = if len == 40 {
            corner_index
        } else {
            corner_index * len / 40
        };
        let direction = if alignment.reverse_direction { -1 } else { 1 };
        let offset = alignment.perimeter_offset as isize;
        self.zones = (0..len)
            .map(|index| {
                let source = (start as isize + offset + direction * index as isize)
                    .rem_euclid(len as isize) as usize;
                let mut zone = self.base_zones[index].clone();
                zone.panel_id = self.base_zones[source].panel_id;
                zone
            })
            .collect();
        self.smoothed_colors = vec![RgbColor::new(0, 0, 0); len];
    }

    /// Builds 40 dedicated perimeter sampling zones calibrated to the exact physical
    /// (x, y) panel coordinates reported by the Nanoleaf 4D NL69 hardware controller.
    fn build_nanoleaf_4d_40_zones(panel_ids: &[u16], border_depth: f32) -> Vec<PerimeterZone> {
        // Physical coordinates from Nanoleaf 4D controller:
        // x in 0..650, y in 0..400.
        // y=400 is TOP, y=0 is BOTTOM, x=0 is LEFT, x=650 is RIGHT.
        const RAW_PANELS: [(u16, f32, f32); 40] = [
            (0, 350.0, 0.0),
            (1, 300.0, 0.0),
            (2, 250.0, 0.0),
            (3, 200.0, 0.0),
            (4, 150.0, 0.0),
            (5, 100.0, 0.0),
            (6, 50.0, 0.0),
            (7, 0.0, 0.0), // Corner: Bottom-Left
            (8, 0.0, 50.0),
            (9, 0.0, 100.0),
            (10, 0.0, 150.0),
            (11, 0.0, 200.0),
            (12, 0.0, 250.0),
            (13, 0.0, 300.0),
            (14, 0.0, 350.0),
            (15, 0.0, 400.0), // Corner: Top-Left
            (16, 50.0, 400.0),
            (17, 100.0, 400.0),
            (18, 150.0, 400.0),
            (19, 200.0, 400.0),
            (20, 250.0, 400.0),
            (21, 300.0, 400.0),
            (22, 350.0, 400.0),
            (23, 400.0, 400.0),
            (24, 450.0, 400.0),
            (25, 500.0, 400.0),
            (26, 550.0, 400.0),
            (27, 600.0, 400.0),
            (28, 650.0, 400.0), // Corner: Top-Right
            (29, 650.0, 350.0),
            (30, 650.0, 300.0),
            (31, 650.0, 250.0),
            (32, 650.0, 200.0),
            (33, 650.0, 150.0),
            (34, 650.0, 100.0),
            (35, 650.0, 50.0), // Corner: Bottom-Right
            (36, 600.0, 50.0),
            (37, 550.0, 50.0),
            (38, 500.0, 50.0),
            (39, 450.0, 50.0),
        ];

        let corner_size = border_depth * 1.5;
        let x_span = 50.0 / 650.0;
        let y_span = 50.0 / 400.0;

        let mut zones = Vec::with_capacity(40);
        for (index, &(_, x_raw, y_raw)) in RAW_PANELS.iter().enumerate() {
            let panel_id = panel_ids.get(index).copied().unwrap_or(index as u16);
            let zone = if index == 7 {
                // Bottom-Left corner
                PerimeterZone {
                    panel_id,
                    x_min: 0.0,
                    x_max: corner_size,
                    y_min: 1.0 - corner_size,
                    y_max: 1.0,
                }
            } else if index == 15 {
                // Top-Left corner
                PerimeterZone {
                    panel_id,
                    x_min: 0.0,
                    x_max: corner_size,
                    y_min: 0.0,
                    y_max: corner_size,
                }
            } else if index == 28 {
                // Top-Right corner
                PerimeterZone {
                    panel_id,
                    x_min: 1.0 - corner_size,
                    x_max: 1.0,
                    y_min: 0.0,
                    y_max: corner_size,
                }
            } else if index == 35 {
                // Bottom-Right corner
                PerimeterZone {
                    panel_id,
                    x_min: 1.0 - corner_size,
                    x_max: 1.0,
                    y_min: 1.0 - corner_size,
                    y_max: 1.0,
                }
            } else if x_raw == 0.0 {
                // Left edge (moving up)
                let cy = 1.0 - (y_raw / 400.0);
                PerimeterZone {
                    panel_id,
                    x_min: 0.0,
                    x_max: border_depth,
                    y_min: (cy - y_span * 0.7).clamp(0.0, 1.0),
                    y_max: (cy + y_span * 0.7).clamp(0.0, 1.0),
                }
            } else if y_raw >= 390.0 {
                // Top edge (moving right)
                let cx = x_raw / 650.0;
                PerimeterZone {
                    panel_id,
                    x_min: (cx - x_span * 0.7).clamp(0.0, 1.0),
                    x_max: (cx + x_span * 0.7).clamp(0.0, 1.0),
                    y_min: 0.0,
                    y_max: border_depth,
                }
            } else if x_raw >= 640.0 {
                // Right edge (moving down)
                let cy = 1.0 - (y_raw / 400.0);
                PerimeterZone {
                    panel_id,
                    x_min: 1.0 - border_depth,
                    x_max: 1.0,
                    y_min: (cy - y_span * 0.7).clamp(0.0, 1.0),
                    y_max: (cy + y_span * 0.7).clamp(0.0, 1.0),
                }
            } else {
                // Bottom edge (panels 0..6 and 36..39)
                let cx = x_raw / 650.0;
                PerimeterZone {
                    panel_id,
                    x_min: (cx - x_span * 0.7).clamp(0.0, 1.0),
                    x_max: (cx + x_span * 0.7).clamp(0.0, 1.0),
                    y_min: 1.0 - border_depth,
                    y_max: 1.0,
                }
            };
            zones.push(zone);
        }
        zones
    }

    /// Generic perimeter zone divider for non-standard segment counts
    fn build_generic_perimeter_zones(
        segment_count: u16,
        panel_ids: &[u16],
        border_depth: f32,
    ) -> Vec<PerimeterZone> {
        let n = segment_count.max(4) as usize;
        let mut resolved_ids = Vec::with_capacity(n);
        if panel_ids.len() >= n {
            resolved_ids.extend_from_slice(&panel_ids[0..n]);
        } else {
            for i in 0..n {
                resolved_ids.push(panel_ids.get(i).copied().unwrap_or(i as u16));
            }
        }

        let total_units = 50.0f32;
        let left_count = ((9.0 / total_units) * n as f32).round().max(1.0) as usize;
        let top_count = ((16.0 / total_units) * n as f32).round().max(1.0) as usize;
        let right_count = ((9.0 / total_units) * n as f32).round().max(1.0) as usize;
        let bottom_count = n
            .saturating_sub(left_count + top_count + right_count)
            .max(1);

        let mut zones = Vec::with_capacity(n);
        let mut id_idx = 0;

        // Bottom left half
        let b_half = bottom_count / 2;
        for i in 0..b_half {
            let x_start = 0.5 - (i as f32 * 0.5 / b_half as f32);
            let x_end = 0.5 - ((i + 1) as f32 * 0.5 / b_half as f32);
            zones.push(PerimeterZone {
                panel_id: resolved_ids[id_idx],
                x_min: x_end.min(x_start),
                x_max: x_end.max(x_start),
                y_min: 1.0 - border_depth,
                y_max: 1.0,
            });
            id_idx += 1;
        }

        // Left edge
        for i in 0..left_count {
            let y_start = 1.0 - (i as f32) / (left_count as f32);
            let y_end = 1.0 - ((i + 1) as f32) / (left_count as f32);
            zones.push(PerimeterZone {
                panel_id: resolved_ids[id_idx],
                x_min: 0.0,
                x_max: border_depth,
                y_min: y_end.min(y_start),
                y_max: y_end.max(y_start),
            });
            id_idx += 1;
        }

        // Top edge
        for i in 0..top_count {
            let x_start = (i as f32) / (top_count as f32);
            let x_end = ((i + 1) as f32) / (top_count as f32);
            zones.push(PerimeterZone {
                panel_id: resolved_ids[id_idx],
                x_min: x_start,
                x_max: x_end,
                y_min: 0.0,
                y_max: border_depth,
            });
            id_idx += 1;
        }

        // Right edge
        for i in 0..right_count {
            let y_start = (i as f32) / (right_count as f32);
            let y_end = ((i + 1) as f32) / (right_count as f32);
            zones.push(PerimeterZone {
                panel_id: resolved_ids[id_idx],
                x_min: 1.0 - border_depth,
                x_max: 1.0,
                y_min: y_start,
                y_max: y_end,
            });
            id_idx += 1;
        }

        // Bottom right half
        let b_rem = bottom_count.saturating_sub(b_half);
        for i in 0..b_rem {
            let x_start = 1.0 - (i as f32 * 0.5 / b_rem as f32);
            let x_end = 1.0 - ((i + 1) as f32 * 0.5 / b_rem as f32);
            zones.push(PerimeterZone {
                panel_id: resolved_ids[id_idx],
                x_min: x_end.min(x_start),
                x_max: x_end.max(x_start),
                y_min: 1.0 - border_depth,
                y_max: 1.0,
            });
            id_idx += 1;
        }

        zones
    }

    pub fn panel_ids(&self) -> Vec<u16> {
        self.zones.iter().map(|z| z.panel_id).collect()
    }

    pub fn set_smoothing_factor(&mut self, factor: f32) {
        let factor = factor.clamp(0.05, 1.0);
        self.rise_smoothing_factor = factor;
        self.fall_smoothing_factor = factor;
    }

    pub fn set_temporal_response(&mut self, rise: f32, fall: f32) {
        self.rise_smoothing_factor = rise.clamp(0.05, 1.0);
        self.fall_smoothing_factor = fall.clamp(0.05, 1.0);
    }

    pub fn set_strict_blackout(&mut self, enabled: bool) {
        self.strict_blackout = enabled;
    }

    pub fn set_brightness_multiplier(&mut self, mult: f32) {
        self.brightness_multiplier = mult.clamp(0.1, 4.0);
    }

    pub fn set_saturation_boost(&mut self, boost: f32) {
        self.saturation_boost = boost.clamp(1.0, 3.0);
    }

    pub fn set_hdr_tone_mapping(&mut self, enabled: bool) {
        self.hdr_tone_mapping = enabled;
    }

    pub fn set_peak_weight(&mut self, weight: f32) {
        self.peak_weight = weight.clamp(0.0, 1.0);
    }

    pub fn set_gamma(&mut self, gamma: f32) {
        self.gamma = gamma.clamp(0.5, 3.0);
    }

    pub fn set_max_color_step(&mut self, step: u8) {
        self.max_color_step = step.max(1);
    }

    pub fn set_noise_gate_threshold(&mut self, threshold: f32) {
        self.noise_gate_threshold = threshold.clamp(0.0, 0.1);
    }

    /// Sets the active illuminated screen area to compensate for widescreen black letterbox bars.
    pub fn set_active_rect(&mut self, rect: ActiveRect) {
        self.active_rect = Some(rect);
    }

    /// Samples colors for all perimeter zones from the downscaled frame buffer.
    pub fn sample_frame(
        &mut self,
        frame_data: &[u8],
        width: u32,
        height: u32,
        is_bgra: bool,
        is_scene_cut: bool,
    ) -> Vec<RgbColor> {
        let (act_x_min, act_x_max, act_y_min, act_y_max) = match self.active_rect {
            Some(rect) => (rect.x_min, rect.x_max, rect.y_min, rect.y_max),
            None => (0.0, 1.0, 0.0, 1.0),
        };
        let act_x_range = (act_x_max - act_x_min).max(0.1);
        let act_y_range = (act_y_max - act_y_min).max(0.1);

        let mut result = Vec::with_capacity(self.zones.len());

        for (i, zone) in self.zones.iter().enumerate() {
            // Remap normalized coordinates to active letterbox area
            let mapped_x_min = act_x_min + zone.x_min * act_x_range;
            let mapped_x_max = act_x_min + zone.x_max * act_x_range;
            let mapped_y_min = act_y_min + zone.y_min * act_y_range;
            let mapped_y_max = act_y_min + zone.y_max * act_y_range;

            let x_start = ((mapped_x_min * width as f32) as u32).min(width - 1);
            let x_end = ((mapped_x_max * width as f32) as u32)
                .min(width)
                .max(x_start + 1);
            let y_start = ((mapped_y_min * height as f32) as u32).min(height - 1);
            let y_end = ((mapped_y_max * height as f32) as u32)
                .min(height)
                .max(y_start + 1);

            let mut weighted_r = 0.0f32;
            let mut weighted_g = 0.0f32;
            let mut weighted_b = 0.0f32;
            let mut total_weight = 0.0f32;
            let mut peak_pixel = RgbColor::new(0, 0, 0);
            let mut max_luma = 0.0f32;

            // 2x2 spatial subsampling for zero-lag 60 FPS performance
            for y in (y_start..y_end).step_by(2) {
                let row_offset = (y * width * 4) as usize;
                for x in (x_start..x_end).step_by(2) {
                    let idx = row_offset + (x * 4) as usize;
                    if idx + 2 >= frame_data.len() {
                        continue;
                    }

                    let (r_raw, g_raw, b_raw) = if is_bgra {
                        (frame_data[idx + 2], frame_data[idx + 1], frame_data[idx])
                    } else {
                        (frame_data[idx], frame_data[idx + 1], frame_data[idx + 2])
                    };

                    let pix = RgbColor::new(r_raw, g_raw, b_raw);
                    let luma = pix.luminance();
                    if luma > max_luma {
                        max_luma = luma;
                        peak_pixel = pix;
                    }

                    let sat = pix.saturation();
                    let weight = 1.0 + self.saturation_boost * (sat * sat);

                    weighted_r += (r_raw as f32) * weight;
                    weighted_g += (g_raw as f32) * weight;
                    weighted_b += (b_raw as f32) * weight;
                    total_weight += weight;
                }
            }

            let mean_color = if total_weight > 0.0 {
                RgbColor::new(
                    (weighted_r / total_weight).clamp(0.0, 255.0) as u8,
                    (weighted_g / total_weight).clamp(0.0, 255.0) as u8,
                    (weighted_b / total_weight).clamp(0.0, 255.0) as u8,
                )
            } else {
                RgbColor::new(0, 0, 0)
            };

            // Blend mean with peak highlight luminance
            let mut processed_color = if self.peak_weight > 0.0 && max_luma > 0.0 {
                mean_color.lerp(peak_pixel, self.peak_weight)
            } else {
                mean_color
            };

            // OLED near-black noise gate
            if self.noise_gate_threshold > 0.0 {
                processed_color = processed_color.apply_noise_gate(self.noise_gate_threshold);
            }

            // True HSV saturation amplification
            if self.saturation_boost > 1.0 {
                processed_color = processed_color.boost_saturation(self.saturation_boost);
            }

            // Dynamic gamma contrast curve
            if (self.gamma - 1.0).abs() >= 0.01 {
                processed_color = processed_color.apply_gamma(self.gamma);
            }

            // Reinhard HDR tone mapping
            if self.hdr_tone_mapping {
                processed_color = processed_color.tone_map_hdr();
            }

            // EMA temporal smoothing
            let current = self.smoothed_colors[i];
            let smoothed = if self.strict_blackout && processed_color == RgbColor::new(0, 0, 0) {
                processed_color
            } else {
                let alpha = if is_scene_cut {
                    1.0
                } else if processed_color.luminance() < current.luminance() {
                    self.fall_smoothing_factor
                } else {
                    self.rise_smoothing_factor
                };
                limit_color_step(
                    current,
                    current.lerp(processed_color, alpha),
                    self.max_color_step,
                )
            };
            self.smoothed_colors[i] = smoothed;

            // Apply brightness multiplier
            let final_r =
                ((smoothed.r as f32 * self.brightness_multiplier).clamp(0.0, 255.0)) as u8;
            let final_g =
                ((smoothed.g as f32 * self.brightness_multiplier).clamp(0.0, 255.0)) as u8;
            let final_b =
                ((smoothed.b as f32 * self.brightness_multiplier).clamp(0.0, 255.0)) as u8;

            result.push(RgbColor::new(final_r, final_g, final_b));
        }

        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_nanoleaf_perimeter_zones_distribution() {
        let panel_ids: Vec<u16> = (1..=30).collect();
        let sampler = NanoleafPerimeterSampler::new(30, &panel_ids, false, 1.5, 0.02, 1.0);

        assert_eq!(sampler.zones.len(), 30);
        assert_eq!(sampler.panel_ids().len(), 30);

        for (i, zone) in sampler.zones.iter().enumerate() {
            assert!(
                zone.x_min >= 0.0 && zone.x_min <= 1.0,
                "Zone {} x_min out of range",
                i
            );
            assert!(
                zone.x_max >= 0.0 && zone.x_max <= 1.0,
                "Zone {} x_max out of range",
                i
            );
            assert!(
                zone.y_min >= 0.0 && zone.y_min <= 1.0,
                "Zone {} y_min out of range",
                i
            );
            assert!(
                zone.y_max >= 0.0 && zone.y_max <= 1.0,
                "Zone {} y_max out of range",
                i
            );
            assert!(zone.x_max >= zone.x_min, "Zone {} x_max < x_min", i);
            assert!(zone.y_max >= zone.y_min, "Zone {} y_max < y_min", i);
        }
    }

    #[test]
    fn strict_blackout_turns_off_gated_segments() {
        let panel_ids = vec![1, 2, 3, 4];
        let mut sampler = NanoleafPerimeterSampler::new(4, &panel_ids, false, 1.0, 0.02, 1.0);
        sampler.set_strict_blackout(true);
        sampler.sample_frame(&vec![255; 64], 4, 4, false, false);

        let colors = sampler.sample_frame(&vec![0; 64], 4, 4, false, false);
        assert!(colors.iter().all(|color| *color == RgbColor::new(0, 0, 0)));
    }

    #[test]
    fn test_nanoleaf_v2_packet_wire_format() {
        let panel_ids = vec![101u16, 202u16];
        // Connect to a local loopback port for test
        let mut streamer = NanoleafUdpStreamer::new("127.0.0.1", 60222, panel_ids).unwrap();

        let colors = vec![RgbColor::new(255, 128, 64), RgbColor::new(10, 20, 30)];

        streamer.send_frame(&colors, 5).unwrap();

        let buf = &streamer.packet_buf;
        // Total size: 2 + 2 * 8 = 18 bytes
        assert_eq!(buf.len(), 18);

        // Header: nPanels = 2 (u16 Big Endian)
        assert_eq!(BigEndian::read_u16(&buf[0..2]), 2);

        // Panel 1: ID = 101, R=255, G=128, B=64, W=0, Transition=5 (0.5s)
        assert_eq!(BigEndian::read_u16(&buf[2..4]), 101);
        assert_eq!(buf[4], 255);
        assert_eq!(buf[5], 128);
        assert_eq!(buf[6], 64);
        assert_eq!(buf[7], 0);
        assert_eq!(BigEndian::read_u16(&buf[8..10]), 5);

        // Panel 2: ID = 202, R=10, G=20, B=30, W=0, Transition=5
        assert_eq!(BigEndian::read_u16(&buf[10..12]), 202);
        assert_eq!(buf[12], 10);
        assert_eq!(buf[13], 20);
        assert_eq!(buf[14], 30);
        assert_eq!(buf[15], 0);
        assert_eq!(BigEndian::read_u16(&buf[16..18]), 5);
    }

    #[test]
    fn test_nanoleaf_scene_cut_respects_limiter() {
        let panel_ids = vec![1u16, 2u16, 3u16, 4u16];
        let mut sampler = NanoleafPerimeterSampler::new(4, &panel_ids, false, 1.0, 0.0, 1.0);

        // Synthetic 4x4 frame of bright red (RGBA)
        let red_frame = vec![255, 0, 0, 255].repeat(16);

        // Scene cuts bypass EMA but must still respect the flash-safety limiter.
        let colors = sampler.sample_frame(&red_frame, 4, 4, false, true);
        assert_eq!(colors.len(), 4);
        for c in &colors {
            assert_eq!(c.r, 12);
            assert_eq!(c.g, 0);
            assert_eq!(c.b, 0);
        }
    }

    #[test]
    fn test_nanoleaf_4d_40_zones() {
        let panel_ids: Vec<u16> = (0..40).collect();
        let mut sampler = NanoleafPerimeterSampler::new(40, &panel_ids, false, 1.0, 0.0, 1.0);

        assert_eq!(sampler.zones.len(), 40);
        assert_eq!(sampler.panel_ids().len(), 40);

        // Check corner panels
        let corner_bl = &sampler.zones[7];
        assert_eq!(corner_bl.panel_id, 7);
        assert_eq!(corner_bl.x_min, 0.0);
        assert_eq!(corner_bl.y_max, 1.0);

        let corner_tl = &sampler.zones[15];
        assert_eq!(corner_tl.panel_id, 15);
        assert_eq!(corner_tl.x_min, 0.0);
        assert_eq!(corner_tl.y_min, 0.0);

        let corner_tr = &sampler.zones[28];
        assert_eq!(corner_tr.panel_id, 28);
        assert_eq!(corner_tr.x_max, 1.0);
        assert_eq!(corner_tr.y_min, 0.0);

        let corner_br = &sampler.zones[35];
        assert_eq!(corner_br.panel_id, 35);
        assert_eq!(corner_br.x_max, 1.0);
        assert_eq!(corner_br.y_max, 1.0);

        // Test letterbox active rect compensation
        sampler.set_active_rect(ActiveRect {
            x_min: 0.0,
            x_max: 1.0,
            y_min: 0.125,
            y_max: 0.875,
        });
        sampler.set_max_color_step(255);

        // Synthetic 4x4 frame with blue color
        let blue_frame = vec![0, 0, 255, 255].repeat(16);
        let colors = sampler.sample_frame(&blue_frame, 4, 4, false, true);
        assert_eq!(colors.len(), 40);
        for c in &colors {
            assert_eq!(c.b, 255);
        }
    }

    #[test]
    fn test_nanoleaf_4d_40_zones_preserve_panel_ids() {
        let panel_ids: Vec<u16> = (100..140).collect();
        let sampler = NanoleafPerimeterSampler::new(40, &panel_ids, false, 1.0, 0.0, 1.0);

        assert_eq!(
            sampler
                .zones
                .iter()
                .map(|zone| zone.panel_id)
                .collect::<Vec<_>>(),
            panel_ids
        );
    }

    #[test]
    fn alignment_remaps_panel_ids_without_moving_sampling_zones() {
        let panel_ids: Vec<u16> = (0..40).collect();
        let mut sampler = NanoleafPerimeterSampler::new(40, &panel_ids, false, 1.0, 0.0, 1.0);
        let original_zones = sampler.zones.clone();

        sampler.set_alignment(NanoleafAlignment {
            start_corner: NanoleafStartCorner::BottomLeft,
            reverse_direction: true,
            perimeter_offset: 2,
        });

        assert_eq!(sampler.panel_ids()[0], 9);
        assert_eq!(sampler.panel_ids()[1], 8);
        for (aligned, original) in sampler.zones.iter().zip(original_zones) {
            assert_eq!(aligned.x_min, original.x_min);
            assert_eq!(aligned.x_max, original.x_max);
            assert_eq!(aligned.y_min, original.y_min);
            assert_eq!(aligned.y_max, original.y_max);
        }
    }
}
