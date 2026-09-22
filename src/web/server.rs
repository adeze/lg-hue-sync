use crate::{
    color::RgbColor,
    config::{Config, ConfigError, HueLightTrim, LightZone, NanoleafConfig},
    hue, nanoleaf,
};
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{Html, IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use std::net::IpAddr;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, RwLock};
use thiserror::Error;
use tokio_util::{sync::CancellationToken, task::TaskTracker};
use tracing::{error, info};

pub const EMBEDDED_UI_HTML: &str = include_str!("ui.html");

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LiveSettings {
    pub brightness_multiplier: f32,
    #[serde(default = "default_output_trim")]
    pub hue_output_brightness: f32,
    #[serde(default)]
    pub hue_light_trims: Vec<HueLightTrim>,
    #[serde(default = "default_output_trim")]
    pub nanoleaf_output_brightness: f32,
    pub saturation_boost: f32,
    pub peak_weight: f32,
    pub gamma: f32,
    pub noise_gate_threshold: f32,
    #[serde(default = "default_smoothing_factor")]
    pub smoothing_factor: f32,
    #[serde(default = "default_smoothing_factor")]
    pub rise_smoothing_factor: f32,
    #[serde(default = "default_smoothing_factor")]
    pub fall_smoothing_factor: f32,
    #[serde(default)]
    pub strict_blackout: bool,
    pub use_xy_gamut: bool,
    pub letterbox_detection: bool,
    pub hdr_tone_mapping: bool,
    pub hue_sync_enabled: bool,
    pub nanoleaf_sync_enabled: bool,
    pub max_color_step: u8,
}

fn default_output_trim() -> f32 {
    1.0
}

fn default_smoothing_factor() -> f32 {
    0.35
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
    pub request_reconfigure: AtomicBool,
    pub request_sync_bridge: AtomicBool,
    pub request_save_config: AtomicBool,
    pub settings_updated: AtomicBool,
    /// Capture polling ceiling, not the media frame rate.
    pub fps_x100: AtomicU32,
    /// Successful light-output updates measured over the preceding second.
    pub light_updates_x100: AtomicU32,
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
            request_reconfigure: AtomicBool::new(false),
            request_sync_bridge: AtomicBool::new(false),
            request_save_config: AtomicBool::new(false),
            settings_updated: AtomicBool::new(false),
            fps_x100: AtomicU32::new(6000),
            light_updates_x100: AtomicU32::new(0),
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

    pub fn set_light_update_fps(&self, fps: f32) {
        self.light_updates_x100
            .store((fps * 100.0) as u32, Ordering::Relaxed);
    }

    pub fn light_update_fps(&self) -> f32 {
        self.light_updates_x100.load(Ordering::Relaxed) as f32 / 100.0
    }
}

#[derive(Clone)]
struct AppState {
    shared: Arc<SharedState>,
    config_path: PathBuf,
}

#[derive(Debug, Clone, Serialize)]
struct StatusResponse {
    is_syncing: bool,
    fps: f32,
    light_update_fps: f32,
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

#[derive(Serialize)]
struct StatusMessage {
    status: &'static str,
}

#[derive(Serialize)]
struct HueAreasResponse {
    selected_area_id: String,
    areas: Vec<hue::EntertainmentAreaSummary>,
}

#[derive(Deserialize)]
struct PairRequest {
    ip: Option<String>,
}

#[derive(Deserialize)]
struct SelectHueAreaRequest {
    area_id: String,
}

#[derive(Debug, Error)]
enum ApiError {
    #[error("{0}")]
    BadRequest(String),
    #[error("{0}")]
    SetupFailed(String),
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let status = match self {
            Self::BadRequest(_) => StatusCode::BAD_REQUEST,
            Self::SetupFailed(_) => StatusCode::BAD_GATEWAY,
        };
        (status, Json(serde_json::json!({"error": self.to_string()}))).into_response()
    }
}

pub async fn start_web_server(
    port: u16,
    shared: Arc<SharedState>,
    config_path: PathBuf,
    shutdown: CancellationToken,
) -> anyhow::Result<TaskTracker> {
    let app_state = AppState {
        shared,
        config_path,
    };
    let app = Router::new()
        .route("/", get(root))
        .route("/index.html", get(root))
        .route("/api/status", get(status))
        .route("/api/start", post(start))
        .route("/api/stop", post(stop))
        .route("/api/toggle", post(toggle))
        .route("/api/settings", post(settings))
        .route("/api/hue/areas", get(hue_areas))
        .route("/api/hue/area", post(select_hue_area))
        .route("/api/save-config", post(save_config))
        .route("/api/restart", post(restart))
        .route("/api/sync-bridge", post(sync_bridge))
        .route("/api/pair/hue", post(pair_hue))
        .route("/api/pair/nanoleaf", post(pair_nanoleaf))
        .route("/api/test-pattern/{pattern}", post(test_pattern))
        .with_state(app_state);
    let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::UNSPECIFIED, port)).await?;
    let tracker = TaskTracker::new();
    tracker.spawn(async move {
        if let Err(error) = axum::serve(listener, app)
            .with_graceful_shutdown(shutdown.cancelled_owned())
            .await
        {
            error!("Web control server failed: {}", error);
        }
    });
    info!("[+] Web control server listening on port {}", port);
    Ok(tracker)
}

