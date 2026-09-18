mod capture;
mod color;
mod config;
mod hue;

use anyhow::Result;
use clap::{Parser, Subcommand};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tracing::{error, info, Level};
use tracing_subscriber::FmtSubscriber;

use capture::create_capture;
use color::{RgbColor, ZoneSampler};
use config::Config;
use hue::{set_stream_active, HueDtlsClient, HueStreamPacketBuilder};

#[derive(Parser)]
#[command(name = "lg-hue-sync")]
#[command(about = "High-performance native screen capture and Philips Hue synchronizer for LG webOS", long_about = None)]
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
        Commands::TestCapture { config } => run_test_capture(config).await,
        Commands::Pair { bridge, output } => run_pair(bridge, output).await,
    }
}

async fn run_daemon(config_path: PathBuf) -> Result<()> {
    let config = Config::load(&config_path)?;
    info!("Loaded configuration for Bridge at {}", config.bridge_ip);

    // Setup graceful shutdown handler
    let running = Arc::new(AtomicBool::new(true));
    let r = running.clone();
    tokio::spawn(async move {
        tokio::signal::ctrl_c().await.ok();
        info!("Received shutdown signal. Stopping sync...");
        r.store(false, Ordering::SeqCst);
    });

    // 1. Activate entertainment streaming on Hue Bridge
    set_stream_active(
        &config.bridge_ip,
        &config.username,
        &config.entertainment_area_id,
        true,
    )?;

    // 2. Connect DTLS client
    let mut dtls_client = HueDtlsClient::connect(
        &config.bridge_ip,
        &config.username,
        &config.clientkey,
    )?;

    // 3. Initialize capture and color samplers
    let mut capture = create_capture(160, 90);
    let mut sampler = ZoneSampler::new(
        config.zones.clone(),
        0.35,
        config.hdr_tone_mapping,
        config.letterbox_detection,
        config.saturation_boost,
        config.noise_gate_threshold,
    );
    let mut packet_builder = HueStreamPacketBuilder::new(Some(config.entertainment_area_id.clone()));

    // Bound framerate between 20 and 60 Hz (supports 23.976, 24, 25, 29.97, 30, 50, 60 fps)
    let target_fps = (config.fps as f64).clamp(20.0, 60.0);
    let frame_interval = Duration::from_secs_f64(1.0 / target_fps);
    info!(
        "Entering sync loop at {:.2} FPS ({:.2} ms cadence, mode: {}, HDR tone-mapping: {}, letterbox: {}, zones: {})...",
        target_fps,
        frame_interval.as_secs_f64() * 1000.0,
        if config.use_xy_gamut { "CIE 1931 xy (Gamut C)" } else { "sRGB" },
        config.hdr_tone_mapping,
        config.letterbox_detection,
        config.zones.len()
    );

    // Notify systemd that service is ready and streaming
    let _ = sd_notify::notify(true, &[
        sd_notify::NotifyState::Ready,
        sd_notify::NotifyState::Status(&format!("Syncing at {:.2} FPS ({} zones)", target_fps, config.zones.len())),
    ]);

    let mut last_channels: Vec<(u8, (u16, u16, u16))> = Vec::new();
    let mut static_frame_count: u32 = 0;
    let mut last_heartbeat = tokio::time::Instant::now();
    let mut last_watchdog = tokio::time::Instant::now();

    while running.load(Ordering::SeqCst) {
        let loop_start = tokio::time::Instant::now();

        // Feed systemd watchdog every 2 seconds
        if last_watchdog.elapsed() >= Duration::from_secs(2) {
            let _ = sd_notify::notify(true, &[sd_notify::NotifyState::Watchdog]);
            last_watchdog = tokio::time::Instant::now();
        }

        match capture.acquire_frame() {
            Ok(frame) => {
                let (sampled_zones, is_scene_cut) = sampler.sample_frame(
                    frame.data,
                    frame.width,
                    frame.height,
                    frame.is_bgra,
                );

                if is_scene_cut {
                    tracing::debug!("Detected scene cut: snapping lighting without latency");
                }

                // Convert colors to 16-bit values (either xy+brightness or raw RGB)
                let channels: Vec<(u8, (u16, u16, u16))> = sampled_zones
                    .into_iter()
                    .map(|(channel_id, color)| {
                        let scaled = RgbColor::new(
                            ((color.r as f32) * config.brightness_multiplier).clamp(0.0, 255.0) as u8,
                            ((color.g as f32) * config.brightness_multiplier).clamp(0.0, 255.0) as u8,
                            ((color.b as f32) * config.brightness_multiplier).clamp(0.0, 255.0) as u8,
                        );
                        if config.use_xy_gamut {
                            (channel_id, scaled.to_xy_u16(color::HueGamut::GamutC))
                        } else {
                            (channel_id, scaled.to_u16())
                        }
                    })
                    .collect();

                // Adaptive deadband throttling:
                // Check if color change across all channels is below Just Noticeable Difference (JND ~ 1%)
                let is_virtually_identical = if !last_channels.is_empty() && last_channels.len() == channels.len() {
                    last_channels.iter().zip(&channels).all(|((_, (a1, a2, a3)), (_, (b1, b2, b3)))| {
                        (*a1 as i32 - *b1 as i32).abs() < 350
                            && (*a2 as i32 - *b2 as i32).abs() < 350
                            && (*a3 as i32 - *b3 as i32).abs() < 350
                    })
                } else {
                    false
                };

                let should_send = if config.adaptive_throttling {
                    if is_virtually_identical {
                        static_frame_count = static_frame_count.saturating_add(1);
                        // Send at 2 Hz heartbeat to prevent Bridge timeout (watchdog is 5.0s)
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
                    let packet = if config.use_xy_gamut {
                        packet_builder.build_xy_packet(&channels)
                    } else {
                        packet_builder.build_rgb_packet(&channels)
                    };

                    if let Err(e) = dtls_client.send(&packet) {
                        error!("Failed to send DTLS HueStream packet: {}", e);
                    }
                    last_channels = channels;
                }
            }
            Err(e) => {
                error!("Capture frame acquisition error: {}", e);
            }
        }

        let elapsed = loop_start.elapsed();
        if elapsed < frame_interval {
            tokio::time::sleep(frame_interval - elapsed).await;
        }
    }

    // Cleanup: deactivate stream
    info!("Deactivating entertainment area stream on bridge...");
    let _ = set_stream_active(
        &config.bridge_ip,
        &config.username,
        &config.entertainment_area_id,
        false,
    );

    info!("lg-hue-sync terminated cleanly.");
    Ok(())
}

async fn run_test_pattern(config_path: PathBuf) -> Result<()> {
    let config = Config::load(&config_path)?;
    info!("Testing connection to Hue Bridge at {}...", config.bridge_ip);

    set_stream_active(
        &config.bridge_ip,
        &config.username,
        &config.entertainment_area_id,
        true,
    )?;

    let mut dtls_client = HueDtlsClient::connect(
        &config.bridge_ip,
        &config.username,
        &config.clientkey,
    )?;

    let mut packet_builder = HueStreamPacketBuilder::new(Some(config.entertainment_area_id.clone()));
    info!("Streaming rainbow test pattern for 10 seconds...");

    let start = std::time::Instant::now();
    while start.elapsed().as_secs() < 10 {
        let hue_offset = (start.elapsed().as_secs_f32() * 60.0) % 360.0;
        let mut channels = Vec::new();

        for (i, zone) in config.zones.iter().enumerate() {
            let hue = (hue_offset + i as f32 * 60.0) % 360.0;
            let (r, g, b) = hsv_to_rgb(hue, 1.0, 1.0);
            channels.push((zone.channel_id, RgbColor::new(r, g, b).to_u16()));
        }

        let packet = packet_builder.build_rgb_packet(&channels);
        dtls_client.send(&packet)?;
        tokio::time::sleep(Duration::from_millis(33)).await;
    }

    set_stream_active(
        &config.bridge_ip,
        &config.username,
        &config.entertainment_area_id,
        false,
    )?;

    info!("[+] Test pattern completed successfully!");
    Ok(())
}

async fn run_test_capture(config_path: PathBuf) -> Result<()> {
    let config = Config::load(&config_path)?;
    let mut capture = create_capture(160, 90);
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
        let (sampled, is_cut) = sampler.sample_frame(frame.data, frame.width, frame.height, frame.is_bgra);
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
                println!("Auto-discovery: {}. Please enter Hue Bridge IP manually:", e);
                use std::io::{stdin, stdout, Write};
                print!("Bridge IP: ");
                stdout().flush().ok();
                let mut line = String::new();
                stdin().read_line(&mut line)?;
                line.trim().to_string()
            }
        },
    };

    let (username, clientkey, area_id) = hue::pair_bridge(&bridge_ip, 45)?;
    let mut config = if output.exists() {
        Config::load(&output).unwrap_or_else(|_| Config::new_default(&bridge_ip, &username, &clientkey, &area_id))
    } else {
        Config::new_default(&bridge_ip, &username, &clientkey, &area_id)
    };

    config.bridge_ip = bridge_ip;
    config.username = username;
    config.clientkey = clientkey;
    config.entertainment_area_id = area_id;

    config.save(&output)?;
    println!("\n[+] Configuration and Hue credentials successfully saved to {:?}", output);
    println!("    You can now run: lg-hue-sync run --config {:?}", output);
    Ok(())
}

