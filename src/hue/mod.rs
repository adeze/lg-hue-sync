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
    Err(anyhow!(
        "No Hue Bridge detected on network via discovery API"
    ))
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
                        username = success
                            .get("username")
                            .and_then(|v| v.as_str())
                            .unwrap_or_default()
                            .to_string();
                        clientkey = success
                            .get("clientkey")
                            .and_then(|v| v.as_str())
                            .unwrap_or_default()
                            .to_string();
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

    let (area_id, _area_name, discovered_zones) =
        sync_entertainment_areas(bridge_ip, &username, None)?;

    Ok((username, clientkey, area_id, discovered_zones))
}

/// Queries the Hue Bridge for Entertainment Areas and translates 3D light coordinates into screen sampling zones.
/// If `target_area` is specified, selects by ID or name; otherwise defaults to the first configured area.
pub fn sync_entertainment_areas(
    bridge_ip: &str,
    username: &str,
    target_area: Option<&str>,
) -> Result<(String, String, Vec<crate::config::LightZone>)> {
    let groups_url = format!("http://{}/api/{}/groups", bridge_ip, username);
    let groups_resp = ureq::get(&groups_url)
        .call()
        .context("Failed to query groups from Hue Bridge")?;

    let groups_json: Value = groups_resp.into_json()?;
    let groups = groups_json
        .as_object()
        .ok_or_else(|| anyhow!("Hue Bridge groups response was not an object"))?;
    let mut entertainment_areas: Vec<_> = groups
        .iter()
        .filter(|(_, group)| group.get("type").and_then(Value::as_str) == Some("Entertainment"))
        .map(|(id, group)| {
            (
                id.clone(),
                group
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or("Unnamed")
                    .to_string(),
            )
        })
        .collect();
    entertainment_areas.sort_by(|left, right| left.0.cmp(&right.0));
    let configurations_url = format!(
        "https://{}/clip/v2/resource/entertainment_configuration",
        bridge_ip
    );
    let configurations: Value = ureq::get(&configurations_url)
        .set("hue-application-key", username)
        .call()
        .with_context(|| {
            format!(
                "Failed to query Hue v2 Entertainment configurations at {}",
                configurations_url
            )
        })?
        .into_json()?;
    let configuration_data = configurations
        .get("data")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("Hue v2 Entertainment configurations response has no data"))?;
    let (area_id, area_name, configuration) =
        select_entertainment_area(&entertainment_areas, configuration_data, target_area)?;
    let discovered_zones = zones_from_v2_configuration(configuration)?;
    if discovered_zones.is_empty() {
        return Err(anyhow!(
            "Hue v2 configuration '{}' has no channel positions",
            area_name
        ));
    }
    Ok((area_id, area_name, discovered_zones))
}

fn select_entertainment_area<'a>(
    groups: &[(String, String)],
    configurations: &'a [Value],
    target: Option<&str>,
) -> Result<(String, String, &'a Value)> {
    let find_configuration = |name: &str| {
        configurations.iter().find(|item| {
            item.pointer("/metadata/name")
                .and_then(Value::as_str)
                .is_some_and(|value| value.eq_ignore_ascii_case(name))
        })
    };
    let selected_group = target
        .and_then(|value| {
            groups
                .iter()
                .find(|(id, name)| id == value || name.eq_ignore_ascii_case(value))
        })
        .or_else(|| (target.is_none() && groups.len() == 1).then(|| &groups[0]));
    if let Some((area_id, area_name)) = selected_group {
        let configuration = find_configuration(area_name)
            .ok_or_else(|| anyhow!("Hue v2 configuration named '{}' was not found", area_name))?;
        return Ok((area_id.clone(), area_name.clone(), configuration));
    }

    if let Some(target) = target {
        let configuration = configurations.iter().find(|item| {
            item.get("id").and_then(Value::as_str) == Some(target)
                || item
                    .pointer("/metadata/name")
                    .and_then(Value::as_str)
                    .is_some_and(|name| name.eq_ignore_ascii_case(target))
        });
        if let Some(configuration) = configuration {
            let name = configuration
                .pointer("/metadata/name")
                .and_then(Value::as_str)
                .ok_or_else(|| anyhow!("Hue v2 configuration has no metadata name"))?;
            let (area_id, area_name) = groups
                .iter()
                .find(|(_, group_name)| group_name.eq_ignore_ascii_case(name))
                .ok_or_else(|| {
                    anyhow!(
                        "Hue v2 configuration '{}' has no matching v1 Entertainment group",
                        name
                    )
                })?;
            return Ok((area_id.clone(), area_name.clone(), configuration));
        }
    }

    Err(anyhow!(
        "Select an explicit Hue Entertainment Area when more than one exists"
    ))
}

