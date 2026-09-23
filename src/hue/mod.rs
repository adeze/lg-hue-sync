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
    let (configurations, certificate_sha256) =
        fetch_v2_configurations(bridge_ip, username, expected_certificate_sha256)?;
    let configuration_data = configurations
        .get("data")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("Hue v2 Entertainment configurations response has no data"))?;
    let (area_name, configuration) = select_entertainment_area(configuration_data, target_area)?;
    let configuration_id = configuration
        .get("id")
        .and_then(Value::as_str)
        .filter(|id| id.len() == 36)
        .ok_or_else(|| anyhow!("Hue v2 configuration '{}' has no UUID", area_name))?
        .to_string();
    let (entertainment_services, _) = fetch_v2_resource(
        bridge_ip,
        username,
        "/clip/v2/resource/entertainment",
        Some(&certificate_sha256),
    )?;
    let (devices, _) = fetch_v2_resource(
        bridge_ip,
        username,
        "/clip/v2/resource/device",
        Some(&certificate_sha256),
    )?;
    let discovered_zones = zones_from_v2_configuration(
        configuration,
        entertainment_services.get("data").and_then(Value::as_array),
        devices.get("data").and_then(Value::as_array),
    )?;
    if discovered_zones.is_empty() {
        return Err(anyhow!(
            "Hue v2 configuration '{}' has no channel positions",
            area_name
        ));
    }
    Ok(EntertainmentArea {
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
    expected_certificate_sha256: Option<&str>,
) -> Result<Vec<EntertainmentAreaSummary>> {
    let (configurations, _) =
        fetch_v2_configurations(bridge_ip, username, expected_certificate_sha256)?;
    let data = configurations
        .get("data")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("Hue v2 Entertainment configurations response has no data"))?;
    let mut areas = data
        .iter()
        .filter_map(|configuration| {
            Some(EntertainmentAreaSummary {
                id: configuration.get("id")?.as_str()?.to_string(),
                name: configuration
                    .pointer("/metadata/name")?
                    .as_str()?
                    .to_string(),
            })
        })
        .collect::<Vec<_>>();
    areas.sort_by(|left, right| left.name.cmp(&right.name).then(left.id.cmp(&right.id)));
    Ok(areas)
}

/// Fetches V2 configurations through a bridge-specific certificate pin.
/// Pairing accepts the first certificate only after the user proves physical access via pushlink.
fn fetch_v2_configurations(
    bridge_ip: &str,
    username: &str,
    expected_certificate_sha256: Option<&str>,
) -> Result<(Value, String)> {
    fetch_v2_resource(
        bridge_ip,
        username,
        "/clip/v2/resource/entertainment_configuration",
        expected_certificate_sha256,
    )
}

fn fetch_v2_resource(
    bridge_ip: &str,
    username: &str,
    path: &str,
    expected_certificate_sha256: Option<&str>,
) -> Result<(Value, String)> {
    v2_request(
        bridge_ip,
        username,
        "GET",
        path,
        None,
        expected_certificate_sha256,
    )
}

fn v2_request(
    bridge_ip: &str,
    username: &str,
    method: &str,
    path: &str,
    payload: Option<&Value>,
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

    let body = payload.map(serde_json::to_vec).transpose()?;
    write!(
        stream,
        "{method} {path} HTTP/1.1\r\nHost: {bridge_ip}\r\nhue-application-key: {username}\r\nConnection: close\r\n"
    )?;
    if let Some(body) = &body {
        write!(
            stream,
            "Content-Type: application/json\r\nContent-Length: {}\r\n",
            body.len()
        )?;
    }
    stream.write_all(b"\r\n")?;
    if let Some(body) = body {
        stream.write_all(&body)?;
    }
    let mut response = Vec::new();
    stream.read_to_end(&mut response)?;
    let header_end = response
        .windows(4)
        .position(|bytes| bytes == b"\r\n\r\n")
        .ok_or_else(|| anyhow!("Malformed Hue Bridge HTTPS response"))?;
    let headers = std::str::from_utf8(&response[..header_end])
        .context("Hue Bridge HTTPS response headers were not UTF-8")?;
    let status = headers
        .split_whitespace()
        .nth(1)
        .and_then(|value| value.parse::<u16>().ok())
        .unwrap_or_default();
    if !(200..300).contains(&status) {
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
    configurations: &'a [Value],
    target: Option<&str>,
) -> Result<(String, &'a Value)> {
    let selected = target
        .and_then(|target| {
            configurations.iter().find(|configuration| {
                configuration.get("id").and_then(Value::as_str) == Some(target)
                    || configuration
                        .pointer("/metadata/name")
                        .and_then(Value::as_str)
                        .is_some_and(|name| name.eq_ignore_ascii_case(target))
            })
        })
        .or_else(|| (target.is_none() && configurations.len() == 1).then(|| &configurations[0]))
        .ok_or_else(|| {
            anyhow!("Select an explicit Hue Entertainment Area when more than one exists")
        })?;
    let name = selected
        .pointer("/metadata/name")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("Hue v2 configuration has no metadata name"))?;
    Ok((name.to_string(), selected))
}

