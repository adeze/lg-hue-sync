use super::diagnostics::{self, Diagnostics};
use super::preview::{self, PreviewFrame, PreviewState};
use crate::{
    color::RgbColor,
    config::{Config, LightZone, NanoleafAlignment, NanoleafConfig, PresetTuning},
    hue, nanoleaf,
};
use axum::{
    extract::{DefaultBodyLimit, Path, State},
    http::StatusCode,
    response::{Html, IntoResponse, Response},
    routing::get,
    Json, Router,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::net::IpAddr;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, RwLock};
use thiserror::Error;
use tokio::sync::{mpsc, oneshot, watch, Mutex, Semaphore};
use tokio_util::{sync::CancellationToken, task::TaskTracker};
use tracing::{error, info};
use utoipa::ToSchema;
use utoipa_axum::{router::OpenApiRouter, routes};

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

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct LiveSettings {
    #[schema(minimum = 0.5, maximum = 2.5)]
    pub brightness_multiplier: f32,
    #[serde(default = "default_output_trim")]
    #[schema(minimum = 0, maximum = 2.5, default = 1.0)]
    pub hue_output_brightness: f32,
    #[serde(default = "default_output_trim")]
    #[schema(minimum = 0, maximum = 2.5, default = 1.0)]
    pub nanoleaf_output_brightness: f32,
    #[schema(minimum = 0.5, maximum = 2.5)]
    pub saturation_boost: f32,
    #[schema(minimum = 0, maximum = 1)]
    pub peak_weight: f32,
    /// Nearby-zone gradient blending, 0 (off) to 0.5.
    #[serde(default)]
    #[schema(minimum = 0, maximum = 0.5, default = 0.0)]
    pub spatial_blend: f32,
    /// Bounded Hue footprint expansion, independent of neighbour blending.
    #[serde(default = "crate::config::default_depth_influence")]
    #[schema(minimum = 0, maximum = 1, default = 0.5)]
    pub depth_influence: f32,
    #[schema(minimum = 0.5, maximum = 2)]
    pub gamma: f32,
    #[schema(minimum = 0, maximum = 0.06)]
    pub noise_gate_threshold: f32,
    #[serde(default = "default_smoothing_factor")]
    #[schema(minimum = 0.05, maximum = 1, default = 0.35)]
    pub smoothing_factor: f32,
    #[serde(default = "default_smoothing_factor")]
    #[schema(minimum = 0.05, maximum = 1, default = 0.35)]
    pub rise_smoothing_factor: f32,
    #[serde(default = "default_smoothing_factor")]
    #[schema(minimum = 0.05, maximum = 1, default = 0.35)]
    pub fall_smoothing_factor: f32,
    #[serde(default)]
    #[schema(default = false)]
    pub strict_blackout: bool,
    pub use_xy_gamut: bool,
    pub letterbox_detection: bool,
    pub hdr_tone_mapping: bool,
    pub hue_sync_enabled: bool,
    pub nanoleaf_sync_enabled: bool,
    #[serde(default = "default_true")]
    #[schema(default = true)]
    pub auto_tv_power: bool,
    #[serde(default)]
    pub nanoleaf_alignment: NanoleafAlignment,
    #[schema(minimum = 4, maximum = 64)]
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
                0.0,
                2.5,
            ),
            (
                "nanoleaf_output_brightness",
                self.nanoleaf_output_brightness,
                0.0,
                2.5,
            ),
            ("saturation_boost", self.saturation_boost, 0.5, 2.5),
            ("peak_weight", self.peak_weight, 0.0, 1.0),
            ("spatial_blend", self.spatial_blend, 0.0, 0.5),
            ("depth_influence", self.depth_influence, 0.0, 1.0),
            ("gamma", self.gamma, 0.5, 2.0),
            ("noise_gate_threshold", self.noise_gate_threshold, 0.0, 0.06),
            ("smoothing_factor", self.smoothing_factor, 0.05, 1.0),
            (
                "rise_smoothing_factor",
                self.rise_smoothing_factor,
                0.05,
                1.0,
            ),
            (
                "fall_smoothing_factor",
                self.fall_smoothing_factor,
                0.05,
                1.0,
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

impl PresetTuning {
    fn capture(settings: &LiveSettings) -> Self {
        Self {
            brightness_multiplier: settings.brightness_multiplier,
            saturation_boost: settings.saturation_boost,
            peak_weight: settings.peak_weight,
            spatial_blend: settings.spatial_blend,
            gamma: settings.gamma,
            noise_gate_threshold: settings.noise_gate_threshold,
            smoothing_factor: settings.smoothing_factor,
            rise_smoothing_factor: settings.rise_smoothing_factor,
            fall_smoothing_factor: settings.fall_smoothing_factor,
            strict_blackout: settings.strict_blackout,
            max_color_step: settings.max_color_step,
            use_xy_gamut: settings.use_xy_gamut,
            letterbox_detection: settings.letterbox_detection,
            hdr_tone_mapping: settings.hdr_tone_mapping,
        }
    }

    // Response fields stay readable in older saved files; colour recall uses global response.
    fn apply(&self, settings: &mut LiveSettings) {
        settings.brightness_multiplier = self.brightness_multiplier;
        settings.saturation_boost = self.saturation_boost;
        settings.peak_weight = self.peak_weight;
        settings.gamma = self.gamma;
        settings.noise_gate_threshold = self.noise_gate_threshold;
        settings.strict_blackout = self.strict_blackout;
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
    preserve_response: bool,
}

impl Preset {
    fn named(name: &str) -> Result<Self, ApiError> {
        let values = match name {
            "softColor" => (0.85, 0.75, 0.1, 0.95, 0.02, 0.35, 0.35, 0.35, false, 12),
            "vividColor" => (1.1, 1.8, 0.4, 1.1, 0.025, 0.35, 0.35, 0.35, true, 12),
            "neutral" => (1.0, 1.0, 0.15, 1.0, 0.02, 0.35, 0.35, 0.35, false, 12),
            "highChroma" => (1.05, 1.65, 0.25, 1.0, 0.015, 0.4, 0.35, 0.4, false, 12),
            "neonContrast" => (1.0, 2.0, 0.45, 1.15, 0.035, 0.55, 0.45, 0.7, true, 10),
            "darkSceneDetail" => (0.8, 1.2, 0.3, 1.18, 0.04, 0.5, 0.3, 0.7, true, 8),
            "fastResponse" => (1.1, 1.35, 0.35, 1.0, 0.015, 0.65, 0.65, 0.7, false, 14),
            "lowStimulation" => (0.55, 1.0, 0.1, 1.05, 0.025, 0.2, 0.18, 0.22, false, 6),
            _ => return Err(ApiError::BadRequest(format!("Unknown preset: {name}"))),
        };
        Ok(Self {
            preserve_response: !matches!(name, "fastResponse" | "lowStimulation"),
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
                "softColor" | "neutral" | "vividColor" | "fastResponse" => 0.0,
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
        if !self.preserve_response {
            settings.smoothing_factor = self.smoothing;
            settings.rise_smoothing_factor = self.rise;
            settings.fall_smoothing_factor = self.fall;
            settings.max_color_step = self.max_color_step;
            settings.spatial_blend = self.spatial_blend;
        }
        settings.strict_blackout = self.strict_blackout;
        if !self.preserve_response {
            settings.use_xy_gamut = false;
            settings.letterbox_detection = true;
            settings.hdr_tone_mapping = true;
        }
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum SyncState {
    Starting,
    Running,
    Stopping,
    Paused,
    Recovering,
    Faulted,
    UnsupportedSource,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
struct SyncStatus {
    requested_running: bool,
    pipeline_state: SyncState,
    last_error: Option<String>,
}

impl SyncStatus {
    fn toggle_should_stop(&self) -> bool {
        match self.pipeline_state {
            SyncState::Running => true,
            SyncState::Paused | SyncState::Faulted => false,
            _ => self.requested_running,
        }
    }
}

pub struct SharedState {
    display_control: Mutex<()>,
    pub display_off: AtomicBool,
    active_preset: RwLock<Option<String>>,
    saved_presets: RwLock<BTreeMap<String, PresetTuning>>,
    pub preview: PreviewState,
    preview_render_gate: Arc<Semaphore>,
    preview_render_started: std::sync::Mutex<Option<std::time::Instant>>,
    diagnostics: RwLock<Diagnostics>,
    sync_status: RwLock<SyncStatus>,
    command_tx: mpsc::Sender<ControlCommand>,
    /// Capture polling ceiling, not the media frame rate.
    pub fps_x100: AtomicU32,
    /// Successful light-output updates measured over the preceding second.
    pub light_updates_x100: AtomicU32,
    pub hue_bridge_ip: RwLock<String>,
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
            display_control: Mutex::new(()),
            display_off: AtomicBool::new(false),
            active_preset: RwLock::new(None),
            saved_presets: RwLock::new(BTreeMap::new()),
            preview: PreviewState::default(),
            preview_render_gate: Arc::new(Semaphore::new(1)),
            preview_render_started: std::sync::Mutex::new(None),
            diagnostics: RwLock::new(Diagnostics::default()),
            sync_status: RwLock::new(SyncStatus {
                requested_running: true,
                pipeline_state: SyncState::Starting,
                last_error: None,
            }),
            command_tx,
            fps_x100: AtomicU32::new(6000),
            light_updates_x100: AtomicU32::new(0),
            hue_bridge_ip: RwLock::new(hue_bridge_ip),
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

    pub fn configure_presets(&self, active: Option<String>, saved: BTreeMap<String, PresetTuning>) {
        let current = self.current_settings.read().unwrap().clone();
        *self.saved_presets.write().unwrap() = saved
            .into_iter()
            .filter(|(name, tuning)| {
                let mut candidate = current.clone();
                tuning.apply(&mut candidate);
                Preset::named(name).is_ok() && candidate.validate().is_ok()
            })
            .collect();
        *self.active_preset.write().unwrap() = active.filter(|name| Preset::named(name).is_ok());
    }

    fn factory_tuning(&self, name: &str) -> Result<PresetTuning, ApiError> {
        let mut settings = self.current_settings.read().unwrap().clone();
        Preset::named(name)?.apply(&mut settings);
        Ok(PresetTuning::capture(&settings))
    }

    pub fn preset_save_snapshot(
        &self,
        settings: &LiveSettings,
    ) -> (Option<String>, BTreeMap<String, PresetTuning>) {
        let active = self.active_preset.read().unwrap().clone();
        let mut saved = self.saved_presets.read().unwrap().clone();
        if let Some(name) = &active {
            let tuning = PresetTuning::capture(settings);
            if self
                .factory_tuning(name)
                .is_ok_and(|factory| factory == tuning)
            {
                saved.remove(name);
            } else {
                saved.insert(name.clone(), tuning);
            }
        }
        (active, saved)
    }

    pub fn mark_presets_saved(&self, saved: BTreeMap<String, PresetTuning>) {
        *self.saved_presets.write().unwrap() = saved;
    }

    fn preset_modified(&self) -> bool {
        let Some(name) = self.active_preset.read().unwrap().clone() else {
            return false;
        };
        let saved = self.saved_presets.read().unwrap().get(&name).cloned();
        let reference = saved.or_else(|| self.factory_tuning(&name).ok());
        let current = self.current_settings.read().unwrap().clone();
        reference.is_some_and(|reference| {
            let mut recalled = current.clone();
            reference.apply(&mut recalled);
            PresetTuning::capture(&recalled) != PresetTuning::capture(&current)
        })
    }

    async fn select_preset(&self, name: String, reset: bool) -> Result<(), ApiError> {
        let factory = Preset::named(&name)?;
        let _guard = self.settings_update_lock.lock().await;
        if !reset && self.active_preset.read().unwrap().as_deref() == Some(name.as_str()) {
            return Ok(());
        }
        let mut settings = self.current_settings.read().unwrap().clone();
        factory.apply(&mut settings);
        if !reset {
            if let Some(saved) = self.saved_presets.read().unwrap().get(&name) {
                saved.apply(&mut settings);
            }
        }
        self.apply_settings_locked(settings).await?;
        *self.active_preset.write().unwrap() = Some(name);
        Ok(())
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

    pub fn set_sync_state(&self, state: SyncState, error: Option<String>) {
        if state != SyncState::Running {
            self.preview.clear();
        }
        let mut status = self.sync_status.write().unwrap();
        status.pipeline_state = state;
        status.last_error = error;
    }

    pub fn set_requested_running(&self, requested: bool) {
        self.sync_status.write().unwrap().requested_running = requested;
    }

    async fn send_command(&self, command: ControlCommand) -> Result<(), ApiError> {
        let permit = self
            .command_tx
            .reserve()
            .await
            .map_err(|_| ApiError::SetupFailed("sync controller is unavailable".to_string()))?;
        let mut status = self.sync_status.write().unwrap();
        match command {
            ControlCommand::Start => status.requested_running = true,
            ControlCommand::Stop => status.requested_running = false,
            _ => {}
        }
        permit.send(command);
        Ok(())
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
        self.apply_settings_locked(settings).await
    }

    async fn apply_settings_locked(&self, settings: LiveSettings) -> Result<(), ApiError> {
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
    bonjour: Arc<Mutex<watch::Sender<bool>>>,
    shared: Arc<SharedState>,
    config_path: PathBuf,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
struct StatusResponse {
    active_preset: Option<String>,
    preset_modified: bool,
    diagnostics: Diagnostics,
    /// Saved discovery preference; multicast visibility depends on the network.
    bonjour_enabled: bool,
    is_syncing: bool,
    #[serde(flatten)]
    sync: SyncStatus,
    #[schema(pattern = "^(active|standby|unknown)$")]
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

#[derive(Serialize, ToSchema)]
struct StatusMessage {
    status: &'static str,
}

#[derive(Serialize, ToSchema)]
struct HueAreasResponse {
    selected_area_id: String,
    areas: Vec<hue::EntertainmentAreaSummary>,
}

#[derive(Deserialize, ToSchema)]
struct PairRequest {
    /// Device IP; omitted or blank enables Hue discovery. Required for Nanoleaf.
    ip: Option<String>,
}

#[derive(Deserialize, ToSchema)]
struct SelectHueAreaRequest {
    area_id: String,
}

#[derive(Serialize, Deserialize, ToSchema)]
struct BonjourSettings {
    enabled: bool,
}

#[derive(Serialize, ToSchema)]
struct ErrorResponse {
    error: String,
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
        (
            status,
            Json(ErrorResponse {
                error: self.to_string(),
            }),
        )
            .into_response()
    }
}

pub async fn start_web_server(
    port: u16,
    bonjour_enabled: bool,
    shared: Arc<SharedState>,
    config_path: PathBuf,
    shutdown: CancellationToken,
) -> anyhow::Result<TaskTracker> {
    let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::UNSPECIFIED, port)).await?;
    let (bonjour_tx, bonjour_rx) = watch::channel(bonjour_enabled);
    let app_state = AppState {
        shared: Arc::clone(&shared),
        config_path,
        bonjour: Arc::new(Mutex::new(bonjour_tx)),
    };
    let app = build_router(app_state)?;
    let tracker = TaskTracker::new();
    tracker.spawn(super::bonjour::manage(
        listener.local_addr()?.port(),
        bonjour_rx,
        shutdown.clone(),
    ));
    let diagnostics_shutdown = shutdown.clone();
    tracker.spawn(async move {
        loop {
            let snapshot = if shared.tv_power_state.load(Ordering::Relaxed) == 2 {
                Diagnostics::default()
            } else {
                tokio::select! {
                    _ = diagnostics_shutdown.cancelled() => break,
                    snapshot = diagnostics::collect() => snapshot,
                }
            };
            *shared.diagnostics.write().unwrap() = snapshot;
            {
                let _guard = shared.display_control.lock().await;
                if let Ok(Some(enabled)) = crate::tv_power::display_enabled().await {
                    shared.display_off.store(!enabled, Ordering::Relaxed);
                }
            }
            tokio::select! {
                _ = diagnostics_shutdown.cancelled() => break,
                _ = tokio::time::sleep(std::time::Duration::from_secs(2)) => {}
            }
        }
    });
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

fn build_router(state: AppState) -> anyhow::Result<Router> {
    let (router, mut document) = OpenApiRouter::<AppState>::new()
        .routes(routes!(status))
        .routes(routes!(display_status))
        .routes(routes!(display_control))
        .routes(routes!(preview_frame))
        .routes(routes!(preview_render))
        .routes(routes!(set_bonjour))
        .routes(routes!(start))
        .routes(routes!(stop))
        .routes(routes!(toggle))
        .routes(routes!(settings))
        .routes(routes!(apply_preset))
        .routes(routes!(reset_preset))
        .routes(routes!(update_nanoleaf_alignment))
        .routes(routes!(hue_areas))
        .routes(routes!(select_hue_area))
        .routes(routes!(save_config))
        .routes(routes!(restart))
        .routes(routes!(sync_bridge))
        .routes(routes!(pair_hue))
        .routes(routes!(pair_nanoleaf))
        .routes(routes!(test_pattern))
        .split_for_parts();
    document.info.title = "LG Hue Sync HTTP API".to_string();
    document.info.version = env!("CARGO_PKG_VERSION").to_string();
    document.info.description = Some("LAN API without authentication. Control acknowledgements do not prove physical light output. JSON extractors can return plain-text errors (400 malformed JSON, 413 oversized body, 415 missing JSON content type, 422 invalid fields). Pairing and area selection persist configuration before requesting controller reconfiguration; a later controller failure does not roll back that save.".to_string());
    let specification = axum::body::Bytes::from(document.to_json()?);
    Ok(router
        .route("/", get(root))
        .route("/index.html", get(root))
        .route(
            "/assets/ios-haptics-3.3.0.js",
            get(|| async {
                (
                    [(
                        axum::http::header::CONTENT_TYPE,
                        "text/javascript; charset=utf-8",
                    )],
                    include_str!("vendor/ios-haptics-3.3.0.js"),
                )
            }),
        )
        .route("/capture-patterns", get(capture_patterns))
        .route(
            "/openapi.json",
            get(move || {
                std::future::ready((
                    [(axum::http::header::CONTENT_TYPE, "application/json")],
                    specification.clone(),
                ))
            }),
        )
        .layer(DefaultBodyLimit::max(512 * 1024))
        .with_state(state))
}

async fn root() -> Html<&'static str> {
    Html(EMBEDDED_UI_HTML)
}

async fn capture_patterns() -> Html<&'static str> {
    Html(CAPTURE_PATTERNS_HTML)
}

#[derive(Deserialize, ToSchema)]
struct PreviewRenderRequest {
    frame: PreviewFrame,
    settings: LiveSettings,
}

fn preview_response(status: StatusCode, body: impl Serialize) -> Response {
    (
        status,
        [(axum::http::header::CACHE_CONTROL, "no-store")],
        Json(body),
    )
        .into_response()
}

/// Request a bounded thumbnail from the existing HDMI capture pipeline.
#[utoipa::path(
    get, path = "/api/preview/frame", tag = "LG Hue Sync",
    responses(
        (status = 200, description = "Recent captured RGB thumbnail; not cached", body = PreviewFrame),
        (status = 202, description = "Waiting for the capture loop to publish a frame", body = ErrorResponse),
        (status = 409, description = "HDMI hardware capture is not running", body = ErrorResponse)
    )
)]
async fn preview_frame(State(state): State<AppState>) -> Response {
    let shared = &state.shared;
    if shared.sync_status.read().unwrap().pipeline_state != SyncState::Running
        || !shared.capture_hardware.load(Ordering::Relaxed)
    {
        shared.preview.clear();
        return preview_response(
            StatusCode::CONFLICT,
            ErrorResponse {
                error: "Preview requires HDMI playback with screen sync running.".into(),
            },
        );
    }
    match shared.preview.request() {
        Some(frame) => preview_response(StatusCode::OK, frame),
        None => preview_response(
            StatusCode::ACCEPTED,
            ErrorResponse {
                error: "Waiting for a captured frame.".into(),
            },
        ),
    }
}

/// Render static colour adjustments without applying settings or touching devices.
#[utoipa::path(
    post, path = "/api/preview/render", tag = "LG Hue Sync",
    request_body = PreviewRenderRequest,
    responses(
        (status = 200, description = "Processed RGB thumbnail", body = PreviewFrame),
        (status = 422, description = "Invalid frame or settings", body = ErrorResponse),
        (status = 429, description = "Preview renderer busy or rate limited", body = ErrorResponse),
        (status = 503, description = "Preview renderer unavailable", body = ErrorResponse)
    )
)]
async fn preview_render(
    State(state): State<AppState>,
    Json(request): Json<PreviewRenderRequest>,
) -> Response {
    if let Err(error) = request.settings.validate() {
        return preview_response(
            StatusCode::UNPROCESSABLE_ENTITY,
            ErrorResponse {
                error: error.to_string(),
            },
        );
    }
    let Ok(permit) = state.shared.preview_render_gate.clone().try_acquire_owned() else {
        return preview_response(
            StatusCode::TOO_MANY_REQUESTS,
            ErrorResponse {
                error: "Preview renderer is busy.".into(),
            },
        );
    };
    {
        let mut last = state.shared.preview_render_started.lock().unwrap();
        if last.is_some_and(|started| started.elapsed() < std::time::Duration::from_millis(100)) {
            return preview_response(
                StatusCode::TOO_MANY_REQUESTS,
                ErrorResponse {
                    error: "Preview requests are too frequent.".into(),
                },
            );
        }
        *last = Some(std::time::Instant::now());
    }
    match tokio::task::spawn_blocking(move || {
        let _permit = permit;
        preview::render(&request.frame, &request.settings)
    })
    .await
    {
        Ok(Ok(frame)) => preview_response(StatusCode::OK, frame),
        Ok(Err(error)) => {
            preview_response(StatusCode::UNPROCESSABLE_ENTITY, ErrorResponse { error })
        }
        Err(_) => preview_response(
            StatusCode::SERVICE_UNAVAILABLE,
            ErrorResponse {
                error: "Preview renderer is unavailable.".into(),
            },
        ),
    }
}

/// Read pipeline state, settings and sampled colors.
#[utoipa::path(
    get, path = "/api/status", tag = "LG Hue Sync",
    responses(
        (status = 200, description = "Success", body = StatusResponse)
    )
)]
async fn status(State(state): State<AppState>) -> Json<StatusResponse> {
    let shared = &state.shared;
    let sync = shared.sync_status.read().unwrap().clone();
    let bonjour_enabled = *state.bonjour.lock().await.borrow();
    let active_preset = shared.active_preset.read().unwrap().clone();
    let preset_modified = shared.preset_modified();
    Json(StatusResponse {
        active_preset,
        preset_modified,
        diagnostics: shared.diagnostics.read().unwrap().clone(),
        bonjour_enabled,
        is_syncing: sync.pipeline_state == SyncState::Running,
        sync,
        tv_power_state: match shared.tv_power_state.load(Ordering::Relaxed) {
            1 => "active",
            2 => "standby",
            _ => "unknown",
        },
        fps: shared.get_fps(),
        light_update_fps: shared.light_update_fps(),
        hue_connected: shared.hue_connected.load(Ordering::Relaxed),
        hue_bridge_ip: shared.hue_bridge_ip.read().unwrap().clone(),
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

/// Save and apply the Bonjour discovery preference without restarting the daemon.
#[utoipa::path(
    post, path = "/api/bonjour", tag = "LG Hue Sync",
    request_body = BonjourSettings,
    responses(
        (status = 200, description = "Discovery preference saved and applied", body = BonjourSettings),
        (status = 400, description = "Malformed JSON", body = String, content_type = "text/plain"),
        (status = 413, description = "Request body too large", body = String, content_type = "text/plain"),
        (status = 415, description = "JSON content type required", body = String, content_type = "text/plain"),
        (status = 422, description = "Invalid JSON fields", body = String, content_type = "text/plain"),
        (status = 502, description = "Configuration save or discovery control failed", body = ErrorResponse)
    )
)]
async fn set_bonjour(
    State(state): State<AppState>,
    Json(request): Json<BonjourSettings>,
) -> Result<Json<BonjourSettings>, ApiError> {
    // Serialize persistence and notification so concurrent requests cannot leave them out of order.
    let sender = state.bonjour.lock().await;
    if sender.is_closed() {
        return Err(ApiError::SetupFailed(
            "discovery controller is unavailable".to_string(),
        ));
    }
    let config_path = state.config_path.clone();
    let enabled = request.enabled;
    tokio::task::spawn_blocking(move || {
        Config::update(&config_path, None, |config| {
            config.bonjour_enabled = enabled;
            Ok(())
        })
    })
    .await
    .map_err(|error| ApiError::SetupFailed(format!("Discovery save task failed: {error}")))?
    .map_err(|error| ApiError::SetupFailed(error.to_string()))?;
    sender.send(enabled).map_err(|_| {
        ApiError::SetupFailed("discovery controller stopped after saving".to_string())
    })?;
    Ok(Json(request))
}

#[derive(Debug, Serialize, ToSchema)]
struct DisplayState {
    /// Actual panel state; null means standby or an unrecognized state.
    enabled: Option<bool>,
}

/// Read the panel state separately from TV standby.
#[utoipa::path(get, path = "/api/display", tag = "LG Hue Sync",
    responses((status = 200, body = DisplayState), (status = 502, body = ErrorResponse)))]
