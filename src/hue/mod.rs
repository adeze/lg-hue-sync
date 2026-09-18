pub mod dtls;
pub mod stream;

use anyhow::{anyhow, Context, Result};
use serde_json::Value;
use std::time::{Duration, Instant};
use tracing::info;

pub use dtls::HueDtlsClient;
pub use stream::HueStreamPacketBuilder;

/// Auto-discovers Hue Bridge on LAN via official discovery service
pub fn discover_bridge() -> Result<String> {
    info!("Searching for Hue Bridge via discovery.meethue.com...");
    let resp = ureq::get("https://discovery.meethue.com/")
        .set("User-Agent", "lg-hue-sync/0.1.0")
        .call()
        .context("Failed to query Hue discovery API")?;

    let json: Value = resp.into_json().context("Failed to parse discovery JSON")?;
    if let Some(arr) = json.as_array() {
        if let Some(first) = arr.first() {
            if let Some(ip) = first.get("internalipaddress").and_then(|v| v.as_str()) {
                info!("Found Hue Bridge at {}", ip);
                return Ok(ip.to_string());
            }
        }
    }
    Err(anyhow!("No Hue Bridge detected on network via discovery API"))
}

/// Polls the bridge for pushlink button press and authenticates
pub fn pair_bridge(
    bridge_ip: &str,
    timeout_secs: u64,
) -> Result<(String, String, String, Vec<crate::config::LightZone>)> {
    let url = format!("http://{}/api", bridge_ip);
    let payload = serde_json::json!({
        "devicetype": "lg-hue-sync#tv",
        "generateclientkey": true
    });

    println!("\n========================================================");
    println!(">>> ACTION REQUIRED:");
    println!(">>> Press the big round button on your Philips Hue Bridge!");
    println!("========================================================\n");

    let start = Instant::now();
    let mut username = String::new();
    let mut clientkey = String::new();

    while start.elapsed() < Duration::from_secs(timeout_secs) {
        let resp = ureq::post(&url)
            .set("Content-Type", "application/json")
            .send_json(payload.clone());

        if let Ok(r) = resp {
            if let Ok(Value::Array(items)) = r.into_json::<Value>() {
                if let Some(first) = items.first() {
                    if let Some(success) = first.get("success") {
                        username = success.get("username").and_then(|v| v.as_str()).unwrap_or_default().to_string();
                        clientkey = success.get("clientkey").and_then(|v| v.as_str()).unwrap_or_default().to_string();
                        println!("\n[+] Successfully paired with Hue Bridge!");
                        break;
                    } else if let Some(err) = first.get("error") {
                        if err.get("type").and_then(|v| v.as_i64()) == Some(101) {
                            print!(".");
                            use std::io::Write;
                            std::io::stdout().flush().ok();
                        }
                    }
                }
            }
        }
        std::thread::sleep(Duration::from_secs(1));
    }

    if username.is_empty() || clientkey.is_empty() {
        return Err(anyhow!("Timed out waiting for Hue Bridge button press"));
    }

    // Query Entertainment Areas and 3D light locations
    let groups_url = format!("http://{}/api/{}/groups", bridge_ip, username);
    let groups_resp = ureq::get(&groups_url)
        .call()
        .context("Failed to query groups from Hue Bridge")?;

    let groups_json: Value = groups_resp.into_json()?;
    let mut area_id = "1".to_string();
    let mut discovered_zones: Vec<crate::config::LightZone> = Vec::new();

    if let Value::Object(groups) = groups_json {
        let mut entertainment_areas = Vec::new();
        for (gid, gdata) in groups {
            if gdata.get("type").and_then(|v| v.as_str()) == Some("Entertainment") {
                let name = gdata.get("name").and_then(|v| v.as_str()).unwrap_or("Unnamed");
                let locations = gdata.get("locations").cloned();
                entertainment_areas.push((gid, name.to_string(), locations));
            }
        }

        if !entertainment_areas.is_empty() {
            println!("\nFound {} Entertainment Area(s):", entertainment_areas.len());
            for (i, (gid, name, _)) in entertainment_areas.iter().enumerate() {
                println!("  [{}] ID: {} - {}", i + 1, gid, name);
            }
            area_id = entertainment_areas[0].0.clone();
            println!("Auto-selected: '{}' (ID: {})", entertainment_areas[0].1, area_id);

            // If locations are present, auto-derive 2D screen sampling zones using 3D coordinates
            if let Some(Value::Object(locs)) = &entertainment_areas[0].2 {
                for (channel_idx, (light_id, coords_val)) in locs.iter().enumerate() {
                    if let Some(coords_arr) = coords_val.as_array() {
                        if coords_arr.len() >= 3 {
                            let x = coords_arr[0].as_f64().unwrap_or(0.0) as f32;
                            let y = coords_arr[1].as_f64().unwrap_or(0.0) as f32;
                            let z = coords_arr[2].as_f64().unwrap_or(0.0) as f32;
                            let zone_name = format!("Light {}", light_id);
                            let zone = crate::config::LightZone::from_3d_position(
                                channel_idx as u8,
                                &zone_name,
                                [x, y, z],
                            );
                            println!("  -> Mapped Light {} at 3D [X:{:.2}, Y:{:.2}, Z:{:.2}] to screen zone [{:.2}..{:.2}, {:.2}..{:.2}]",
                                light_id, x, y, z, zone.x_min, zone.x_max, zone.y_min, zone.y_max);
                            discovered_zones.push(zone);
                        }
                    }
                }
            }
        } else {
            println!("\n[!] Warning: No Entertainment Areas found. Please configure one in Hue app.");
        }
    }

    Ok((username, clientkey, area_id, discovered_zones))
}

