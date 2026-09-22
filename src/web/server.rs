use crate::{color::RgbColor, config::LightZone};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, RwLock};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tracing::{error, info, warn};

pub const EMBEDDED_UI_HTML: &str = include_str!("ui.html");

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LiveSettings {
    pub brightness_multiplier: f32,
    pub saturation_boost: f32,
    pub peak_weight: f32,
    pub gamma: f32,
    pub noise_gate_threshold: f32,
    pub smoothing_factor: f32,
    pub use_xy_gamut: bool,
    pub letterbox_detection: bool,
    pub hdr_tone_mapping: bool,
    pub hue_sync_enabled: bool,
    pub nanoleaf_sync_enabled: bool,
    pub max_color_step: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum CalibrationPattern {
    Red,
    Green,
    Blue,
    White,
    WarmWhite,
    DaylightWhite,
    Quadrants,
    Perimeter,
}

impl CalibrationPattern {
    fn parse(value: &str) -> Option<Option<Self>> {
        let pattern = match value {
            "red" => Self::Red,
            "green" => Self::Green,
            "blue" => Self::Blue,
            "white" => Self::White,
            "warm-white" => Self::WarmWhite,
            "daylight-white" => Self::DaylightWhite,
            "quadrants" => Self::Quadrants,
            "perimeter" => Self::Perimeter,
            "off" => return Some(None),
            _ => return None,
        };
        Some(Some(pattern))
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Red => "red",
            Self::Green => "green",
            Self::Blue => "blue",
            Self::White => "white",
            Self::WarmWhite => "warm-white",
            Self::DaylightWhite => "daylight-white",
            Self::Quadrants => "quadrants",
            Self::Perimeter => "perimeter",
        }
    }
}

pub struct SharedState {
    pub is_syncing: AtomicBool,
    pub request_start: AtomicBool,
    pub request_stop: AtomicBool,
    pub request_restart: AtomicBool,
    pub request_sync_bridge: AtomicBool,
    pub request_save_config: AtomicBool,
    pub settings_updated: AtomicBool,
    pub fps_x100: AtomicU32,
    pub hue_bridge_ip: String,
    pub hue_connected: AtomicBool,
    pub nanoleaf_connected: AtomicBool,
    pub capture_hardware: AtomicBool,
    pub capture_resolution: RwLock<String>,
    pub current_settings: RwLock<LiveSettings>,
    pub live_hue_colors: RwLock<Vec<(u8, RgbColor)>>,
    pub live_nanoleaf_colors: RwLock<Vec<RgbColor>>,
    pub hue_zones: RwLock<Vec<LightZone>>,
    pub calibration_pattern: RwLock<Option<CalibrationPattern>>,
}

impl SharedState {
    pub fn new(
        initial_settings: LiveSettings,
        hue_bridge_ip: String,
        capture_res: String,
        hue_zones: Vec<LightZone>,
    ) -> Self {
        Self {
            is_syncing: AtomicBool::new(true),
            request_start: AtomicBool::new(false),
            request_stop: AtomicBool::new(false),
            request_restart: AtomicBool::new(false),
            request_sync_bridge: AtomicBool::new(false),
            request_save_config: AtomicBool::new(false),
            settings_updated: AtomicBool::new(false),
            fps_x100: AtomicU32::new(6000),
            hue_bridge_ip,
            hue_connected: AtomicBool::new(false),
            nanoleaf_connected: AtomicBool::new(false),
            capture_hardware: AtomicBool::new(false),
            capture_resolution: RwLock::new(capture_res),
            current_settings: RwLock::new(initial_settings),
            live_hue_colors: RwLock::new(Vec::new()),
            live_nanoleaf_colors: RwLock::new(Vec::new()),
            hue_zones: RwLock::new(hue_zones),
            calibration_pattern: RwLock::new(None),
        }
    }

    pub fn set_fps(&self, fps: f32) {
        self.fps_x100.store((fps * 100.0) as u32, Ordering::Relaxed);
    }

    pub fn get_fps(&self) -> f32 {
        self.fps_x100.load(Ordering::Relaxed) as f32 / 100.0
    }
}

#[derive(Debug, Clone, Serialize)]
struct StatusResponse {
    is_syncing: bool,
    fps: f32,
    hue_connected: bool,
    hue_bridge_ip: String,
    nanoleaf_connected: bool,
    capture_hardware: bool,
    capture_resolution: String,
    settings: LiveSettings,
    nanoleaf_colors: Vec<RgbColor>,
    hue_colors: Vec<(u8, RgbColor)>,
    hue_zones: Vec<LightZone>,
    calibration_pattern: Option<&'static str>,
}

