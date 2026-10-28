pub mod discovery;
pub mod dtls;
pub mod events;
pub mod output;
pub mod reconnect;
pub mod stream;
pub mod tls;

use anyhow::{anyhow, Context, Result};
use serde_json::Value;
use std::io::{Read, Write};
use std::net::{Shutdown, TcpStream};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::thread;
use std::time::{Duration, Instant};

pub use discovery::discover_bridge;
pub use dtls::HueDtlsClient;
pub use stream::HueStreamPacketBuilder;
pub use tls::{connect_bridge_tls, peer_bridge_id, BridgeTrust};

#[derive(Debug, Clone)]
pub struct EntertainmentArea {
    pub name: String,
    pub configuration_id: String,
    pub zones: Vec<crate::config::LightZone>,
    pub certificate_sha256: String,
    pub bridge_id: String,
}

#[derive(Debug, Clone, serde::Serialize, utoipa::ToSchema)]
pub struct EntertainmentAreaSummary {
    pub id: String,
    pub name: String,
}

pub struct PairedBridge {
    pub username: String,
    pub clientkey: String,
    pub area: Option<EntertainmentArea>,
    pub bridge_id: String,
    pub certificate_sha256: String,
}

/// Pair over CA-verified HTTPS after validating the bridge identity, never falling back to HTTP.
pub fn pair_bridge(bridge_ip: &str, timeout_secs: u64) -> Result<PairedBridge> {
    let identity = raw_request(
        bridge_ip,
        "",
        "GET",
        "/api/0/config",
        None,
        BridgeTrust::default(),
    )?;
    let config: Value = serde_json::from_slice(&identity.body)?;
    if !config
        .get("bridgeid")
        .and_then(Value::as_str)
        .is_some_and(|id| id.eq_ignore_ascii_case(&identity.bridge_id))
    {
        return Err(anyhow!(
            "Hue Bridge configuration and certificate identities do not match"
        ));
    }
    let trust = BridgeTrust {
        bridge_id: Some(&identity.bridge_id),
        certificate_sha256: None,
    };
    let payload = serde_json::json!({ "devicetype": "lg-hue-sync#tv", "generateclientkey": true });
    println!("Press the round button on your Philips Hue Bridge to pair.");
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(timeout_secs) {
        let (response, _) = v2_request(bridge_ip, "", "POST", "/api", Some(&payload), trust)?;
        if let Some(success) = response
            .as_array()
            .and_then(|items| items.first())
            .and_then(|item| item.get("success"))
        {
            let username = success
                .get("username")
                .and_then(Value::as_str)
                .filter(|key| !key.is_empty())
                .ok_or_else(|| anyhow!("Hue pairing returned no application key"))?;
            let clientkey = success
                .get("clientkey")
                .and_then(Value::as_str)
                .filter(|key| key.len() == 32 && key.bytes().all(|b| b.is_ascii_hexdigit()))
                .ok_or_else(|| anyhow!("Hue pairing returned an invalid client key"))?;
            // The client key is returned only once; layout discovery must not discard it.
            let area = match sync_entertainment_areas(bridge_ip, username, None, trust) {
                Ok(area) => Some(area),
                Err(_) => {
                    tracing::warn!("Hue paired; Entertainment area selection is required when the bridge is reachable");
                    None
                }
            };
            return Ok(PairedBridge {
                username: username.to_owned(),
                clientkey: clientkey.to_owned(),
                area,
                bridge_id: identity.bridge_id,
                certificate_sha256: identity.certificate_sha256,
            });
        }
        if response.pointer("/0/error/type").and_then(Value::as_i64) != Some(101) {
            return Err(anyhow!("Hue Bridge rejected pairing"));
        }
        thread::sleep(Duration::from_secs(1));
    }
    Err(anyhow!("Timed out waiting for Hue Bridge button press"))
}

