pub mod dtls;
pub mod stream;

use anyhow::{anyhow, Context, Result};
use openssl::hash::MessageDigest;
use openssl::ssl::{SslConnector, SslMethod, SslVerifyMode};
use serde_json::Value;
use std::fmt::Write as _;
use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::{Duration, Instant};
use tracing::info;

pub use dtls::HueDtlsClient;
pub use stream::HueStreamPacketBuilder;

#[derive(Debug, Clone)]
pub struct EntertainmentArea {
    pub legacy_group_id: String,
    pub name: String,
    pub configuration_id: String,
    pub zones: Vec<crate::config::LightZone>,
    pub certificate_sha256: String,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct EntertainmentAreaSummary {
    pub id: String,
    pub name: String,
}

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
) -> Result<(String, String, EntertainmentArea)> {
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

    let area = sync_entertainment_areas(bridge_ip, &username, None, None)?;

    Ok((username, clientkey, area))
}

/// Queries the Hue Bridge for Entertainment Areas and translates 3D light coordinates into screen sampling zones.
/// If `target_area` is specified, selects by ID or name; otherwise defaults to the first configured area.
pub fn sync_entertainment_areas(
    bridge_ip: &str,
    username: &str,
    target_area: Option<&str>,
    expected_certificate_sha256: Option<&str>,
) -> Result<EntertainmentArea> {
    let entertainment_areas = list_entertainment_areas(bridge_ip, username)?
        .into_iter()
        .map(|area| (area.id, area.name))
        .collect::<Vec<_>>();
    let (configurations, certificate_sha256) =
        fetch_v2_configurations(bridge_ip, username, expected_certificate_sha256)?;
    let configuration_data = configurations
        .get("data")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("Hue v2 Entertainment configurations response has no data"))?;
    let (area_id, area_name, configuration) =
        select_entertainment_area(&entertainment_areas, configuration_data, target_area)?;
    let configuration_id = configuration
        .get("id")
        .and_then(Value::as_str)
        .filter(|id| id.len() == 36)
        .ok_or_else(|| anyhow!("Hue v2 configuration '{}' has no UUID", area_name))?
        .to_string();
    let discovered_zones = zones_from_v2_configuration(configuration)?;
    if discovered_zones.is_empty() {
        return Err(anyhow!(
            "Hue v2 configuration '{}' has no channel positions",
            area_name
        ));
    }
    Ok(EntertainmentArea {
        legacy_group_id: area_id,
        name: area_name,
        configuration_id,
        zones: discovered_zones,
        certificate_sha256,
    })
}

/// Lists the Hue Entertainment Areas available on the configured Bridge.
/// This is configuration discovery only; it does not alter the active stream.
pub fn list_entertainment_areas(
    bridge_ip: &str,
    username: &str,
) -> Result<Vec<EntertainmentAreaSummary>> {
    let groups_url = format!("http://{}/api/{}/groups", bridge_ip, username);
    let groups_json: Value = ureq::get(&groups_url)
        .call()
        .context("Failed to query groups from Hue Bridge")?
        .into_json()?;
    entertainment_areas_from_groups(&groups_json)
}

fn entertainment_areas_from_groups(groups_json: &Value) -> Result<Vec<EntertainmentAreaSummary>> {
    let groups = groups_json
        .as_object()
        .ok_or_else(|| anyhow!("Hue Bridge groups response was not an object"))?;
    let mut areas = groups
        .iter()
        .filter(|(_, group)| group.get("type").and_then(Value::as_str) == Some("Entertainment"))
        .map(|(id, group)| EntertainmentAreaSummary {
            id: id.clone(),
            name: group
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or("Unnamed")
                .to_string(),
        })
        .collect::<Vec<_>>();
    areas.sort_by(|left, right| left.id.cmp(&right.id));
    Ok(areas)
}