async fn display_status(State(state): State<AppState>) -> Result<Json<DisplayState>, ApiError> {
    let _guard = state.shared.display_control.lock().await;
    let enabled = crate::tv_power::display_enabled()
        .await
        .map_err(|error| ApiError::SetupFailed(error.to_string()))?;
    if let Some(enabled) = enabled {
        state.shared.display_off.store(!enabled, Ordering::Relaxed);
    }
    Ok(Json(DisplayState { enabled }))
}

/// Set or toggle the panel without powering the TV on or off.
#[utoipa::path(post, path = "/api/display/{action}", tag = "LG Hue Sync",
    params(("action" = String, Path, description = "on, off or toggle; toggle is not retry-safe")),
    responses((status = 200, body = DisplayState), (status = 400, body = ErrorResponse),
        (status = 502, body = ErrorResponse)))]
async fn display_control(
    State(state): State<AppState>,
    Path(action): Path<String>,
) -> Result<Json<DisplayState>, ApiError> {
    if !matches!(action.as_str(), "on" | "off" | "toggle") {
        return Err(ApiError::BadRequest(
            "Display action must be on, off or toggle".into(),
        ));
    }
    let _guard = state.shared.display_control.lock().await;
    let current = crate::tv_power::display_enabled()
        .await
        .map_err(|error| ApiError::SetupFailed(error.to_string()))?
        .ok_or_else(|| {
            ApiError::SetupFailed(
                "Display control unavailable in standby or unknown TV state".into(),
            )
        })?;
    let enabled = match action.as_str() {
        "on" => true,
        "off" => false,
        _ => !current,
    };
    let actual = if enabled == current {
        Some(current)
    } else {
        crate::tv_power::set_display(enabled)
            .await
            .map_err(|error| ApiError::SetupFailed(error.to_string()))?
    };
    if let Some(enabled) = actual {
        state.shared.display_off.store(!enabled, Ordering::Relaxed);
    }
    Ok(Json(DisplayState { enabled: actual }))
}

