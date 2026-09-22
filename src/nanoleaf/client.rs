use anyhow::{anyhow, Context, Result};
use serde::Deserialize;
use serde_json::Value;
use std::time::{Duration, Instant};
use tracing::info;

#[derive(Debug, Clone, Deserialize)]
#[allow(dead_code)]
pub struct PanelPosition {
    #[serde(rename = "panelId")]
    pub panel_id: u16,
    pub x: i32,
    pub y: i32,
    pub o: Option<i32>,
    #[serde(rename = "shapeType")]
    pub shape_type: Option<u32>,
}

#[derive(Debug, Clone, Deserialize)]
#[allow(dead_code)]
pub struct PanelLayout {
    #[serde(rename = "numPanels")]
    pub num_panels: u16,
    #[serde(rename = "sideLength")]
    pub side_length: Option<u32>,
    #[serde(rename = "positionData", default)]
    pub position_data: Vec<PanelPosition>,
}

/// Pairs with a Nanoleaf controller on the local network.
/// The user must hold the power button for 5-7 seconds until the LED starts flashing.
pub fn pair_nanoleaf(ip: &str, timeout_secs: u64) -> Result<(String, u16, Vec<u16>)> {
    let url = format!("http://{}:16021/api/v1/new", ip);

    println!("\n========================================================");
    println!(">>> ACTION REQUIRED FOR NANOLEAF 4D:");
    println!(">>> Press and hold the power button on your Nanoleaf controller");
    println!(">>> for 5-7 seconds until the status LED starts blinking!");
    println!("========================================================\n");

    let start = Instant::now();
    let mut auth_token = String::new();

    while start.elapsed() < Duration::from_secs(timeout_secs) {
        let resp = ureq::post(&url)
            .set("Content-Type", "application/json")
            .call();

        match resp {
            Ok(r) => {
                if let Ok(json) = r.into_json::<Value>() {
                    if let Some(token) = json.get("auth_token").and_then(|v| v.as_str()) {
                        auth_token = token.to_string();
                        println!("\n[+] Successfully paired with Nanoleaf controller!");
                        break;
                    }
                }
            }
            Err(ureq::Error::Status(403, _)) => {
                // Not in pairing mode yet
                print!(".");
                use std::io::Write;
                std::io::stdout().flush().ok();
            }
            Err(e) => {
                // Network glitch or connection refused
                tracing::debug!("Nanoleaf pairing poll error: {}", e);
            }
        }
        std::thread::sleep(Duration::from_secs(1));
    }

    if auth_token.is_empty() {
        return Err(anyhow!(
            "Timed out waiting for Nanoleaf pairing button press"
        ));
    }

    // Retrieve panel layout to discover segments
    let layout = get_panel_layout(ip, &auth_token)?;
    let mut panel_ids = Vec::new();
    for pos in &layout.position_data {
        panel_ids.push(pos.panel_id);
    }

    info!(
        "Discovered Nanoleaf layout: {} panels/segments, {} position data points",
        layout.num_panels,
        panel_ids.len()
    );

    Ok((auth_token, layout.num_panels, panel_ids))
}

/// Retrieves the physical panel/segment layout and IDs from the controller
pub fn get_panel_layout(ip: &str, auth_token: &str) -> Result<PanelLayout> {
    let url = format!(
        "http://{}:16021/api/v1/{}/panelLayout/layout",
        ip, auth_token
    );
    let resp = ureq::get(&url)
        .set("Content-Type", "application/json")
        .call()
        .with_context(|| format!("Failed to fetch panel layout from {}", url))?;

    let layout: PanelLayout = resp
        .into_json()
        .with_context(|| "Failed to parse Nanoleaf panel layout JSON")?;

    Ok(layout)
}

/// Activates external control mode (UDP v2 streaming) on the Nanoleaf device.
/// Returns the UDP port to stream to (typically 60222).
pub fn enable_external_control(ip: &str, auth_token: &str) -> Result<u16> {
    let url = format!("http://{}:16021/api/v1/{}/effects", ip, auth_token);
    let payload = serde_json::json!({
        "write": {
            "command": "display",
            "animType": "extControl",
            "extControlVersion": "v2"
        }
    });

    info!(
        "Enabling Nanoleaf external control (UDP v2) mode on {}...",
        ip
    );
    let resp = ureq::put(&url)
        .timeout(Duration::from_secs(3))
        .set("Content-Type", "application/json")
        .send_json(payload)
        .with_context(|| format!("Failed to enable extControl on {}", url))?;

    let default_port = 60222u16;
    // Some firmwares return UDP details in the response body or headers
    if let Ok(val) = resp.into_json::<Value>() {
        if let Some(port) = val.get("streamControlPort").and_then(|v| v.as_u64()) {
            info!("Nanoleaf specified streamControlPort: {}", port);
            return Ok(port as u16);
        }
    }

    Ok(default_port)
}
