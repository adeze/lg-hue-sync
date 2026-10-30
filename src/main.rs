mod capture;
mod capture_source;
mod color;
mod config;
mod hue;
mod local_http;
mod luna;
mod nanoleaf;
mod runtime;
mod setup;
mod tv_power;
mod video_mode;
mod web;

use anyhow::{anyhow, Context, Result};
use clap::{Parser, Subcommand};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn, Level};
use tracing_subscriber::FmtSubscriber;

use capture::{
    create_capture, detect_source_fps, release_capture, CaptureRecovery, MockCapture,
    RecoveryAction, ScreenCapture, VtCapture,
};
use color::{RgbColor, ZoneSampler};
use config::{Config, ConfigError};
use hue::{
    reconnect::HueReconnect, sync_entertainment_areas, HueDtlsClient, HueStreamPacketBuilder,
};
use nanoleaf::{NanoleafPerimeterSampler, NanoleafUdpStreamer};
use runtime::{CaptureSourcePolicy, PendingCommands, PipelineState, RetryState};
use setup::{run_pair, run_pair_nanoleaf, run_sync_hue};
use web::{start_web_server, CalibrationPattern, LiveSettings, SharedState, SyncState};

#[derive(Parser)]
#[command(name = "lg-hue-sync")]
#[command(version)]
#[command(about = "High-performance native screen capture and ambient lighting synchronizer for LG webOS (Hue & Nanoleaf 4D)", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Start the live sync daemon
    Run {
        #[arg(short, long, default_value = "config.json")]
        config: PathBuf,
    },
    /// Stream a test color pattern to Hue lights to verify DTLS connection
    TestPattern {
        #[arg(short, long, default_value = "config.json")]
        config: PathBuf,
    },
    /// Stream a test color pattern to Nanoleaf 4D lightstrip on port 60222
    TestNanoleaf {
        #[arg(short, long, default_value = "config.json")]
        config: PathBuf,
    },
    /// Run screen capture only and log sampled zone colors to console
    TestCapture {
        #[arg(short, long, default_value = "config.json")]
        config: PathBuf,
        /// Report central raw NV12 statistics without saving frame pixels or sending light output
        #[arg(long)]
        raw_nv12_stats: bool,
    },
    /// Discover and pair with a Philips Hue Bridge via pushlink button
    Pair {
        #[arg(short, long)]
        bridge: Option<String>,
        #[arg(short, long, default_value = "config.json")]
        output: PathBuf,
    },
    /// Re-sync entertainment area and 3D light coordinates from Philips Hue app
    SyncHue {
        #[arg(short, long, default_value = "config.json")]
        config: PathBuf,
        #[arg(short, long)]
        area: Option<String>,
    },
    /// Pair with Nanoleaf 4D controller and save credentials to config
    PairNanoleaf {
        #[arg(short, long)]
        ip: Option<String>,
        #[arg(short, long, default_value = "config.json")]
        config: PathBuf,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    let subscriber = FmtSubscriber::builder()
        .with_max_level(Level::INFO)
        .finish();
    tracing::subscriber::set_global_default(subscriber)?;

    let cli = Cli::parse();

    match cli.command {
        Commands::Run { config } => run_daemon(config).await,
        Commands::TestPattern { config } => run_test_pattern(config).await,
        Commands::TestNanoleaf { config } => run_test_nanoleaf(config).await,
        Commands::TestCapture {
            config,
            raw_nv12_stats,
        } => run_test_capture(config, raw_nv12_stats).await,
        Commands::Pair { bridge, output } => run_pair(bridge, output).await,
        Commands::SyncHue { config, area } => run_sync_hue(config, area).await,
        Commands::PairNanoleaf { ip, config } => run_pair_nanoleaf(ip, config).await,
    }
}

struct ConnectedHue {
    output: hue::output::RepeatingHueStream,
    bridge_ip: String,
}

fn reconnect_hue(config: &Config) -> Result<ConnectedHue> {
    let verified = hue::connect_bridge_tls(&config.bridge_ip, config.hue_trust())?;
    let mut resolved = config.clone();
    resolved.bridge_ip = verified.get_ref().peer_addr()?.ip().to_string();
    drop(verified);
    let config = &resolved;
    set_hue_stream_active(config, true)?;
    let application_id =
        hue::fetch_application_id(&config.bridge_ip, &config.username, config.hue_trust())?;
    let client = HueDtlsClient::connect(&config.bridge_ip, &application_id, &config.clientkey)?;
    Ok(ConnectedHue {
        output: hue::output::RepeatingHueStream::new(client)?,
        bridge_ip: config.bridge_ip.clone(),
    })
}

fn hue_configuration_id(config: &Config) -> Option<String> {
    config
        .entertainment_configuration_id
        .as_deref()
        .filter(|id| hue::valid_configuration_id(id))
        .map(ToOwned::to_owned)
}

fn set_hue_stream_active(config: &Config, active: bool) -> Result<()> {
    let configuration_id = hue_configuration_id(config)
        .ok_or_else(|| anyhow!("Hue V2 Entertainment configuration has not been selected"))?;
    hue::set_stream_active(
        &config.bridge_ip,
        &config.username,
        &configuration_id,
        config.hue_trust(),
        active,
    )
}

struct HueStatus {
    events: hue::events::HueEvents,
    enabled: bool,
    blocked: bool,
    needs_refresh: bool,
}

impl HueStatus {
    fn new() -> Result<Self> {
        let events = hue::events::HueEvents::new(
            |config| {
                let id = hue_configuration_id(config)
                    .ok_or_else(|| anyhow!("Select a Hue Entertainment Area"))?;
                let mut snapshot = hue::fetch_stream_configuration(
                    &config.bridge_ip,
                    &config.username,
                    &id,
                    config.hue_trust(),
                )?;
                let app_id = hue::fetch_application_id(
                    &config.bridge_ip,
                    &config.username,
                    config.hue_trust(),
                )?;
                snapshot["_application_id"] = serde_json::Value::String(app_id);
                Ok(snapshot)
            },
            |config| {
                use std::io::Write;
                if !config.hue_trust().known() || !hue::valid_application_key(&config.username) {
                    anyhow::bail!("Invalid Hue application key");
                }
                let mut stream = hue::connect_bridge_tls(&config.bridge_ip, config.hue_trust())?;
                let socket = stream.get_ref().try_clone()?;
                socket.set_read_timeout(Some(Duration::from_secs(60)))?;
                write!(stream, "GET /eventstream/clip/v2 HTTP/1.1\r\nHost: {}\r\nhue-application-key: {}\r\nAccept: text/event-stream\r\nConnection: keep-alive\r\n\r\n", config.bridge_ip, config.username)?;
                stream.flush()?;
                Ok(hue::events::EventConnection::new(Box::new(stream), socket))
            },
        )?;
        Ok(Self {
            events,
            enabled: false,
            blocked: false,
            needs_refresh: false,
        })
    }

    fn request(&mut self, config: Option<&Config>) {
        if self.enabled == config.is_some() {
            return;
        }
        self.enabled = config.is_some();
        self.events.request(config);
    }
}

fn observe_hue_status(
    worker: &mut HueStatus,
    config: &Config,
    commands: &mut PendingCommands,
    enabled: bool,
) -> Option<hue::StreamState> {
    // Defer processing during management work without cancelling the subscription.
    worker.request(enabled.then_some(config));
    if enabled && !commands.has_management_action() {
        poll_hue_status(worker, config, commands)
    } else {
        None
    }
}

fn poll_hue_status(
    worker: &mut HueStatus,
    config: &Config,
    commands: &mut PendingCommands,
) -> Option<hue::StreamState> {
    worker.request(Some(config));
    match worker.events.poll()? {
        Ok(event) => {
            commands.sync_bridge |= event.topology_changed || worker.needs_refresh;
            worker.blocked |= event.topology_changed;
            worker.needs_refresh |= event.topology_changed;
            let application_id = event
                .configuration
                .get("_application_id")
                .and_then(serde_json::Value::as_str)?;
            match hue::parse_stream_state(&event.configuration, application_id) {
                Ok(mut state) => {
                    if event.stream_interrupted {
                        state.active = false;
                    }
                    Some(state)
                }
                Err(error) => {
                    warn!("Hue state unavailable: {error:#}");
                    None
                }
            }
        }
        Err(error) => {
            worker.blocked |= error
                .downcast_ref::<hue::events::SessionInvalid>()
                .is_some();
            warn!("Hue events unavailable: {error:#}");
            None
        }
    }
}

fn calibration_pattern(state: &SharedState) -> Option<CalibrationPattern> {
    *state.calibration_pattern.read().unwrap()
}

fn calibration_zone_colors(
    pattern: CalibrationPattern,
    zones: &[config::LightZone],
) -> Vec<(u8, RgbColor)> {
    zones
        .iter()
        .map(|zone| {
            (
                zone.channel_id,
                calibration_color(pattern, zone_center(zone)),
            )
        })
        .collect()
}

fn zone_center(zone: &config::LightZone) -> (f32, f32) {
    (
        (zone.x_min + zone.x_max) * 0.5,
        (zone.y_min + zone.y_max) * 0.5,
    )
}