/// Queue capture and output start.
#[utoipa::path(
    post, path = "/api/start", tag = "LG Hue Sync",
    responses(
        (status = 200, description = "Success", body = StatusMessage),
        (status = 502, description = "Controller, device or configuration operation failed", body = ErrorResponse)
    )
)]
async fn start(State(state): State<AppState>) -> Result<Json<StatusMessage>, ApiError> {
    state.shared.send_command(ControlCommand::Start).await?;
    Ok(Json(StatusMessage { status: "starting" }))
}

/// Queue capture and output stop.
#[utoipa::path(
    post, path = "/api/stop", tag = "LG Hue Sync",
    responses(
        (status = 200, description = "Success", body = StatusMessage),
        (status = 502, description = "Controller, device or configuration operation failed", body = ErrorResponse)
    )
)]
async fn stop(State(state): State<AppState>) -> Result<Json<StatusMessage>, ApiError> {
    state.shared.send_command(ControlCommand::Stop).await?;
    Ok(Json(StatusMessage { status: "stopping" }))
}

/// Queue start or stop according to actual pipeline state.
#[utoipa::path(
    post, path = "/api/toggle", tag = "LG Hue Sync",
    responses(
        (status = 200, description = "Success", body = StatusMessage),
        (status = 502, description = "Controller, device or configuration operation failed", body = ErrorResponse)
    )
)]
async fn toggle(State(state): State<AppState>) -> Result<Json<StatusMessage>, ApiError> {
    let should_stop = state
        .shared
        .sync_status
        .read()
        .unwrap()
        .toggle_should_stop();
    if should_stop {
        stop(State(state)).await
    } else {
        start(State(state)).await
    }
}

