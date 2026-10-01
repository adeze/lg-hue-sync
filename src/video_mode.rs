use serde_json::Value;
use std::time::Duration;
use tokio::{process::Command, time::timeout};

const VIDEO_STATUS_URI: &str = "luna://com.webos.service.videooutput/getStatus";
const PICTURE_SETTINGS_URI: &str = "luna://com.webos.settingsservice/getSystemSettings";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DynamicRange {
    Sdr,
    Hdr10,
    Hlg,
    DolbyVision,
    HdrOther,
    Unknown,
}

impl DynamicRange {
    pub fn label(self) -> &'static str {
        match self {
            Self::Sdr => "SDR",
            Self::Hdr10 => "HDR10",
            Self::Hlg => "HLG",
            Self::DolbyVision => "Dolby Vision",
            Self::HdrOther => "HDR (unspecified)",
            Self::Unknown => "Unknown",
        }
    }
}

fn parse_range(value: &str) -> DynamicRange {
    match value.trim().to_ascii_lowercase().as_str() {
        "sdr" | "none" => DynamicRange::Sdr,
        "hdr10" => DynamicRange::Hdr10,
        "hlg" => DynamicRange::Hlg,
        "dolbyvision" | "dolbyhdr" | "dolby vision" => DynamicRange::DolbyVision,
        "hdr" | "hdr10plus" | "hdr10+" => DynamicRange::HdrOther,
        _ => DynamicRange::Unknown,
    }
}

fn parse_video_status(value: &Value) -> DynamicRange {
    value
        .pointer("/video/0/videoInfo/hdrType")
        .and_then(Value::as_str)
        .map(parse_range)
        .unwrap_or(DynamicRange::Unknown)
}

fn parse_picture_settings(value: &Value) -> DynamicRange {
    value
        .pointer("/dimension/dynamicRange")
        .and_then(Value::as_str)
        .map(parse_range)
        .unwrap_or(DynamicRange::Unknown)
}

async fn luna_query(uri: &str, payload: &str) -> Option<Value> {
    let output = timeout(
        Duration::from_millis(1500),
        Command::new("/usr/bin/luna-send")
            .kill_on_drop(true)
            .args(["-n", "1", "-f", "-w", "1000", uri, payload])
            .output(),
    )
    .await
    .ok()?
    .ok()?;
    if !output.status.success() {
        return None;
    }
    let value: Value = serde_json::from_slice(&output.stdout).ok()?;
    if value.get("returnValue") == Some(&Value::Bool(false)) {
        return None;
    }
    Some(value)
}

/// TV output status is only a mode hint; it does not describe vtCapture pixels.
pub async fn current_dynamic_range() -> DynamicRange {
    let video = luna_query(VIDEO_STATUS_URI, "{}").await;
    let picture = luna_query(PICTURE_SETTINGS_URI, r#"{"category":"picture"}"#).await;
    let video = video
        .as_ref()
        .map(parse_video_status)
        .unwrap_or(DynamicRange::Unknown);
    let picture = picture
        .as_ref()
        .map(parse_picture_settings)
        .unwrap_or(DynamicRange::Unknown);
    reconcile(video, picture)
}

fn reconcile(video: DynamicRange, picture: DynamicRange) -> DynamicRange {
    match (video, picture) {
        (DynamicRange::Unknown, other) | (other, DynamicRange::Unknown) => other,
        (a, b) if a == b => a,
        _ => DynamicRange::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_reported_modes_without_promoting_generic_hdr_to_hdr10() {
        for (reported, expected) in [
            ("sdr", DynamicRange::Sdr),
            ("HDR10", DynamicRange::Hdr10),
            ("HLG", DynamicRange::Hlg),
            ("DolbyVision", DynamicRange::DolbyVision),
            ("dolbyHdr", DynamicRange::DolbyVision),
            ("hdr", DynamicRange::HdrOther),
            ("unexpected", DynamicRange::Unknown),
        ] {
            assert_eq!(parse_range(reported), expected);
        }
        assert_eq!(
            parse_video_status(&json!({"video": [{"videoInfo": {"hdrType": "DolbyVision"}}]})),
            DynamicRange::DolbyVision
        );
        assert_eq!(
            parse_picture_settings(&json!({"dimension": {"dynamicRange": "HDR10"}})),
            DynamicRange::Hdr10
        );
        assert_eq!(
            parse_video_status(&json!({"video": []})),
            DynamicRange::Unknown
        );
        assert_eq!(
            reconcile(DynamicRange::DolbyVision, DynamicRange::Sdr),
            DynamicRange::Unknown
        );
    }
}