fn zones_from_v2_configuration(configuration: &Value) -> Result<Vec<crate::config::LightZone>> {
    let mut zones = configuration
        .get("channels")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("Hue v2 configuration has no channels"))?
        .iter()
        .map(|channel| {
            let channel_id = channel
                .get("channel_id")
                .and_then(Value::as_u64)
                .and_then(|id| u8::try_from(id).ok())
                .ok_or_else(|| anyhow!("Hue v2 channel has an invalid channel_id"))?;
            let position = channel
                .get("position")
                .ok_or_else(|| anyhow!("Hue v2 channel {} has no position", channel_id))?;
            let coordinate = |axis| {
                position
                    .get(axis)
                    .and_then(Value::as_f64)
                    .map(|value| value as f32)
                    .filter(|value| value.is_finite())
                    .ok_or_else(|| {
                        anyhow!(
                            "Hue v2 channel {} has invalid {} coordinate",
                            channel_id,
                            axis
                        )
                    })
            };
            Ok(crate::config::LightZone::from_3d_position(
                channel_id,
                &format!("Hue channel {}", channel_id),
                [coordinate("x")?, coordinate("y")?, coordinate("z")?],
            ))
        })
        .collect::<Result<Vec<_>>>()?;
    zones.sort_by_key(|zone| zone.channel_id);
    Ok(zones)
}

#[cfg(test)]
mod selection_tests {
    use super::*;

    #[test]
    fn v2_configuration_id_resolves_to_matching_v1_group() {
        let groups = vec![
            ("1".to_string(), "Cinema".to_string()),
            ("2".to_string(), "Music".to_string()),
        ];
        let configurations = vec![serde_json::json!({
            "id": "12345678-1234-1234-1234-123456789abc",
            "metadata": { "name": "Cinema" },
            "channels": []
        })];

        let (id, name, configuration) = select_entertainment_area(
            &groups,
            &configurations,
            Some("12345678-1234-1234-1234-123456789abc"),
        )
        .unwrap();

        assert_eq!(id, "1");
        assert_eq!(name, "Cinema");
        assert_eq!(configuration["id"], "12345678-1234-1234-1234-123456789abc");
    }
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
pub fn get_stream_state(bridge_ip: &str, username: &str, group_id: &str) -> Result<StreamState> {
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
pub fn get_stream_status(bridge_ip: &str, username: &str, group_id: &str) -> Result<bool> {
    get_stream_state(bridge_ip, username, group_id).map(|s| s.active)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn v2_channels_keep_explicit_channel_ids() {
        let configuration = serde_json::json!({
            "channels": [
                {"channel_id": 7, "position": {"x": 1.0, "y": 1.0, "z": 0.0}},
                {"channel_id": 2, "position": {"x": -1.0, "y": -1.0, "z": 0.0}}
            ]
        });
        let zones = zones_from_v2_configuration(&configuration).unwrap();
        assert_eq!(
            zones.iter().map(|zone| zone.channel_id).collect::<Vec<_>>(),
            vec![2, 7]
        );
        assert!(zones[0].x_max < 0.5);
        assert!(zones[1].x_min > 0.5);
    }
}