/// Apply live settings without saving configuration.
#[utoipa::path(
    post, path = "/api/settings", tag = "LG Hue Sync",
    request_body = LiveSettings,
    responses(
        (status = 200, description = "Success", body = StatusMessage),
        (status = 400, description = "Invalid request", content((ErrorResponse = "application/json"), (String = "text/plain"))),
        (status = 413, description = "Request body exceeds the 512 KiB limit", body = String, content_type = "text/plain"),
        (status = 415, description = "JSON content type required", body = String, content_type = "text/plain"),
        (status = 422, description = "Invalid JSON fields or types", body = String, content_type = "text/plain"),
        (status = 502, description = "Controller, device or configuration operation failed", body = ErrorResponse)
    )
)]
async fn settings(
    State(state): State<AppState>,
    Json(settings): Json<LiveSettings>,
) -> Result<Json<StatusMessage>, ApiError> {
    settings.validate()?;
    state.shared.apply_settings(settings).await?;
    Ok(Json(StatusMessage { status: "ok" }))
}

/// Apply a color preset while preserving temporal response; legacy fastResponse/lowStimulation names remain full profiles.
#[utoipa::path(
    post, path = "/api/presets/{name}", tag = "LG Hue Sync",
    params(("name" = String, Path, pattern = "^(softColor|neutral|vividColor|highChroma|neonContrast|darkSceneDetail|fastResponse|lowStimulation)$")),
    responses(
        (status = 200, description = "Success", body = StatusMessage),
        (status = 400, description = "Invalid request", content((ErrorResponse = "application/json"), (String = "text/plain"))),
        (status = 502, description = "Controller, device or configuration operation failed", body = ErrorResponse)
    )
)]
async fn apply_preset(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> Result<Json<StatusMessage>, ApiError> {
    state.shared.select_preset(name, false).await?;
    Ok(Json(StatusMessage { status: "ok" }))
}

/// Restore factory tuning for a preset; Save is required to persist the reset.
#[utoipa::path(
    post, path = "/api/presets/{name}/reset", tag = "LG Hue Sync",
    params(("name" = String, Path, description = "Preset name")),
    responses(
        (status = 200, description = "Factory tuning applied live; not saved", body = StatusMessage),
        (status = 400, description = "Unknown preset", body = ErrorResponse),
        (status = 502, description = "Controller unavailable", body = ErrorResponse)
    )
)]
async fn reset_preset(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> Result<Json<StatusMessage>, ApiError> {
    state.shared.select_preset(name, true).await?;
    Ok(Json(StatusMessage { status: "reset" }))
}

/// Save Nanoleaf alignment, then apply live settings.
#[utoipa::path(
    post, path = "/api/nanoleaf/alignment", tag = "LG Hue Sync",
    request_body = NanoleafAlignment,
    responses(
        (status = 200, description = "Success", body = StatusMessage),
        (status = 400, description = "Invalid request", content((ErrorResponse = "application/json"), (String = "text/plain"))),
        (status = 413, description = "Request body exceeds the 512 KiB limit", body = String, content_type = "text/plain"),
        (status = 415, description = "JSON content type required", body = String, content_type = "text/plain"),
        (status = 422, description = "Invalid JSON fields or types", body = String, content_type = "text/plain"),
        (status = 502, description = "Controller, device or configuration operation failed", body = ErrorResponse)
    )
)]
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