/// Queries the Hue Bridge for Entertainment Areas and translates 3D light coordinates into screen sampling zones.
/// If `target_area` is specified, selects by ID or name; otherwise requires exactly one area.
pub fn sync_entertainment_areas(
    bridge_ip: &str,
    username: &str,
    target_area: Option<&str>,
    trust: BridgeTrust<'_>,
) -> Result<EntertainmentArea> {
    let response = raw_request(
        bridge_ip,
        username,
        "GET",
        "/clip/v2/resource/entertainment_configuration",
        None,
        trust,
    )?;
    let configurations: Value = serde_json::from_slice(&response.body)?;
    let certificate_sha256 = response.certificate_sha256;
    let bridge_id = response.bridge_id;
    let trust = BridgeTrust {
        bridge_id: Some(&bridge_id),
        certificate_sha256: None,
    };
    let configuration_data = configurations
        .get("data")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("Hue v2 Entertainment configurations response has no data"))?;
    let (area_name, configuration) = select_entertainment_area(configuration_data, target_area)?;
    let configuration_id = configuration
        .get("id")
        .and_then(Value::as_str)
        .filter(|id| valid_configuration_id(id))
        .ok_or_else(|| anyhow!("Hue v2 configuration '{}' has no UUID", area_name))?
        .to_string();
    let (entertainment_services, _) = fetch_v2_resource(
        bridge_ip,
        username,
        "/clip/v2/resource/entertainment",
        trust,
    )?;
    let (devices, _) = fetch_v2_resource(bridge_ip, username, "/clip/v2/resource/device", trust)?;
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
        bridge_id,
    })
}