fn calibration_perimeter_colors(pattern: CalibrationPattern, count: usize) -> Vec<RgbColor> {
    let active_segment = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| (duration.as_millis() / 150) as usize % count.max(1))
        .unwrap_or(0);
    (0..count)
        .map(|index| {
            if pattern == CalibrationPattern::Perimeter {
                return if index == active_segment {
                    RgbColor::new(255, 255, 255)
                } else {
                    RgbColor::new(0, 0, 0)
                };
            }
            let position = if count == 40 {
                if (16..=28).contains(&index) {
                    (0.5, 0.0)
                } else if (29..=35).contains(&index) {
                    (1.0, 0.5)
                } else if index <= 7 || index >= 36 {
                    (0.5, 1.0)
                } else {
                    (0.0, 0.5)
                }
            } else {
                (index as f32 / count.max(1) as f32, 0.5)
            };
            calibration_color(pattern, position)
        })
        .collect()
}

fn calibration_color(pattern: CalibrationPattern, (x, y): (f32, f32)) -> RgbColor {
    match pattern {
        CalibrationPattern::Red => RgbColor::new(255, 0, 0),
        CalibrationPattern::Green => RgbColor::new(0, 255, 0),
        CalibrationPattern::Blue => RgbColor::new(0, 0, 255),
        CalibrationPattern::White => RgbColor::new(255, 255, 255),
        CalibrationPattern::WarmWhite => RgbColor::new(255, 180, 107),
        CalibrationPattern::DaylightWhite => RgbColor::new(255, 255, 255),
        CalibrationPattern::Quadrants => {
            if y < 0.33 {
                RgbColor::new(255, 0, 0)
            } else if x > 0.66 {
                RgbColor::new(0, 255, 0)
            } else if y > 0.66 {
                RgbColor::new(0, 0, 255)
            } else {
                RgbColor::new(255, 255, 0)
            }
        }
        CalibrationPattern::Perimeter => RgbColor::new(255, 255, 255),
    }
}

fn frame_average(data: &[u8], width: u32, height: u32, is_bgra: bool) -> RgbColor {
    let mut total = [0u64; 3];
    let mut count = 0u64;
    for y in (0..height as usize).step_by(8) {
        for x in (0..width as usize).step_by(8) {
            let offset = (y * width as usize + x) * 4;
            if offset + 2 >= data.len() {
                continue;
            }
            let (r, g, b) = if is_bgra {
                (data[offset + 2], data[offset + 1], data[offset])
            } else {
                (data[offset], data[offset + 1], data[offset + 2])
            };
            total[0] += r as u64;
            total[1] += g as u64;
            total[2] += b as u64;
            count += 1;
        }
    }
    RgbColor::new(
        total[0].checked_div(count).unwrap_or(0) as u8,
        total[1].checked_div(count).unwrap_or(0) as u8,
        total[2].checked_div(count).unwrap_or(0) as u8,
    )
}

/// Suppresses duplicate transport frames while retaining a 2 Hz keepalive.
fn colors_changed(previous: &[RgbColor], current: &[RgbColor]) -> bool {
    previous.len() != current.len()
        || previous
            .iter()
            .zip(current)
            .any(|(left, right)| left.delta(*right) >= 0.005)
}