async fn root() -> Html<&'static str> {
    Html(EMBEDDED_UI_HTML)
}

async fn status(State(state): State<AppState>) -> Json<StatusResponse> {
    let shared = &state.shared;
    Json(StatusResponse {
        is_syncing: shared.is_syncing.load(Ordering::Relaxed),
        fps: shared.get_fps(),
        light_update_fps: shared.light_update_fps(),
        hue_connected: shared.hue_connected.load(Ordering::Relaxed),
        hue_bridge_ip: shared.hue_bridge_ip.clone(),
        nanoleaf_connected: shared.nanoleaf_connected.load(Ordering::Relaxed),
        capture_hardware: shared.capture_hardware.load(Ordering::Relaxed),
        capture_resolution: shared.capture_resolution.read().unwrap().clone(),
        settings: shared.current_settings.read().unwrap().clone(),
        nanoleaf_colors: shared.live_nanoleaf_colors.read().unwrap().clone(),
        hue_colors: shared.live_hue_colors.read().unwrap().clone(),
        hue_zones: shared.hue_zones.read().unwrap().clone(),
        calibration_pattern: shared
            .calibration_pattern
            .read()
            .unwrap()
            .map(CalibrationPattern::name),
    })
}

async fn start(State(state): State<AppState>) -> Json<StatusMessage> {
    state.shared.request_start.store(true, Ordering::SeqCst);
    state.shared.is_syncing.store(true, Ordering::SeqCst);
    Json(StatusMessage { status: "starting" })
}

async fn stop(State(state): State<AppState>) -> Json<StatusMessage> {
    state.shared.request_stop.store(true, Ordering::SeqCst);
    state.shared.is_syncing.store(false, Ordering::SeqCst);
    Json(StatusMessage { status: "stopping" })
}

async fn toggle(State(state): State<AppState>) -> Json<StatusMessage> {
    if state.shared.is_syncing.load(Ordering::SeqCst) {
        stop(State(state)).await
    } else {
        start(State(state)).await
    }
}

async fn settings(
    State(state): State<AppState>,
    Json(settings): Json<LiveSettings>,
) -> Json<StatusMessage> {
    *state.shared.current_settings.write().unwrap() = settings;
    state.shared.settings_updated.store(true, Ordering::SeqCst);
    Json(StatusMessage { status: "ok" })
}

async fn save_config(State(state): State<AppState>) -> Json<StatusMessage> {
    state
        .shared
        .request_save_config
        .store(true, Ordering::SeqCst);
    Json(StatusMessage { status: "saved" })
}

async fn restart(State(state): State<AppState>) -> Json<StatusMessage> {
    state.shared.request_restart.store(true, Ordering::SeqCst);
    Json(StatusMessage {
        status: "restarting",
    })
}

async fn sync_bridge(State(state): State<AppState>) -> (StatusCode, Json<StatusMessage>) {
    state
        .shared
        .request_sync_bridge
        .store(true, Ordering::SeqCst);
    (
        StatusCode::ACCEPTED,
        Json(StatusMessage { status: "queued" }),
    )
}

async fn hue_areas(State(state): State<AppState>) -> Result<Json<HueAreasResponse>, ApiError> {
    let config_path = state.config_path.clone();
    let response = tokio::task::spawn_blocking(move || {
        let config = Config::load(&config_path).map_err(|error| error.to_string())?;
        if config.bridge_ip.is_empty() || config.username.is_empty() {
            return Err("Pair a Hue Bridge before selecting an Entertainment Area".to_string());
        }
        let areas = hue::list_entertainment_areas(
            &config.bridge_ip,
            &config.username,
            config.hue_bridge_certificate_sha256.as_deref(),
        )
        .map_err(|error| error.to_string())?;
        Ok(HueAreasResponse {
            selected_area_id: config
                .entertainment_configuration_id
                .unwrap_or(config.entertainment_area_id),
            areas,
        })
    })
    .await
    .map_err(|error| ApiError::SetupFailed(format!("Hue area lookup failed: {error}")))?;
    response.map(Json).map_err(ApiError::SetupFailed)
}