/// Lists the Hue Entertainment Areas available on the configured Bridge.
/// This is configuration discovery only; it does not alter the active stream.
pub fn list_entertainment_areas(
    bridge_ip: &str,
    username: &str,
    trust: BridgeTrust<'_>,
) -> Result<Vec<EntertainmentAreaSummary>> {
    let (configurations, _) = fetch_v2_configurations(bridge_ip, username, trust)?;
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

/// Fetches V2 configurations through the verified bridge identity.
fn fetch_v2_configurations(
    bridge_ip: &str,
    username: &str,
    trust: BridgeTrust<'_>,
) -> Result<(Value, String)> {
    fetch_v2_resource(
        bridge_ip,
        username,
        "/clip/v2/resource/entertainment_configuration",
        trust,
    )
}

fn fetch_v2_resource(
    bridge_ip: &str,
    username: &str,
    path: &str,
    trust: BridgeTrust<'_>,
) -> Result<(Value, String)> {
    v2_request(bridge_ip, username, "GET", path, None, trust)
}

fn v2_request(
    bridge_ip: &str,
    username: &str,
    method: &str,
    path: &str,
    payload: Option<&Value>,
    trust: BridgeTrust<'_>,
) -> Result<(Value, String)> {
    let response = raw_request(bridge_ip, username, method, path, payload, trust)?;
    let json: Value = serde_json::from_slice(&response.body)?;
    if json
        .get("errors")
        .and_then(Value::as_array)
        .is_some_and(|errors| !errors.is_empty())
    {
        return Err(anyhow!("Hue Bridge rejected the API request"));
    }
    Ok((json, response.certificate_sha256))
}

struct HttpResponse {
    headers: String,
    body: Vec<u8>,
    certificate_sha256: String,
    bridge_id: String,
}

fn raw_request(
    bridge_ip: &str,
    username: &str,
    method: &str,
    path: &str,
    payload: Option<&Value>,
    trust: BridgeTrust<'_>,
) -> Result<HttpResponse> {
    if !username.is_empty() && !trust.known() {
        return Err(anyhow!(
            "Hue Bridge has no saved identity; pair it explicitly before sending credentials"
        ));
    }
    if (!username.is_empty() && !valid_application_key(username))
        || !path.starts_with('/')
        || !path.bytes().all(|b| b.is_ascii_graphic())
        || !matches!(method, "GET" | "POST" | "PUT")
    {
        return Err(anyhow!("Invalid Hue HTTP request parameters"));
    }
    let mut stream = connect_bridge_tls(bridge_ip, trust)?;
    let certificate_sha256 = tls::peer_certificate_sha256(&stream)?;
    let bridge_id = peer_bridge_id(&stream)?;
    let deadline = Instant::now() + Duration::from_secs(15);
    let _deadline_guard = SocketDeadline::new(stream.get_ref(), deadline)?;
    let body = payload.map(serde_json::to_vec).transpose()?;
    write!(
        stream,
        "{method} {path} HTTP/1.1\r\nHost: {bridge_ip}\r\nConnection: close\r\n"
    )?;
    if !username.is_empty() {
        write!(stream, "hue-application-key: {username}\r\n")?;
    }
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
    let response = read_bounded_response(
        |buffer, remaining| {
            stream.get_ref().set_read_timeout(Some(remaining))?;
            stream.read(buffer)
        },
        deadline,
    )?;
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
    check_status(status)?;
    let body = decode_http_body(headers, &response[header_end + 4..])?;
    Ok(HttpResponse {
        headers: headers.to_owned(),
        body,
        certificate_sha256,
        bridge_id,
    })
}

fn check_status(status: u16) -> Result<()> {
    match status {
        200..=299 => Ok(()),
        401 | 403 => {
            Err(events::SessionInvalid("Hue authorization rejected; re-pair the bridge").into())
        }
        404 => Err(events::SessionInvalid(
            "Hue resource no longer exists; reselect the Entertainment area after migration",
        )
        .into()),
        429 | 503 => Err(anyhow!("Hue Bridge is busy ({status}); retry with backoff")),
        _ => Err(anyhow!(
            "Hue Bridge HTTPS request failed with status {status}"
        )),
    }
}

/// The application ID is a response header, not the application key and not a JSON body field.
pub fn fetch_application_id(
    bridge_ip: &str,
    username: &str,
    trust: BridgeTrust<'_>,
) -> Result<String> {
    let response = raw_request(bridge_ip, username, "GET", "/auth/v1", None, trust)?;
    application_id_header(&response.headers)
}

fn application_id_header(headers: &str) -> Result<String> {
    let ids = headers
        .lines()
        .filter_map(|line| line.split_once(':'))
        .filter(|(name, _)| name.eq_ignore_ascii_case("hue-application-id"))
        .map(|(_, value)| value.trim())
        .collect::<Vec<_>>();
    match ids.as_slice() {
        [id] if valid_configuration_id(id) => Ok(id.to_ascii_lowercase()),
        _ => Err(anyhow!(
            "Hue authentication response has no valid application ID"
        )),
    }
}

pub fn valid_configuration_id(id: &str) -> bool {
    id.len() == 36
        && id.bytes().enumerate().all(|(index, byte)| {
            if matches!(index, 8 | 13 | 18 | 23) {
                byte == b'-'
            } else {
                byte.is_ascii_hexdigit()
            }
        })
}

pub fn valid_application_key(key: &str) -> bool {
    !key.is_empty() && key.bytes().all(|byte| byte.is_ascii_graphic())
}

struct SocketDeadline {
    cancel: mpsc::Sender<()>,
    watchdog: Option<thread::JoinHandle<()>>,
}

impl SocketDeadline {
    fn new(stream: &TcpStream, deadline: Instant) -> std::io::Result<Self> {
        let stream = stream.try_clone()?;
        let (cancel, receiver) = mpsc::channel();
        // ponytail: one watchdog thread per request; use nonblocking I/O if request rate grows.
        let watchdog = thread::spawn(move || {
            if receiver.recv_timeout(deadline.saturating_duration_since(Instant::now()))
                == Err(RecvTimeoutError::Timeout)
            {
                let _ = stream.shutdown(Shutdown::Both);
            }
        });
        Ok(Self {
            cancel,
            watchdog: Some(watchdog),
        })
    }
}

impl Drop for SocketDeadline {
    fn drop(&mut self) {
        let _ = self.cancel.send(());
        if let Some(watchdog) = self.watchdog.take() {
            let _ = watchdog.join();
        }
    }
}

fn read_bounded_response(
    mut read: impl FnMut(&mut [u8], Duration) -> std::io::Result<usize>,
    deadline: Instant,
) -> Result<Vec<u8>> {
    const MAX_RESPONSE_BYTES: usize = 4 * 1024 * 1024;
    let mut response = Vec::new();
    let mut buffer = [0; 8192];
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(anyhow!("Hue Bridge HTTPS response timed out"));
        }
        let count = read(&mut buffer, remaining)?;
        if count == 0 {
            return Ok(response);
        }
        if response.len() + count > MAX_RESPONSE_BYTES {
            return Err(anyhow!("Hue Bridge HTTPS response exceeded 4 MiB"));
        }
        response.extend_from_slice(&buffer[..count]);
    }
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
            zone.hue_positions = configuration
                .pointer("/locations/service_locations")
                .and_then(Value::as_array)
                .and_then(|locations| {
                    locations.iter().find(|location| {
                        service_id.is_some()
                            && location.pointer("/service/rid").and_then(Value::as_str)
                                == service_id
                    })
                })
                .and_then(|location| location.get("positions"))
                .and_then(Value::as_array)
                .and_then(|positions| {
                    positions
                        .iter()
                        .map(|position| {
                            let axis = |name| {
                                position
                                    .get(name)
                                    .and_then(Value::as_f64)
                                    .map(|value| value as f32)
                                    .filter(|value| value.is_finite())
                            };
                            Some([axis("x")?, axis("y")?, axis("z")?])
                        })
                        .collect::<Option<Vec<_>>>()
                })
                .unwrap_or_default();
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
    fn v2_deadline_closes_stalled_connection() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let client = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let (mut server, _) = listener.accept().unwrap();
        let _guard =
            SocketDeadline::new(&client, Instant::now() + Duration::from_millis(50)).unwrap();
        server
            .set_read_timeout(Some(Duration::from_secs(1)))
            .unwrap();
        assert_eq!(server.read(&mut [0; 1]).unwrap(), 0);
    }

    #[test]
    fn bounded_v2_response_preserves_small_replies_and_rejects_stalls_and_oversize() {
        let mut valid = std::io::Cursor::new(b"HTTP/1.1 200 OK\r\n\r\n{}".as_slice());
        let response = read_bounded_response(
            |buffer, _| valid.read(buffer),
            Instant::now() + Duration::from_secs(1),
        )
        .unwrap();
        assert_eq!(response, b"HTTP/1.1 200 OK\r\n\r\n{}");

        let mut oversized = std::io::Cursor::new(vec![b'x'; 4 * 1024 * 1024 + 1]);
        let error = read_bounded_response(
            |buffer, _| oversized.read(buffer),
            Instant::now() + Duration::from_secs(1),
        )
        .unwrap_err();
        assert!(error.to_string().contains("exceeded 4 MiB"));

        let error = read_bounded_response(
            |_, _| panic!("expired response must not be read"),
            Instant::now() - Duration::from_secs(1),
        )
        .unwrap_err();
        assert!(error.to_string().contains("timed out"));
    }

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

