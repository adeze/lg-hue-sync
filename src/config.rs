use serde::{Deserialize, Serialize};
use std::ffi::OsString;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::os::fd::AsRawFd;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use thiserror::Error;

static NEXT_TEMP_FILE: AtomicU64 = AtomicU64::new(0);
static CONFIG_WRITE_MUTEX: Mutex<()> = Mutex::new(());

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("failed to open config at {path}")]
    Open {
        path: std::path::PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to parse config at {path}")]
    Parse {
        path: std::path::PathBuf,
        #[source]
        source: serde_json::Error,
    },
    #[error("failed to write config at {path}")]
    Write {
        path: std::path::PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to lock config at {path}")]
    Lock {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to serialize config")]
    Serialize(#[from] serde_json::Error),
    #[error("invalid config at {path}: {reason}")]
    Invalid {
        path: std::path::PathBuf,
        reason: &'static str,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LightZone {
    pub channel_id: u8,
    pub name: String,
    /// Hue V2 owner device for this channel. Empty on pre-V2 configurations.
    #[serde(default)]
    pub hue_device_id: Option<String>,
    /// Zero-based gradient segment index, when the light exposes segments.
    #[serde(default)]
    pub hue_segment_index: Option<u8>,
    #[serde(default)]
    pub hue_segment_count: Option<u8>,
    /// Normalized coordinates: 0.0 to 1.0
    pub x_min: f32,
    pub x_max: f32,
    pub y_min: f32,
    pub y_max: f32,
}

impl LightZone {
    /// Maps 3D room coordinates [X, Y, Z] from Philips Hue Entertainment API into a 2D screen sampling box.
    /// - X: -1.0 (left wall) to +1.0 (right wall)
    /// - Y: -1.0 (behind listening seat) to +1.0 (front TV wall)
    /// - Z: -1.0 (floor) to +1.0 (ceiling)
    pub fn from_3d_position(channel_id: u8, name: &str, pos: [f32; 3]) -> Self {
        let x = pos[0].clamp(-1.0, 1.0);
        let y = pos[1].clamp(-1.0, 1.0);
        let z = pos[2].clamp(-1.0, 1.0);

        // 1. Calculate base 2D screen center (X and Z)
        let center_x = (x * 0.5 + 0.5).clamp(0.0, 1.0);
        // Invert Z so +1.0 (ceiling/high) maps to top of screen (Y=0.0 in raster space)
        let center_y = (1.0 - (z * 0.5 + 0.5)).clamp(0.0, 1.0);

        // 2. Depth scaling (Y-axis):
        // Front lights (near TV wall, Y >= 0.5): narrow, sharp directional span (25% screen box).
        // Rear/surround lights (behind seat, Y < 0.2): expand span into a wide diffuse ambient zone (up to 70%).
        let depth_factor = ((1.0 - y) * 0.5).clamp(0.0, 1.0); // 0.0 at TV wall, 1.0 behind couch
        let span_x = 0.25 + 0.40 * depth_factor;
        let span_y = 0.25 + 0.35 * depth_factor;

        let x_min = (center_x - span_x * 0.5).clamp(0.0, 1.0);
        let x_max = (center_x + span_x * 0.5).clamp(0.0, 1.0);
        let y_min = (center_y - span_y * 0.5).clamp(0.0, 1.0);
        let y_max = (center_y + span_y * 0.5).clamp(0.0, 1.0);

        Self {
            channel_id,
            name: name.to_string(),
            hue_device_id: None,
            hue_segment_index: None,
            hue_segment_count: None,
            x_min,
            x_max,
            y_min,
            y_max,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NanoleafConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    pub ip: String,
    pub auth_token: String,
    #[serde(default = "default_nanoleaf_port")]
    pub udp_port: u16,
    /// Number of addressable LED segments along the lightstrip (default 30 for 4D strip)
    #[serde(default = "default_nanoleaf_segments")]
    pub segments: u16,
    /// Specific panel IDs retrieved from Nanoleaf layout
    #[serde(default)]
    pub panel_ids: Vec<u16>,
    #[serde(default)]
    pub alignment: NanoleafAlignment,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct NanoleafAlignment {
    #[serde(default)]
    pub start_corner: NanoleafStartCorner,
    #[serde(default)]
    pub reverse_direction: bool,
    #[serde(default)]
    pub perimeter_offset: u8,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum NanoleafStartCorner {
    #[default]
    BottomCenter,
    BottomLeft,
    TopLeft,
    TopRight,
    BottomRight,
}

fn default_nanoleaf_port() -> u16 {
    60222
}

fn default_nanoleaf_segments() -> u16 {
    30
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    /// Automatically pause/resume outputs with the TV's active/standby state.
    #[serde(default = "default_true")]
    pub auto_tv_power: bool,
    #[serde(default = "default_true")]
    pub hue_enabled: bool,
    /// Runtime preference: keep Hue configured but release its entertainment area when off.
    #[serde(default = "default_true")]
    pub hue_sync_enabled: bool,
    pub bridge_ip: String,
    pub username: String,
    pub clientkey: String,
    /// SHA-256 fingerprint of the bridge's local V2 HTTPS certificate.
    #[serde(default)]
    pub hue_bridge_certificate_sha256: Option<String>,
    /// Deprecated V1 selector retained only to load existing configurations.
    /// New configurations store the V2 UUID here as well as in `entertainment_configuration_id`.
    pub entertainment_area_id: String,
    /// V2 entertainment configuration UUID embedded in HueStream packets.
    #[serde(default)]
    pub entertainment_configuration_id: Option<String>,
    #[serde(default)]
    pub nanoleaf: Option<NanoleafConfig>,
    /// Runtime preference: stop sending Nanoleaf UDP frames when off.
    #[serde(default = "default_true")]
    pub nanoleaf_sync_enabled: bool,
    #[serde(default = "default_fps")]
    pub fps: u32,
    #[serde(default = "default_brightness")]
    pub brightness_multiplier: f32,
    /// Independent final output trim for Hue Entertainment lights.
    #[serde(default = "default_output_trim")]
    pub hue_output_brightness: f32,
    /// Independent final output trim for the Nanoleaf 4D perimeter.
    #[serde(default = "default_output_trim")]
    pub nanoleaf_output_brightness: f32,
    #[serde(default = "default_false")]
    pub use_xy_gamut: bool,
    #[serde(default = "default_true")]
    pub hdr_tone_mapping: bool,
    #[serde(default = "default_true")]
    pub letterbox_detection: bool,
    #[serde(default = "default_saturation_boost")]
    pub saturation_boost: f32,
    #[serde(default = "default_peak_weight")]
    pub peak_weight: f32,
    #[serde(default = "default_gamma")]
    pub gamma: f32,
    #[serde(default = "default_noise_gate")]
    pub noise_gate_threshold: f32,
    #[serde(default = "default_smoothing_factor")]
    pub smoothing_factor: f32,
    #[serde(default = "default_smoothing_factor")]
    pub rise_smoothing_factor: f32,
    #[serde(default = "default_smoothing_factor")]
    pub fall_smoothing_factor: f32,
    /// When a sampled region passes the black gate, turn that output fully off without afterglow.
    #[serde(default)]
    pub strict_blackout: bool,
    /// Maximum per-component RGB change per reference frame at 30 FPS.
    #[serde(default = "default_max_color_step")]
    pub max_color_step: u8,
    #[serde(default = "default_true")]
    pub adaptive_throttling: bool,
    #[serde(default = "default_zones")]
    pub zones: Vec<LightZone>,
    #[serde(default = "default_capture_width")]
    pub capture_width: u32,
    #[serde(default = "default_capture_height")]
    pub capture_height: u32,
}

fn default_capture_width() -> u32 {
    320
}

fn default_capture_height() -> u32 {
    180
}

fn default_fps() -> u32 {
    30
}

fn default_brightness() -> f32 {
    1.0
}

fn default_output_trim() -> f32 {
    1.0
}

fn default_true() -> bool {
    true
}

fn default_false() -> bool {
    false
}

fn default_saturation_boost() -> f32 {
    1.5
}

fn default_peak_weight() -> f32 {
    0.35
}

fn default_gamma() -> f32 {
    1.0
}

fn default_noise_gate() -> f32 {
    0.02
}

fn default_smoothing_factor() -> f32 {
    0.35
}

fn default_max_color_step() -> u8 {
    12
}

fn default_zones() -> Vec<LightZone> {
    vec![
        LightZone {
            channel_id: 0,
            name: "Left".to_string(),
            hue_device_id: None,
            hue_segment_index: None,
            hue_segment_count: None,
            x_min: 0.0,
            x_max: 0.25,
            y_min: 0.1,
            y_max: 0.9,
        },
        LightZone {
            channel_id: 1,
            name: "Top".to_string(),
            hue_device_id: None,
            hue_segment_index: None,
            hue_segment_count: None,
            x_min: 0.2,
            x_max: 0.8,
            y_min: 0.0,
            y_max: 0.3,
        },
        LightZone {
            channel_id: 2,
            name: "Right".to_string(),
            hue_device_id: None,
            hue_segment_index: None,
            hue_segment_count: None,
            x_min: 0.75,
            x_max: 1.0,
            y_min: 0.1,
            y_max: 0.9,
        },
        LightZone {
            channel_id: 3,
            name: "Bottom".to_string(),
            hue_device_id: None,
            hue_segment_index: None,
            hue_segment_count: None,
            x_min: 0.2,
            x_max: 0.8,
            y_min: 0.7,
            y_max: 1.0,
        },
    ]
}

impl Config {
    fn validate_capture_dimensions(&self, path: &Path) -> Result<(), ConfigError> {
        // Bounds both native capture requests and MockCapture's u32 RGBA allocation math.
        if self.capture_width == 0
            || self.capture_height == 0
            || self.capture_width > 4096
            || self.capture_height > 4096
            || self.capture_width as u64 * self.capture_height as u64 > 3840 * 2160
        {
            return Err(ConfigError::Invalid {
                path: path.to_path_buf(),
                reason: "capture dimensions must fit within 4096 per side and 4K pixels",
            });
        }
        Ok(())
    }

    pub fn new_default(bridge_ip: &str, username: &str, clientkey: &str, area_id: &str) -> Self {
        Self {
            auto_tv_power: true,
            hue_enabled: true,
            hue_sync_enabled: true,
            bridge_ip: bridge_ip.to_string(),
            username: username.to_string(),
            clientkey: clientkey.to_string(),
            hue_bridge_certificate_sha256: None,
            entertainment_area_id: area_id.to_string(),
            entertainment_configuration_id: None,
            nanoleaf: None,
            nanoleaf_sync_enabled: true,
            fps: default_fps(),
            brightness_multiplier: default_brightness(),
            hue_output_brightness: default_output_trim(),
            nanoleaf_output_brightness: default_output_trim(),
            use_xy_gamut: false,
            hdr_tone_mapping: false,
            letterbox_detection: true,
            saturation_boost: default_saturation_boost(),
            peak_weight: default_peak_weight(),
            gamma: default_gamma(),
            noise_gate_threshold: default_noise_gate(),
            smoothing_factor: default_smoothing_factor(),
            rise_smoothing_factor: default_smoothing_factor(),
            fall_smoothing_factor: default_smoothing_factor(),
            strict_blackout: false,
            max_color_step: default_max_color_step(),
            adaptive_throttling: true,
            zones: default_zones(),
            capture_width: default_capture_width(),
            capture_height: default_capture_height(),
        }
    }

    pub fn load<P: AsRef<Path>>(path: P) -> Result<Self, ConfigError> {
        let path = path.as_ref();
        let file = File::open(path).map_err(|source| ConfigError::Open {
            path: path.to_path_buf(),
            source,
        })?;
        let config: Config =
            serde_json::from_reader(file).map_err(|source| ConfigError::Parse {
                path: path.to_path_buf(),
                source,
            })?;
        config.validate_capture_dimensions(path)?;
        Ok(config)
    }

    /// Read, mutate, and atomically save under a lock shared by cooperating writers.
    /// `on_missing` is used only when the config file does not exist.
    pub fn update<P: AsRef<Path>>(
        path: P,
        on_missing: Option<Self>,
        mutate: impl FnOnce(&mut Self) -> anyhow::Result<()>,
    ) -> anyhow::Result<Self> {
        let path = path.as_ref();
        let _process_lock = CONFIG_WRITE_MUTEX.lock().map_err(|_| ConfigError::Lock {
            path: path.to_path_buf(),
            source: std::io::Error::other("config write mutex poisoned"),
        })?;
        let _file_lock = lock_config(path)?;
        let mut config = match (Self::load(path), on_missing) {
            (Ok(config), _) => config,
            (Err(ConfigError::Open { source, .. }), Some(default))
                if source.kind() == std::io::ErrorKind::NotFound =>
            {
                default
            }
            (Err(error), _) => return Err(error.into()),
        };
        mutate(&mut config)?;
        config.save_unlocked(path)?;
        Ok(config)
    }

    fn save_unlocked(&self, path: &Path) -> Result<(), ConfigError> {
        self.validate_capture_dimensions(path)?;
        let temp_path = path.with_extension(format!(
            "new-{}-{}",
            std::process::id(),
            NEXT_TEMP_FILE.fetch_add(1, Ordering::Relaxed)
        ));
        let json = serde_json::to_string_pretty(self)?;
        let mut options = OpenOptions::new();
        options.create_new(true).write(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options
            .open(&temp_path)
            .map_err(|source| ConfigError::Write {
                path: temp_path.clone(),
                source,
            })?;
        let result = (|| {
            file.write_all(json.as_bytes())
                .map_err(|source| ConfigError::Write {
                    path: temp_path.clone(),
                    source,
                })?;
            file.sync_all().map_err(|source| ConfigError::Write {
                path: temp_path.clone(),
                source,
            })?;
            fs::rename(&temp_path, path).map_err(|source| ConfigError::Write {
                path: path.to_path_buf(),
                source,
            })
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temp_path);
        }
        result
    }
}

fn lock_config(path: &Path) -> Result<File, ConfigError> {
    let mut name: OsString = path.as_os_str().to_os_string();
    name.push(".lock");
    let lock_path = PathBuf::from(name);
    let mut options = OpenOptions::new();
    options.create(true).read(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
        options.mode(0o600);
        options.custom_flags(libc::O_NOFOLLOW);
        let file = options
            .open(&lock_path)
            .map_err(|source| ConfigError::Lock {
                path: lock_path.clone(),
                source,
            })?;
        file.set_permissions(fs::Permissions::from_mode(0o600))
            .map_err(|source| ConfigError::Lock {
                path: lock_path.clone(),
                source,
            })?;
        loop {
            if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) } == 0 {
                return Ok(file);
            }
            let source = std::io::Error::last_os_error();
            if source.kind() != std::io::ErrorKind::Interrupted {
                return Err(ConfigError::Lock {
                    path: lock_path,
                    source,
                });
            }
        }
    }
    #[cfg(not(unix))]
    {
        options
            .open(&lock_path)
            .map_err(|source| ConfigError::Lock {
                path: lock_path,
                source,
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_configs_disable_unverified_hdr_curve_without_changing_legacy_missing_field() {
        let config = Config::new_default("", "", "", "");
        assert!(!config.hdr_tone_mapping);

        let mut legacy = serde_json::to_value(config).unwrap();
        legacy.as_object_mut().unwrap().remove("hdr_tone_mapping");
        let loaded: Config = serde_json::from_value(legacy).unwrap();
        assert!(loaded.hdr_tone_mapping);
    }

    #[test]
    fn rejects_capture_dimensions_that_cannot_safely_allocate_rgba() {
        let path = std::env::temp_dir().join(format!(
            "lg-hue-sync-config-validation-{}-{}.json",
            std::process::id(),
            NEXT_TEMP_FILE.fetch_add(1, Ordering::Relaxed)
        ));
        let mut config = Config::new_default("", "", "", "");
        for (width, height) in [(0, 180), (320, 0), (u32::MAX, 180), (4096, 4096)] {
            config.capture_width = width;
            config.capture_height = height;
            fs::write(&path, serde_json::to_vec(&config).unwrap()).unwrap();
            assert!(matches!(
                Config::load(&path),
                Err(ConfigError::Invalid { .. })
            ));
            fs::remove_file(&path).unwrap();
            assert!(Config::update(&path, Some(config.clone()), |_| Ok(())).is_err());
        }
        fs::remove_file(format!("{}.lock", path.display())).unwrap();
    }

    #[test]
    fn concurrent_updates_produce_valid_config() {
        let path = std::env::temp_dir().join(format!(
            "lg-hue-sync-concurrent-save-{}-{}.json",
            std::process::id(),
            NEXT_TEMP_FILE.fetch_add(1, Ordering::Relaxed)
        ));
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(4));
        let threads: Vec<_> = (0..4)
            .map(|index| {
                let path = path.clone();
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    Config::update(&path, Some(Config::new_default("", "", "", "")), |config| {
                        config.gamma = 1.0 + index as f32 * 0.1;
                        Ok(())
                    })
                })
            })
            .collect();
        for thread in threads {
            thread.join().unwrap().unwrap();
        }
        Config::load(&path).unwrap();
        fs::remove_file(&path).unwrap();
        fs::remove_file(format!("{}.lock", path.display())).unwrap();
    }

    #[test]
    fn concurrent_updates_preserve_disjoint_fields() {
        let path = std::env::temp_dir().join(format!(
            "lg-hue-sync-concurrent-update-{}-{}.json",
            std::process::id(),
            NEXT_TEMP_FILE.fetch_add(1, Ordering::Relaxed)
        ));
        let mut initial = Config::new_default("", "", "", "");
        initial.capture_width = 320;
        initial.capture_height = 180;
        Config::update(&path, Some(initial), |_| Ok(())).unwrap();

        let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
        let threads: Vec<_> = (0..2)
            .map(|field| {
                let path = path.clone();
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    for _ in 0..50 {
                        Config::update(&path, None, |config| {
                            if field == 0 {
                                config.capture_width += 1;
                            } else {
                                config.capture_height += 1;
                            }
                            Ok(())
                        })?;
                    }
                    Ok::<_, anyhow::Error>(())
                })
            })
            .collect();
        for thread in threads {
            thread.join().unwrap().unwrap();
        }
        let saved = Config::load(&path).unwrap();
        assert_eq!((saved.capture_width, saved.capture_height), (370, 230));
        fs::remove_file(&path).unwrap();
        fs::remove_file(format!("{}.lock", path.display())).unwrap();
    }

    #[test]
    fn legacy_per_light_trim_fields_are_ignored() {
        let config: Config = serde_json::from_str(
            r#"{"bridge_ip":"127.0.0.1","username":"user","clientkey":"key","entertainment_area_id":"area","hue_light_trims":[{"device_id":"id","output_trim":1.3}],"zones":[{"channel_id":0,"name":"Hue","output_trim":1.3,"x_min":0.0,"x_max":1.0,"y_min":0.0,"y_max":1.0}]}"#,
        )
        .unwrap();
        let saved = serde_json::to_value(config).unwrap();
        assert!(saved.get("hue_light_trims").is_none());
        assert!(saved["zones"][0].get("output_trim").is_none());
    }

    #[test]
    fn test_3d_front_light_has_tight_directional_span() {
        // Front Left light right near the TV wall (Y = 1.0)
        let zone = LightZone::from_3d_position(0, "Front Left", [-0.8, 1.0, 0.0]);
        let span_x = zone.x_max - zone.x_min;
        let span_y = zone.y_max - zone.y_min;

        // Front lights should have tight directional focus (~0.25)
        assert!(
            (span_x - 0.25).abs() < 0.05,
            "Front light span_x should be tight ~0.25, got {}",
            span_x
        );
        assert!(
            (span_y - 0.25).abs() < 0.05,
            "Front light span_y should be tight ~0.25, got {}",
            span_y
        );
        assert!(
            zone.x_min < 0.2,
            "Front left should be on left edge of screen"
        );
    }

    #[test]
    fn test_3d_rear_surround_has_expanded_ambient_span() {
        // Center rear surround behind listening position (X = 0.0, Y = -1.0)
        let center_rear = LightZone::from_3d_position(1, "Center Rear", [0.0, -1.0, 0.0]);
        let span_x = center_rear.x_max - center_rear.x_min;
        let span_y = center_rear.y_max - center_rear.y_min;

        // Rear lights should expand into wide diffuse ambient reflection (~0.65)
        assert!(
            (span_x - 0.65).abs() < 0.05,
            "Center rear light span_x should be wide ~0.65, got {}",
            span_x
        );
        assert!(
            (span_y - 0.60).abs() < 0.05,
            "Center rear light span_y should be wide ~0.60, got {}",
            span_y
        );

        // Rear Right surround (X = 0.8, Y = -1.0): reaches right edge and extends deep into screen
        let rear_right = LightZone::from_3d_position(2, "Rear Right", [0.8, -1.0, 0.0]);
        assert_eq!(rear_right.x_max, 1.0);
        assert!(
            rear_right.x_min <= 0.60,
            "Rear right should cover broad portion of right screen"
        );
    }

    #[test]
    fn test_3d_height_mapping() {
        // Ceiling light (Z = 1.0): maps to top of screen (Y near 0.0)
        let ceiling = LightZone::from_3d_position(2, "Top Atmos", [0.0, 0.5, 1.0]);
        assert!(
            ceiling.y_min < 0.1,
            "Ceiling light should map to top of screen (y_min < 0.1), got {}",
            ceiling.y_min
        );

        // Floor light (Z = -1.0): maps to bottom of screen (Y near 1.0)
        let floor = LightZone::from_3d_position(3, "Floor Light", [0.0, 0.5, -1.0]);
        assert!(
            floor.y_max > 0.9,
            "Floor light should map to bottom of screen (y_max > 0.9), got {}",
            floor.y_max
        );
    }
}