async fn select_hue_area(
    State(state): State<AppState>,
    Json(request): Json<SelectHueAreaRequest>,
) -> Result<Json<StatusMessage>, ApiError> {
    let area_id = request.area_id.trim().to_string();
    if area_id.is_empty() {
        return Err(ApiError::BadRequest(
            "Select a Hue Entertainment Area".to_string(),
        ));
    }
    let config_path = state.config_path.clone();
    let result = tokio::task::spawn_blocking(move || {
        let mut config = Config::load(&config_path).map_err(|error| error.to_string())?;
        if config.bridge_ip.is_empty() || config.username.is_empty() {
            return Err("Pair a Hue Bridge before selecting an Entertainment Area".to_string());
        }
        let area = hue::sync_entertainment_areas(
            &config.bridge_ip,
            &config.username,
            Some(&area_id),
            config.hue_bridge_certificate_sha256.as_deref(),
        )
        .map_err(|error| error.to_string())?;
        let previous_zones = config.zones.clone();
        config.entertainment_area_id = area.configuration_id.clone();
        config.entertainment_configuration_id = Some(area.configuration_id);
        config.hue_bridge_certificate_sha256 = Some(area.certificate_sha256);
        config.zones = area.zones;
        for zone in &mut config.zones {
            if let Some(device_id) = zone.hue_device_id.as_deref() {
                if let Some(previous) = previous_zones
                    .iter()
                    .find(|previous| previous.hue_device_id.as_deref() == Some(device_id))
                {
                    zone.output_trim = previous.output_trim;
                }
            }
        }
        config.save(&config_path).map_err(|error| error.to_string())
    })
    .await
    .map_err(|error| ApiError::SetupFailed(format!("Hue area selection failed: {error}")))?;
    result.map_err(ApiError::SetupFailed)?;
    state
        .shared
        .request_reconfigure
        .store(true, Ordering::SeqCst);
    Ok(Json(StatusMessage { status: "selected" }))
}

async fn test_pattern(
    State(state): State<AppState>,
    Path(pattern): Path<String>,
) -> Result<Json<StatusMessage>, ApiError> {
    let pattern = CalibrationPattern::parse(&pattern)
        .ok_or_else(|| ApiError::BadRequest("unknown test pattern".to_string()))?;
    *state.shared.calibration_pattern.write().unwrap() = pattern;
    Ok(Json(StatusMessage { status: "ok" }))
}

async fn pair_hue(
    State(state): State<AppState>,
    Json(request): Json<PairRequest>,
) -> Result<Json<StatusMessage>, ApiError> {
    let config_path = state.config_path.clone();
    let bridge_ip = request.ip.filter(|ip| !ip.trim().is_empty());
    let result = tokio::task::spawn_blocking(move || {
        let bridge_ip = match bridge_ip {
            Some(ip) => validate_device_ip(&ip)?,
            None => {
                validate_device_ip(&hue::discover_bridge().map_err(|error| error.to_string())?)?
            }
        };
        let (username, clientkey, area) =
            hue::pair_bridge(&bridge_ip, 45).map_err(|error| error.to_string())?;
        let mut config = load_setup_config(&config_path)?;
        config.bridge_ip = bridge_ip;
        config.username = username;
        config.clientkey = clientkey;
        config.hue_enabled = true;
        config.hue_sync_enabled = true;
        config.entertainment_area_id = area.configuration_id.clone();
        config.entertainment_configuration_id = Some(area.configuration_id);
        config.hue_bridge_certificate_sha256 = Some(area.certificate_sha256);
        config.zones = area.zones;
        config.save(&config_path).map_err(|error| error.to_string())
    })
    .await
    .map_err(|error| ApiError::SetupFailed(format!("Hue pairing task failed: {error}")))?;
    result.map_err(ApiError::SetupFailed)?;
    state
        .shared
        .request_reconfigure
        .store(true, Ordering::SeqCst);
    Ok(Json(StatusMessage { status: "paired" }))
}

async fn pair_nanoleaf(
    State(state): State<AppState>,
    Json(request): Json<PairRequest>,
) -> Result<Json<StatusMessage>, ApiError> {
    let ip = request
        .ip
        .ok_or_else(|| ApiError::BadRequest("Nanoleaf controller IP is required".to_string()))?;
    let config_path = state.config_path.clone();
    let result = tokio::task::spawn_blocking(move || {
        let ip = validate_device_ip(&ip)?;
        let (auth_token, segments, panel_ids) =
            nanoleaf::pair_nanoleaf(&ip, 45).map_err(|error| error.to_string())?;
        let mut config = load_setup_config(&config_path)?;
        config.nanoleaf = Some(NanoleafConfig {
            enabled: true,
            ip,
            auth_token,
            udp_port: 60222,
            segments: segments.max(30),
            panel_ids,
        });
        config.nanoleaf_sync_enabled = true;
        config.save(&config_path).map_err(|error| error.to_string())
    })
    .await
    .map_err(|error| ApiError::SetupFailed(format!("Nanoleaf pairing task failed: {error}")))?;
    result.map_err(ApiError::SetupFailed)?;
    state
        .shared
        .request_reconfigure
        .store(true, Ordering::SeqCst);
    Ok(Json(StatusMessage { status: "paired" }))
}

fn validate_device_ip(value: &str) -> Result<String, String> {
    let ip = value
        .trim()
        .parse::<IpAddr>()
        .map_err(|_| "Enter a valid device IP address".to_string())?;
    if ip.is_loopback() || ip.is_multicast() || ip.is_unspecified() {
        return Err("Device IP must be a reachable local-network address".to_string());
    }
    Ok(ip.to_string())
}

fn load_setup_config(path: &PathBuf) -> Result<Config, String> {
    match Config::load(path) {
        Ok(config) => Ok(config),
        Err(ConfigError::Open { source, .. }) if source.kind() == std::io::ErrorKind::NotFound => {
            Ok(Config::new_default("", "", "", ""))
        }
        Err(error) => Err(error.to_string()),
    }
}
