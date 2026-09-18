use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs::File;
use std::io::Write;
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LightZone {
    pub channel_id: u8,
    pub name: String,
    /// Normalized coordinates: 0.0 to 1.0
    pub x_min: f32,
    pub x_max: f32,
    pub y_min: f32,
    pub y_max: f32,
}

impl LightZone {
    #[allow(dead_code)]
    pub fn from_3d_position(channel_id: u8, name: &str, pos: [f32; 3]) -> Self {
        // Hue Entertainment coordinate space:
        // X: -1.0 (left) to 1.0 (right)
        // Y: -1.0 (behind) to 1.0 (front)
        // Z: -1.0 (bottom) to 1.0 (top)
        let center_x = (pos[0] * 0.5 + 0.5).clamp(0.0, 1.0);
        let center_y = (1.0 - (pos[2] * 0.5 + 0.5)).clamp(0.0, 1.0);

        let span = 0.30;
        let x_min = (center_x - span * 0.5).clamp(0.0, 1.0);
        let x_max = (center_x + span * 0.5).clamp(0.0, 1.0);
        let y_min = (center_y - span * 0.5).clamp(0.0, 1.0);
        let y_max = (center_y + span * 0.5).clamp(0.0, 1.0);

        Self {
            channel_id,
            name: name.to_string(),
            x_min,
            x_max,
            y_min,
            y_max,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub bridge_ip: String,
    pub username: String,
    pub clientkey: String,
    pub entertainment_area_id: String,
    #[serde(default = "default_fps")]
    pub fps: u32,
    #[serde(default = "default_brightness")]
    pub brightness_multiplier: f32,
    #[serde(default = "default_true")]
    pub use_xy_gamut: bool,
    #[serde(default = "default_true")]
    pub hdr_tone_mapping: bool,
    #[serde(default = "default_zones")]
    pub zones: Vec<LightZone>,
}

fn default_fps() -> u32 {
    30
}

fn default_brightness() -> f32 {
    1.0
}

fn default_true() -> bool {
    true
}

fn default_zones() -> Vec<LightZone> {
    vec![
        LightZone {
            channel_id: 0,
            name: "Left".to_string(),
            x_min: 0.0,
            x_max: 0.25,
            y_min: 0.1,
            y_max: 0.9,
        },
        LightZone {
            channel_id: 1,
            name: "Top".to_string(),
            x_min: 0.2,
            x_max: 0.8,
            y_min: 0.0,
            y_max: 0.3,
        },
        LightZone {
            channel_id: 2,
            name: "Right".to_string(),
            x_min: 0.75,
            x_max: 1.0,
            y_min: 0.1,
            y_max: 0.9,
        },
        LightZone {
            channel_id: 3,
            name: "Bottom".to_string(),
            x_min: 0.2,
            x_max: 0.8,
            y_min: 0.7,
            y_max: 1.0,
        },
    ]
}

impl Config {
    pub fn new_default(bridge_ip: &str, username: &str, clientkey: &str, area_id: &str) -> Self {
        Self {
            bridge_ip: bridge_ip.to_string(),
            username: username.to_string(),
            clientkey: clientkey.to_string(),
            entertainment_area_id: area_id.to_string(),
            fps: default_fps(),
            brightness_multiplier: default_brightness(),
            use_xy_gamut: true,
            hdr_tone_mapping: true,
            zones: default_zones(),
        }
    }

    pub fn load<P: AsRef<Path>>(path: P) -> Result<Self> {
        let file = File::open(&path)
            .with_context(|| format!("Failed to open config file at {:?}", path.as_ref()))?;
        let config: Config = serde_json::from_reader(file)
            .with_context(|| format!("Failed to parse config file at {:?}", path.as_ref()))?;
        Ok(config)
    }

    pub fn save<P: AsRef<Path>>(&self, path: P) -> Result<()> {
        let mut file = File::create(&path)
            .with_context(|| format!("Failed to create config file at {:?}", path.as_ref()))?;
        let json = serde_json::to_string_pretty(self)?;
        file.write_all(json.as_bytes())?;
        Ok(())
    }
}