fn zones_from_v2_configuration(
    configuration: &Value,
    entertainment_services: Option<&Vec<Value>>,
    devices: Option<&Vec<Value>>,
) -> Result<Vec<crate::config::LightZone>> {
    let channels = configuration
        .get("channels")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("Hue v2 configuration has no channels"))?;
    let member_counts = channels
        .iter()
        .flat_map(|channel| {
            channel
                .get("members")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
        })
        .filter_map(|member| {
            Some((
                member.pointer("/service/rid")?.as_str()?.to_string(),
                u8::try_from(member.get("index")?.as_u64()?.saturating_add(1)).ok()?,
            ))
        })
        .fold(
            std::collections::BTreeMap::<String, u8>::new(),
            |mut counts, (id, count)| {
                counts
                    .entry(id)
                    .and_modify(|existing| *existing = (*existing).max(count))
                    .or_insert(count);
                counts
            },
        );
    let mut zones = channels
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
            let members = channel
                .get("members")
                .and_then(Value::as_array)
                .map(Vec::as_slice)
                .unwrap_or_default();
            let service_id = members
                .first()
                .and_then(|member| member.pointer("/service/rid"))
                .and_then(Value::as_str);
            let service = service_id.and_then(|id| {
                entertainment_services?
                    .iter()
                    .find(|service| service.get("id").and_then(Value::as_str) == Some(id))
            });
            let device_id = service
                .and_then(|service| service.pointer("/owner/rid"))
                .and_then(Value::as_str);
            let device_name = device_id
                .and_then(|id| {
                    devices?
                        .iter()
                        .find(|device| device.get("id").and_then(Value::as_str) == Some(id))
                })
                .and_then(|device| device.pointer("/metadata/name"))
                .and_then(Value::as_str)
                .unwrap_or("Hue light");
            let mut segment_indices: Vec<u8> = members
                .iter()
                .filter(|member| {
                    member.pointer("/service/rid").and_then(Value::as_str) == service_id
                })
                .filter_map(|member| member.get("index").and_then(Value::as_u64))
                .filter_map(|index| u8::try_from(index).ok())
                .collect();
            segment_indices.sort_unstable();
            segment_indices.dedup();
            let segment_index = segment_indices.first().copied();
            let service_segment_count = service
                .and_then(|service| service.get("segments"))
                .and_then(Value::as_array)
                .and_then(|segments| u8::try_from(segments.len()).ok())
                .filter(|count| *count > 0);
            let segment_count = service_segment_count
                .into_iter()
                .chain(service_id.and_then(|id| member_counts.get(id).copied()))
                .max();
            let name = match (segment_indices.as_slice(), segment_count) {
                ([index], Some(count)) if count > 1 => {
                    format!("{device_name} · Segment {}", index + 1)
                }
                ([first, rest @ ..], Some(_)) if !rest.is_empty() => {
                    let contiguous = segment_indices
                        .windows(2)
                        .all(|pair| pair[1] == pair[0].saturating_add(1));
                    if contiguous {
                        format!(
                            "{device_name} · Segments {}–{}",
                            first + 1,
                            segment_indices.last().unwrap() + 1
                        )
                    } else {
                        format!(
                            "{device_name} · Segments {}",
                            segment_indices
                                .iter()
                                .map(|index| (index + 1).to_string())
                                .collect::<Vec<_>>()
                                .join(", ")
                        )
                    }
                }
                _ if service_id.is_some() => device_name.to_string(),
                _ => format!("Hue channel {channel_id}"),
            };
            let mut zone = crate::config::LightZone::from_3d_position(
                channel_id,
                &name,
                [coordinate("x")?, coordinate("y")?, coordinate("z")?],
            );
            zone.hue_device_id = device_id.map(ToOwned::to_owned);
            zone.hue_segment_index = segment_index;
            zone.hue_segment_count = segment_count;
            Ok(zone)
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
    fn v2_configuration_id_selects_the_configuration_directly() {
        let configurations = vec![serde_json::json!({
            "id": "12345678-1234-1234-1234-123456789abc",
            "metadata": { "name": "Cinema" },
            "channels": []
        })];

        let (name, configuration) = select_entertainment_area(
            &configurations,
            Some("12345678-1234-1234-1234-123456789abc"),
        )
        .unwrap();

        assert_eq!(name, "Cinema");
        assert_eq!(configuration["id"], "12345678-1234-1234-1234-123456789abc");
    }
}