async fn run_daemon(config_path: PathBuf) -> Result<()> {
    let mut config = match Config::load(&config_path) {
        Ok(config) => config,
        Err(ConfigError::Open { source, .. }) if source.kind() == std::io::ErrorKind::NotFound => {
            info!("No configuration found; starting dashboard setup mode.");
            Config::new_default("", "", "", "")
        }
        Err(error) => return Err(error.into()),
    };

    let hue_active =
        config.hue_enabled && !config.bridge_ip.is_empty() && !config.username.is_empty();
    let nanoleaf_active = config
        .nanoleaf
        .as_ref()
        .map(|n| n.enabled && !n.ip.is_empty() && !n.auth_token.is_empty())
        .unwrap_or(false);

    let mut observed_tv_power = if config.auto_tv_power {
        match tv_power::is_active().await {
            Ok(state) => state,
            Err(error) => {
                warn!("Could not read TV power state; starting normally and retrying: {error}");
                None
            }
        }
    } else {
        None
    };
    let startup_requested = observed_tv_power != Some(false);
    let initial_source = crate::capture_source::current_source().await;
    info!("Foreground capture source: {}", initial_source.label());
    let mut source_policy =
        CaptureSourcePolicy::new(initial_source.compatible(), startup_requested);
    let pipeline_should_run = startup_requested && !source_policy.blocked();
    if !pipeline_should_run && hue_active {
        let _ = set_hue_stream_active(&config, false);
    }

    if hue_active {
        let bridge_identity = (
            config.bridge_ip.clone(),
            config.username.clone(),
            config.hue_bridge_certificate_sha256.clone(),
            config.hue_bridge_id.clone(),
        );
        match sync_entertainment_areas(
            &config.bridge_ip,
            &config.username,
            config.entertainment_configuration_id.as_deref(),
            config.hue_trust(),
        ) {
            Ok(area) => {
                config.entertainment_area_id = area.configuration_id.clone();
                config.entertainment_configuration_id = Some(area.configuration_id);
                config.hue_bridge_id = Some(area.bridge_id.clone());
                config.hue_bridge_certificate_sha256 = Some(area.certificate_sha256);
                config.zones = area.zones;
                if let Err(error) = Config::update(&config_path, None, |current| {
                    if (
                        current.bridge_ip.as_str(),
                        current.username.as_str(),
                        current.hue_bridge_certificate_sha256.as_ref(),
                        current.hue_bridge_id.as_ref(),
                    ) != (
                        bridge_identity.0.as_str(),
                        bridge_identity.1.as_str(),
                        bridge_identity.2.as_ref(),
                        bridge_identity.3.as_ref(),
                    ) {
                        anyhow::bail!("Hue Bridge configuration changed during sync; retry");
                    }
                    current.entertainment_area_id = config.entertainment_area_id.clone();
                    current.entertainment_configuration_id =
                        config.entertainment_configuration_id.clone();
                    current.hue_bridge_certificate_sha256 =
                        config.hue_bridge_certificate_sha256.clone();
                    current.hue_bridge_id = config.hue_bridge_id.clone();
                    current.zones = config.zones.clone();
                    Ok(())
                }) {
                    warn!(
                        "Resolved Hue V2 configuration but could not persist it: {}",
                        error
                    );
                }
            }
            Err(error) => warn!(
                "Hue configuration refresh failed; retaining saved configuration: {}",
                error
            ),
        }
    }

    info!(
        "Loaded configuration (Hue: {}, Nanoleaf: {})",
        if hue_active {
            format!("ACTIVE @ {}", config.bridge_ip)
        } else {
            "DISABLED".to_string()
        },
        if nanoleaf_active {
            format!("ACTIVE @ {}", config.nanoleaf.as_ref().unwrap().ip)
        } else {
            "DISABLED".to_string()
        }
    );

    // Setup graceful shutdown handler for SIGINT and SIGTERM
    let running = Arc::new(AtomicBool::new(true));
    let shutdown = CancellationToken::new();
    let signal_shutdown = shutdown.clone();
    let r = running.clone();
    let signal_task = tokio::spawn(async move {
        #[cfg(unix)]
        {
            use tokio::signal::unix::{signal, SignalKind};
            let mut sigterm = match signal(SignalKind::terminate()) {
                Ok(s) => Some(s),
                Err(e) => {
                    tracing::warn!("Failed to register SIGTERM handler: {}", e);
                    None
                }
            };
            tokio::select! {
                _ = tokio::signal::ctrl_c() => {
                    info!("Received SIGINT (Ctrl+C). Stopping sync...");
                }
                _ = async {
                    if let Some(ref mut st) = sigterm {
                        st.recv().await;
                    } else {
                        std::future::pending::<()>().await;
                    }
                } => {
                    info!("Received SIGTERM from systemd. Stopping sync gracefully...");
                }
            }
        }
        #[cfg(not(unix))]
        {
            tokio::signal::ctrl_c().await.ok();
            info!("Received shutdown signal. Stopping sync...");
        }
        r.store(false, Ordering::SeqCst);
        signal_shutdown.cancel();
    });

    // 1. Initialize Philips Hue DTLS client if active
    let mut hue_sync_enabled = config.hue_sync_enabled;
    let mut nanoleaf_sync_enabled = config.nanoleaf_sync_enabled;
    let mut hue_dtls = None;
    let mut hue_reconnect =
        HueReconnect::new(reconnect_hue, |config| set_hue_stream_active(config, false))?;
    let mut hue_status = HueStatus::new()?;
    // The first SSE snapshot triggers a layout refresh before Hue output starts.
    hue_status.blocked = hue_active;
    hue_status.needs_refresh = hue_active;

    for zone in &mut config.zones {
        zone.set_depth_influence(config.depth_influence);
    }
    let mut hue_sampler = if hue_active {
        Some(ZoneSampler::new(
            config.zones.clone(),
            0.35,
            config.hdr_tone_mapping,
            config.letterbox_detection,
            config.saturation_boost,
            config.noise_gate_threshold,
        ))
    } else {
        None
    };
    if let Some(ref mut sampler) = hue_sampler {
        sampler.set_temporal_response(config.rise_smoothing_factor, config.fall_smoothing_factor);
        sampler.set_strict_blackout(config.strict_blackout);
        sampler.set_peak_weight(config.peak_weight);
        sampler.set_spatial_blend(config.spatial_blend);
        sampler.set_gamma(config.gamma);
        sampler.set_max_color_step(config.max_color_step);
    }

    let mut hue_packet_builder = if hue_active {
        Some(HueStreamPacketBuilder::new(hue_configuration_id(&config)))
    } else {
        None
    };

    // 2. Initialize Nanoleaf 4D UDP streamer if active
    let (mut nanoleaf_sampler, mut nanoleaf_streamer) = if nanoleaf_active {
        let n_cfg = config.nanoleaf.as_ref().unwrap();
        let sampler = NanoleafPerimeterSampler::new_aligned(
            n_cfg.segments,
            &n_cfg.panel_ids,
            config.hdr_tone_mapping,
            config.saturation_boost,
            config.noise_gate_threshold,
            config.brightness_multiplier * config.nanoleaf_output_brightness,
            n_cfg.alignment,
        );
        let streamer = if nanoleaf_sync_enabled && pipeline_should_run {
            let port = nanoleaf::enable_external_control(&n_cfg.ip, &n_cfg.auth_token)?;
            let streamer = NanoleafUdpStreamer::new(&n_cfg.ip, port, sampler.panel_ids())?;
            info!(
                "[+] Nanoleaf 4D streaming ready: {} perimeter segments on UDP port {}",
                streamer.panel_count(),
                port
            );
            Some(streamer)
        } else {
            None
        };
        (Some(sampler), streamer)
    } else {
        (None, None)
    };
    if let Some(ref mut sampler) = nanoleaf_sampler {
        sampler.set_temporal_response(config.rise_smoothing_factor, config.fall_smoothing_factor);
        sampler.set_strict_blackout(config.strict_blackout);
        sampler.set_peak_weight(config.peak_weight);
        sampler.set_spatial_blend(config.spatial_blend);
        sampler.set_gamma(config.gamma);
        sampler.set_max_color_step(config.max_color_step);
    }

    // 3. Initialize capture
    let mut capture: Box<dyn ScreenCapture> = if pipeline_should_run {
        create_capture(config.capture_width, config.capture_height)
    } else {
        Box::new(MockCapture::new(
            config.capture_width,
            config.capture_height,
        ))
    };

    // Determine target framerate (supports auto-matching source refresh rate 23.976..60.0 Hz)
    let detected_fps = if config.fps == 0 {
        detect_source_fps().await
    } else {
        None
    };
    let initial_fps = if config.fps == 0 {
        detected_fps.unwrap_or(60.0)
    } else {
        config.fps as f64
    };
    let mut target_fps = initial_fps.clamp(20.0, 60.0);
    let mut frame_interval = Duration::from_secs_f64(1.0 / target_fps);

    info!(
        "Entering sync loop at {:.2} FPS ({:.2} ms cadence{}, mode: {}, HDR tone-mapping: {}, letterbox: {})...",
        target_fps,
        frame_interval.as_secs_f64() * 1000.0,
        if detected_fps.is_some() { " [source auto-matched]" } else { "" },
        if config.use_xy_gamut { "CIE 1931 xy (Gamut C)" } else { "sRGB" },
        config.hdr_tone_mapping,
        config.letterbox_detection
    );

    // Notify systemd that service is ready and streaming
    let status_desc = format!(
        "Streaming active ({:.1} FPS, {} Hue zones, {} Nanoleaf segments)",
        target_fps,
        if hue_active { config.zones.len() } else { 0 },
        nanoleaf_streamer
            .as_ref()
            .map(|s| s.panel_count())
            .unwrap_or(0)
    );
    let _ = sd_notify::notify(&[
        sd_notify::NotifyState::Ready,
        sd_notify::NotifyState::Status(&status_desc),
    ]);
    let mut watchdog = WatchdogHeartbeat::new();

    let initial_settings = LiveSettings {
        brightness_multiplier: config.brightness_multiplier,
        hue_output_brightness: config.hue_output_brightness,
        nanoleaf_output_brightness: config.nanoleaf_output_brightness,
        saturation_boost: config.saturation_boost,
        peak_weight: config.peak_weight,
        spatial_blend: config.spatial_blend,
        depth_influence: config.depth_influence,
        gamma: config.gamma,
        noise_gate_threshold: config.noise_gate_threshold,
        smoothing_factor: config.smoothing_factor,
        rise_smoothing_factor: config.rise_smoothing_factor,
        fall_smoothing_factor: config.fall_smoothing_factor,
        strict_blackout: config.strict_blackout,
        use_xy_gamut: config.use_xy_gamut,
        letterbox_detection: config.letterbox_detection,
        hdr_tone_mapping: config.hdr_tone_mapping,
        hue_sync_enabled,
        nanoleaf_sync_enabled,
        auto_tv_power: config.auto_tv_power,
        nanoleaf_alignment: config
            .nanoleaf
            .as_ref()
            .map(|nanoleaf| nanoleaf.alignment)
            .unwrap_or_default(),
        max_color_step: config.max_color_step,
    };

    let (command_tx, mut command_rx) = mpsc::channel(32);
    let shared_state = Arc::new(SharedState::new(
        initial_settings,
        config.bridge_ip.clone(),
        config
            .nanoleaf
            .as_ref()
            .map(|nanoleaf| nanoleaf.ip.clone())
            .unwrap_or_default(),
        format!("{}x{}", config.capture_width, config.capture_height),
        config.zones.clone(),
        command_tx,
    ));
    shared_state.configure_presets(
        config.active_preset.clone(),
        config.preset_overrides.clone(),
    );
    shared_state.set_requested_running(startup_requested);
    if !pipeline_should_run {
        shared_state.set_sync_state(SyncState::Paused, None);
    }
    shared_state.set_tv_power_state(observed_tv_power);
    shared_state.set_fps(target_fps as f32);

    shared_state
        .hue_connected
        .store(hue_dtls.is_some(), Ordering::Relaxed);
    shared_state
        .nanoleaf_connected
        .store(nanoleaf_streamer.is_some(), Ordering::Relaxed);
    shared_state
        .capture_hardware
        .store(capture.is_real_hardware(), Ordering::Relaxed);

    let web_shutdown = CancellationToken::new();
    let web_tasks = match start_web_server(
        8088,
        config.bonjour_enabled,
        shared_state.clone(),
        config_path.clone(),
        web_shutdown.clone(),
    )
    .await
    {
        Ok(tasks) => Some(tasks),
        Err(error) => {
            warn!("Failed to start web control server on port 8088: {}", error);
            None
        }
    };

    let mut current_hue_brightness = config.brightness_multiplier * config.hue_output_brightness;
    let mut current_use_xy = config.use_xy_gamut;
    let mut auto_tv_power = config.auto_tv_power;

    let mut last_channels: Vec<(u8, (u16, u16, u16))> = Vec::new();
    let mut last_nanoleaf_colors: Vec<RgbColor> = Vec::new();
    let mut last_heartbeat = tokio::time::Instant::now();
    let mut last_nanoleaf_heartbeat = tokio::time::Instant::now();
    let mut light_update_count = 0u32;
    let mut light_update_window = tokio::time::Instant::now();
    let mut capture_retry = RetryState::new();
    if !capture.is_real_hardware() {
        capture_retry.failure();
    }
    let mut capture_recovery = CaptureRecovery::default();
    let mut capture_faulted = false;
    let mut last_tv_power_poll = tokio::time::Instant::now() - Duration::from_secs(2);
    let mut last_tv_power_warning = tokio::time::Instant::now() - Duration::from_secs(30);
    let mut last_source_fps_poll = tokio::time::Instant::now();
    let mut hue_retry = RetryState::new();
    let mut nanoleaf_retry = RetryState::new();
    let mut nanoleaf_only_frame_count = 0u64;
    let mut nanoleaf_only_global = RgbColor::new(0, 0, 0);
    let mut pending_commands = PendingCommands {
        desired_running: (!pipeline_should_run).then_some(false),
        ..PendingCommands::default()
    };
    let mut last_source_poll = tokio::time::Instant::now() - Duration::from_secs(2);
    let mut pipeline_state = if pipeline_should_run {
        PipelineState::Running
    } else {
        PipelineState::Paused
    };

    'sync_loop: while running.load(Ordering::SeqCst) {
        pending_commands.receive(&mut command_rx);
        source_policy.manual_request(pending_commands.manual_running.take());
        source_policy.observe_display(shared_state.display_off.load(Ordering::Relaxed));
        let loop_start = tokio::time::Instant::now();
        let mut emitted_light_update = false;

        watchdog.tick();

        if last_source_poll.elapsed() >= Duration::from_secs(2) {
            last_source_poll = tokio::time::Instant::now();
            source_policy.observe(crate::capture_source::current_source().await.compatible());
        }

        if auto_tv_power && last_tv_power_poll.elapsed() >= Duration::from_secs(2) {
            last_tv_power_poll = tokio::time::Instant::now();
            match tv_power::is_active().await {
                Ok(state @ Some(_)) => {
                    shared_state.set_tv_power_state(state);
                    let active = state.unwrap();
                    let changed = observed_tv_power
                        .map(|previous| previous != active)
                        .unwrap_or(!active && pipeline_state == PipelineState::Running);
                    if changed {
                        info!(
                            "TV power changed: {}",
                            if active { "active" } else { "standby" }
                        );
                        if pending_commands.desired_running.is_none()
                            && !source_policy.manually_stopped()
                        {
                            pending_commands.desired_running = Some(active);
                        }
                    }
                    observed_tv_power = Some(active);
                }
                Ok(None) => shared_state.set_tv_power_state(None),
                Err(error) => {
                    shared_state.set_tv_power_state(None);
                    if last_tv_power_warning.elapsed() >= Duration::from_secs(30) {
                        warn!("TV power state unavailable; leaving sync unchanged: {error}");
                        last_tv_power_warning = tokio::time::Instant::now();
                    }
                }
            }
        }

        // 1. Check for web UI or external stop requests
        if pending_commands.desired_running == Some(true) || pending_commands.restart {
            hue_status.blocked = hue_status.needs_refresh;
        }
        let web_stop_requested = pending_commands.take_stop_if_running(pipeline_state);

        // Bridge reads run on their own worker; only current-session results affect output.
        if let Some(state) = observe_hue_status(
            &mut hue_status,
            &config,
            &mut pending_commands,
            hue_active && hue_sync_enabled && !web_stop_requested,
        ) {
            if !state.active && hue_dtls.is_some() {
                // Ownership loss suspends Hue only; Nanoleaf keeps its own session.
                hue_status.blocked = true;
            }
            if let Some(ref mut sampler) = hue_sampler {
                sampler.set_smoothing_factor(state.smoothing_factor);
            }
            if let Some(ref mut sampler) = nanoleaf_sampler {
                sampler.set_smoothing_factor(state.smoothing_factor);
            }
        }

        if hue_status.blocked {
            hue_dtls = None;
            hue_reconnect.request(None);
            shared_state.hue_connected.store(false, Ordering::Relaxed);
        }

        if source_policy.blocked()
            && pipeline_state == PipelineState::Running
            && !capture_faulted
            && !web_stop_requested
        {
            source_policy.pause_for_source();
        }
        if web_stop_requested || pipeline_state == PipelineState::Paused || source_policy.blocked()
        {
            info!(
                "Sync was stopped/paused (web={}). Entering paused idle state...",
                web_stop_requested
            );
            if !capture_faulted {
                shared_state.set_sync_state(SyncState::Stopping, None);
            }
            release_capture(&mut capture);
            shared_state
                .capture_hardware
                .store(false, Ordering::Relaxed);
            shared_state.set_fps(0.0);
            shared_state.set_light_update_fps(0.0);
            light_update_count = 0;
            light_update_window = tokio::time::Instant::now();
            pipeline_state = PipelineState::Paused;

            hue_status.request(None);
            hue_reconnect.request(None);
            hue_dtls = None;
            shared_state.hue_connected.store(false, Ordering::Relaxed);

            // Fade Nanoleaf to black while paused
            if let Some(ref mut ns) = nanoleaf_streamer {
                let black = vec![RgbColor::new(0, 0, 0); ns.panel_count()];
                let _ = ns.send_frame(&black, 0);
            }
            nanoleaf_streamer = None;
            shared_state
                .nanoleaf_connected
                .store(false, Ordering::Relaxed);
            if source_policy.source_paused()
                && source_policy.blocked()
                && !shared_state.display_off.load(Ordering::Relaxed)
            {
                shared_state.set_sync_state(SyncState::UnsupportedSource, Some("Built-in apps and Home are not supported by this capture backend; select HDMI to resume.".into()));
            } else if !capture_faulted {
                shared_state.set_sync_state(SyncState::Paused, None);
            }

            while running.load(Ordering::SeqCst) {
                tokio::time::sleep(watchdog.pause_poll_period()).await;
                pending_commands.receive(&mut command_rx);
                source_policy.manual_request(pending_commands.manual_running.take());
                source_policy.observe_display(shared_state.display_off.load(Ordering::Relaxed));
                watchdog.tick();
                if last_source_poll.elapsed() >= Duration::from_secs(2) {
                    last_source_poll = tokio::time::Instant::now();
                    source_policy
                        .observe(crate::capture_source::current_source().await.compatible());
                }
                if source_policy.source_paused()
                    && source_policy.blocked()
                    && !shared_state.display_off.load(Ordering::Relaxed)
                {
                    shared_state.set_sync_state(SyncState::UnsupportedSource, Some("Built-in apps and Home are not supported by this capture backend; select HDMI to resume.".into()));
                } else if source_policy.manually_stopped() && !capture_faulted {
                    shared_state.set_sync_state(SyncState::Paused, None);
                }
                pending_commands.resume_fault_for_restart(capture_faulted);
                if pending_commands.has_management_action()
                    && (pending_commands.desired_running != Some(true) || source_policy.blocked())
                {
                    break;
                }

                let configured_auto_tv_power =
                    shared_state.current_settings.read().unwrap().auto_tv_power;
                if auto_tv_power != configured_auto_tv_power {
                    auto_tv_power = configured_auto_tv_power;
                    observed_tv_power = None;
                    shared_state.set_tv_power_state(None);
                    last_tv_power_poll = tokio::time::Instant::now() - Duration::from_secs(2);
                }

                let mut requested_running = pending_commands.desired_running.take();
                if capture_faulted {
                    if requested_running != Some(true) && !source_policy.auto_resume() {
                        continue;
                    }
                    // A manual Start retries a fault; TV/app auto-resume must not loop on it.
                    pending_commands.restart = true;
                }
                let mut start_requested =
                    requested_running == Some(true) || source_policy.auto_resume();
                if auto_tv_power && last_tv_power_poll.elapsed() >= Duration::from_secs(2) {
                    last_tv_power_poll = tokio::time::Instant::now();
                    match tv_power::is_active().await {
                        Ok(state @ Some(_)) => {
                            shared_state.set_tv_power_state(state);
                            let active = state.unwrap();
                            if requested_running.is_none()
                                && !source_policy.manually_stopped()
                                && observed_tv_power == Some(false)
                                && active
                            {
                                start_requested = true;
                            }
                            observed_tv_power = Some(active);
                        }
                        Ok(None) => shared_state.set_tv_power_state(None),
                        Err(error) => {
                            shared_state.set_tv_power_state(None);
                            if last_tv_power_warning.elapsed() >= Duration::from_secs(30) {
                                warn!("TV power state unavailable; leaving sync paused: {error}");
                                last_tv_power_warning = tokio::time::Instant::now();
                            }
                        }
                    }
                }
                let app_reactivated = if !start_requested
                    && !source_policy.manually_stopped()
                    && !source_policy.blocked()
                    && requested_running != Some(false)
                    && hue_active
                    && hue_sync_enabled
                    && hue_reconnect.is_settled()
                    && !(auto_tv_power && observed_tv_power == Some(false))
                {
                    poll_hue_status(&mut hue_status, &config, &mut pending_commands)
                        .is_some_and(|state| state.active)
                } else {
                    hue_status.request(None);
                    false
                };

                // A Stop received during an awaited observation must beat auto-resume.
                pending_commands.receive(&mut command_rx);
                source_policy.manual_request(pending_commands.manual_running.take());
                source_policy.observe_display(shared_state.display_off.load(Ordering::Relaxed));
                if pending_commands.reconfigure {
                    break;
                }
                if let Some(latest) = pending_commands.desired_running.take() {
                    requested_running = Some(latest);
                    start_requested = latest;
                }
                if source_policy.blocked() {
                    if start_requested {
                        source_policy.pause_for_source();
                    }
                    continue;
                }
                if (start_requested || app_reactivated)
                    && !source_policy.manually_stopped()
                    && !(auto_tv_power
                        && observed_tv_power == Some(false)
                        && requested_running != Some(true))
                {
                    source_policy.resumed();
                    hue_status.request(None);
                    info!(
                        "Sync resume requested! Reactivating Hue & Nanoleaf streaming sessions..."
                    );
                    if requested_running == Some(true) {
                        hue_status.blocked = hue_status.needs_refresh;
                    }
                    if hue_active && hue_sync_enabled && !hue_status.blocked {
                        hue_reconnect.request(Some(&config));
                    }

                    if !capture_faulted {
                        shared_state.set_sync_state(SyncState::Starting, None);
                    }
                    if !pending_commands.restart {
                        capture = create_capture(config.capture_width, config.capture_height);
                    }
                    shared_state
                        .capture_hardware
                        .store(capture.is_real_hardware(), Ordering::Relaxed);
                    shared_state.set_fps(target_fps as f32);
                    capture_retry.success();
                    capture_recovery = CaptureRecovery::default();
                    pipeline_state = PipelineState::Running;

                    if nanoleaf_active && nanoleaf_sync_enabled {
                        if let Some(ref n_cfg) = config.nanoleaf {
                            match nanoleaf::enable_external_control(&n_cfg.ip, &n_cfg.auth_token) {
                                Ok(port) => {
                                    info!(
                                        "[+] Re-enabled Nanoleaf external control on UDP port {}",
                                        port
                                    );
                                    if let Some(sampler) = nanoleaf_sampler.as_ref() {
                                        match NanoleafUdpStreamer::new(
                                            &n_cfg.ip,
                                            port,
                                            sampler.panel_ids(),
                                        ) {
                                            Ok(new_streamer) => {
                                                nanoleaf_streamer = Some(new_streamer);
                                                shared_state
                                                    .nanoleaf_connected
                                                    .store(true, Ordering::Relaxed);
                                            }
                                            Err(e) => {
                                                shared_state
                                                    .nanoleaf_connected
                                                    .store(false, Ordering::Relaxed);
                                                error!(
                                                    "Failed to recreate Nanoleaf streamer: {}",
                                                    e
                                                );
                                            }
                                        }
                                    }
                                }
                                Err(e) => {
                                    error!("Failed to re-enable Nanoleaf external control: {}", e)
                                }
                            }
                        }
                    }

                    break;
                }
            }
            if !running.load(Ordering::SeqCst) {
                break 'sync_loop;
            }
        }

        // 2. Check for live settings update from Web UI
        if let Some(live_st) = pending_commands.apply_settings.take() {
            if auto_tv_power != live_st.auto_tv_power {
                auto_tv_power = live_st.auto_tv_power;
                observed_tv_power = None;
                last_tv_power_poll = tokio::time::Instant::now() - Duration::from_secs(2);
            }
            current_hue_brightness = live_st.brightness_multiplier * live_st.hue_output_brightness;
            current_use_xy = live_st.use_xy_gamut;
            config.depth_influence = live_st.depth_influence;
            for zone in &mut config.zones {
                zone.set_depth_influence(live_st.depth_influence);
            }
            *shared_state.hue_zones.write().unwrap() = config.zones.clone();
            if hue_sync_enabled != live_st.hue_sync_enabled {
                hue_status.request(None);
                hue_sync_enabled = live_st.hue_sync_enabled;
                hue_status.blocked = hue_status.needs_refresh;
                hue_dtls = None;
                shared_state.hue_connected.store(false, Ordering::Relaxed);
                hue_retry.success();
                hue_reconnect.request(
                    (hue_sync_enabled
                        && !hue_status.blocked
                        && pipeline_state == PipelineState::Running)
                        .then_some(&config),
                );
            }

            if nanoleaf_sync_enabled != live_st.nanoleaf_sync_enabled {
                nanoleaf_sync_enabled = live_st.nanoleaf_sync_enabled;
                if nanoleaf_sync_enabled && pipeline_state == PipelineState::Running {
                    if let (Some(n_cfg), Some(sampler)) =
                        (config.nanoleaf.as_ref(), nanoleaf_sampler.as_ref())
                    {
                        match nanoleaf::enable_external_control(&n_cfg.ip, &n_cfg.auth_token)
                            .and_then(|port| {
                                NanoleafUdpStreamer::new(&n_cfg.ip, port, sampler.panel_ids())
                            }) {
                            Ok(streamer) => {
                                nanoleaf_streamer = Some(streamer);
                                shared_state
                                    .nanoleaf_connected
                                    .store(true, Ordering::Relaxed);
                            }
                            Err(error) => {
                                nanoleaf_sync_enabled = false;
                                shared_state
                                    .nanoleaf_connected
                                    .store(false, Ordering::Relaxed);
                                warn!("Nanoleaf sync remains off: {}", error);
                            }
                        }
                    }
                } else {
                    nanoleaf_streamer = None;
                    shared_state
                        .nanoleaf_connected
                        .store(false, Ordering::Relaxed);
                }
            }
            if let Some(ref mut s) = hue_sampler {
                s.set_temporal_response(
                    live_st.rise_smoothing_factor,
                    live_st.fall_smoothing_factor,
                );
                s.set_strict_blackout(live_st.strict_blackout);
                s.set_hdr_tone_mapping(live_st.hdr_tone_mapping);
                s.set_letterbox_detection(live_st.letterbox_detection);
                s.set_saturation_boost(live_st.saturation_boost);
                s.set_peak_weight(live_st.peak_weight);
                s.set_spatial_blend(live_st.spatial_blend);
                s.set_depth_influence(live_st.depth_influence);
                s.set_gamma(live_st.gamma);
                s.set_noise_gate_threshold(live_st.noise_gate_threshold);
                s.set_max_color_step(live_st.max_color_step);
            }
            if let Some(ref mut ns) = nanoleaf_sampler {
                ns.set_temporal_response(
                    live_st.rise_smoothing_factor,
                    live_st.fall_smoothing_factor,
                );
                ns.set_strict_blackout(live_st.strict_blackout);
                ns.set_hdr_tone_mapping(live_st.hdr_tone_mapping);
                ns.set_saturation_boost(live_st.saturation_boost);
                ns.set_brightness_multiplier(
                    live_st.brightness_multiplier * live_st.nanoleaf_output_brightness,
                );
                ns.set_peak_weight(live_st.peak_weight);
                ns.set_spatial_blend(live_st.spatial_blend);
                ns.set_gamma(live_st.gamma);
                ns.set_noise_gate_threshold(live_st.noise_gate_threshold);
                ns.set_max_color_step(live_st.max_color_step);
                ns.set_alignment(live_st.nanoleaf_alignment);
            }
            info!(
                "Applied live settings: Hue={:.1}x, Nanoleaf={:.1}x, saturation={:.1}x, smoothing={:.2}, xy_mode={}",
                current_hue_brightness,
                live_st.brightness_multiplier * live_st.nanoleaf_output_brightness,
                live_st.saturation_boost,
                live_st.smoothing_factor,
                current_use_xy
            );
        }

        // 3. Check for save config request from Web UI
        if !pending_commands.save_config.is_empty() {
            let live_st = shared_state.current_settings.read().unwrap().clone();
            let (active_preset, preset_overrides) = shared_state.preset_save_snapshot(&live_st);
            let result = Config::update(&config_path, Some(config.clone()), |save_cfg| {
                save_cfg.active_preset = active_preset.clone();
                save_cfg.preset_overrides = preset_overrides.clone();
                save_cfg.brightness_multiplier = live_st.brightness_multiplier;
                save_cfg.hue_output_brightness = live_st.hue_output_brightness;
                save_cfg.nanoleaf_output_brightness = live_st.nanoleaf_output_brightness;
                save_cfg.saturation_boost = live_st.saturation_boost;
                save_cfg.peak_weight = live_st.peak_weight;
                save_cfg.spatial_blend = live_st.spatial_blend;
                save_cfg.depth_influence = live_st.depth_influence;
                save_cfg.gamma = live_st.gamma;
                save_cfg.noise_gate_threshold = live_st.noise_gate_threshold;
                save_cfg.smoothing_factor = live_st.smoothing_factor;
                save_cfg.rise_smoothing_factor = live_st.rise_smoothing_factor;
                save_cfg.fall_smoothing_factor = live_st.fall_smoothing_factor;
                save_cfg.strict_blackout = live_st.strict_blackout;
                save_cfg.use_xy_gamut = live_st.use_xy_gamut;
                save_cfg.letterbox_detection = live_st.letterbox_detection;
                save_cfg.hdr_tone_mapping = live_st.hdr_tone_mapping;
                save_cfg.hue_sync_enabled = hue_sync_enabled;
                save_cfg.nanoleaf_sync_enabled = nanoleaf_sync_enabled;
                save_cfg.auto_tv_power = live_st.auto_tv_power;
                save_cfg.max_color_step = live_st.max_color_step;
                Ok(())
            })
            .map(|_| ())
            .map_err(|error| error.to_string());
            if result.is_ok() {
                shared_state.mark_presets_saved(preset_overrides);
                info!("[+] Successfully saved live settings to {:?}", config_path);
            } else if let Err(ref error) = result {
                error!(
                    "Failed to save updated config to {:?}: {}",
                    config_path, error
                );
            }
            for reply in std::mem::take(&mut pending_commands.save_config) {
                let _ = reply.send(result.clone());
            }
        }

        if std::mem::take(&mut pending_commands.sync_bridge) && hue_active {
            hue_status.blocked = true;
            hue_status.needs_refresh = true;
            shared_state.hue_connected.store(false, Ordering::Relaxed);
            // Stop repeating the old layout before fetching changed channel membership.
            hue_dtls = None;
            hue_reconnect.request(None);
            let bridge_identity = (
                config.bridge_ip.clone(),
                config.username.clone(),
                config.hue_bridge_certificate_sha256.clone(),
                config.hue_bridge_id.clone(),
            );
            match sync_entertainment_areas(
                &config.bridge_ip,
                &config.username,
                config.entertainment_configuration_id.as_deref(),
                config.hue_trust(),
            ) {
                Ok(area) if !area.zones.is_empty() => {
                    hue_status.blocked = false;
                    hue_status.needs_refresh = false;
                    let configuration_changed = config.entertainment_configuration_id.as_deref()
                        != Some(&area.configuration_id);
                    config.entertainment_area_id = area.configuration_id.clone();
                    config.entertainment_configuration_id = Some(area.configuration_id);
                    config.hue_bridge_id = Some(area.bridge_id.clone());
                    config.hue_bridge_certificate_sha256 = Some(area.certificate_sha256);
                    let live_st = shared_state.current_settings.read().unwrap().clone();
                    config.zones = area.zones;
                    for zone in &mut config.zones {
                        zone.set_depth_influence(live_st.depth_influence);
                    }
                    *shared_state.hue_zones.write().unwrap() = config.zones.clone();
                    hue_sampler = Some(ZoneSampler::new(
                        config.zones.clone(),
                        live_st.smoothing_factor,
                        live_st.hdr_tone_mapping,
                        live_st.letterbox_detection,
                        live_st.saturation_boost,
                        live_st.noise_gate_threshold,
                    ));
                    if let Some(ref mut sampler) = hue_sampler {
                        sampler.set_temporal_response(
                            live_st.rise_smoothing_factor,
                            live_st.fall_smoothing_factor,
                        );
                        sampler.set_strict_blackout(live_st.strict_blackout);
                        sampler.set_peak_weight(live_st.peak_weight);
                        sampler.set_spatial_blend(live_st.spatial_blend);
                        sampler.set_gamma(live_st.gamma);
                        sampler.set_max_color_step(live_st.max_color_step);
                    }
                    if configuration_changed {
                        hue_status.request(None);
                        hue_packet_builder =
                            Some(HueStreamPacketBuilder::new(hue_configuration_id(&config)));
                        hue_dtls = None;
                        shared_state.hue_connected.store(false, Ordering::Relaxed);
                        hue_retry.success();
                        hue_reconnect.request(
                            (hue_sync_enabled && pipeline_state == PipelineState::Running)
                                .then_some(&config),
                        );
                    }

                    if let Err(error) = Config::update(&config_path, None, |current| {
                        if (
                            current.bridge_ip.as_str(),
                            current.username.as_str(),
                            current.hue_bridge_certificate_sha256.as_ref(),
                            current.hue_bridge_id.as_ref(),
                        ) != (
                            bridge_identity.0.as_str(),
                            bridge_identity.1.as_str(),
                            bridge_identity.2.as_ref(),
                            bridge_identity.3.as_ref(),
                        ) {
                            anyhow::bail!("Hue Bridge configuration changed during sync; retry");
                        }
                        current.entertainment_area_id = config.entertainment_area_id.clone();
                        current.entertainment_configuration_id =
                            config.entertainment_configuration_id.clone();
                        current.hue_bridge_certificate_sha256 =
                            config.hue_bridge_certificate_sha256.clone();
                        current.hue_bridge_id = config.hue_bridge_id.clone();
                        current.zones = config.zones.clone();
                        Ok(())
                    }) {
                        warn!(
                            "Hue area sync succeeded but could not persist its certificate pin: {}",
                            error
                        );
                    }
                    info!(
                        "Synced and persisted {} Hue channel positions from '{}'.",
                        config.zones.len(),
                        area.name
                    );
                }
                Ok(area) => warn!(
                    "Hue area '{}' did not provide channel positions; leaving zones unchanged.",
                    area.name
                ),
                Err(e) => warn!("Hue bridge sync failed; leaving zones unchanged: {}", e),
            }
        }

        if let Some(result) = hue_reconnect.poll() {
            match result {
                Ok(client)
                    if hue_active
                        && hue_sync_enabled
                        && pipeline_state == PipelineState::Running
                        && !hue_status.blocked =>
                {
                    if config.bridge_ip != client.bridge_ip {
                        let previous_ip = config.bridge_ip.clone();
                        config.bridge_ip = client.bridge_ip.clone();
                        *shared_state.hue_bridge_ip.write().unwrap() = config.bridge_ip.clone();
                        if let Err(error) = Config::update(&config_path, None, |current| {
                            if current.bridge_ip != previous_ip
                                || current.username != config.username
                                || current.hue_bridge_id != config.hue_bridge_id
                            {
                                anyhow::bail!("Hue Bridge configuration changed during address recovery; retry");
                            }
                            current.bridge_ip = config.bridge_ip.clone();
                            Ok(())
                        }) {
                            warn!("Recovered Hue address could not be persisted: {error:#}");
                        }
                    }
                    hue_dtls = Some(client.output);
                    hue_retry.success();
                    shared_state.hue_connected.store(true, Ordering::Relaxed);
                    info!("[+] Re-established Hue DTLS streaming session.");
                }
                Ok(_) => {}
                Err(error) => {
                    let delay = hue_retry.failure();
                    warn!("Hue reconnect failed: {error:#}. Retrying in {delay:?}.");
                }
            }
        }
        if hue_active
            && hue_sync_enabled
            && pipeline_state == PipelineState::Running
            && hue_dtls.is_none()
            && !hue_status.blocked
            && !hue_reconnect.pending()
            && hue_retry.ready()
        {
            hue_reconnect.request(Some(&config));
        }

        // 4. Check for pipeline restart request from Web UI
        if std::mem::take(&mut pending_commands.reconfigure) {
            info!("Device credentials saved; restarting daemon to load the new configuration.");
            break;
        }

        if std::mem::take(&mut pending_commands.restart) {
            info!("Pipeline restart requested via Web UI. Re-initializing capture...");
            if source_policy.blocked() {
                if !source_policy.manually_stopped() {
                    source_policy.pause_for_source();
                }
                continue 'sync_loop;
            }
            if !capture_faulted {
                shared_state.set_sync_state(SyncState::Stopping, None);
            }
            release_capture(&mut capture);
            shared_state
                .capture_hardware
                .store(false, Ordering::Relaxed);
            if !runtime::wait_for_capture_retry(Duration::from_millis(500), &shutdown).await {
                break 'sync_loop;
            }
            pending_commands.receive(&mut command_rx);
            source_policy.manual_request(pending_commands.manual_running.take());
            source_policy.observe_display(shared_state.display_off.load(Ordering::Relaxed));
            source_policy.observe(crate::capture_source::current_source().await.compatible());
            last_source_poll = tokio::time::Instant::now();
            pending_commands.receive(&mut command_rx);
            source_policy.manual_request(pending_commands.manual_running.take());
            source_policy.observe_display(shared_state.display_off.load(Ordering::Relaxed));
            if pending_commands.capture_restart_cancelled()
                || source_policy.blocked()
                || source_policy.manually_stopped()
            {
                if source_policy.blocked() && !source_policy.manually_stopped() {
                    source_policy.pause_for_source();
                }
                continue 'sync_loop;
            }
            capture_faulted = false;
            capture_recovery = CaptureRecovery::default();
            capture_retry.success();
            if pipeline_state == PipelineState::Running {
                shared_state.set_sync_state(SyncState::Starting, None);
                capture = create_capture(config.capture_width, config.capture_height);
            } else {
                shared_state.set_sync_state(SyncState::Paused, None);
            }
            shared_state
                .capture_hardware
                .store(capture.is_real_hardware(), Ordering::Relaxed);
        }

        if pipeline_state == PipelineState::Paused {
            continue 'sync_loop;
        }

        // In auto FPS mode, dynamically update cadence if TV source rate changed (e.g. film started)
        if config.fps == 0 && last_source_fps_poll.elapsed() >= Duration::from_secs(2) {
            last_source_fps_poll = tokio::time::Instant::now();
            if let Some(new_fps) = detect_source_fps().await {
                let clamped = new_fps.clamp(20.0, 60.0);
                if (clamped - target_fps).abs() > 0.5 {
                    target_fps = clamped;
                    frame_interval = Duration::from_secs_f64(1.0 / target_fps);
                    shared_state.set_fps(target_fps as f32);
                    info!(
                        "Video source timing changed: adapting sync cadence to {:.2} FPS",
                        target_fps
                    );
                }
            }
        }

        // If currently on fallback MockCapture, rapidly probe if hardware VtCapture has become available
        // (e.g. if the TV was on the home screen at boot or during input/format switch)
        if !capture.is_real_hardware() && capture_retry.ready() {
            match VtCapture::try_new(config.capture_width, config.capture_height) {
                Ok(hw_capture) => {
                    info!("[+] Successfully upgraded from MockCapture to hardware VtCapture!");
                    capture = Box::new(hw_capture);
                    capture_retry.success();
                    shared_state.capture_hardware.store(true, Ordering::Relaxed);
                }
                Err(error) => {
                    let delay = capture_retry.failure();
                    shared_state.set_sync_state(
                        SyncState::Recovering,
                        Some(format!("Hardware capture unavailable: {error:#}")),
                    );
                    warn!("Hardware capture unavailable ({error:#}); retrying in {delay:?}");
                }
            }
        }

        let hardware_frame = capture.is_real_hardware();
        match capture.acquire_frame() {
            Ok(frame) => {
                if hardware_frame {
                    shared_state.preview.publish(&frame);
                    capture_recovery.frame_received(std::time::Instant::now());
                    shared_state.set_sync_state(SyncState::Running, None);
                }
                let mut is_scene_cut = false;

                if !hue_sync_enabled && nanoleaf_sync_enabled {
                    nanoleaf_only_frame_count += 1;
                    let global =
                        frame_average(frame.data, frame.width, frame.height, frame.is_bgra);
                    is_scene_cut =
                        nanoleaf_only_frame_count > 1 && global.delta(nanoleaf_only_global) > 0.35;
                    nanoleaf_only_global = global;
                    if config.letterbox_detection
                        && (nanoleaf_only_frame_count == 1
                            || nanoleaf_only_frame_count.is_multiple_of(15))
                    {
                        if let Some(ref mut sampler) = nanoleaf_sampler {
                            sampler.set_active_rect(ZoneSampler::detect_active_rect(
                                frame.data,
                                frame.width,
                                frame.height,
                                frame.is_bgra,
                            ));
                        }
                    }
                }

                // 1. Process Philips Hue entertainment zones
                if hue_active && hue_sync_enabled {
                    if let (Some(ref mut sampler), Some(ref mut dtls), Some(ref mut builder)) =
                        (&mut hue_sampler, &mut hue_dtls, &mut hue_packet_builder)
                    {
                        let (sampled_zones, cut) = sampler.sample_frame(
                            frame.data,
                            frame.width,
                            frame.height,
                            frame.is_bgra,
                        );
                        is_scene_cut = cut;

                        let active_rect = sampler.active_rect();
                        if let Some(ref mut nl_s) = nanoleaf_sampler {
                            nl_s.set_active_rect(active_rect);
                        }

                        let calibration = calibration_pattern(&shared_state);
                        let sampled_zones = calibration
                            .map(|pattern| calibration_zone_colors(pattern, &config.zones))
                            .unwrap_or(sampled_zones);
                        let channels: Vec<(u8, (u16, u16, u16))> = sampled_zones
                            .iter()
                            .map(|(channel_id, color)| {
                                let scaled = if calibration.is_some() {
                                    *color
                                } else {
                                    color.scale(current_hue_brightness)
                                };
                                if calibration.is_none() && current_use_xy {
                                    (*channel_id, scaled.to_xy_u16(color::HueGamut::GamutC))
                                } else {
                                    (*channel_id, scaled.to_u16())
                                }
                            })
                            .collect();

                        // Share live sampled Hue colors with Web UI
                        *shared_state.live_hue_colors.write().unwrap() = sampled_zones;

                        // Adaptive deadband throttling for Hue Bridge
                        let is_virtually_identical =
                            if !last_channels.is_empty() && last_channels.len() == channels.len() {
                                last_channels.iter().zip(&channels).all(
                                    |((left_id, (a1, a2, a3)), (right_id, (b1, b2, b3)))| {
                                        left_id == right_id
                                            && (*a1 as i32 - *b1 as i32).abs() < 350
                                            && (*a2 as i32 - *b2 as i32).abs() < 350
                                            && (*a3 as i32 - *b3 as i32).abs() < 350
                                    },
                                )
                            } else {
                                false
                            };

                        let should_send = if config.adaptive_throttling {
                            if is_virtually_identical {
                                if last_heartbeat.elapsed() >= Duration::from_millis(500) {
                                    last_heartbeat = tokio::time::Instant::now();
                                    true
                                } else {
                                    false
                                }
                            } else {
                                last_heartbeat = tokio::time::Instant::now();
                                true
                            }
                        } else {
                            true
                        };

                        if should_send {
                            let packet = if calibration.is_none() && current_use_xy {
                                builder.build_xy_packet(&channels)
                            } else {
                                builder.build_rgb_packet(&channels)
                            };

                            if let Err(e) = dtls.send(&packet) {
                                shared_state.hue_connected.store(false, Ordering::Relaxed);
                                error!("Failed to send DTLS HueStream packet: {}", e);
                                hue_dtls = None;
                            } else {
                                hue_retry.success();
                                shared_state.hue_connected.store(true, Ordering::Relaxed);
                                emitted_light_update = true;
                            }
                            last_channels = channels;
                        }
                    }
                }

                // 2. Process Nanoleaf 4D TV perimeter lightstrip
                if nanoleaf_active && nanoleaf_sync_enabled {
                    if let (Some(ref mut nl_sampler), Some(ref mut nl_streamer)) =
                        (&mut nanoleaf_sampler, &mut nanoleaf_streamer)
                    {
                        let nl_colors = nl_sampler.sample_frame(
                            frame.data,
                            frame.width,
                            frame.height,
                            frame.is_bgra,
                            is_scene_cut,
                        );
                        let nl_colors = calibration_pattern(&shared_state)
                            .map(|pattern| calibration_perimeter_colors(pattern, nl_colors.len()))
                            .unwrap_or(nl_colors);

                        // Share live sampled Nanoleaf colors with Web UI
                        *shared_state.live_nanoleaf_colors.write().unwrap() = nl_colors.clone();
                        let nanoleaf_changed = colors_changed(&last_nanoleaf_colors, &nl_colors);
                        let should_send = !config.adaptive_throttling
                            || nanoleaf_changed
                            || last_nanoleaf_heartbeat.elapsed() >= Duration::from_millis(500);
                        if should_send {
                            last_nanoleaf_heartbeat = tokio::time::Instant::now();
                            if let Err(e) = nl_streamer.send_frame(&nl_colors, 0) {
                                shared_state
                                    .nanoleaf_connected
                                    .store(false, Ordering::Relaxed);
                                warn!(
                                "Nanoleaf UDP frame send error ({}). Triggering external control refresh...",
                                e
                            );
                                if nanoleaf_retry.ready() {
                                    if let Some(ref n_cfg) = config.nanoleaf {
                                        match nanoleaf::enable_external_control(
                                            &n_cfg.ip,
                                            &n_cfg.auth_token,
                                        ) {
                                            Ok(port) => {
                                                info!(
                                                "[+] Re-enabled Nanoleaf external control on UDP port {}.",
                                                port
                                            );
                                                match NanoleafUdpStreamer::new(
                                                    &n_cfg.ip,
                                                    port,
                                                    nl_streamer.panel_ids().to_vec(),
                                                ) {
                                                    Ok(new_streamer) => {
                                                        *nl_streamer = new_streamer;
                                                        nanoleaf_retry.success();
                                                    }
                                                    Err(recreate_err) => {
                                                        let delay = nanoleaf_retry.failure();
                                                        error!(
                                                        "Failed to recreate Nanoleaf streamer: {}. Retrying in {:.1}s.",
                                                        recreate_err,
                                                        delay.as_secs_f32()
                                                    );
                                                    }
                                                }
                                            }
                                            Err(ext_err) => {
                                                let delay = nanoleaf_retry.failure();
                                                error!(
                                                "Failed to refresh Nanoleaf external control: {}. Retrying in {:.1}s.",
                                                ext_err,
                                                delay.as_secs_f32()
                                            );
                                            }
                                        }
                                    }
                                }
                            } else {
                                nanoleaf_retry.success();
                                shared_state
                                    .nanoleaf_connected
                                    .store(true, Ordering::Relaxed);
                                last_nanoleaf_colors = nl_colors;
                                emitted_light_update = true;
                            }
                        }
                    }
                }
            }
            Err(e) => {
                shared_state.set_sync_state(SyncState::Recovering, Some(e.to_string()));
                let delay = match capture_recovery.failed(&e) {
                    RecoveryAction::Retry => {
                        if !runtime::wait_for_capture_retry(frame_interval, &shutdown).await {
                            break 'sync_loop;
                        }
                        continue 'sync_loop;
                    }
                    RecoveryAction::Pause => {
                        error!("Capture paused ({e:#}); invalid frame or recovery limit reached. Use Start or Restart to retry.");
                        capture = Box::new(MockCapture::new(
                            config.capture_width,
                            config.capture_height,
                        ));
                        shared_state
                            .capture_hardware
                            .store(false, Ordering::Relaxed);
                        capture_faulted = true;
                        shared_state.set_sync_state(SyncState::Faulted, Some(e.to_string()));
                        pending_commands.desired_running = Some(false);
                        continue 'sync_loop;
                    }
                    RecoveryAction::Restart(delay) => delay,
                };
                warn!("Capture recovery ({e:#}); releasing hardware scaler, retrying in {delay:?}");
                capture = Box::new(MockCapture::new(
                    config.capture_width,
                    config.capture_height,
                ));
                shared_state
                    .capture_hardware
                    .store(false, Ordering::Relaxed);

                // Clear/black out Nanoleaf during video transition
                if let Some(ref mut ns) = nanoleaf_streamer {
                    let black = vec![RgbColor::new(0, 0, 0); ns.panel_count()];
                    let _ = ns.send_frame(&black, 0);
                }

                if !runtime::wait_for_capture_retry(delay, &shutdown).await {
                    break 'sync_loop;
                }
                watchdog.tick();
                pending_commands.receive(&mut command_rx);
                source_policy.manual_request(pending_commands.manual_running.take());
                source_policy.observe_display(shared_state.display_off.load(Ordering::Relaxed));
                source_policy.observe(crate::capture_source::current_source().await.compatible());
                last_source_poll = tokio::time::Instant::now();
                pending_commands.receive(&mut command_rx);
                source_policy.manual_request(pending_commands.manual_running.take());
                source_policy.observe_display(shared_state.display_off.load(Ordering::Relaxed));
                if pending_commands.capture_restart_cancelled()
                    || source_policy.blocked()
                    || source_policy.manually_stopped()
                {
                    if source_policy.blocked() && !source_policy.manually_stopped() {
                        source_policy.pause_for_source();
                    }
                    continue 'sync_loop;
                }

                // Auto-adapt to new source refresh rate if enabled
                if config.fps == 0 {
                    if let Some(new_fps) = detect_source_fps().await {
                        let clamped = new_fps.clamp(20.0, 60.0);
                        if (clamped - target_fps).abs() > 0.5 {
                            target_fps = clamped;
                            frame_interval = Duration::from_secs_f64(1.0 / target_fps);
                            shared_state.set_fps(target_fps as f32);
                            info!(
                                "Video source timing changed: adapting sync cadence to {:.2} FPS",
                                target_fps
                            );
                        }
                    }
                }

                // Re-initialize capture driver
                capture = create_capture(config.capture_width, config.capture_height);
                shared_state
                    .capture_hardware
                    .store(capture.is_real_hardware(), Ordering::Relaxed);
                capture_retry.failure();
            }
        }

        if emitted_light_update {
            light_update_count = light_update_count.saturating_add(1);
        }
        let output_elapsed = light_update_window.elapsed();
        if output_elapsed >= Duration::from_secs(1) {
            shared_state
                .set_light_update_fps(light_update_count as f32 / output_elapsed.as_secs_f32());
            light_update_count = 0;
            light_update_window = tokio::time::Instant::now();
        }

        let elapsed = loop_start.elapsed();
        if elapsed < frame_interval {
            tokio::time::sleep(frame_interval - elapsed).await;
        }
    }

    shared_state.set_sync_state(SyncState::Stopping, None);
    release_capture(&mut capture);
    shared_state
        .capture_hardware
        .store(false, Ordering::Relaxed);
    shared_state.set_fps(0.0);
    shared_state.set_light_update_fps(0.0);

    // Join the serial worker after invalidating every in-flight reconnect.
    drop(hue_dtls);
    drop(hue_status);
    drop(hue_reconnect);

    signal_task.abort();
    let _ = signal_task.await;
    pending_commands.shutdown(command_rx);
    web_shutdown.cancel();
    if let Some(tasks) = web_tasks {
        tasks.close();
        tasks.wait().await;
    }

    info!("lg-hue-sync terminated cleanly.");
    Ok(())
}

