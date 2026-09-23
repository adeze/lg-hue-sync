mod capture;
mod color;
mod config;
mod hue;
mod nanoleaf;
mod runtime;
mod tv_power;
mod web;

use anyhow::{anyhow, Result};
use clap::{Parser, Subcommand};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn, Level};
use tracing_subscriber::FmtSubscriber;

use capture::{create_capture, detect_source_fps, VtCapture};
use color::{RgbColor, ZoneSampler};
use config::{Config, ConfigError, NanoleafAlignment};
use hue::{sync_entertainment_areas, HueDtlsClient, HueStreamPacketBuilder};
use nanoleaf::{NanoleafPerimeterSampler, NanoleafUdpStreamer};
use runtime::{PendingCommands, PipelineState, RetryState};
use web::{start_web_server, CalibrationPattern, LiveSettings, SharedState};

#[derive(Parser)]
#[command(name = "lg-hue-sync")]
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
        Commands::TestCapture { config } => run_test_capture(config).await,
        Commands::Pair { bridge, output } => run_pair(bridge, output).await,
        Commands::SyncHue { config, area } => run_sync_hue(config, area).await,
        Commands::PairNanoleaf { ip, config } => run_pair_nanoleaf(ip, config).await,
    }
}

fn reconnect_hue(config: &Config) -> Result<HueDtlsClient> {
    set_hue_stream_active(config, true)?;
    HueDtlsClient::connect(&config.bridge_ip, &config.username, &config.clientkey)
}

fn hue_configuration_id(config: &Config) -> Option<String> {
    config
        .entertainment_configuration_id
        .as_deref()
        .filter(|id| id.len() == 36)
        .map(ToOwned::to_owned)
}