/// Activates or deactivates the entertainment streaming session on the Hue Bridge
pub fn set_stream_active(
    bridge_ip: &str,
    username: &str,
    group_id: &str,
    active: bool,
) -> Result<()> {
    let url = format!("http://{}/api/{}/groups/{}", bridge_ip, username, group_id);
    let payload = serde_json::json!({
        "stream": {
            "active": active
        }
    });

    info!(
        "Sending stream {} request to Hue Bridge at group {}...",
        if active { "START" } else { "STOP" },
        group_id
    );

    let resp = ureq::put(&url)
        .set("Content-Type", "application/json")
        .send_json(payload)
        .with_context(|| format!("Failed to send stream active state to {}", url))?;

    let text = resp.into_string()?;
    info!("Bridge response: {}", text);
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StreamState {
    pub active: bool,
    /// 0.0 to 1.0 (if reported by bridge, default 1.0)
    pub brightness: f32,
    /// Suggested smoothing factor: 0.15 (subtle) to 0.85 (extreme)
    pub smoothing_factor: f32,
}

/// Queries whether the entertainment area is currently active on the Hue Bridge and reads settings
pub fn get_stream_state(
    bridge_ip: &str,
    username: &str,
    group_id: &str,
) -> Result<StreamState> {
    let url = format!("http://{}/api/{}/groups/{}", bridge_ip, username, group_id);
    let resp = ureq::get(&url)
        .timeout(Duration::from_secs(3))
        .call()
        .with_context(|| format!("Failed to query group status from {}", url))?;

    let json: Value = resp.into_json()?;
    let stream_obj = json.get("stream");
    let is_active = stream_obj
        .and_then(|s| s.get("active"))
        .and_then(|a| a.as_bool())
        .unwrap_or(false);

    // Read any action / bri if available from group
    let bri = json
        .get("action")
        .and_then(|a| a.get("bri"))
        .and_then(|b| b.as_f64())
        .map(|b| (b / 254.0) as f32)
        .unwrap_or(1.0);

    // Map proxy mode or default to 0.35 (moderate)
    let smoothing = 0.35f32;

    Ok(StreamState {
        active: is_active,
        brightness: bri,
        smoothing_factor: smoothing,
    })
}

/// Queries whether the entertainment area is currently active on the Hue Bridge
#[allow(dead_code)]
pub fn get_stream_status(
    bridge_ip: &str,
    username: &str,
    group_id: &str,
) -> Result<bool> {
    get_stream_state(bridge_ip, username, group_id).map(|s| s.active)
}