async fn run_test_pattern(config_path: PathBuf) -> Result<()> {
    let config = Config::load(&config_path)?;
    info!(
        "Testing connection to Hue Bridge at {}...",
        config.bridge_ip
    );

    set_hue_stream_active(&config, true)?;

    let mut dtls_client = HueDtlsClient::connect(
        &config.bridge_ip,
        &hue::fetch_application_id(&config.bridge_ip, &config.username, config.hue_trust())?,
        &config.clientkey,
    )?;

    let mut packet_builder = HueStreamPacketBuilder::new(hue_configuration_id(&config));
    info!("Streaming fast rainbow test pattern across all Hue entertainment channels for 15 seconds...");

    let start = std::time::Instant::now();
    while start.elapsed().as_secs() < 15 {
        let hue_offset = (start.elapsed().as_secs_f32() * 180.0) % 360.0; // Fast rotation (180 deg/sec)
        let mut channels = Vec::new();

        // Address only channels returned by the selected Entertainment configuration.
        for (index, zone) in config.zones.iter().enumerate() {
            let channel_id = zone.channel_id;
            let hue = (hue_offset + (index as f32) * 45.0) % 360.0;
            let (r, g, b) = hsv_to_rgb(hue, 1.0, 1.0);
            channels.push((channel_id, RgbColor::new(r, g, b).to_u16()));
        }

        let packet = packet_builder.build_rgb_packet(&channels);
        dtls_client.send(&packet)?;
        tokio::time::sleep(Duration::from_millis(20)).await; // 50 Hz fast cadence
    }

    set_hue_stream_active(&config, false)?;

    info!("[+] Hue test pattern completed successfully!");
    Ok(())
}