fn set_hue_stream_active(config: &Config, active: bool) -> Result<()> {
    let configuration_id = hue_configuration_id(config)
        .ok_or_else(|| anyhow!("Hue V2 Entertainment configuration has not been selected"))?;
    hue::set_stream_active(
        &config.bridge_ip,
        &config.username,
        &configuration_id,
        config.hue_bridge_certificate_sha256.as_deref(),
        active,
    )
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
    let pipeline_should_run = observed_tv_power != Some(false);
    if !pipeline_should_run && hue_active {
        let _ = set_hue_stream_active(&config, false);
    }

    if hue_active && hue_configuration_id(&config).is_none() {
        match sync_entertainment_areas(
            &config.bridge_ip,
            &config.username,
            config.entertainment_configuration_id.as_deref(),
            config.hue_bridge_certificate_sha256.as_deref(),
        ) {
            Ok(area) => {
                config.entertainment_area_id = area.configuration_id.clone();
                config.entertainment_configuration_id = Some(area.configuration_id);
                config.hue_bridge_certificate_sha256 = Some(area.certificate_sha256);
                config.zones = area.zones;
                if let Err(error) = config.save(&config_path) {
                    warn!(
                        "Resolved Hue V2 configuration but could not persist it: {}",
                        error
                    );
                }
            }
            Err(error) => warn!(
                "Hue V2 configuration discovery failed; using legacy packet header: {}",
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
    let r = running.clone();
    tokio::spawn(async move {
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
    });

    // 1. Initialize Philips Hue DTLS client if active
    let mut hue_sync_enabled = config.hue_sync_enabled;
    let mut nanoleaf_sync_enabled = config.nanoleaf_sync_enabled;
    let mut hue_dtls = if hue_active && hue_sync_enabled && pipeline_should_run {
        let client = reconnect_hue(&config)?;
        Some(client)
    } else {
        None
    };

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
        sampler.set_gamma(config.gamma);
        sampler.set_max_color_step(config.max_color_step);
    }

    // 3. Initialize capture
    let mut capture = create_capture(config.capture_width, config.capture_height);

    // Determine target framerate (supports auto-matching source refresh rate 23.976..60.0 Hz)
    let detected_fps = if config.fps == 0 {
        detect_source_fps()
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
    let _ = sd_notify::notify(
        true,
        &[
            sd_notify::NotifyState::Ready,
            sd_notify::NotifyState::Status(&status_desc),
        ],
    );

    let initial_settings = LiveSettings {
        brightness_multiplier: config.brightness_multiplier,
        hue_output_brightness: config.hue_output_brightness,
        nanoleaf_output_brightness: config.nanoleaf_output_brightness,
        saturation_boost: config.saturation_boost,
        peak_weight: config.peak_weight,
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
    shared_state
        .is_syncing
        .store(pipeline_should_run, Ordering::Relaxed);
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
    let mut static_frame_count: u32 = 0;
    let mut last_heartbeat = tokio::time::Instant::now();
    let mut last_nanoleaf_heartbeat = tokio::time::Instant::now();
    let mut light_update_count = 0u32;
    let mut light_update_window = tokio::time::Instant::now();
    let mut last_watchdog = tokio::time::Instant::now();
    let mut last_status_check = tokio::time::Instant::now();
    let mut last_hw_probe = tokio::time::Instant::now();
    let mut last_tv_power_poll = tokio::time::Instant::now() - Duration::from_secs(2);
    let mut last_tv_power_warning = tokio::time::Instant::now() - Duration::from_secs(30);
    let mut hue_retry = RetryState::new();
    let mut nanoleaf_retry = RetryState::new();
    let mut nanoleaf_only_frame_count = 0u64;
    let mut nanoleaf_only_global = RgbColor::new(0, 0, 0);
    let mut pending_commands = PendingCommands {
        desired_running: (!pipeline_should_run).then_some(false),
        ..PendingCommands::default()
    };
    let mut pipeline_state = if pipeline_should_run {
        PipelineState::Running
    } else {
        PipelineState::Paused
    };

    while running.load(Ordering::SeqCst) {
        pending_commands.receive(&mut command_rx);
        let loop_start = tokio::time::Instant::now();
        let mut emitted_light_update = false;

        // Feed systemd watchdog every 2 seconds
        if last_watchdog.elapsed() >= Duration::from_secs(2) {
            let _ = sd_notify::notify(true, &[sd_notify::NotifyState::Watchdog]);
            last_watchdog = tokio::time::Instant::now();
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
                        pending_commands.desired_running = Some(active);
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
        let web_stop_requested = pending_commands.desired_running.take() == Some(false);
        let mut external_stop = false;

        // Periodically verify if the user stopped sync from the official Hue mobile app (every 5s)
        if last_status_check.elapsed() >= Duration::from_secs(5) {
            last_status_check = tokio::time::Instant::now();
            if hue_active && hue_sync_enabled && hue_dtls.is_some() {
                if let Some(configuration_id) = hue_configuration_id(&config) {
                    if let Ok(state) = hue::get_stream_state(
                        &config.bridge_ip,
                        &config.username,
                        &configuration_id,
                        config.hue_bridge_certificate_sha256.as_deref(),
                    ) {
                        if !state.active {
                            external_stop = true;
                        }
                        if let Some(ref mut s) = hue_sampler {
                            s.set_smoothing_factor(state.smoothing_factor);
                        }
                        if let Some(ref mut ns) = nanoleaf_sampler {
                            ns.set_smoothing_factor(state.smoothing_factor);
                        }
                    }
                }
            }
        }

        if web_stop_requested || external_stop {
            info!(
                "Sync was stopped/paused (web={}, external={}). Entering paused idle state...",
                web_stop_requested, external_stop
            );
            shared_state.is_syncing.store(false, Ordering::SeqCst);
            pipeline_state = PipelineState::Paused;

            // Deactivate Hue stream on bridge if web requested stop
            if (web_stop_requested || (auto_tv_power && observed_tv_power == Some(false)))
                && hue_active
            {
                let _ = set_hue_stream_active(&config, false);
            }
            hue_dtls = None;
            shared_state.hue_connected.store(false, Ordering::Relaxed);

            // Fade Nanoleaf to black while paused
            if let Some(ref mut ns) = nanoleaf_streamer {
                let black = vec![RgbColor::new(0, 0, 0); ns.panel_count()];
                let _ = ns.send_frame(&black, 0);
            }

            while running.load(Ordering::SeqCst) {
                tokio::time::sleep(Duration::from_millis(250)).await;
                pending_commands.receive(&mut command_rx);
                let _ = sd_notify::notify(true, &[sd_notify::NotifyState::Watchdog]);

                let configured_auto_tv_power =
                    shared_state.current_settings.read().unwrap().auto_tv_power;
                if auto_tv_power != configured_auto_tv_power {
                    auto_tv_power = configured_auto_tv_power;
                    observed_tv_power = None;
                    shared_state.set_tv_power_state(None);
                    last_tv_power_poll = tokio::time::Instant::now() - Duration::from_secs(2);
                }

                let mut start_requested = pending_commands.desired_running.take() == Some(true);
                if auto_tv_power && last_tv_power_poll.elapsed() >= Duration::from_secs(2) {
                    last_tv_power_poll = tokio::time::Instant::now();
                    match tv_power::is_active().await {
                        Ok(state @ Some(_)) => {
                            shared_state.set_tv_power_state(state);
                            let active = state.unwrap();
                            if observed_tv_power == Some(false) && active {
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
                let mut app_reactivated = false;
                if !start_requested
                    && hue_active
                    && !(auto_tv_power && observed_tv_power == Some(false))
                {
                    if let Some(configuration_id) = hue_configuration_id(&config) {
                        if let Ok(st) = hue::get_stream_state(
                            &config.bridge_ip,
                            &config.username,
                            &configuration_id,
                            config.hue_bridge_certificate_sha256.as_deref(),
                        ) {
                            if st.active {
                                app_reactivated = true;
                            }
                        }
                    }
                }

                if start_requested || app_reactivated {
                    info!(
                        "Sync resume requested! Reactivating Hue & Nanoleaf streaming sessions..."
                    );
                    let mut hue_resumed = !hue_active;
                    if hue_active {
                        match reconnect_hue(&config) {
                            Ok(client) => {
                                info!("[+] Re-established Hue DTLS streaming session.");
                                hue_dtls = Some(client);
                                shared_state.hue_connected.store(true, Ordering::Relaxed);
                                hue_resumed = true;
                            }
                            Err(e) => {
                                error!("Failed to re-establish Hue DTLS streaming session: {}", e);
                                shared_state.hue_connected.store(false, Ordering::Relaxed);
                            }
                        }
                    }

                    if !hue_resumed {
                        shared_state.is_syncing.store(false, Ordering::SeqCst);
                        continue;
                    }

                    shared_state.is_syncing.store(true, Ordering::SeqCst);
                    pipeline_state = PipelineState::Running;

                    if nanoleaf_active {
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
            *shared_state.hue_zones.write().unwrap() = config.zones.clone();
            if hue_sync_enabled != live_st.hue_sync_enabled {
                hue_sync_enabled = live_st.hue_sync_enabled;
                if hue_sync_enabled && pipeline_state == PipelineState::Running {
                    match reconnect_hue(&config) {
                        Ok(client) => {
                            hue_dtls = Some(client);
                            shared_state.hue_connected.store(true, Ordering::Relaxed);
                        }
                        Err(error) => {
                            hue_sync_enabled = false;
                            shared_state.hue_connected.store(false, Ordering::Relaxed);
                            warn!("Hue sync remains off: {}", error);
                        }
                    }
                } else {
                    let _ = set_hue_stream_active(&config, false);
                    hue_dtls = None;
                    shared_state.hue_connected.store(false, Ordering::Relaxed);
                }
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
        if std::mem::take(&mut pending_commands.save_config) {
            let live_st = shared_state.current_settings.read().unwrap().clone();
            let mut save_cfg = config.clone();
            save_cfg.brightness_multiplier = live_st.brightness_multiplier;
            save_cfg.hue_output_brightness = live_st.hue_output_brightness;
            save_cfg.nanoleaf_output_brightness = live_st.nanoleaf_output_brightness;
            save_cfg.saturation_boost = live_st.saturation_boost;
            save_cfg.peak_weight = live_st.peak_weight;
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
            if let Some(nanoleaf) = save_cfg.nanoleaf.as_mut() {
                nanoleaf.alignment = live_st.nanoleaf_alignment;
            }
            save_cfg.max_color_step = live_st.max_color_step;
            if let Err(e) = save_cfg.save(&config_path) {
                error!("Failed to save updated config to {:?}: {}", config_path, e);
            } else {
                info!("[+] Successfully saved live settings to {:?}", config_path);
            }
        }

        if std::mem::take(&mut pending_commands.sync_bridge) && hue_active {
            match sync_entertainment_areas(
                &config.bridge_ip,
                &config.username,
                config.entertainment_configuration_id.as_deref(),
                config.hue_bridge_certificate_sha256.as_deref(),
            ) {
                Ok(area) if !area.zones.is_empty() => {
                    let configuration_changed = config.entertainment_configuration_id.as_deref()
                        != Some(&area.configuration_id);
                    if configuration_changed {
                        let _ = set_hue_stream_active(&config, false);
                    }
                    config.entertainment_area_id = area.configuration_id.clone();
                    config.entertainment_configuration_id = Some(area.configuration_id);
                    config.hue_bridge_certificate_sha256 = Some(area.certificate_sha256);
                    config.zones = area.zones;
                    *shared_state.hue_zones.write().unwrap() = config.zones.clone();
                    let live_st = shared_state.current_settings.read().unwrap().clone();
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
                        sampler.set_gamma(live_st.gamma);
                        sampler.set_max_color_step(live_st.max_color_step);
                    }
                    if configuration_changed {
                        hue_packet_builder =
                            Some(HueStreamPacketBuilder::new(hue_configuration_id(&config)));
                        match reconnect_hue(&config) {
                            Ok(client) => {
                                hue_dtls = Some(client);
                                shared_state.hue_connected.store(true, Ordering::Relaxed);
                            }
                            Err(error) => {
                                hue_dtls = None;
                                shared_state.hue_connected.store(false, Ordering::Relaxed);
                                warn!("Hue area changed but DTLS rebind failed: {}", error);
                            }
                        }
                    }
                    if let Err(error) = config.save(&config_path) {
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

        // 4. Check for pipeline restart request from Web UI
        if std::mem::take(&mut pending_commands.reconfigure) {
            info!("Device credentials saved; restarting daemon to load the new configuration.");
            break;
        }

        if std::mem::take(&mut pending_commands.restart) {
            info!("Pipeline restart requested via Web UI. Re-initializing capture...");
            drop(capture);
            tokio::time::sleep(Duration::from_millis(500)).await;
            capture = create_capture(config.capture_width, config.capture_height);
            shared_state
                .capture_hardware
                .store(capture.is_real_hardware(), Ordering::Relaxed);
        }

        // In auto FPS mode, dynamically update cadence if TV source rate changed (e.g. film started)
        if config.fps == 0 {
            if let Some(new_fps) = detect_source_fps() {
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
        if !capture.is_real_hardware() && last_hw_probe.elapsed() >= Duration::from_millis(500) {
            last_hw_probe = tokio::time::Instant::now();
            match VtCapture::try_new(config.capture_width, config.capture_height) {
                Ok(hw_capture) => {
                    info!("[+] Successfully upgraded from MockCapture to hardware VtCapture!");
                    capture = Box::new(hw_capture);
                    shared_state.capture_hardware.store(true, Ordering::Relaxed);
                }
                Err(_) => {
                    // Hardware capture still settling
                }
            }
        }

        match capture.acquire_frame() {
            Ok(frame) => {
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
                                    |((_, (a1, a2, a3)), (_, (b1, b2, b3)))| {
                                        (*a1 as i32 - *b1 as i32).abs() < 350
                                            && (*a2 as i32 - *b2 as i32).abs() < 350
                                            && (*a3 as i32 - *b3 as i32).abs() < 350
                                    },
                                )
                            } else {
                                false
                            };

                        let should_send = if config.adaptive_throttling {
                            if is_virtually_identical {
                                static_frame_count = static_frame_count.saturating_add(1);
                                if last_heartbeat.elapsed() >= Duration::from_millis(500) {
                                    last_heartbeat = tokio::time::Instant::now();
                                    true
                                } else {
                                    false
                                }
                            } else {
                                static_frame_count = 0;
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
                                if hue_retry.ready() {
                                    match reconnect_hue(&config) {
                                        Ok(new_dtls) => {
                                            *dtls = new_dtls;
                                            hue_retry.success();
                                            shared_state
                                                .hue_connected
                                                .store(true, Ordering::Relaxed);
                                            info!("[+] Successfully reconnected Hue DTLS session.");
                                        }
                                        Err(conn_err) => {
                                            let delay = hue_retry.failure();
                                            warn!(
                                                "DTLS reconnect failed: {}. Retrying in {:.1}s.",
                                                conn_err,
                                                delay.as_secs_f32()
                                            );
                                        }
                                    }
                                }
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
                        shared_state
                            .nanoleaf_connected
                            .store(true, Ordering::Relaxed);

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
                                last_nanoleaf_colors = nl_colors;
                                emitted_light_update = true;
                            }
                        }
                    }
                }
            }
            Err(e) => {
                warn!(
                    "Capture frame acquisition interrupted ({}) - likely video format/timing change. Releasing hardware scaler...",
                    e
                );
                // Explicitly drop capture to close /dev/video60 and free hardware scaler
                drop(capture);

                // Clear/black out Nanoleaf during video transition
                if let Some(ref mut ns) = nanoleaf_streamer {
                    let black = vec![RgbColor::new(0, 0, 0); ns.panel_count()];
                    let _ = ns.send_frame(&black, 0);
                }

                // Sleep 750ms for webOS Display Engine and HDMI PLL/scaler to settle
                tokio::time::sleep(Duration::from_millis(750)).await;
                let _ = sd_notify::notify(true, &[sd_notify::NotifyState::Watchdog]);

                // Auto-adapt to new source refresh rate if enabled
                if config.fps == 0 {
                    if let Some(new_fps) = detect_source_fps() {
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
                last_hw_probe = tokio::time::Instant::now();
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

    // Cleanup: deactivate Hue stream
    if hue_active {
        info!("Deactivating entertainment area stream on Hue Bridge...");
        let _ = set_hue_stream_active(&config, false);
    }

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

    let mut dtls_client =
        HueDtlsClient::connect(&config.bridge_ip, &config.username, &config.clientkey)?;

    let mut packet_builder = HueStreamPacketBuilder::new(hue_configuration_id(&config));
    info!("Streaming fast rainbow test pattern across all Hue entertainment channels for 15 seconds...");

    let start = std::time::Instant::now();
    while start.elapsed().as_secs() < 15 {
        let hue_offset = (start.elapsed().as_secs_f32() * 180.0) % 360.0; // Fast rotation (180 deg/sec)
        let mut channels = Vec::new();

        // Broadcast across all 8 channels in the entertainment area (lights + gradient lightstrip)
        for channel_id in 0..8u8 {
            let hue = (hue_offset + (channel_id as f32) * 45.0) % 360.0;
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

async fn run_test_capture(config_path: PathBuf) -> Result<()> {
    let config = Config::load(&config_path)?;
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

async fn run_pair(bridge_opt: Option<String>, output: PathBuf) -> Result<()> {
    let bridge_ip = match bridge_opt {
        Some(ip) => ip,
        None => match hue::discover_bridge() {
            Ok(ip) => ip,
            Err(e) => {
                println!(
                    "Auto-discovery: {}. Please enter Hue Bridge IP manually:",
                    e
                );
                use std::io::{stdin, stdout, Write};
                print!("Bridge IP: ");
                stdout().flush().ok();
                let mut line = String::new();
                stdin().read_line(&mut line)?;
                line.trim().to_string()
            }
        },
    };

    let (username, clientkey, area) = hue::pair_bridge(&bridge_ip, 45)?;
    let mut config = if output.exists() {
        Config::load(&output).unwrap_or_else(|_| {
            Config::new_default(&bridge_ip, &username, &clientkey, &area.configuration_id)
        })
    } else {
        Config::new_default(&bridge_ip, &username, &clientkey, &area.configuration_id)
    };

    config.bridge_ip = bridge_ip;
    config.username = username;
    config.clientkey = clientkey;
    config.entertainment_area_id = area.configuration_id.clone();
    config.entertainment_configuration_id = Some(area.configuration_id);
    config.hue_bridge_certificate_sha256 = Some(area.certificate_sha256);
    if !area.zones.is_empty() {
        config.zones = area.zones;
    }

    config.save(&output)?;
    println!(
        "\n[+] Configuration and Hue credentials successfully saved to {:?}",
        output
    );
    println!("    You can now run: lg-hue-sync run --config {:?}", output);
    Ok(())
}

async fn run_sync_hue(config_path: PathBuf, target_area: Option<String>) -> Result<()> {
    let mut config = Config::load(&config_path)?;
    info!(
        "Querying Hue Bridge at {} for Entertainment Areas...",
        config.bridge_ip
    );

    let area = hue::sync_entertainment_areas(
        &config.bridge_ip,
        &config.username,
        target_area.as_deref(),
        config.hue_bridge_certificate_sha256.as_deref(),
    )?;

    config.entertainment_area_id = area.configuration_id.clone();
    config.entertainment_configuration_id = Some(area.configuration_id);
    config.hue_bridge_certificate_sha256 = Some(area.certificate_sha256);
    if !area.zones.is_empty() {
        config.zones = area.zones;
    }
    config.save(&config_path)?;

    println!(
        "\n[+] Entertainment area '{}' (ID: {}) synced successfully!",
        area.name, config.entertainment_area_id
    );
    println!(
        "    Updated {} light zones in {:?}",
        config.zones.len(),
        config_path
    );
    println!("    3D light positions have been refreshed and projected to screen sampling boxes.");
    Ok(())
}

async fn run_pair_nanoleaf(ip_opt: Option<String>, config_path: PathBuf) -> Result<()> {
    let ip = match ip_opt {
        Some(ip) => ip,
        None => {
            use std::io::{stdin, stdout, Write};
            print!("Enter Nanoleaf 4D IP address: ");
            stdout().flush().ok();
            let mut line = String::new();
            stdin().read_line(&mut line)?;
            line.trim().to_string()
        }
    };

    let (auth_token, num_panels, panel_ids) = nanoleaf::pair_nanoleaf(&ip, 45)?;
    let mut config = if config_path.exists() {
        Config::load(&config_path).unwrap_or_else(|_| Config::new_default("", "", "", ""))
    } else {
        Config::new_default("", "", "", "")
    };

    config.nanoleaf = Some(crate::config::NanoleafConfig {
        enabled: true,
        ip: ip.clone(),
        auth_token,
        udp_port: 60222,
        segments: num_panels.max(30),
        panel_ids,
        alignment: NanoleafAlignment::default(),
    });

    config.save(&config_path)?;
    println!(
        "\n[+] Nanoleaf 4D controller at {} successfully paired and saved to {:?}",
        ip, config_path
    );
    println!(
        "    You can test it with: lg-hue-sync test-nanoleaf --config {:?}",
        config_path
    );
    println!(
        "    Run live sync with:   lg-hue-sync run --config {:?}",
        config_path
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn duplicate_output_frames_are_suppressed_but_meaningful_changes_pass() {
        let previous = vec![RgbColor::new(10, 20, 30)];
        assert!(!colors_changed(&previous, &previous));
        assert!(colors_changed(&previous, &[RgbColor::new(13, 20, 30)]));
    }
}
