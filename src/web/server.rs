use crate::{
    color::RgbColor,
    config::{Config, LightZone, NanoleafAlignment, NanoleafConfig},
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
use tokio::sync::{mpsc, oneshot, Mutex};
use tokio_util::{sync::CancellationToken, task::TaskTracker};
use tracing::{error, info};

pub const EMBEDDED_UI_HTML: &str = include_str!("ui.html");
const CAPTURE_PATTERNS_HTML: &str = include_str!("../../calibration-patterns/capture.html");

#[derive(Debug)]
pub enum ControlCommand {
    Start,
    Stop,
    Restart,
    Reconfigure,
    SyncBridge,
    SaveConfig(oneshot::Sender<Result<(), String>>),
    ApplySettings(LiveSettings),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LiveSettings {
    pub brightness_multiplier: f32,
    #[serde(default = "default_output_trim")]
    pub hue_output_brightness: f32,
    #[serde(default = "default_output_trim")]
    pub nanoleaf_output_brightness: f32,
    pub saturation_boost: f32,
    pub peak_weight: f32,
    /// Nearby-zone gradient blending, 0 (off) to 0.5.
    #[serde(default)]
    pub spatial_blend: f32,
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
    #[serde(default = "default_true")]
    pub auto_tv_power: bool,
    #[serde(default)]
    pub nanoleaf_alignment: NanoleafAlignment,
    pub max_color_step: u8,
}

fn default_output_trim() -> f32 {
    1.0
}

fn default_true() -> bool {
    true
}

fn default_smoothing_factor() -> f32 {
    0.35
}

impl LiveSettings {
    fn validate(&self) -> Result<(), ApiError> {
        let ranges = [
            (
                "brightness_multiplier",
                self.brightness_multiplier,
                0.5,
                2.5,
            ),
            (
                "hue_output_brightness",
                self.hue_output_brightness,
                0.25,
                2.0,
            ),
            (
                "nanoleaf_output_brightness",
                self.nanoleaf_output_brightness,
                0.25,
                2.0,
            ),
            ("saturation_boost", self.saturation_boost, 1.0, 2.5),
            ("peak_weight", self.peak_weight, 0.0, 0.8),
            ("spatial_blend", self.spatial_blend, 0.0, 0.5),
            ("gamma", self.gamma, 0.8, 2.0),
            ("noise_gate_threshold", self.noise_gate_threshold, 0.0, 0.06),
            ("smoothing_factor", self.smoothing_factor, 0.05, 0.8),
            (
                "rise_smoothing_factor",
                self.rise_smoothing_factor,
                0.05,
                0.8,
            ),
            (
                "fall_smoothing_factor",
                self.fall_smoothing_factor,
                0.05,
                0.8,
            ),
        ];
        for (name, value, min, max) in ranges {
            if !value.is_finite() || !(min..=max).contains(&value) {
                return Err(ApiError::BadRequest(format!(
                    "{name} must be between {min} and {max}"
                )));
            }
        }
        if !(4..=64).contains(&self.max_color_step) {
            return Err(ApiError::BadRequest(
                "max_color_step must be between 4 and 64".to_string(),
            ));
        }
        if self.nanoleaf_alignment.perimeter_offset >= 40 {
            return Err(ApiError::BadRequest(
                "Nanoleaf perimeter offset must be between 0 and 39".to_string(),
            ));
        }
        Ok(())
    }
}

struct Preset {
    brightness: f32,
    saturation: f32,
    peak: f32,
    gamma: f32,
    black_gate: f32,
    smoothing: f32,
    rise: f32,
    fall: f32,
    strict_blackout: bool,
    max_color_step: u8,
    spatial_blend: f32,
}

impl Preset {
    fn named(name: &str) -> Result<Self, ApiError> {
        let values = match name {
            "neutral" => (1.0, 1.0, 0.15, 1.0, 0.02, 0.35, 0.35, 0.35, false, 12),
            "highChroma" => (1.05, 1.65, 0.25, 1.0, 0.015, 0.4, 0.35, 0.4, false, 12),
            "neonContrast" => (1.0, 2.0, 0.45, 1.15, 0.035, 0.55, 0.45, 0.7, true, 10),
            "darkSceneDetail" => (0.8, 1.2, 0.3, 1.18, 0.04, 0.5, 0.3, 0.7, true, 8),
            "fastResponse" => (1.1, 1.35, 0.35, 1.0, 0.015, 0.65, 0.65, 0.7, false, 14),
            "lowStimulation" => (0.55, 1.0, 0.1, 1.05, 0.025, 0.2, 0.18, 0.22, false, 6),
            _ => return Err(ApiError::BadRequest(format!("Unknown preset: {name}"))),
        };
        Ok(Self {
            brightness: values.0,
            saturation: values.1,
            peak: values.2,
            gamma: values.3,
            black_gate: values.4,
            smoothing: values.5,
            rise: values.6,
            fall: values.7,
            strict_blackout: values.8,
            max_color_step: values.9,
            spatial_blend: match name {
                "neutral" | "fastResponse" => 0.0,
                "highChroma" => 0.15,
                "neonContrast" => 0.08,
                "darkSceneDetail" => 0.1,
                "lowStimulation" => 0.3,
                _ => unreachable!(),
            },
        })
    }

    fn apply(self, settings: &mut LiveSettings) {
        settings.brightness_multiplier = self.brightness;
        settings.saturation_boost = self.saturation;
        settings.peak_weight = self.peak;
        settings.gamma = self.gamma;
        settings.noise_gate_threshold = self.black_gate;
        settings.smoothing_factor = self.smoothing;
        settings.rise_smoothing_factor = self.rise;
        settings.fall_smoothing_factor = self.fall;
        settings.strict_blackout = self.strict_blackout;
        settings.max_color_step = self.max_color_step;
        settings.spatial_blend = self.spatial_blend;
    }
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
    command_tx: mpsc::Sender<ControlCommand>,
    /// Capture polling ceiling, not the media frame rate.
    pub fps_x100: AtomicU32,
    /// Successful light-output updates measured over the preceding second.
    pub light_updates_x100: AtomicU32,
    pub hue_bridge_ip: String,
    pub nanoleaf_ip: RwLock<String>,
    pub hue_connected: AtomicBool,
    pub nanoleaf_connected: AtomicBool,
    pub capture_hardware: AtomicBool,
    /// 0 = unknown, 1 = active, 2 = standby/off.
    pub tv_power_state: AtomicU32,
    pub capture_resolution: RwLock<String>,
    pub current_settings: RwLock<LiveSettings>,
    settings_update_lock: Mutex<()>,
    pub live_hue_colors: RwLock<Vec<(u8, RgbColor)>>,
    pub live_nanoleaf_colors: RwLock<Vec<RgbColor>>,
    pub hue_zones: RwLock<Vec<LightZone>>,
    pub calibration_pattern: RwLock<Option<CalibrationPattern>>,
}

impl SharedState {
    pub fn new(
        initial_settings: LiveSettings,
        hue_bridge_ip: String,
        nanoleaf_ip: String,
        capture_res: String,
        hue_zones: Vec<LightZone>,
        command_tx: mpsc::Sender<ControlCommand>,
    ) -> Self {
        Self {
            is_syncing: AtomicBool::new(true),
            command_tx,
            fps_x100: AtomicU32::new(6000),
            light_updates_x100: AtomicU32::new(0),
            hue_bridge_ip,
            nanoleaf_ip: RwLock::new(nanoleaf_ip),
            hue_connected: AtomicBool::new(false),
            nanoleaf_connected: AtomicBool::new(false),
            capture_hardware: AtomicBool::new(false),
            tv_power_state: AtomicU32::new(0),
            capture_resolution: RwLock::new(capture_res),
            current_settings: RwLock::new(initial_settings),
            settings_update_lock: Mutex::new(()),
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

    pub fn set_tv_power_state(&self, active: Option<bool>) {
        self.tv_power_state.store(
            match active {
                Some(true) => 1,
                Some(false) => 2,
                None => 0,
            },
            Ordering::Relaxed,
        );
    }

    async fn send_command(&self, command: ControlCommand) -> Result<(), ApiError> {
        self.command_tx
            .send(command)
            .await
            .map_err(|_| ApiError::SetupFailed("sync controller is unavailable".to_string()))
    }

    async fn apply_settings(&self, settings: LiveSettings) -> Result<(), ApiError> {
        self.update_settings(|current| *current = settings).await
    }

    async fn update_settings(
        &self,
        update: impl FnOnce(&mut LiveSettings) + Send,
    ) -> Result<(), ApiError> {
        let _guard = self.settings_update_lock.lock().await;
        let mut settings = self.current_settings.read().unwrap().clone();
        update(&mut settings);
        settings.validate()?;
        let permit = self
            .command_tx
            .reserve()
            .await
            .map_err(|_| ApiError::SetupFailed("sync controller is unavailable".to_string()))?;
        *self.current_settings.write().unwrap() = settings.clone();
        permit.send(ControlCommand::ApplySettings(settings));
        Ok(())
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
    tv_power_state: &'static str,
    fps: f32,
    light_update_fps: f32,
    hue_connected: bool,
    hue_bridge_ip: String,
    nanoleaf_connected: bool,
    nanoleaf_ip: String,
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
        .route("/capture-patterns", get(capture_patterns))
        .route("/api/status", get(status))
        .route("/api/start", post(start))
        .route("/api/stop", post(stop))
        .route("/api/toggle", post(toggle))
        .route("/api/settings", post(settings))
        .route("/api/presets/{name}", post(apply_preset))
        .route("/api/nanoleaf/alignment", post(update_nanoleaf_alignment))
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

async fn capture_patterns() -> Html<&'static str> {
    Html(CAPTURE_PATTERNS_HTML)
}

async fn status(State(state): State<AppState>) -> Json<StatusResponse> {
    let shared = &state.shared;
    Json(StatusResponse {
        is_syncing: shared.is_syncing.load(Ordering::Relaxed),
        tv_power_state: match shared.tv_power_state.load(Ordering::Relaxed) {
            1 => "active",
            2 => "standby",
            _ => "unknown",
        },
        fps: shared.get_fps(),
        light_update_fps: shared.light_update_fps(),
        hue_connected: shared.hue_connected.load(Ordering::Relaxed),
        hue_bridge_ip: shared.hue_bridge_ip.clone(),
        nanoleaf_connected: shared.nanoleaf_connected.load(Ordering::Relaxed),
        nanoleaf_ip: shared.nanoleaf_ip.read().unwrap().clone(),
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

async fn start(State(state): State<AppState>) -> Result<Json<StatusMessage>, ApiError> {
    state.shared.send_command(ControlCommand::Start).await?;
    state.shared.is_syncing.store(true, Ordering::SeqCst);
    Ok(Json(StatusMessage { status: "starting" }))
}

async fn stop(State(state): State<AppState>) -> Result<Json<StatusMessage>, ApiError> {
    state.shared.send_command(ControlCommand::Stop).await?;
    state.shared.is_syncing.store(false, Ordering::SeqCst);
    Ok(Json(StatusMessage { status: "stopping" }))
}

async fn toggle(State(state): State<AppState>) -> Result<Json<StatusMessage>, ApiError> {
    if state.shared.is_syncing.load(Ordering::SeqCst) {
        stop(State(state)).await
    } else {
        start(State(state)).await
    }
}

async fn settings(
    State(state): State<AppState>,
    Json(settings): Json<LiveSettings>,
) -> Result<Json<StatusMessage>, ApiError> {
    settings.validate()?;
    state.shared.apply_settings(settings).await?;
    Ok(Json(StatusMessage { status: "ok" }))
}

async fn apply_preset(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> Result<Json<StatusMessage>, ApiError> {
    let preset = Preset::named(&name)?;
    state
        .shared
        .update_settings(|settings| preset.apply(settings))
        .await?;
    Ok(Json(StatusMessage { status: "ok" }))
}

async fn update_nanoleaf_alignment(
    State(state): State<AppState>,
    Json(alignment): Json<NanoleafAlignment>,
) -> Result<Json<StatusMessage>, ApiError> {
    if alignment.perimeter_offset >= 40 {
        return Err(ApiError::BadRequest(
            "Nanoleaf perimeter offset must be between 0 and 39".to_string(),
        ));
    }
    let config_path = state.config_path.clone();
    tokio::task::spawn_blocking(move || {
        Config::update(&config_path, None, |config| {
            let nanoleaf = config.nanoleaf.as_mut().ok_or_else(|| {
                anyhow::anyhow!("Pair a Nanoleaf 4D before applying its alignment")
            })?;
            nanoleaf.alignment = alignment;
            Ok(())
        })
        .map(|_| ())
        .map_err(|error| error.to_string())
    })
    .await
    .map_err(|error| ApiError::SetupFailed(format!("Nanoleaf alignment task failed: {error}")))?
    .map_err(ApiError::SetupFailed)?;
    state
        .shared
        .update_settings(|settings| settings.nanoleaf_alignment = alignment)
        .await?;
    Ok(Json(StatusMessage { status: "saved" }))
}

async fn save_config(State(state): State<AppState>) -> Result<Json<StatusMessage>, ApiError> {
    let (ack_tx, ack_rx) = oneshot::channel();
    state
        .shared
        .send_command(ControlCommand::SaveConfig(ack_tx))
        .await?;
    tokio::time::timeout(std::time::Duration::from_secs(5), ack_rx)
        .await
        .map_err(|_| ApiError::SetupFailed("config save timed out".to_string()))?
        .map_err(|_| {
            ApiError::SetupFailed("sync controller stopped before saving config".to_string())
        })?
        .map_err(ApiError::SetupFailed)?;
    Ok(Json(StatusMessage { status: "saved" }))
}

async fn restart(State(state): State<AppState>) -> Result<Json<StatusMessage>, ApiError> {
    state.shared.send_command(ControlCommand::Restart).await?;
    Ok(Json(StatusMessage {
        status: "restarting",
    }))
}

async fn sync_bridge(
    State(state): State<AppState>,
) -> Result<(StatusCode, Json<StatusMessage>), ApiError> {
    state
        .shared
        .send_command(ControlCommand::SyncBridge)
        .await?;
    Ok((
        StatusCode::ACCEPTED,
        Json(StatusMessage { status: "queued" }),
    ))
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
        let config = Config::load(&config_path).map_err(|error| error.to_string())?;
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
        Config::update(&config_path, None, |current| {
            if current.bridge_ip != config.bridge_ip
                || current.username != config.username
                || current.hue_bridge_certificate_sha256 != config.hue_bridge_certificate_sha256
            {
                anyhow::bail!("Hue Bridge configuration changed during area selection; retry");
            }
            current.entertainment_area_id = area.configuration_id.clone();
            current.entertainment_configuration_id = Some(area.configuration_id);
            current.hue_bridge_certificate_sha256 = Some(area.certificate_sha256);
            current.zones = area.zones;
            Ok(())
        })
        .map(|_| ())
        .map_err(|error| error.to_string())
    })
    .await
    .map_err(|error| ApiError::SetupFailed(format!("Hue area selection failed: {error}")))?;
    result.map_err(ApiError::SetupFailed)?;
    state
        .shared
        .send_command(ControlCommand::Reconfigure)
        .await?;
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
        Config::update(
            &config_path,
            Some(Config::new_default("", "", "", "")),
            |config| {
                config.bridge_ip = bridge_ip;
                config.username = username;
                config.clientkey = clientkey;
                config.hue_enabled = true;
                config.hue_sync_enabled = true;
                config.entertainment_area_id = area.configuration_id.clone();
                config.entertainment_configuration_id = Some(area.configuration_id);
                config.hue_bridge_certificate_sha256 = Some(area.certificate_sha256);
                config.zones = area.zones;
                Ok(())
            },
        )
        .map(|_| ())
        .map_err(|error| error.to_string())
    })
    .await
    .map_err(|error| ApiError::SetupFailed(format!("Hue pairing task failed: {error}")))?;
    result.map_err(ApiError::SetupFailed)?;
    state
        .shared
        .send_command(ControlCommand::Reconfigure)
        .await?;
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
        Config::update(
            &config_path,
            Some(Config::new_default("", "", "", "")),
            |config| {
                config.nanoleaf = Some(NanoleafConfig {
                    enabled: true,
                    ip: ip.clone(),
                    auth_token,
                    udp_port: 60222,
                    segments: segments.max(30),
                    panel_ids,
                    alignment: NanoleafAlignment::default(),
                });
                config.nanoleaf_sync_enabled = true;
                Ok(())
            },
        )
        .map_err(|error| error.to_string())?;
        Ok::<_, String>(ip)
    })
    .await
    .map_err(|error| ApiError::SetupFailed(format!("Nanoleaf pairing task failed: {error}")))?;
    let ip = result.map_err(ApiError::SetupFailed)?;
    *state.shared.nanoleaf_ip.write().unwrap() = ip;
    state
        .shared
        .send_command(ControlCommand::Reconfigure)
        .await?;
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

#[cfg(test)]
mod tests {
    use super::*;

    fn settings() -> LiveSettings {
        LiveSettings {
            brightness_multiplier: 1.0,
            hue_output_brightness: 1.0,
            nanoleaf_output_brightness: 1.0,
            saturation_boost: 1.5,
            peak_weight: 0.35,
            spatial_blend: 0.0,
            gamma: 1.0,
            noise_gate_threshold: 0.02,
            smoothing_factor: 0.35,
            rise_smoothing_factor: 0.35,
            fall_smoothing_factor: 0.35,
            strict_blackout: false,
            use_xy_gamut: false,
            letterbox_detection: true,
            hdr_tone_mapping: false,
            hue_sync_enabled: true,
            nanoleaf_sync_enabled: true,
            auto_tv_power: true,
            nanoleaf_alignment: NanoleafAlignment::default(),
            max_color_step: 12,
        }
    }

    #[test]
    fn rejects_out_of_range_dashboard_settings() {
        let mut input = settings();
        assert!(input.validate().is_ok());
        input.gamma = -1.0;
        assert!(matches!(input.validate(), Err(ApiError::BadRequest(_))));
        input.gamma = 1.0;
        input.nanoleaf_alignment.perimeter_offset = 40;
        assert!(matches!(input.validate(), Err(ApiError::BadRequest(_))));
    }

    #[test]
    fn preset_changes_only_shared_color_controls() {
        for (name, blend) in [
            ("neutral", 0.0),
            ("highChroma", 0.15),
            ("neonContrast", 0.08),
            ("darkSceneDetail", 0.1),
            ("fastResponse", 0.0),
            ("lowStimulation", 0.3),
        ] {
            let mut candidate = settings();
            Preset::named(name).unwrap().apply(&mut candidate);
            assert!(candidate.validate().is_ok(), "{name}");
            assert_eq!(candidate.spatial_blend, blend, "{name}");
        }
        let mut input = settings();
        input.hue_output_brightness = 0.7;
        input.nanoleaf_output_brightness = 0.8;
        input.hue_sync_enabled = false;
        input.nanoleaf_alignment.perimeter_offset = 7;
        input.spatial_blend = 0.25;
        Preset::named("neonContrast").unwrap().apply(&mut input);
        assert_eq!(input.saturation_boost, 2.0);
        assert_eq!(input.spatial_blend, 0.08);
        assert_eq!(input.rise_smoothing_factor, 0.45);
        assert!(input.strict_blackout);
        assert_eq!(input.hue_output_brightness, 0.7);
        assert_eq!(input.nanoleaf_output_brightness, 0.8);
        assert!(!input.hue_sync_enabled);
        assert_eq!(input.nanoleaf_alignment.perimeter_offset, 7);
        assert!(input.validate().is_ok());
        assert!(Preset::named("unknown").is_err());
    }

    #[tokio::test]
    async fn preset_request_queues_live_settings() {
        let (sender, mut receiver) = mpsc::channel(1);
        let shared = Arc::new(SharedState::new(
            settings(),
            String::new(),
            String::new(),
            String::new(),
            Vec::new(),
            sender,
        ));
        let state = AppState {
            shared: Arc::clone(&shared),
            config_path: PathBuf::new(),
        };
        assert!(
            apply_preset(State(state), Path("lowStimulation".to_string()))
                .await
                .is_ok()
        );
        assert_eq!(
            shared
                .current_settings
                .read()
                .unwrap()
                .brightness_multiplier,
            0.55
        );
        assert!(matches!(
            receiver.recv().await,
            Some(ControlCommand::ApplySettings(value)) if value.brightness_multiplier == 0.55
        ));
    }

    #[tokio::test]
    async fn failed_command_send_does_not_change_live_settings() {
        let (sender, receiver) = mpsc::channel(1);
        drop(receiver);
        let original = settings();
        let shared = SharedState::new(
            original.clone(),
            String::new(),
            String::new(),
            String::new(),
            Vec::new(),
            sender,
        );
        let mut update = original;
        update.gamma = 1.5;
        assert!(shared.apply_settings(update).await.is_err());
        assert_eq!(shared.current_settings.read().unwrap().gamma, 1.0);
    }

    #[test]
    fn spatial_blend_defaults_and_validation_are_backward_compatible() {
        let mut json = serde_json::to_value(settings()).unwrap();
        json.as_object_mut().unwrap().remove("spatial_blend");
        let old: LiveSettings = serde_json::from_value(json).unwrap();
        assert_eq!(old.spatial_blend, 0.0);
        for value in [0.0, 0.5, -0.1, 0.51, f32::NAN, f32::INFINITY] {
            let mut input = settings();
            input.spatial_blend = value;
            assert_eq!(
                input.validate().is_ok(),
                value.is_finite() && (0.0..=0.5).contains(&value)
            );
        }
    }
}