/// Changes only our own stream; another application's stream is never stopped or taken over.
pub fn set_stream_active(
    bridge_ip: &str,
    username: &str,
    configuration_id: &str,
    trust: BridgeTrust<'_>,
    active: bool,
) -> Result<()> {
    let state = get_stream_state(bridge_ip, username, configuration_id, trust)?;
    if !stream_action_needed(&state, active)? {
        return Ok(());
    }
    let path = format!("/clip/v2/resource/entertainment_configuration/{configuration_id}");
    let _ = v2_request(
        bridge_ip,
        username,
        "PUT",
        &path,
        Some(&serde_json::json!({ "action": if active { "start" } else { "stop" } })),
        trust,
    )?;
    if active && !get_stream_state(bridge_ip, username, configuration_id, trust)?.active {
        return Err(anyhow!(
            "Hue did not grant stream ownership to this application"
        ));
    }
    Ok(())
}

fn stream_action_needed(state: &StreamState, active: bool) -> Result<bool> {
    if state.busy && active {
        return Err(anyhow!(
            "Hue Entertainment area is in use by another application"
        ));
    }
    Ok(!state.busy && state.active != active)
}

#[derive(Debug, Clone, PartialEq)]
pub struct StreamState {
    /// Active only when the bridge identifies this application as the streamer.
    pub active: bool,
    pub busy: bool,
    pub application_id: String,
    pub active_streamer: Option<String>,
    pub brightness: f32,
    pub smoothing_factor: f32,
}

