use anyhow::{anyhow, Context, Result};
use serde_json::Value;

const POWER_STATE_URI: &str = "luna://com.webos.service.tvpower/power/getPowerState";

/// Panel state is unavailable in standby or for an unknown power state.
pub async fn display_enabled() -> Result<Option<bool>> {
    let response = crate::luna::query(POWER_STATE_URI, "{}").await?;
    Ok(parse_display_state(&response))
}

fn parse_display_state(response: &Value) -> Option<bool> {
    match response
        .get("state")?
        .as_str()?
        .trim()
        .to_ascii_lowercase()
        .as_str()
    {
        "active" | "on" => Some(true),
        "screen off" => Some(false),
        _ => None,
    }
}

fn display_uri(enabled: bool) -> &'static str {
    if enabled {
        "luna://com.webos.service.tvpower/power/turnOnScreen"
    } else {
        "luna://com.webos.service.tvpower/power/turnOffScreen"
    }
}

pub async fn set_display(enabled: bool) -> Result<Option<bool>> {
    let response = crate::luna::query(display_uri(enabled), "{}").await?;
    match parse_display_state(&response) {
        Some(state) => Ok(Some(state)),
        None => display_enabled().await,
    }
}

pub async fn is_active() -> Result<Option<bool>> {
    let response = crate::luna::query(POWER_STATE_URI, "{}")
        .await
        .context("webOS TV power query failed")?;
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
    fn panel_state_and_commands_are_separate_from_standby() {
        for (state, expected) in [
            ("Active", Some(true)),
            ("Screen Off", Some(false)),
            ("Active Standby", None),
            ("Suspend", None),
            ("unknown", None),
        ] {
            assert_eq!(
                parse_display_state(&serde_json::json!({"state": state})),
                expected
            );
        }
        assert_eq!(parse_display_state(&serde_json::json!({})), None);
        assert!(display_uri(true).ends_with("/turnOnScreen"));
        assert!(display_uri(false).ends_with("/turnOffScreen"));
    }

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