/// Save current configuration; wait up to five seconds for acknowledgement.
#[utoipa::path(
    post, path = "/api/save-config", tag = "LG Hue Sync",
    responses(
        (status = 200, description = "Success", body = StatusMessage),
        (status = 502, description = "Controller, device or configuration operation failed", body = ErrorResponse)
    )
)]
async fn save_config(State(state): State<AppState>) -> Result<Json<StatusMessage>, ApiError> {
    let _guard = state.shared.settings_update_lock.lock().await;
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

/// Queue pipeline restart.
#[utoipa::path(
    post, path = "/api/restart", tag = "LG Hue Sync",
    responses(
        (status = 200, description = "Success", body = StatusMessage),
        (status = 502, description = "Controller, device or configuration operation failed", body = ErrorResponse)
    )
)]
async fn restart(State(state): State<AppState>) -> Result<Json<StatusMessage>, ApiError> {
    state.shared.send_command(ControlCommand::Restart).await?;
    Ok(Json(StatusMessage {
        status: "restarting",
    }))
}

/// Queue Hue Bridge layout refresh; completion is asynchronous.
#[utoipa::path(
    post, path = "/api/sync-bridge", tag = "LG Hue Sync",
    responses(
        (status = 202, description = "Success", body = StatusMessage),
        (status = 502, description = "Controller, device or configuration operation failed", body = ErrorResponse)
    )
)]
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

/// List Entertainment Areas from the paired Hue Bridge.
#[utoipa::path(
    get, path = "/api/hue/areas", tag = "LG Hue Sync",
    responses(
        (status = 200, description = "Success", body = HueAreasResponse),
        (status = 502, description = "Controller, device or configuration operation failed", body = ErrorResponse)
    )
)]
async fn hue_areas(State(state): State<AppState>) -> Result<Json<HueAreasResponse>, ApiError> {
    let config_path = state.config_path.clone();
    let response = tokio::task::spawn_blocking(move || {
        let config = Config::load(&config_path).map_err(|error| error.to_string())?;
        if config.bridge_ip.is_empty() || config.username.is_empty() {
            return Err("Pair a Hue Bridge before selecting an Entertainment Area".to_string());
        }
        let areas =
            hue::list_entertainment_areas(&config.bridge_ip, &config.username, config.hue_trust())
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

/// Save a Hue Entertainment Area and queue reconfiguration.
#[utoipa::path(
    post, path = "/api/hue/area", tag = "LG Hue Sync",
    request_body = SelectHueAreaRequest,
    responses(
        (status = 200, description = "Success", body = StatusMessage),
        (status = 400, description = "Invalid request", content((ErrorResponse = "application/json"), (String = "text/plain"))),
        (status = 413, description = "Request body exceeds the 512 KiB limit", body = String, content_type = "text/plain"),
        (status = 415, description = "JSON content type required", body = String, content_type = "text/plain"),
        (status = 422, description = "Invalid JSON fields or types", body = String, content_type = "text/plain"),
        (status = 502, description = "Controller, device or configuration operation failed", body = ErrorResponse)
    )
)]
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
            config.hue_trust(),
        )
        .map_err(|error| error.to_string())?;
        Config::update(&config_path, None, |current| {
            if current.bridge_ip != config.bridge_ip
                || current.username != config.username
                || current.hue_bridge_certificate_sha256 != config.hue_bridge_certificate_sha256
                || current.hue_bridge_id != config.hue_bridge_id
            {
                anyhow::bail!("Hue Bridge configuration changed during area selection; retry");
            }
            current.entertainment_area_id = area.configuration_id.clone();
            current.entertainment_configuration_id = Some(area.configuration_id);
            current.hue_bridge_id = Some(area.bridge_id.clone());
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

/// Select a live calibration pattern, or off for normal capture.
#[utoipa::path(
    post, path = "/api/test-pattern/{pattern}", tag = "LG Hue Sync",
    params(("pattern" = String, Path, pattern = "^(red|green|blue|white|warm-white|daylight-white|quadrants|perimeter|off)$")),
    responses(
        (status = 200, description = "Success", body = StatusMessage),
        (status = 400, description = "Invalid request", content((ErrorResponse = "application/json"), (String = "text/plain")))
    )
)]
async fn test_pattern(
    State(state): State<AppState>,
    Path(pattern): Path<String>,
) -> Result<Json<StatusMessage>, ApiError> {
    let pattern = CalibrationPattern::parse(&pattern)
        .ok_or_else(|| ApiError::BadRequest("unknown test pattern".to_string()))?;
    *state.shared.calibration_pattern.write().unwrap() = pattern;
    Ok(Json(StatusMessage { status: "ok" }))
}