pub fn parse_stream_state(configuration: &Value, application_id: &str) -> Result<StreamState> {
    if !valid_configuration_id(application_id) {
        return Err(anyhow!("Invalid Hue application ID"));
    }
    let streaming = match configuration.get("status").and_then(Value::as_str) {
        Some("active") => true,
        Some("inactive") => false,
        _ => {
            return Err(anyhow!(
                "Hue Entertainment configuration has no valid status"
            ))
        }
    };
    let active_streamer = streaming
        .then(|| {
            configuration
                .pointer("/active_streamer/rid")
                .and_then(Value::as_str)
                .map(str::to_owned)
        })
        .flatten();
    let ours = active_streamer
        .as_ref()
        .is_some_and(|id| id.eq_ignore_ascii_case(application_id))
        && configuration
            .pointer("/active_streamer/rtype")
            .and_then(Value::as_str)
            == Some("auth_v1");
    Ok(StreamState {
        active: streaming && ours,
        busy: streaming && !ours,
        application_id: application_id.to_owned(),
        active_streamer,
        brightness: 1.0,
        smoothing_factor: 0.35,
    })
}

pub fn fetch_stream_configuration(
    bridge_ip: &str,
    username: &str,
    configuration_id: &str,
    trust: BridgeTrust<'_>,
) -> Result<Value> {
    if !valid_configuration_id(configuration_id) {
        return Err(anyhow!("Invalid Hue Entertainment configuration UUID"));
    }
    let path = format!("/clip/v2/resource/entertainment_configuration/{configuration_id}");
    let (json, _) = fetch_v2_resource(bridge_ip, username, &path, trust)?;
    json.get("data")
        .and_then(Value::as_array)
        .and_then(|items| {
            items
                .iter()
                .find(|item| item.get("id").and_then(Value::as_str) == Some(configuration_id))
        })
        .cloned()
        .ok_or_else(|| anyhow!("Hue V2 configuration response has no selected area"))
}

pub fn get_stream_state(
    bridge_ip: &str,
    username: &str,
    configuration_id: &str,
    trust: BridgeTrust<'_>,
) -> Result<StreamState> {
    let application_id = fetch_application_id(bridge_ip, username, trust)?;
    let configuration = fetch_stream_configuration(bridge_ip, username, configuration_id, trust)?;
    parse_stream_state(&configuration, &application_id)
}