async fn run_test_nanoleaf(config_path: PathBuf) -> Result<()> {
    let config = Config::load(&config_path)?;
    let n_cfg = config
        .nanoleaf
        .ok_or_else(|| anyhow!("No Nanoleaf configuration found in {:?}", config_path))?;

    info!("Connecting to Nanoleaf 4D at {}...", n_cfg.ip);
    let udp_port = nanoleaf::enable_external_control(&n_cfg.ip, &n_cfg.auth_token)?;
    let segments = n_cfg.segments.max(30);
    let mut panel_ids = n_cfg.panel_ids.clone();
    if panel_ids.is_empty() {
        panel_ids = (1..=segments).collect();
    }

    let mut streamer = NanoleafUdpStreamer::new(&n_cfg.ip, udp_port, panel_ids)?;
    info!("Streaming rotating rainbow chase around TV perimeter for 10 seconds...");

    let start = std::time::Instant::now();
    while start.elapsed().as_secs() < 10 {
        let hue_offset = (start.elapsed().as_secs_f32() * 90.0) % 360.0;
        let mut colors = Vec::with_capacity(segments as usize);

        for i in 0..segments {
            let hue = (hue_offset + (i as f32 / segments as f32) * 360.0) % 360.0;
            let (r, g, b) = hsv_to_rgb(hue, 1.0, 1.0);
            colors.push(RgbColor::new(r, g, b));
        }

        streamer.send_frame(&colors, 0)?;
        tokio::time::sleep(Duration::from_millis(33)).await;
    }

    info!("[+] Nanoleaf 4D test pattern completed successfully!");
    Ok(())
}