/// Activates or deactivates the V2 Entertainment configuration through the pinned bridge API.
pub fn set_stream_active(
    bridge_ip: &str,
    username: &str,
    configuration_id: &str,
    expected_certificate_sha256: Option<&str>,
    active: bool,
) -> Result<()> {
    info!(
        "Sending V2 stream {} request to Hue Bridge...",
        if active { "START" } else { "STOP" },
    );
    let path = format!("/clip/v2/resource/entertainment_configuration/{configuration_id}");
    let (response, _) = v2_request(
        bridge_ip,
        username,
        "PUT",
        &path,
        Some(&serde_json::json!({ "action": if active { "start" } else { "stop" } })),
        expected_certificate_sha256,
    )?;
    if response
        .get("errors")
        .and_then(Value::as_array)
        .is_some_and(|errors| !errors.is_empty())
    {
        return Err(anyhow!("Hue Bridge rejected V2 stream action"));
    }
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

/// Queries the V2 configuration state. Hue does not expose a stream brightness or smoothing value.
pub fn get_stream_state(
    bridge_ip: &str,
    username: &str,
    configuration_id: &str,
    expected_certificate_sha256: Option<&str>,
) -> Result<StreamState> {
    let path = format!("/clip/v2/resource/entertainment_configuration/{configuration_id}");
    let (json, _) = fetch_v2_resource(bridge_ip, username, &path, expected_certificate_sha256)?;
    let configuration = json
        .get("data")
        .and_then(Value::as_array)
        .and_then(|items| items.first())
        .ok_or_else(|| anyhow!("Hue V2 configuration response has no data"))?;
    Ok(StreamState {
        active: configuration.get("status").and_then(Value::as_str) == Some("active"),
        brightness: 1.0,
        smoothing_factor: 0.35,
    })
}

/// Queries whether the entertainment area is currently active on the Hue Bridge
#[allow(dead_code)]
pub fn get_stream_status(
    bridge_ip: &str,
    username: &str,
    configuration_id: &str,
    expected_certificate_sha256: Option<&str>,
) -> Result<bool> {
    get_stream_state(
        bridge_ip,
        username,
        configuration_id,
        expected_certificate_sha256,
    )
    .map(|s| s.active)
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
        let zones = zones_from_v2_configuration(&configuration, None, None).unwrap();
        assert_eq!(
            zones.iter().map(|zone| zone.channel_id).collect::<Vec<_>>(),
            vec![2, 7]
        );
        assert!(zones[0].x_max < 0.5);
        assert!(zones[1].x_min > 0.5);
    }

    #[test]
    fn v2_gradient_channels_keep_device_and_segment_identity() {
        let configuration = serde_json::json!({
            "channels": [{
                "channel_id": 3,
                "position": {"x": 0.0, "y": -1.0, "z": 0.0},
                "members": [{"service": {"rid": "service-1"}, "index": 1}]
            }]
        });
        let services = vec![serde_json::json!({
            "id": "service-1", "owner": {"rid": "device-1"}, "segments": [{}, {}, {}]
        })];
        let devices = vec![serde_json::json!({
            "id": "device-1", "metadata": {"name": "Hue Flux"}
        })];
        let zones =
            zones_from_v2_configuration(&configuration, Some(&services), Some(&devices)).unwrap();
        assert_eq!(zones[0].name, "Hue Flux · Segment 2");
        assert_eq!(zones[0].hue_device_id.as_deref(), Some("device-1"));
        assert_eq!(zones[0].hue_segment_count, Some(3));
    }

    #[test]
    fn v2_configuration_maps_all_three_gradient_segments() {
        let configuration = serde_json::json!({
            "channels": [
                {"channel_id": 3, "position": {"x": 0.5, "y": 1.0, "z": 0.0}, "members": [{"service": {"rid": "service-1"}, "index": 0}]},
                {"channel_id": 4, "position": {"x": 0.5, "y": 1.0, "z": 0.0}, "members": [{"service": {"rid": "service-1"}, "index": 1}]},
                {"channel_id": 5, "position": {"x": 0.5, "y": 1.0, "z": 0.0}, "members": [{"service": {"rid": "service-1"}, "index": 2}]}
            ]
        });
        let services = vec![serde_json::json!({
            "id": "service-1", "owner": {"rid": "device-1"}, "segments": [{}, {}, {}]
        })];
        let devices = vec![serde_json::json!({
            "id": "device-1", "metadata": {"name": "Hue Flux"}
        })];

        let zones =
            zones_from_v2_configuration(&configuration, Some(&services), Some(&devices)).unwrap();

        assert_eq!(zones.len(), 3);
        assert_eq!(
            zones
                .iter()
                .map(|zone| zone.hue_segment_index.unwrap())
                .collect::<Vec<_>>(),
            vec![0, 1, 2]
        );
        assert_eq!(
            zones
                .iter()
                .map(|zone| zone.name.as_str())
                .collect::<Vec<_>>(),
            vec![
                "Hue Flux · Segment 1",
                "Hue Flux · Segment 2",
                "Hue Flux · Segment 3"
            ]
        );
    }

    #[test]
    fn v2_grouped_gradient_members_report_the_actual_channel_ranges() {
        let configuration = serde_json::json!({
            "channels": [
                {"channel_id": 3, "position": {"x": 0.8, "y": -0.8, "z": 0.0}, "members": [
                    {"service": {"rid": "service-1"}, "index": 0},
                    {"service": {"rid": "service-1"}, "index": 1}
                ]},
                {"channel_id": 4, "position": {"x": 0.4, "y": 0.8, "z": 0.0}, "members": [
                    {"service": {"rid": "service-1"}, "index": 2},
                    {"service": {"rid": "service-1"}, "index": 3},
                    {"service": {"rid": "service-1"}, "index": 4},
                    {"service": {"rid": "service-1"}, "index": 5},
                    {"service": {"rid": "service-1"}, "index": 6}
                ]}
            ]
        });
        let services = vec![serde_json::json!({
            "id": "service-1", "owner": {"rid": "device-1"}
        })];
        let devices = vec![serde_json::json!({
            "id": "device-1", "metadata": {"name": "Hue Flux ultra bright SL 1"}
        })];

        let zones =
            zones_from_v2_configuration(&configuration, Some(&services), Some(&devices)).unwrap();

        assert_eq!(
            zones.len(),
            2,
            "Hue output has two independently driven channels"
        );
        assert_eq!(zones[0].name, "Hue Flux ultra bright SL 1 · Segments 1–2");
        assert_eq!(zones[1].name, "Hue Flux ultra bright SL 1 · Segments 3–7");
        assert_eq!(zones[0].hue_segment_count, Some(7));
        assert_eq!(zones[1].hue_segment_count, Some(7));
    }
}
