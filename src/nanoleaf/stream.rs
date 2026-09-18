use anyhow::{Context, Result};
use byteorder::{BigEndian, ByteOrder};
use std::net::{SocketAddr, UdpSocket};
use tracing::error;

use crate::color::RgbColor;

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
            .with_context(|| format!("Invalid Nanoleaf target address: {}:{}", target_ip, target_port))?;

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

    /// Packs and transmits an RGB frame to the Nanoleaf controller.
    /// `transition_time` is in units of 100ms (0 = instant update).
    pub fn send_frame(&mut self, colors: &[RgbColor], transition_time: u16) -> Result<()> {
        let count = colors.len().min(self.panel_ids.len());

        for i in 0..count {
            let offset = 2 + i * 8;
            let c = colors[i];
            self.packet_buf[offset + 2] = c.r;
            self.packet_buf[offset + 3] = c.g;
            self.packet_buf[offset + 4] = c.b;
            self.packet_buf[offset + 5] = 0; // White channel
            BigEndian::write_u16(&mut self.packet_buf[offset + 6..offset + 8], transition_time);
        }

        if let Err(e) = self.socket.send(&self.packet_buf) {
            error!("Failed to send Nanoleaf UDP frame to {}: {}", self.target_addr, e);
        }

        Ok(())
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
    zones: Vec<PerimeterZone>,
    smoothed_colors: Vec<RgbColor>,
    smoothing_factor: f32,
    hdr_tone_mapping: bool,
    saturation_boost: f32,
    noise_gate_threshold: f32,
    brightness_multiplier: f32,
}

impl NanoleafPerimeterSampler {
    /// Generates perimeter sampling zones around the 16:9 TV screen.
    /// Follows standard 4D strip routing: Bottom-Left -> Up Left -> Across Top -> Down Right -> Across Bottom.
    pub fn new(
        segment_count: u16,
        panel_ids: &[u16],
        hdr_tone_mapping: bool,
        saturation_boost: f32,
        noise_gate_threshold: f32,
        brightness_multiplier: f32,
    ) -> Self {
        let n = segment_count.max(4) as usize;
        let mut resolved_ids = Vec::with_capacity(n);
        if panel_ids.len() >= n {
            resolved_ids.extend_from_slice(&panel_ids[0..n]);
        } else {
            for i in 0..n {
                resolved_ids.push(panel_ids.get(i).copied().unwrap_or((i + 1) as u16));
            }
        }

        // Divide perimeter based on 16:9 aspect ratio (Top:16, Bottom:16, Left:9, Right:9 => total 50)
        let total_units = 50.0f32;
        let left_count = ((9.0 / total_units) * n as f32).round().max(1.0) as usize;
        let top_count = ((16.0 / total_units) * n as f32).round().max(1.0) as usize;
        let right_count = ((9.0 / total_units) * n as f32).round().max(1.0) as usize;
        let bottom_count = n.saturating_sub(left_count + top_count + right_count).max(1);

        let mut zones = Vec::with_capacity(n);
        let border_depth = 0.08f32; // Sample outer 8% edge of screen
        let mut id_idx = 0;

        // 1. Left edge (bottom to top): Y from 1.0 down to 0.0
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

        // 2. Top edge (left to right): X from 0.0 to 1.0
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

        // 3. Right edge (top to bottom): Y from 0.0 to 1.0
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

        // 4. Bottom edge (right to left): X from 1.0 down to 0.0
        for i in 0..bottom_count {
            let x_start = 1.0 - (i as f32) / (bottom_count as f32);
            let x_end = 1.0 - ((i + 1) as f32) / (bottom_count as f32);
            zones.push(PerimeterZone {
                panel_id: resolved_ids[id_idx],
                x_min: x_end.min(x_start),
                x_max: x_end.max(x_start),
                y_min: 1.0 - border_depth,
                y_max: 1.0,
            });
            id_idx += 1;
        }

        let smoothed_colors = vec![RgbColor::new(0, 0, 0); zones.len()];

        Self {
            zones,
            smoothed_colors,
            smoothing_factor: 0.35,
            hdr_tone_mapping,
            saturation_boost,
            noise_gate_threshold,
            brightness_multiplier,
        }
    }

    pub fn panel_ids(&self) -> Vec<u16> {
        self.zones.iter().map(|z| z.panel_id).collect()
    }