/// Pair Hue (45-second pairing window), save credentials and queue reconfiguration.
#[utoipa::path(
    post, path = "/api/pair/hue", tag = "LG Hue Sync",
    request_body = PairRequest,
    responses(
        (status = 200, description = "Success", body = StatusMessage),
        (status = 400, description = "Invalid request", content((ErrorResponse = "application/json"), (String = "text/plain"))),
        (status = 413, description = "Request body exceeds the 512 KiB limit", body = String, content_type = "text/plain"),
        (status = 415, description = "JSON content type required", body = String, content_type = "text/plain"),
        (status = 422, description = "Invalid JSON fields or types", body = String, content_type = "text/plain"),
        (status = 502, description = "Controller, device or configuration operation failed", body = ErrorResponse)
    )
)]
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
        let paired = hue::pair_bridge(&bridge_ip, 45).map_err(|error| error.to_string())?;
        Config::update(
            &config_path,
            Some(Config::new_default("", "", "", "")),
            |config| {
                config.apply_hue_pairing(bridge_ip, paired);
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

/// Pair Nanoleaf (45-second pairing window), save credentials and queue reconfiguration.
#[utoipa::path(
    post, path = "/api/pair/nanoleaf", tag = "LG Hue Sync",
    request_body = PairRequest,
    responses(
        (status = 200, description = "Success", body = StatusMessage),
        (status = 400, description = "Invalid request", content((ErrorResponse = "application/json"), (String = "text/plain"))),
        (status = 413, description = "Request body exceeds the 512 KiB limit", body = String, content_type = "text/plain"),
        (status = 415, description = "JSON content type required", body = String, content_type = "text/plain"),
        (status = 422, description = "Invalid JSON fields or types", body = String, content_type = "text/plain"),
        (status = 502, description = "Controller, device or configuration operation failed", body = ErrorResponse)
    )
)]
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
            depth_influence: crate::config::default_depth_influence(),
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

    fn test_router() -> (Router, mpsc::Receiver<ControlCommand>) {
        let (sender, receiver) = mpsc::channel(8);
        let shared = Arc::new(SharedState::new(
            settings(),
            String::new(),
            String::new(),
            String::new(),
            Vec::new(),
            sender,
        ));
        (
            build_router(AppState {
                shared,
                config_path: PathBuf::new(),
                bonjour: Arc::new(Mutex::new(watch::channel(false).0)),
            })
            .unwrap(),
            receiver,
        )
    }

    #[tokio::test]
    async fn preview_requires_running_hardware_and_clears_on_source_pause() {
        use tower::ServiceExt;
        let (sender, _receiver) = mpsc::channel(2);
        let shared = Arc::new(SharedState::new(
            settings(),
            String::new(),
            String::new(),
            String::new(),
            Vec::new(),
            sender,
        ));
        let app = build_router(AppState {
            shared: shared.clone(),
            config_path: PathBuf::new(),
            bonjour: Arc::new(Mutex::new(watch::channel(false).0)),
        })
        .unwrap();
        let request = || {
            axum::http::Request::builder()
                .uri("/api/preview/frame")
                .body(axum::body::Body::empty())
                .unwrap()
        };
        let response = app.clone().oneshot(request()).await.unwrap();
        assert_eq!(response.status(), StatusCode::CONFLICT);
        assert_eq!(response.headers()["cache-control"], "no-store");
        shared.capture_hardware.store(true, Ordering::Relaxed);
        shared.set_sync_state(SyncState::Running, None);
        assert_eq!(
            app.clone().oneshot(request()).await.unwrap().status(),
            StatusCode::ACCEPTED
        );
        shared.preview.publish(&crate::capture::CapturedFrame {
            data: &[10, 20, 30, 255],
            width: 1,
            height: 1,
            is_bgra: false,
        });
        let response = app.clone().oneshot(request()).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()["cache-control"], "no-store");
        let bytes = axum::body::to_bytes(response.into_body(), 1024)
            .await
            .unwrap();
        let frame: PreviewFrame = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(frame.rgb, [10, 20, 30]);
        shared.set_sync_state(SyncState::UnsupportedSource, None);
        assert_eq!(
            app.clone().oneshot(request()).await.unwrap().status(),
            StatusCode::CONFLICT
        );
        shared.set_sync_state(SyncState::Running, None);
        assert_eq!(
            app.oneshot(request()).await.unwrap().status(),
            StatusCode::ACCEPTED
        );
    }

    #[tokio::test]
    async fn preview_render_is_bounded_rate_limited_and_does_not_apply_settings() {
        use tower::ServiceExt;
        let (sender, mut receiver) = mpsc::channel(2);
        let shared = Arc::new(SharedState::new(
            settings(),
            String::new(),
            String::new(),
            String::new(),
            Vec::new(),
            sender,
        ));
        let app = build_router(AppState {
            shared: shared.clone(),
            config_path: PathBuf::new(),
            bonjour: Arc::new(Mutex::new(watch::channel(false).0)),
        })
        .unwrap();
        let mut adjusted = settings();
        adjusted.brightness_multiplier = 0.5;
        let mut payload = serde_json::json!({"frame":{"width":1,"height":1,"rgb":[200,0,0],"captured_unix_ms":1},"settings":adjusted});
        let request = |body: String| {
            axum::http::Request::builder()
                .method("POST")
                .uri("/api/preview/render")
                .header("content-type", "application/json")
                .body(axum::body::Body::from(body))
                .unwrap()
        };
        let permit = shared
            .preview_render_gate
            .clone()
            .try_acquire_owned()
            .unwrap();
        assert_eq!(
            app.clone()
                .oneshot(request(payload.to_string()))
                .await
                .unwrap()
                .status(),
            StatusCode::TOO_MANY_REQUESTS
        );
        drop(permit);
        let response = app
            .clone()
            .oneshot(request(payload.to_string()))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()["cache-control"], "no-store");
        let bytes = axum::body::to_bytes(response.into_body(), 1024)
            .await
            .unwrap();
        let frame: PreviewFrame = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(frame.rgb, [100, 0, 0]);
        assert_eq!(
            shared
                .current_settings
                .read()
                .unwrap()
                .brightness_multiplier,
            settings().brightness_multiplier
        );
        assert!(receiver.try_recv().is_err());
        assert_eq!(
            app.clone()
                .oneshot(request(payload.to_string()))
                .await
                .unwrap()
                .status(),
            StatusCode::TOO_MANY_REQUESTS
        );
        *shared.preview_render_started.lock().unwrap() = None;
        payload["frame"]["width"] = serde_json::json!(129);
        assert_eq!(
            app.clone()
                .oneshot(request(payload.to_string()))
                .await
                .unwrap()
                .status(),
            StatusCode::UNPROCESSABLE_ENTITY
        );
        payload["settings"]["gamma"] = serde_json::json!(10);
        assert_eq!(
            app.clone()
                .oneshot(request(payload.to_string()))
                .await
                .unwrap()
                .status(),
            StatusCode::UNPROCESSABLE_ENTITY
        );
        assert_eq!(
            app.oneshot(request(" ".repeat(512 * 1024 + 1)))
                .await
                .unwrap()
                .status(),
            StatusCode::PAYLOAD_TOO_LARGE
        );
    }

    #[tokio::test]
    async fn disabled_bonjour_manager_and_http_shut_down_without_advertising() {
        let (sender, _receiver) = mpsc::channel(1);
        let shared = Arc::new(SharedState::new(
            settings(),
            String::new(),
            String::new(),
            String::new(),
            Vec::new(),
            sender,
        ));
        let shutdown = CancellationToken::new();
        let tasks = start_web_server(0, false, shared, PathBuf::new(), shutdown.clone())
            .await
            .unwrap();
        assert_eq!(tasks.len(), 3);
        tasks.close();
        shutdown.cancel();
        tokio::time::timeout(std::time::Duration::from_secs(1), tasks.wait())
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn bonjour_toggle_persists_and_notifies_without_changing_pairing() {
        let directory =
            std::env::temp_dir().join(format!("lg-hue-sync-bonjour-{}", std::process::id()));
        std::fs::create_dir(&directory).unwrap();
        let path = directory.join("config.json");
        Config::update(
            &path,
            Some(Config::new_default("", "preserve-pairing", "", "")),
            |_| Ok(()),
        )
        .unwrap();
        let (sender, _commands) = mpsc::channel(1);
        let (bonjour, mut changes) = watch::channel(false);
        let state = AppState {
            shared: Arc::new(SharedState::new(
                settings(),
                String::new(),
                String::new(),
                String::new(),
                Vec::new(),
                sender,
            )),
            config_path: path.clone(),
            bonjour: Arc::new(Mutex::new(bonjour)),
        };
        for enabled in [true, false] {
            let response = set_bonjour(State(state.clone()), Json(BonjourSettings { enabled }))
                .await
                .unwrap();
            assert_eq!(response.0.enabled, enabled);
            changes.changed().await.unwrap();
            assert_eq!(*changes.borrow_and_update(), enabled);
            let saved = Config::load(&path).unwrap();
            assert_eq!(saved.bonjour_enabled, enabled);
            assert_eq!(saved.username, "preserve-pairing");
            assert_eq!(
                status(State(state.clone())).await.0.bonjour_enabled,
                enabled
            );
        }
        std::fs::remove_file(path).unwrap();
        std::fs::remove_file(directory.join("config.json.lock")).unwrap();
        std::fs::remove_dir(directory).unwrap();
    }

    #[tokio::test]
    async fn failed_bonjour_save_does_not_change_discovery() {
        let (sender, _commands) = mpsc::channel(1);
        let (bonjour, mut changes) = watch::channel(false);
        let state = AppState {
            shared: Arc::new(SharedState::new(
                settings(),
                String::new(),
                String::new(),
                String::new(),
                Vec::new(),
                sender,
            )),
            config_path: std::env::temp_dir()
                .join(format!("lg-hue-sync-missing-{}", std::process::id()))
                .join("config.json"),
            bonjour: Arc::new(Mutex::new(bonjour)),
        };
        let result = set_bonjour(
            State(state.clone()),
            Json(BonjourSettings { enabled: true }),
        )
        .await;
        assert!(matches!(result, Err(ApiError::SetupFailed(_))));
        assert!(!*changes.borrow_and_update());
        assert!(!changes.has_changed().unwrap());
        drop(changes);
        let result = set_bonjour(State(state), Json(BonjourSettings { enabled: true })).await;
        assert!(matches!(result, Err(ApiError::SetupFailed(_))));
    }

    #[tokio::test]
    async fn openapi_describes_registered_routes_and_wire_schemas() {
        use tower::ServiceExt;
        let (app, _receiver) = test_router();
        let response = app
            .clone()
            .oneshot(
                axum::http::Request::builder()
                    .uri("/openapi.json")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()["content-type"], "application/json");
        let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024)
            .await
            .unwrap();
        let document: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(document["info"]["version"], env!("CARGO_PKG_VERSION"));
        let paths = document["paths"].as_object().unwrap();
        assert_eq!(paths.len(), 21);
        for (path, method) in [
            ("/api/status", "get"),
            ("/api/display", "get"),
            ("/api/display/{action}", "post"),
            ("/api/preview/frame", "get"),
            ("/api/preview/render", "post"),
            ("/api/bonjour", "post"),
            ("/api/start", "post"),
            ("/api/stop", "post"),
            ("/api/toggle", "post"),
            ("/api/settings", "post"),
            ("/api/presets/{name}", "post"),
            ("/api/presets/{name}/reset", "post"),
            ("/api/nanoleaf/alignment", "post"),
            ("/api/hue/areas", "get"),
            ("/api/hue/area", "post"),
            ("/api/save-config", "post"),
            ("/api/restart", "post"),
            ("/api/sync-bridge", "post"),
            ("/api/pair/hue", "post"),
            ("/api/pair/nanoleaf", "post"),
            ("/api/test-pattern/{pattern}", "post"),
        ] {
            assert!(paths[path].get(method).is_some(), "{method} {path}");
        }
        assert!(paths["/api/sync-bridge"]["post"]["responses"]
            .get("202")
            .is_some());
        let schemas = &document["components"]["schemas"];
        let live = &schemas["LiveSettings"];
        assert_eq!(live["properties"]["gamma"]["minimum"], 0.5);
        assert_eq!(live["properties"]["gamma"]["maximum"], 2.0);
        assert!(live["required"]
            .as_array()
            .unwrap()
            .contains(&serde_json::json!("gamma")));
        assert!(!live["required"]
            .as_array()
            .unwrap()
            .contains(&serde_json::json!("auto_tv_power")));
        assert_eq!(
            schemas["NanoleafAlignment"]["properties"]["perimeter_offset"]["maximum"],
            39
        );
        assert_eq!(live["properties"]["auto_tv_power"]["default"], true);
        let status_parts = schemas["StatusResponse"]["allOf"].as_array().unwrap();
        assert!(status_parts
            .iter()
            .any(|part| part["$ref"] == "#/components/schemas/SyncStatus"));
        let properties = status_parts
            .iter()
            .find_map(|part| part.get("properties"))
            .unwrap();
        let hue_pair = &properties["hue_colors"]["items"];
        assert_eq!(hue_pair["type"], "array");
        assert_eq!(hue_pair["prefixItems"][0]["type"], "integer");
        assert_eq!(hue_pair["prefixItems"][1]["type"], "object");
        assert_eq!(hue_pair["items"], false);
        assert!(schemas.get("Config").is_none());
        assert!(schemas.get("NanoleafConfig").is_none());
        assert_eq!(schemas["SyncState"]["enum"][0], "starting");
        assert!(schemas.get("Diagnostics").is_some());
        let response = app
            .oneshot(
                axum::http::Request::builder()
                    .uri("/api/status")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024)
            .await
            .unwrap();
        let status: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert!(status.get("pipeline_state").is_some());
        assert!(status.get("sync").is_none());
        assert_eq!(status["diagnostics"]["sampled_unix_ms"], 0);
        assert_eq!(status["diagnostics"]["video"]["available"], false);
    }

    #[test]
    fn coarse_tuning_range_extremes_validate() {
        let mut settings = settings();
        settings.hue_output_brightness = 0.0;
        settings.nanoleaf_output_brightness = 2.5;
        assert!(settings.validate().is_ok());
        settings.hue_output_brightness = 2.5;
        settings.nanoleaf_output_brightness = 0.0;
        settings.saturation_boost = 0.5;
        settings.gamma = 0.5;
        settings.peak_weight = 1.0;
        settings.smoothing_factor = 1.0;
        settings.rise_smoothing_factor = 1.0;
        settings.fall_smoothing_factor = 1.0;
        assert!(settings.validate().is_ok());
        settings.saturation_boost = 0.49;
        assert!(settings.validate().is_err());
    }

    #[tokio::test]
    async fn display_rejects_unknown_actions_without_device_access() {
        use tower::ServiceExt;
        let (app, _) = test_router();
        let response = app
            .oneshot(
                axum::http::Request::builder()
                    .method("POST")
                    .uri("/api/display/power-off")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), axum::http::StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn documented_router_preserves_requests_and_errors() {
        use tower::ServiceExt;
        let (app, mut receiver) = test_router();
        for (method, path, body, expected, content_type) in [
            (
                "GET",
                "/api/start",
                "",
                StatusCode::METHOD_NOT_ALLOWED,
                None,
            ),
            (
                "POST",
                "/api/settings",
                "{",
                StatusCode::BAD_REQUEST,
                Some("text/plain"),
            ),
            (
                "POST",
                "/api/settings",
                "{}",
                StatusCode::UNPROCESSABLE_ENTITY,
                Some("text/plain"),
            ),
            (
                "POST",
                "/api/presets/unknown",
                "",
                StatusCode::BAD_REQUEST,
                Some("application/json"),
            ),
            (
                "POST",
                "/api/test-pattern/unknown",
                "",
                StatusCode::BAD_REQUEST,
                Some("application/json"),
            ),
            (
                "POST",
                "/api/sync-bridge",
                "",
                StatusCode::ACCEPTED,
                Some("application/json"),
            ),
        ] {
            let response = app
                .clone()
                .oneshot(
                    axum::http::Request::builder()
                        .method(method)
                        .uri(path)
                        .header("content-type", "application/json")
                        .body(axum::body::Body::from(body))
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), expected, "{method} {path}");
            if let Some(content_type) = content_type {
                assert!(response.headers()["content-type"]
                    .to_str()
                    .unwrap()
                    .starts_with(content_type));
            }
        }
        assert!(matches!(
            receiver.recv().await,
            Some(ControlCommand::SyncBridge)
        ));
        drop(receiver);
        let response = app
            .oneshot(
                axum::http::Request::builder()
                    .method("POST")
                    .uri("/api/start")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
        let bytes = axum::body::to_bytes(response.into_body(), 1024)
            .await
            .unwrap();
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&bytes).unwrap(),
            serde_json::json!({"error": "sync controller is unavailable"})
        );
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
            ("highChroma", 0.0),
            ("neonContrast", 0.0),
            ("darkSceneDetail", 0.0),
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
        assert_eq!(input.spatial_blend, 0.25);
        assert_eq!(input.rise_smoothing_factor, 0.35);
        assert!(input.strict_blackout);
        assert_eq!(input.hue_output_brightness, 0.7);
        assert_eq!(input.nanoleaf_output_brightness, 0.8);
        assert!(!input.hue_sync_enabled);
        assert_eq!(input.nanoleaf_alignment.perimeter_offset, 7);
        assert!(input.validate().is_ok());
        assert!(Preset::named("unknown").is_err());
    }

    #[tokio::test]
    async fn control_requests_preserve_actual_state_until_controller_transition() {
        let (sender, mut receiver) = mpsc::channel(2);
        let shared = Arc::new(SharedState::new(
            settings(),
            String::new(),
            String::new(),
            String::new(),
            Vec::new(),
            sender,
        ));
        shared.set_sync_state(SyncState::Paused, None);
        shared.set_requested_running(false);
        let state = AppState {
            shared: shared.clone(),
            config_path: PathBuf::new(),
            bonjour: Arc::new(Mutex::new(watch::channel(false).0)),
        };
        let _ = start(State(state.clone())).await.unwrap();
        let snapshot = status(State(state.clone())).await.0;
        assert!(!snapshot.is_syncing);
        assert!(snapshot.sync.requested_running);
        assert_eq!(snapshot.sync.pipeline_state, SyncState::Paused);
        assert!(matches!(receiver.recv().await, Some(ControlCommand::Start)));
        shared.set_sync_state(SyncState::Starting, None);
        assert!(!status(State(state.clone())).await.0.is_syncing);
        shared.set_sync_state(SyncState::Running, None);
        let _ = stop(State(state.clone())).await.unwrap();
        let snapshot = status(State(state.clone())).await.0;
        assert!(snapshot.is_syncing);
        assert!(!snapshot.sync.requested_running);
        assert!(matches!(receiver.recv().await, Some(ControlCommand::Stop)));
        for phase in [
            SyncState::Stopping,
            SyncState::UnsupportedSource,
            SyncState::Paused,
            SyncState::Recovering,
            SyncState::Faulted,
        ] {
            shared.set_sync_state(phase, Some("capture unavailable".into()));
            let snapshot = status(State(state.clone())).await.0;
            assert!(!snapshot.is_syncing);
            let json = serde_json::to_value(snapshot).unwrap();
            assert_eq!(json["requested_running"], false);
            assert_eq!(json["last_error"], "capture unavailable");
            assert!(json.get("fps").is_some());
        }
        shared.set_sync_state(SyncState::Running, None);
        assert!(status(State(state)).await.0.sync.last_error.is_none());
    }

    #[tokio::test]
    async fn toggle_follows_actual_state_after_automatic_transitions() {
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
            shared: shared.clone(),
            config_path: PathBuf::new(),
            bonjour: Arc::new(Mutex::new(watch::channel(false).0)),
        };
        shared.set_requested_running(false);
        shared.set_sync_state(SyncState::Running, None);
        let _ = toggle(State(state.clone())).await.unwrap();
        assert!(matches!(receiver.recv().await, Some(ControlCommand::Stop)));
        for phase in [SyncState::Paused, SyncState::Faulted] {
            shared.set_requested_running(true);
            shared.set_sync_state(phase, None);
            let _ = toggle(State(state.clone())).await.unwrap();
            assert!(matches!(receiver.recv().await, Some(ControlCommand::Start)));
        }
    }

    #[tokio::test]
    async fn closed_or_full_command_queue_does_not_change_request_or_actual_state() {
        let (sender, mut receiver) = mpsc::channel(1);
        sender.send(ControlCommand::Restart).await.unwrap();
        let shared = SharedState::new(
            settings(),
            String::new(),
            String::new(),
            String::new(),
            Vec::new(),
            sender,
        );
        shared.set_requested_running(false);
        shared.set_sync_state(SyncState::Faulted, Some("capture busy".into()));
        assert!(tokio::time::timeout(
            std::time::Duration::from_millis(10),
            shared.send_command(ControlCommand::Start)
        )
        .await
        .is_err());
        assert!(!shared.sync_status.read().unwrap().requested_running);
        assert!(matches!(
            receiver.recv().await,
            Some(ControlCommand::Restart)
        ));
        drop(receiver);
        assert!(shared.send_command(ControlCommand::Start).await.is_err());
        let snapshot = shared.sync_status.read().unwrap();
        assert!(!snapshot.requested_running);
        assert_eq!(snapshot.pipeline_state, SyncState::Faulted);
        assert_eq!(snapshot.last_error.as_deref(), Some("capture busy"));
    }

    #[tokio::test]
    async fn color_presets_preserve_independent_response() {
        let defaults = Config::new_default("", "", "", "");
        assert_eq!(defaults.active_preset.as_deref(), Some("neutral"));
        assert_eq!(
            (
                defaults.saturation_boost,
                defaults.spatial_blend,
                defaults.smoothing_factor
            ),
            (1.0, 0.15, 0.35)
        );
        let (sender, mut receiver) = mpsc::channel(8);
        let shared = SharedState::new(
            settings(),
            String::new(),
            String::new(),
            String::new(),
            Vec::new(),
            sender,
        );
        shared
            .select_preset("neonContrast".into(), false)
            .await
            .unwrap();
        receiver.recv().await.unwrap();
        let mut slow = shared.current_settings.read().unwrap().clone();
        slow.smoothing_factor = 0.2;
        slow.rise_smoothing_factor = 0.18;
        slow.fall_smoothing_factor = 0.22;
        slow.max_color_step = 6;
        slow.spatial_blend = 0.3;
        shared.apply_settings(slow.clone()).await.unwrap();
        receiver.recv().await.unwrap();
        assert!(
            !shared.preset_modified(),
            "Response changes must not modify the colour profile"
        );
        slow.saturation_boost = 1.8;
        shared.apply_settings(slow.clone()).await.unwrap();
        receiver.recv().await.unwrap();
        let (_, saved) = shared.preset_save_snapshot(&slow);
        shared.mark_presets_saved(saved);
        for (name, reset) in [
            ("darkSceneDetail", false),
            ("neonContrast", false),
            ("neonContrast", true),
            ("softColor", false),
            ("vividColor", false),
            ("neutral", false),
        ] {
            shared.select_preset(name.into(), reset).await.unwrap();
            receiver.recv().await.unwrap();
            let current = shared.current_settings.read().unwrap();
            assert_eq!(
                (
                    current.smoothing_factor,
                    current.rise_smoothing_factor,
                    current.fall_smoothing_factor,
                    current.max_color_step
                ),
                (0.2, 0.18, 0.22, 6)
            );
            assert_eq!(current.spatial_blend, 0.3);
            if name == "neonContrast" && !reset {
                assert_eq!(current.saturation_boost, 1.8);
            }
        }
    }

    #[tokio::test]
    async fn preset_request_queues_live_settings() {
        let (sender, mut receiver) = mpsc::channel(2);
        let mut initial = settings();
        initial.hue_output_brightness = 0.7;
        let shared = Arc::new(SharedState::new(
            initial,
            String::new(),
            String::new(),
            String::new(),
            Vec::new(),
            sender,
        ));
        shared
            .select_preset("lowStimulation".into(), false)
            .await
            .unwrap();
        assert!(
            matches!(receiver.recv().await, Some(ControlCommand::ApplySettings(value)) if value.brightness_multiplier == 0.55)
        );
        let mut edited = shared.current_settings.read().unwrap().clone();
        edited.brightness_multiplier = 0.65;
        shared.apply_settings(edited.clone()).await.unwrap();
        receiver.recv().await.unwrap();
        shared
            .select_preset("lowStimulation".into(), false)
            .await
            .unwrap();
        assert!(
            receiver.try_recv().is_err(),
            "Selecting the active preset must preserve edits"
        );
        assert!(shared.preset_modified());
        let (active, saved) = shared.preset_save_snapshot(&edited);
        assert_eq!(active.as_deref(), Some("lowStimulation"));
        assert_eq!(saved["lowStimulation"].brightness_multiplier, 0.65);
        shared.mark_presets_saved(saved);
        assert!(!shared.preset_modified());
        shared.select_preset("neutral".into(), false).await.unwrap();
        receiver.recv().await.unwrap();
        shared
            .select_preset("lowStimulation".into(), false)
            .await
            .unwrap();
        receiver.recv().await.unwrap();
        assert_eq!(
            shared
                .current_settings
                .read()
                .unwrap()
                .brightness_multiplier,
            0.65
        );
        shared
            .select_preset("lowStimulation".into(), true)
            .await
            .unwrap();
        receiver.recv().await.unwrap();
        let reset = shared.current_settings.read().unwrap().clone();
        assert_eq!(reset.brightness_multiplier, 0.55);
        assert_eq!(reset.hue_output_brightness, 0.7);
        assert!(shared.preset_modified());
        let (_, saved) = shared.preset_save_snapshot(&reset);
        assert!(!saved.contains_key("lowStimulation"));
        drop(receiver);
        assert!(shared.select_preset("neutral".into(), false).await.is_err());
        assert_eq!(
            shared.active_preset.read().unwrap().as_deref(),
            Some("lowStimulation")
        );
        assert_eq!(
            shared
                .current_settings
                .read()
                .unwrap()
                .brightness_multiplier,
            0.55
        );
        shared.configure_presets(
            Some("unknown".into()),
            BTreeMap::from([("unknown".into(), PresetTuning::capture(&reset))]),
        );
        assert!(shared.active_preset.read().unwrap().is_none());
        assert!(shared.saved_presets.read().unwrap().is_empty());
    }

    #[tokio::test]
    async fn failed_save_preserves_saved_preset_baseline() {
        let (sender, mut receiver) = mpsc::channel(2);
        let shared = Arc::new(SharedState::new(
            settings(),
            String::new(),
            String::new(),
            String::new(),
            Vec::new(),
            sender,
        ));
        shared.select_preset("neutral".into(), false).await.unwrap();
        receiver.recv().await.unwrap();
        let mut edited = shared.current_settings.read().unwrap().clone();
        edited.gamma = 1.5;
        shared.apply_settings(edited).await.unwrap();
        receiver.recv().await.unwrap();
        let state = AppState {
            shared: shared.clone(),
            config_path: PathBuf::new(),
            bonjour: Arc::new(Mutex::new(watch::channel(false).0)),
        };
        let save = tokio::spawn(save_config(State(state)));
        let Some(ControlCommand::SaveConfig(ack)) = receiver.recv().await else {
            panic!("expected save command")
        };
        ack.send(Err("disk unavailable".into())).unwrap();
        assert!(save.await.unwrap().is_err());
        assert!(shared.saved_presets.read().unwrap().is_empty());
        assert!(shared.preset_modified());
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
        json.as_object_mut().unwrap().remove("depth_influence");
        let legacy: LiveSettings = serde_json::from_value(json).unwrap();
        assert_eq!(legacy.depth_influence, 0.5);
        for value in [0.0, 0.5, 1.0] {
            let mut input = settings();
            input.depth_influence = value;
            assert!(input.validate().is_ok());
            Preset::named("vividColor").unwrap().apply(&mut input);
            assert_eq!(input.depth_influence, value);
        }
        for value in [-0.1, 1.1, f32::NAN, f32::INFINITY] {
            let mut input = settings();
            input.depth_influence = value;
            assert!(input.validate().is_err());
        }
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
