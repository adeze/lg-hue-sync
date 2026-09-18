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
    #[serde(default = "default_zones")]
    pub zones: Vec<LightZone>,
}

fn default_fps() -> u32 {
    30
}

fn default_brightness() -> f32 {
    1.0
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
    pub fn load<P: AsRef<Path>>(path: P) -> Result<Self> {
        let file = File::open(&path)
            .with_context(|| format!("Failed to open config file at {:?}", path.as_ref()))?;
        let config: Config = serde_json::from_reader(file)
            .with_context(|| format!("Failed to parse config file at {:?}", path.as_ref()))?;
        Ok(config)
    }

    #[allow(dead_code)]
    pub fn save<P: AsRef<Path>>(&self, path: P) -> Result<()> {
        let mut file = File::create(&path)
            .with_context(|| format!("Failed to create config file at {:?}", path.as_ref()))?;
        let json = serde_json::to_string_pretty(self)?;
        file.write_all(json.as_bytes())?;
        Ok(())
    }
}
