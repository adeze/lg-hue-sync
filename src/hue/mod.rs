pub mod dtls;
pub mod stream;

use anyhow::{Context, Result};
use tracing::info;

pub use dtls::HueDtlsClient;
pub use stream::HueStreamPacketBuilder;

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