pub async fn start_web_server(port: u16, state: Arc<SharedState>) -> Result<()> {
    let addr = format!("0.0.0.0:{}", port);
    let listener = TcpListener::bind(&addr).await?;
    info!("[+] Web control server listening at http://{}", addr);

    tokio::spawn(async move {
        loop {
            match listener.accept().await {
                Ok((stream, _)) => {
                    let st = state.clone();
                    tokio::spawn(async move {
                        if let Err(e) = handle_connection(stream, st).await {
                            // Suppress broken pipe noise from fast client disconnects
                            let err_str = e.to_string();
                            if !err_str.contains("Broken pipe")
                                && !err_str.contains("Connection reset")
                            {
                                warn!("HTTP connection handling error: {}", e);
                            }
                        }
                    });
                }
                Err(e) => {
                    error!("TCP listener accept error: {}", e);
                    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
                }
            }
        }
    });

    Ok(())
}

async fn handle_connection(mut stream: TcpStream, state: Arc<SharedState>) -> Result<()> {
    let mut request_bytes = Vec::with_capacity(1024);
    let mut chunk = [0u8; 1024];
    let body_end = loop {
        let bytes_read = stream.read(&mut chunk).await?;
        if bytes_read == 0 {
            return Ok(());
        }
        request_bytes.extend_from_slice(&chunk[..bytes_read]);
        let header_end = request_bytes
            .windows(4)
            .position(|window| window == b"\r\n\r\n")
            .map(|index| index + 4)
            .or_else(|| {
                request_bytes
                    .windows(2)
                    .position(|window| window == b"\n\n")
                    .map(|index| index + 2)
            });
        if let Some(header_end) = header_end {
            let headers = String::from_utf8_lossy(&request_bytes[..header_end]);
            let content_length = headers
                .lines()
                .find_map(|line| {
                    line.split_once(':')
                        .filter(|(name, _)| name.eq_ignore_ascii_case("content-length"))
                        .map(|(_, value)| value)
                })
                .and_then(|value| value.trim().parse::<usize>().ok())
                .unwrap_or(0);
            if request_bytes.len() >= header_end + content_length {
                break header_end + content_length;
            }
        }
        if request_bytes.len() > 16 * 1024 {
            return Err(anyhow::anyhow!("HTTP request exceeds 16 KiB limit"));
        }
    };

    let request = String::from_utf8_lossy(&request_bytes[..body_end]);
    let mut lines = request.lines();
    let request_line = lines.next().unwrap_or("");
    let mut parts = request_line.split_whitespace();

    let method = parts.next().unwrap_or("");
    let path = parts.next().unwrap_or("");

    match (method, path) {
        ("GET", "/") | ("GET", "/index.html") => {
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nAccess-Control-Allow-Origin: *\r\nConnection: close\r\n\r\n{}",
                EMBEDDED_UI_HTML.len(),
                EMBEDDED_UI_HTML
            );
            stream.write_all(response.as_bytes()).await?;
        }
        ("HEAD", "/") | ("HEAD", "/index.html") => {
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nAccess-Control-Allow-Origin: *\r\nConnection: close\r\n\r\n",
                EMBEDDED_UI_HTML.len()
            );
            stream.write_all(response.as_bytes()).await?;
        }
        ("OPTIONS", _) => {
            let response = "HTTP/1.1 204 No Content\r\nAccess-Control-Allow-Origin: *\r\nAccess-Control-Allow-Methods: GET, POST, OPTIONS\r\nAccess-Control-Allow-Headers: Content-Type\r\nConnection: close\r\n\r\n";
            stream.write_all(response.as_bytes()).await?;
        }
        ("GET", "/api/status") => {
            let status = {
                let settings = state.current_settings.read().unwrap().clone();
                let res = state.capture_resolution.read().unwrap().clone();
                let nl_colors = state.live_nanoleaf_colors.read().unwrap().clone();
                let hue_colors = state.live_hue_colors.read().unwrap().clone();
                let hue_zones = state.hue_zones.read().unwrap().clone();
                let calibration_pattern = state
                    .calibration_pattern
                    .read()
                    .unwrap()
                    .map(CalibrationPattern::name);
                StatusResponse {
                    is_syncing: state.is_syncing.load(Ordering::Relaxed),
                    fps: state.get_fps(),
                    hue_connected: state.hue_connected.load(Ordering::Relaxed),
                    hue_bridge_ip: state.hue_bridge_ip.clone(),
                    nanoleaf_connected: state.nanoleaf_connected.load(Ordering::Relaxed),
                    capture_hardware: state.capture_hardware.load(Ordering::Relaxed),
                    capture_resolution: res,
                    settings,
                    nanoleaf_colors: nl_colors,
                    hue_colors,
                    hue_zones,
                    calibration_pattern,
                }
            };
            let json = serde_json::to_string(&status)?;
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nAccess-Control-Allow-Origin: *\r\nConnection: close\r\n\r\n{}",
                json.len(),
                json
            );
            stream.write_all(response.as_bytes()).await?;
        }
        ("POST", "/api/start") => {
            state.request_start.store(true, Ordering::SeqCst);
            state.is_syncing.store(true, Ordering::SeqCst);
            let body = r#"{"status":"starting"}"#;
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nAccess-Control-Allow-Origin: *\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            stream.write_all(response.as_bytes()).await?;
        }
        ("POST", "/api/stop") => {
            state.request_stop.store(true, Ordering::SeqCst);
            state.is_syncing.store(false, Ordering::SeqCst);
            let body = r#"{"status":"stopping"}"#;
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nAccess-Control-Allow-Origin: *\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            stream.write_all(response.as_bytes()).await?;
        }
        ("POST", "/api/toggle") => {
            let currently_syncing = state.is_syncing.load(Ordering::SeqCst);
            if currently_syncing {
                state.request_stop.store(true, Ordering::SeqCst);
                state.is_syncing.store(false, Ordering::SeqCst);
            } else {
                state.request_start.store(true, Ordering::SeqCst);
                state.is_syncing.store(true, Ordering::SeqCst);
            }
            let body = format!(r#"{{"is_syncing":{}}}"#, !currently_syncing);
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nAccess-Control-Allow-Origin: *\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            stream.write_all(response.as_bytes()).await?;
        }
        ("POST", "/api/settings") => {
            // Find body after \r\n\r\n or \n\n
            if let Some(pos) = request.find("\r\n\r\n").or_else(|| request.find("\n\n")) {
                let offset = if request[pos..].starts_with("\r\n\r\n") {
                    pos + 4
                } else {
                    pos + 2
                };
                let body_str = &request[offset..];
                if let Ok(new_settings) = serde_json::from_str::<LiveSettings>(body_str.trim()) {
                    *state.current_settings.write().unwrap() = new_settings;
                    state.settings_updated.store(true, Ordering::SeqCst);
                }
            }
            let body = r#"{"status":"ok"}"#;
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nAccess-Control-Allow-Origin: *\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            stream.write_all(response.as_bytes()).await?;
        }
        ("POST", "/api/save-config") => {
            state.request_save_config.store(true, Ordering::SeqCst);
            let body = r#"{"status":"saved"}"#;
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nAccess-Control-Allow-Origin: *\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            stream.write_all(response.as_bytes()).await?;
        }
        ("POST", "/api/restart") => {
            state.request_restart.store(true, Ordering::SeqCst);
            let body = r#"{"status":"restarting"}"#;
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nAccess-Control-Allow-Origin: *\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            stream.write_all(response.as_bytes()).await?;
        }
        ("POST", "/api/sync-bridge") => {
            state.request_sync_bridge.store(true, Ordering::SeqCst);
            let body = r#"{"status":"queued"}"#;
            let response = format!(
                "HTTP/1.1 202 Accepted\r\nContent-Type: application/json\r\nContent-Length: {}\r\nAccess-Control-Allow-Origin: *\r\nConnection: close\r\n\r\n{}",
                body.len(), body
            );
            stream.write_all(response.as_bytes()).await?;
        }
        ("POST", path) if path.starts_with("/api/test-pattern/") => {
            let pattern = path.trim_start_matches("/api/test-pattern/");
            let (status, body) = match CalibrationPattern::parse(pattern) {
                Some(pattern) => {
                    *state.calibration_pattern.write().unwrap() = pattern;
                    ("200 OK", r#"{"status":"ok"}"#)
                }
                None => ("400 Bad Request", r#"{"error":"unknown test pattern"}"#),
            };
            let response = format!(
                "HTTP/1.1 {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nAccess-Control-Allow-Origin: *\r\nConnection: close\r\n\r\n{}",
                status, body.len(), body
            );
            stream.write_all(response.as_bytes()).await?;
        }
        _ => {
            let body = "Not Found";
            let response = format!(
                "HTTP/1.1 404 Not Found\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            stream.write_all(response.as_bytes()).await?;
        }
    }

    stream.flush().await?;
    Ok(())
}
