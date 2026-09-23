use anyhow::{anyhow, Context, Result};
use serde_json::Value;
use std::time::Duration;
use tokio::process::Command;
use tokio::time::timeout;

const POWER_STATE_URI: &str = "luna://com.webos.service.tvpower/power/getPowerState";

pub async fn is_active() -> Result<Option<bool>> {
    let output = timeout(
        Duration::from_millis(1500),
        Command::new("/usr/bin/luna-send")
            .args(["-n", "1", "-f", "-w", "1000", POWER_STATE_URI, "{}"])
            .output(),
    )
    .await
    .context("timed out querying webOS TV power state")??;

    if !output.status.success() {
        return Err(anyhow!("luna-send exited with {}", output.status));
    }

    let response: Value = serde_json::from_slice(&output.stdout)
        .context("webOS TV power query returned invalid JSON")?;
    if response.get("returnValue") == Some(&Value::Bool(false)) {
        return Err(anyhow!("webOS TV power query failed"));
    }

    Ok(response
        .get("state")
        .and_then(Value::as_str)
        .and_then(parse_state))
}

fn parse_state(state: &str) -> Option<bool> {
    match state.trim().to_ascii_lowercase().as_str() {
        "active" | "on" => Some(true),
        "active standby" | "standby" | "suspend" | "screen off" | "off" | "screen saver" => {
            Some(false)
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_known_webos_power_states_and_preserves_unknown() {
        assert_eq!(parse_state("Active"), Some(true));
        assert_eq!(parse_state("on"), Some(true));
        for state in ["Active Standby", "Suspend", "Screen Off", "off"] {
            assert_eq!(parse_state(state), Some(false));
        }
        assert_eq!(parse_state("unrecognized"), None);
    }
}