    pub fn set_smoothing_factor(&mut self, factor: f32) {
        self.smoothing_factor = factor.clamp(0.05, 1.0);
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
        let alpha = if is_scene_cut { 1.0 } else { self.smoothing_factor };
        let mut result = Vec::with_capacity(self.zones.len());

        for (i, zone) in self.zones.iter().enumerate() {
            let x_start = ((zone.x_min * width as f32) as u32).min(width - 1);
            let x_end = ((zone.x_max * width as f32) as u32).min(width).max(x_start + 1);
            let y_start = ((zone.y_min * height as f32) as u32).min(height - 1);
            let y_end = ((zone.y_max * height as f32) as u32).min(height).max(y_start + 1);

            let mut weighted_r = 0.0f32;
            let mut weighted_g = 0.0f32;
            let mut weighted_b = 0.0f32;
            let mut total_weight = 0.0f32;

            for y in y_start..y_end {
                for x in x_start..x_end {
                    let idx = ((y * width + x) * 4) as usize;
                    if idx + 3 >= frame_data.len() {
                        continue;
                    }

                    let (r_raw, g_raw, b_raw) = if is_bgra {
                        (frame_data[idx + 2], frame_data[idx + 1], frame_data[idx])
                    } else {
                        (frame_data[idx], frame_data[idx + 1], frame_data[idx + 2])
                    };

                    let pix = RgbColor::new(r_raw, g_raw, b_raw);
                    let sat = pix.saturation();
                    let weight = 1.0 + self.saturation_boost * (sat * sat);

                    weighted_r += (r_raw as f32) * weight;
                    weighted_g += (g_raw as f32) * weight;
                    weighted_b += (b_raw as f32) * weight;
                    total_weight += weight;
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

            // OLED near-black noise gate
            if self.noise_gate_threshold > 0.0 {
                raw_color = raw_color.apply_noise_gate(self.noise_gate_threshold);
            }

            // Reinhard HDR tone mapping
            if self.hdr_tone_mapping {
                raw_color = raw_color.tone_map_hdr();
            }

            // EMA temporal smoothing
            let current = self.smoothed_colors[i];
            let smoothed = current.lerp(raw_color, alpha);
            self.smoothed_colors[i] = smoothed;

            // Apply brightness multiplier
            let final_r = ((smoothed.r as f32 * self.brightness_multiplier).clamp(0.0, 255.0)) as u8;
            let final_g = ((smoothed.g as f32 * self.brightness_multiplier).clamp(0.0, 255.0)) as u8;
            let final_b = ((smoothed.b as f32 * self.brightness_multiplier).clamp(0.0, 255.0)) as u8;

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
        let sampler = NanoleafPerimeterSampler::new(
            30,
            &panel_ids,
            false,
            1.5,
            0.02,
            1.0,
        );

        assert_eq!(sampler.zones.len(), 30);
        assert_eq!(sampler.panel_ids().len(), 30);

        for (i, zone) in sampler.zones.iter().enumerate() {
            assert!(zone.x_min >= 0.0 && zone.x_min <= 1.0, "Zone {} x_min out of range", i);
            assert!(zone.x_max >= 0.0 && zone.x_max <= 1.0, "Zone {} x_max out of range", i);
            assert!(zone.y_min >= 0.0 && zone.y_min <= 1.0, "Zone {} y_min out of range", i);
            assert!(zone.y_max >= 0.0 && zone.y_max <= 1.0, "Zone {} y_max out of range", i);
            assert!(zone.x_max >= zone.x_min, "Zone {} x_max < x_min", i);
            assert!(zone.y_max >= zone.y_min, "Zone {} y_max < y_min", i);
        }
    }

    #[test]
    fn test_nanoleaf_v2_packet_wire_format() {
        let panel_ids = vec![101u16, 202u16];
        // Connect to a local loopback port for test
        let mut streamer = NanoleafUdpStreamer::new("127.0.0.1", 60222, panel_ids).unwrap();

        let colors = vec![
            RgbColor::new(255, 128, 64),
            RgbColor::new(10, 20, 30),
        ];

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
    fn test_nanoleaf_scene_cut_snaps_instantly() {
        let panel_ids = vec![1u16, 2u16, 3u16, 4u16];
        let mut sampler = NanoleafPerimeterSampler::new(
            4,
            &panel_ids,
            false,
            1.0,
            0.0,
            1.0,
        );

        // Synthetic 4x4 frame of bright red (RGBA)
        let red_frame = vec![255, 0, 0, 255].repeat(16);

        // Scene cut = true: should snap immediately to red
        let colors = sampler.sample_frame(&red_frame, 4, 4, false, true);
        assert_eq!(colors.len(), 4);
        for c in &colors {
            assert_eq!(c.r, 255);
            assert_eq!(c.g, 0);
            assert_eq!(c.b, 0);
        }
    }
}