#[allow(dead_code)]
pub fn get_stream_status(
    bridge_ip: &str,
    username: &str,
    configuration_id: &str,
    trust: BridgeTrust<'_>,
) -> Result<bool> {
    get_stream_state(bridge_ip, username, configuration_id, trust).map(|s| s.active)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ownership_never_treats_another_or_unknown_streamer_as_ours() {
        let app = "12345678-1234-1234-1234-123456789abc";
        let other = "12345678-1234-1234-1234-123456789abd";
        let config = serde_json::json!({"status":"active", "active_streamer":{"rtype":"auth_v1", "rid":app}});
        let state = parse_stream_state(&config, app).unwrap();
        assert!(state.active && !state.busy);
        assert!(!stream_action_needed(&state, true).unwrap());
        assert!(stream_action_needed(&state, false).unwrap());
        let state = parse_stream_state(&config, other).unwrap();
        assert!(!state.active && state.busy);
        assert!(stream_action_needed(&state, true).is_err());
        assert!(!stream_action_needed(&state, false).unwrap());
        let state = parse_stream_state(&serde_json::json!({"status":"active"}), app).unwrap();
        assert!(!state.active && state.busy);
        let state = parse_stream_state(&serde_json::json!({"status":"inactive", "active_streamer":{"rtype":"auth_v1", "rid":other}}), app).unwrap();
        assert!(!state.active && !state.busy && state.active_streamer.is_none());
        assert!(stream_action_needed(&state, true).unwrap());
        assert!(!stream_action_needed(&state, false).unwrap());
        assert!(parse_stream_state(&serde_json::json!({}), app).is_err());
        assert!(valid_configuration_id(app));
        assert!(!valid_configuration_id(
            "xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx"
        ));
        assert!(!valid_configuration_id(
            "1234567811234-1234-1234-123456789abc"
        ));
    }

    #[test]
    fn authentication_headers_and_http_failures_are_unambiguous() {
        let app = "12345678-1234-1234-1234-123456789abc";
        assert_eq!(
            application_id_header(&format!("HTTP/1.1 200 OK\r\nHue-Application-Id: {app}\r\n"))
                .unwrap(),
            app
        );
        assert!(application_id_header("HTTP/1.1 200 OK\r\n").is_err());
        assert!(application_id_header("hue-application-id: bad").is_err());
        assert!(application_id_header(&format!(
            "hue-application-id: {app}\r\nhue-application-id: {app}"
        ))
        .is_err());
        assert!(valid_application_key("opaque-key-123"));
        for key in ["", "key\r\ninjected: true", "key\0", "key with spaces"] {
            assert!(!valid_application_key(key));
        }
        assert!(check_status(204).is_ok());
        for status in [401, 403, 404] {
            assert!(check_status(status)
                .unwrap_err()
                .is::<events::SessionInvalid>());
        }
        assert!(check_status(401)
            .unwrap_err()
            .to_string()
            .contains("re-pair"));
        assert!(check_status(404)
            .unwrap_err()
            .to_string()
            .contains("reselect"));
        assert!(check_status(429)
            .unwrap_err()
            .to_string()
            .contains("backoff"));
    }

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
    fn preview_layout_preserves_hue_points_without_changing_sampling() {
        let mut configuration = serde_json::json!({
            "channels": [{"channel_id": 4, "position": {"x": 0.8, "y": -0.8, "z": 0.0},
                "members": [{"service": {"rid": "strip"}, "index": 0}]}],
            "locations": {"service_locations": [{"service": {"rid": "strip"}, "positions": [
                {"x": 1.0, "y": -0.7, "z": 0.04}, {"x": 1.0, "y": 0.65, "z": 0.04}
            ]}]}
        });
        let zones = zones_from_v2_configuration(&configuration, None, None).unwrap();
        assert_eq!(
            zones[0].hue_positions,
            vec![[1.0, -0.7, 0.04], [1.0, 0.65, 0.04]]
        );
        let expected = crate::config::LightZone::from_3d_position(4, "", [0.8, -0.8, 0.0]);
        assert_eq!(
            (
                zones[0].x_min,
                zones[0].x_max,
                zones[0].y_min,
                zones[0].y_max
            ),
            (
                expected.x_min,
                expected.x_max,
                expected.y_min,
                expected.y_max
            )
        );
        configuration["locations"]["service_locations"][0]["positions"][1]["z"] =
            serde_json::Value::Null;
        assert!(
            zones_from_v2_configuration(&configuration, None, None).unwrap()[0]
                .hue_positions
                .is_empty()
        );
        configuration.as_object_mut().unwrap().remove("locations");
        assert!(
            zones_from_v2_configuration(&configuration, None, None).unwrap()[0]
                .hue_positions
                .is_empty()
        );
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
