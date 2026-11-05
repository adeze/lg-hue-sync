use serde_json::Value;

const FOREGROUND_URI: &str = "luna://com.webos.applicationManager/getForegroundAppInfo";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptureSource {
    Hdmi,
    Unsupported,
    Unknown,
}

impl CaptureSource {
    pub fn label(self) -> &'static str {
        match self {
            Self::Hdmi => "HDMI",
            Self::Unsupported => "Unsupported",
            Self::Unknown => "Unknown",
        }
    }

    pub fn compatible(self) -> Option<bool> {
        match self {
            Self::Hdmi => Some(true),
            Self::Unsupported => Some(false),
            Self::Unknown => None,
        }
    }
}

pub(crate) fn parse_source(value: &Value) -> CaptureSource {
    if value["returnValue"] != true {
        return CaptureSource::Unknown;
    }
    let Some(id) = value["appId"].as_str().filter(|id| {
        !id.is_empty()
            && id.len() <= 256
            && id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"._-".contains(&byte))
    }) else {
        return CaptureSource::Unknown;
    };
    match id {
        "com.webos.app.hdmi1"
        | "com.webos.app.hdmi2"
        | "com.webos.app.hdmi3"
        | "com.webos.app.hdmi4" => CaptureSource::Hdmi,
        _ => CaptureSource::Unsupported,
    }
}

/// Foreground identity is a capture-policy hint, not DRM detection.
pub async fn current_source() -> CaptureSource {
    crate::luna::query(FOREGROUND_URI, "{}")
        .await
        .as_ref()
        .map(parse_source)
        .unwrap_or(CaptureSource::Unknown)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn foreground_identity_classifies_only_exact_hdmi_ids_and_preserves_unknowns() {
        for id in [
            "com.webos.app.hdmi1",
            "com.webos.app.hdmi2",
            "com.webos.app.hdmi3",
            "com.webos.app.hdmi4",
        ] {
            let source =
                parse_source(&json!({"returnValue":true,"appId":id,"title":"private title"}));
            assert_eq!(source, CaptureSource::Hdmi);
            assert_eq!(source.compatible(), Some(true));
            assert_eq!(source.label(), "HDMI");
        }
        for id in [
            "netflix",
            "com.webos.app.netflix",
            "com.webos.app.home",
            "com.webos.app.browser",
            "com.webos.app.livetv",
            "com.webos.app.hdmi5",
            "com.webos.app.hdmi1.extra",
        ] {
            let source = parse_source(&json!({"returnValue":true,"appId":id}));
            assert_eq!(source, CaptureSource::Unsupported);
            assert_eq!(source.compatible(), Some(false));
        }
        for value in [
            json!({}),
            json!({"appId":"com.webos.app.hdmi1"}),
            json!({"returnValue":false,"appId":"netflix"}),
            json!({"returnValue":true,"appId":""}),
            json!({"returnValue":true,"appId":12}),
            json!({"returnValue":true,"appId":"bad\napp"}),
            json!({"returnValue":true,"foregroundAppInfo":[{"appId":"netflix"}]}),
        ] {
            assert_eq!(parse_source(&value), CaptureSource::Unknown);
        }
        assert_eq!(
            parse_source(&json!("not an object")),
            CaptureSource::Unknown
        );
        assert_eq!(CaptureSource::Unknown.compatible(), None);
        assert_eq!(CaptureSource::Unknown.label(), "Unknown");
    }
}