async fn run_test_capture(config_path: PathBuf, raw_nv12_stats: bool) -> Result<()> {
    let config = Config::load(&config_path)?;
    if raw_nv12_stats {
        info!(
            "TV reports {} mode; this does not establish the capture colour space",
            video_mode::current_dynamic_range().await.label()
        );
        let mut capture = VtCapture::try_new(config.capture_width, config.capture_height)
            .context("raw NV12 statistics require available vtCapture hardware")?;
        capture.enable_raw_stats();
        let mut collected = 0;
        for _ in 0..20 {
            let _ = capture.acquire_frame()?;
            if let Some(stats) = capture.take_raw_stats() {
                collected += 1;
                info!(
                    "Raw NV12 centre frame {collected}: Y min/mean/max={}/{:.1}/{}, U mean={:.1}, V mean={:.1}, luma samples={}",
                    stats.y_min,
                    stats.y_mean,
                    stats.y_max,
                    stats.u_mean,
                    stats.v_mean,
                    stats.luma_samples
                );
                if collected == 5 {
                    return Ok(());
                }
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        return Err(anyhow!("fewer than five fresh NV12 frames were available"));
    }
    let mut capture = create_capture(config.capture_width, config.capture_height);
    let mut sampler = ZoneSampler::new(
        config.zones.clone(),
        0.35,
        config.hdr_tone_mapping,
        config.letterbox_detection,
        config.saturation_boost,
        config.noise_gate_threshold,
    );

    info!("Sampling capture for 5 frames...");
    for frame_idx in 1..=5 {
        let frame = capture.acquire_frame()?;
        let (sampled, is_cut) =
            sampler.sample_frame(frame.data, frame.width, frame.height, frame.is_bgra);
        info!("Frame {} (scene cut: {}):", frame_idx, is_cut);
        for (channel_id, color) in sampled {
            let zone_name = config
                .zones
                .iter()
                .find(|z| z.channel_id == channel_id)
                .map(|z| z.name.as_str())
                .unwrap_or("Unknown");
            info!(
                "  Zone '{}' (channel {}): RGB({}, {}, {})",
                zone_name, channel_id, color.r, color.g, color.b
            );
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    Ok(())
}

fn hsv_to_rgb(h: f32, s: f32, v: f32) -> (u8, u8, u8) {
    let c = v * s;
    let x = c * (1.0 - ((h / 60.0) % 2.0 - 1.0).abs());
    let m = v - c;
    let (r1, g1, b1) = match (h / 60.0) as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    (
        ((r1 + m) * 255.0) as u8,
        ((g1 + m) * 255.0) as u8,
        ((b1 + m) * 255.0) as u8,
    )
}

fn watchdog_period(deadline: Duration) -> Duration {
    (deadline / 2).max(Duration::from_millis(1))
}

struct WatchdogHeartbeat {
    period: Option<Duration>,
    last: tokio::time::Instant,
}

impl WatchdogHeartbeat {
    fn new() -> Self {
        Self {
            period: sd_notify::watchdog_enabled().map(watchdog_period),
            last: tokio::time::Instant::now(),
        }
    }

    fn tick(&mut self) {
        if self
            .period
            .is_some_and(|period| self.last.elapsed() >= period)
        {
            let _ = sd_notify::notify(&[sd_notify::NotifyState::Watchdog]);
            self.last = tokio::time::Instant::now();
        }
    }

    fn pause_poll_period(&self) -> Duration {
        self.period
            .unwrap_or(Duration::from_millis(250))
            .min(Duration::from_millis(250))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn management_updates_keep_hue_subscription_until_output_stops() {
        let events = hue::events::HueEvents::new(
            |_| anyhow::bail!("unexpected snapshot"),
            |_| anyhow::bail!("unexpected connection"),
        )
        .unwrap();
        let mut worker = HueStatus {
            events,
            enabled: true,
            blocked: false,
            needs_refresh: false,
        };
        let config = Config::new_default("", "", "", "");
        let mut commands = PendingCommands::default();
        commands.save_config.push(tokio::sync::oneshot::channel().0);
        assert!(observe_hue_status(&mut worker, &config, &mut commands, true).is_none());
        assert!(
            worker.enabled,
            "management work must not restart Hue observation"
        );
        assert!(!worker.blocked && !commands.sync_bridge);
        observe_hue_status(&mut worker, &config, &mut commands, false);
        assert!(
            !worker.enabled,
            "stopping output must still cancel observation"
        );
    }

    #[test]
    fn duplicate_output_frames_are_suppressed_but_meaningful_changes_pass() {
        let previous = vec![RgbColor::new(10, 20, 30)];
        assert!(!colors_changed(&previous, &previous));
        assert!(colors_changed(&previous, &[RgbColor::new(13, 20, 30)]));
    }

    #[test]
    fn watchdog_uses_half_the_reported_deadline() {
        assert_eq!(
            watchdog_period(Duration::from_secs(10)),
            Duration::from_secs(5)
        );
        assert_eq!(watchdog_period(Duration::ZERO), Duration::from_millis(1));
        let idle = WatchdogHeartbeat {
            period: None,
            last: tokio::time::Instant::now(),
        };
        assert_eq!(idle.pause_poll_period(), Duration::from_millis(250));
        let fast = WatchdogHeartbeat {
            period: Some(Duration::from_millis(50)),
            last: tokio::time::Instant::now(),
        };
        assert_eq!(fast.pause_poll_period(), Duration::from_millis(50));
    }
}