/// Fetches V2 configurations through a bridge-specific certificate pin.
/// Pairing accepts the first certificate only after the user proves physical access via pushlink.
fn fetch_v2_configurations(
    bridge_ip: &str,
    username: &str,
    expected_certificate_sha256: Option<&str>,
) -> Result<(Value, String)> {
    let address = format!("{}:443", bridge_ip)
        .to_socket_addrs()
        .context("Failed to resolve Hue Bridge address")?
        .next()
        .ok_or_else(|| anyhow!("Hue Bridge address did not resolve"))?;
    let tcp = TcpStream::connect_timeout(&address, Duration::from_secs(5))
        .context("Failed to connect to Hue Bridge HTTPS endpoint")?;
    let mut builder = SslConnector::builder(SslMethod::tls())?;
    // The local bridge presents a self-signed certificate. Verification below pins its SHA-256 DER fingerprint.
    builder.set_verify(SslVerifyMode::NONE);
    let connector = builder.build();
    let mut stream = connector
        .connect(bridge_ip, tcp)
        .context("Hue Bridge TLS handshake failed")?;
    let certificate = stream
        .ssl()
        .peer_certificate()
        .ok_or_else(|| anyhow!("Hue Bridge did not present a TLS certificate"))?;
    let digest = certificate.digest(MessageDigest::sha256())?;
    let mut certificate_sha256 = String::with_capacity(digest.len() * 2);
    for byte in digest.as_ref() {
        write!(&mut certificate_sha256, "{byte:02x}")?;
    }
    if let Some(expected) = expected_certificate_sha256 {
        if !expected.eq_ignore_ascii_case(&certificate_sha256) {
            return Err(anyhow!(
                "Hue Bridge certificate fingerprint changed; refusing V2 request"
            ));
        }
    }

    write!(
        stream,
        "GET /clip/v2/resource/entertainment_configuration HTTP/1.1\r\nHost: {bridge_ip}\r\nhue-application-key: {username}\r\nConnection: close\r\n\r\n"
    )?;
    let mut response = Vec::new();
    stream.read_to_end(&mut response)?;
    let header_end = response
        .windows(4)
        .position(|bytes| bytes == b"\r\n\r\n")
        .ok_or_else(|| anyhow!("Malformed Hue Bridge HTTPS response"))?;
    let headers = std::str::from_utf8(&response[..header_end])
        .context("Hue Bridge HTTPS response headers were not UTF-8")?;
    if !headers.starts_with("HTTP/1.1 200") {
        return Err(anyhow!(
            "Hue Bridge V2 request failed: {}",
            headers.lines().next().unwrap_or("unknown status")
        ));
    }
    let body = decode_http_body(headers, &response[header_end + 4..])?;
    Ok((serde_json::from_slice(&body)?, certificate_sha256))
}

fn decode_http_body(headers: &str, body: &[u8]) -> Result<Vec<u8>> {
    if !headers.lines().any(|line| {
        line.split_once(':').is_some_and(|(name, value)| {
            name.eq_ignore_ascii_case("transfer-encoding")
                && value.trim().eq_ignore_ascii_case("chunked")
        })
    }) {
        return Ok(body.to_vec());
    }

    let mut remaining = body;
    let mut decoded = Vec::new();
    loop {
        let line_end = remaining
            .windows(2)
            .position(|bytes| bytes == b"\r\n")
            .ok_or_else(|| anyhow!("Malformed chunked Hue Bridge response"))?;
        let size = std::str::from_utf8(&remaining[..line_end])?
            .split(';')
            .next()
            .ok_or_else(|| anyhow!("Missing Hue Bridge chunk size"))?
            .trim();
        let size = usize::from_str_radix(size, 16).context("Invalid Hue Bridge chunk size")?;
        remaining = &remaining[line_end + 2..];
        if size == 0 {
            return Ok(decoded);
        }
        let chunk_end = size
            .checked_add(2)
            .filter(|end| *end <= remaining.len())
            .ok_or_else(|| anyhow!("Truncated Hue Bridge response chunk"))?;
        if &remaining[size..chunk_end] != b"\r\n" {
            return Err(anyhow!("Malformed Hue Bridge response chunk terminator"));
        }
        decoded.extend_from_slice(&remaining[..size]);
        remaining = &remaining[chunk_end..];
    }
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
    fn decodes_chunked_v2_response_body() {
        let body = decode_http_body(
            "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked",
            b"6\r\n{\"data\r\n5\r\n\":[]}\r\n0\r\n\r\n",
        )
        .unwrap();
        assert_eq!(body, br#"{"data":[]}"#);
    }

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

    #[test]
    fn lists_only_entertainment_areas_in_stable_order() {
        let groups = serde_json::json!({
            "4": {"type": "Room", "name": "Living room"},
            "9": {"type": "Entertainment", "name": "Cinema"},
            "2": {"type": "Entertainment", "name": "TV area"}
        });
        let areas = entertainment_areas_from_groups(&groups).unwrap();
        assert_eq!(
            areas
                .iter()
                .map(|area| (area.id.as_str(), area.name.as_str()))
                .collect::<Vec<_>>(),
            vec![("2", "TV area"), ("9", "Cinema")]
        );
    }
}
