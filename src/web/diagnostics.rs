use serde::Serialize;
use serde_json::Value;
use std::{
    collections::BTreeMap,
    fs::File,
    io::{self, Read},
    time::{SystemTime, UNIX_EPOCH},
};
use utoipa::ToSchema;

const MAX_DRIVER_REPLY: u64 = 64 * 1024;

#[derive(Debug, Clone, Default, Serialize, ToSchema)]
pub(super) struct Diagnostics {
    pub sampled_unix_ms: u64,
    pub video: VideoDiagnostics,
    pub player: VideoDiagnostics,
    pub display: DisplayDiagnostics,
    pub transport: TransportDiagnostics,
    pub decoder: DecoderDiagnostics,
    pub output: OutputDiagnostics,
}

#[derive(Debug, Clone, Default, Serialize, ToSchema)]
pub(super) struct VideoDiagnostics {
    pub available: bool,
    pub input: Option<String>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub frame_rate: Option<f64>,
    pub scan_type: Option<String>,
    pub hdr: Option<String>,
    pub pixel_encoding: Option<String>,
    pub range: Option<String>,
    pub colourimetry: Option<String>,
    pub bit_depth: Option<u8>,
}

#[derive(Debug, Clone, Default, Serialize, ToSchema)]
pub(super) struct DisplayDiagnostics {
    pub available: bool,
    pub source: Option<String>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub refresh_rate: Option<f64>,
    pub enabled: Option<bool>,
    pub muted: Option<bool>,
    pub locked: Option<bool>,
}

#[derive(Debug, Clone, Default, Serialize, ToSchema)]
pub(super) struct TransportDiagnostics {
    pub available: bool,
    pub codec: Option<String>,
    pub sample_rate_hz: Option<u32>,
    pub channels: Option<u16>,
    pub bit_depth: Option<u8>,
}

#[derive(Debug, Clone, Default, Serialize, ToSchema)]
pub(super) struct DecoderDiagnostics {
    pub available: bool,
    pub codec: Option<String>,
    pub sample_rate_hz: Option<u32>,
    pub source_channels: Option<u16>,
    pub output_codec: Option<String>,
    pub output_channels: Option<u16>,
    pub atmos_reported: Option<bool>,
}

#[derive(Debug, Clone, Default, Serialize, ToSchema)]
pub(super) struct OutputDiagnostics {
    pub available: bool,
    pub arc_state: Option<String>,
    pub passthrough: Option<bool>,
}

#[derive(Default)]
struct DriverFiles {
    hdmi: Option<String>,
    decoders: [Option<String>; 2],
    sndout: Option<String>,
    vbe: Option<String>,
}

pub(super) async fn collect() -> Diagnostics {
    let (video, arc, drivers) = tokio::join!(
        crate::luna::query("luna://com.webos.service.videooutput/getStatus", "{}"),
        crate::luna::query("luna://com.webos.service.arccontroller/getARCState", "{}"),
        tokio::task::spawn_blocking(read_driver_files),
    );
    snapshot(
        video.as_ref().ok(),
        arc.as_ref().ok(),
        &drivers.unwrap_or_default(),
    )
}

fn read_driver_files() -> DriverFiles {
    let read = |path| File::open(path).and_then(read_bounded).ok();
    DriverFiles {
        hdmi: read("/proc/lgtv-driver/audio/hdmi"),
        decoders: [
            read("/proc/lgtv-driver/audio/adec.p0"),
            read("/proc/lgtv-driver/audio/adec.p1"),
        ],
        sndout: read("/proc/lgtv-driver/audio/sndout"),
        vbe: read("/proc/lgtv-driver/vbe/vbe-status"),
    }
}

fn read_bounded(reader: impl Read) -> io::Result<String> {
    let mut bytes = Vec::new();
    reader.take(MAX_DRIVER_REPLY + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_DRIVER_REPLY {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "driver status exceeds limit",
        ));
    }
    String::from_utf8(bytes).map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
}

fn text(value: &Value) -> Option<String> {
    value
        .as_str()
        .map(str::trim)
        .filter(|value| !value.is_empty() && value.len() <= 96)
        .map(str::to_owned)
}

fn positive_u32(value: &Value) -> Option<u32> {
    value
        .as_u64()
        .and_then(|value| u32::try_from(value).ok())
        .filter(|value| *value > 0)
}

fn frame_rate(value: &Value) -> Option<f64> {
    let rate = value.as_f64().or_else(|| {
        let value = value.as_str()?.trim();
        if let Some((numerator, denominator)) = value.split_once('/') {
            Some(numerator.trim().parse::<f64>().ok()? / denominator.trim().parse::<f64>().ok()?)
        } else {
            value.parse().ok()
        }
    })?;
    (rate.is_finite() && rate > 0.0).then_some(rate)
}

fn fields<'a>(value: &'a str, section: Option<&str>) -> BTreeMap<&'a str, &'a str> {
    let mut active = section.is_none();
    let mut result = BTreeMap::new();
    for line in value.lines().map(str::trim) {
        if let Some(name) = line
            .strip_prefix('[')
            .and_then(|line| line.strip_suffix(']'))
        {
            active = section.is_none_or(|section| name.eq_ignore_ascii_case(section));
        } else if active {
            if let Some((key, value)) = line.split_once('=') {
                result.insert(key.trim(), value.trim());
            }
        }
    }
    result
}

fn driver_text(fields: &BTreeMap<&str, &str>, key: &str) -> Option<String> {
    fields
        .get(key)
        .filter(|value| {
            !value.is_empty() && value.len() <= 96 && !value.eq_ignore_ascii_case("unknown")
        })
        .map(|value| (*value).to_owned())
}

fn driver_number<T: std::str::FromStr + PartialOrd + From<u8>>(
    fields: &BTreeMap<&str, &str>,
    key: &str,
) -> Option<T> {
    fields
        .get(key)?
        .parse::<T>()
        .ok()
        .filter(|value| *value > T::from(0))
}

fn video_snapshot(sink: &Value) -> VideoDiagnostics {
    let info = &sink["videoInfo"];
    let range = info["pixelEncoding"].as_str().and_then(|encoding| {
        if encoding.eq_ignore_ascii_case("RGB") {
            text(&info["rgbRange"])
        } else if encoding.to_ascii_lowercase().starts_with("ycbcr") {
            text(&info["ycbcrRange"])
        } else {
            None
        }
    });
    VideoDiagnostics {
        available: true,
        input: text(&sink["contentType"]),
        width: positive_u32(&sink["width"]),
        height: positive_u32(&sink["height"]),
        frame_rate: frame_rate(&sink["frameRate"]),
        scan_type: text(&sink["scanType"]),
        hdr: text(&info["hdrType"]),
        pixel_encoding: text(&info["pixelEncoding"]),
        range,
        colourimetry: text(&info["colormetry"]),
        bit_depth: None,
    }
}

fn snapshot(
    video_reply: Option<&Value>,
    arc_reply: Option<&Value>,
    drivers: &DriverFiles,
) -> Diagnostics {
    let main = video_reply
        .filter(|reply| reply["returnValue"] == true)
        .and_then(|reply| reply["video"].as_array())
        .and_then(|sinks| {
            sinks
                .iter()
                .find(|sink| sink["sink"] == "MAIN" && sink["connected"] == true)
        });
    let hdmi_main = main.filter(|sink| sink["connectedSource"] == "HDMI");
    let video = hdmi_main.map(video_snapshot).unwrap_or_default();
    let player = main
        .filter(|sink| sink["connectedSource"] == "VDEC")
        .map(video_snapshot)
        .unwrap_or_default();
    let vbe = fields(drivers.vbe.as_deref().unwrap_or_default(), None);
    let switch = |key| match vbe.get(key).copied() {
        Some("on") => Some(true),
        Some("off") => Some(false),
        _ => None,
    };
    let refresh_rate = driver_number::<f64>(&vbe, "vbe_outputFrameRate")
        .filter(|rate| rate.is_finite() && *rate <= 1000.0);
    let enabled = switch("vbe_displayOutput");
    let muted = switch("vbe_mute");
    let locked = match vbe.get("vbe_lockStatus").copied() {
        Some("Locked") => Some(true),
        Some("Unlocked") => Some(false),
        _ => None,
    };
    let display = DisplayDiagnostics {
        available: main.is_some()
            || refresh_rate.is_some()
            || enabled.is_some()
            || muted.is_some()
            || locked.is_some(),
        source: main.and_then(|sink| text(&sink["connectedSource"])),
        width: main.and_then(|sink| positive_u32(&sink["displayOutput"]["width"])),
        height: main.and_then(|sink| positive_u32(&sink["displayOutput"]["height"])),
        refresh_rate,
        enabled,
        muted,
        locked,
    };
    let hdmi = fields(drivers.hdmi.as_deref().unwrap_or_default(), None);
    let input_port = hdmi_main.and_then(|sink| sink["connectedSourcePort"].as_u64());
    let candidates: Vec<_> = drivers
        .decoders
        .iter()
        .flatten()
        .map(|file| fields(file, None))
        .filter(|decoder| decoder.get("Start") == Some(&"1"))
        .filter(|decoder| {
            decoder
                .get("Connect")
                .and_then(|connect| connect.strip_prefix("HDMI_PORT"))
                .and_then(|port| port.parse::<u64>().ok())
                .is_some_and(|port| input_port.is_none_or(|input| input == port))
        })
        .collect();
    let live = video.available
        && hdmi.get("HDMIConnected") == Some(&"1")
        && hdmi.get("AudioData") == Some(&"1")
        && driver_number::<u32>(&hdmi, "SamplingRate").is_some()
        && candidates.len() == 1;
    let transport = if live {
        TransportDiagnostics {
            available: true,
            codec: driver_text(&hdmi, "Codec"),
            sample_rate_hz: driver_number(&hdmi, "SamplingRate"),
            ..Default::default()
        }
    } else {
        TransportDiagnostics::default()
    };
    let decoder = if live {
        let decoder = &candidates[0];
        let codec = driver_text(decoder, "SrcCodec").or_else(|| driver_text(decoder, "CurCodec"));
        DecoderDiagnostics {
            available: true,
            atmos_reported: codec
                .as_ref()
                .map(|codec| codec.to_ascii_uppercase().contains("ATMOS")),
            codec,
            sample_rate_hz: driver_number(decoder, "SrcSamplingRate"),
            source_channels: driver_number(decoder, "SrcChannel"),
            output_codec: driver_text(decoder, "OutputCodec"),
            output_channels: driver_number(decoder, "OutputChannel"),
        }
    } else {
        DecoderDiagnostics::default()
    };
    let arc_state = arc_reply
        .filter(|reply| reply["returnValue"] == true)
        .and_then(|reply| reply["arcActivation"].as_str())
        .and_then(|state| match state.to_ascii_uppercase().as_str() {
            "ARC" => Some("ARC".to_string()),
            "EARC" => Some("eARC".to_string()),
            "OFF" | "NONE" | "INACTIVE" => Some("Off".to_string()),
            _ => None,
        });
    let arc = fields(drivers.sndout.as_deref().unwrap_or_default(), Some("ARC"));
    let passthrough = (arc_state.as_deref() != Some("Off") && arc.get("Connect") == Some(&"1"))
        .then(|| driver_text(&arc, "OutputCodec"))
        .flatten()
        .map(|codec| codec.eq_ignore_ascii_case("Pass-Through"));
    let output = OutputDiagnostics {
        available: arc_state.is_some() || passthrough.is_some(),
        arc_state,
        passthrough,
    };
    Diagnostics {
        sampled_unix_ms: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .ok()
            .and_then(|duration| u64::try_from(duration.as_millis()).ok())
            .unwrap_or_default(),
        video,
        player,
        display,
        transport,
        decoder,
        output,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn main_video() -> Value {
        json!({"returnValue": true, "video": [{"sink": "MAIN", "connected": true, "connectedSource": "HDMI",
            "connectedSourcePort": 1, "contentType": "hdmi3", "width": 3840, "height": 2160,
            "frameRate": "60000/1001", "scanType": "progressive", "videoInfo": {
                "hdrType": "none", "pixelEncoding": "YCbCr444", "colormetry": "ITU709",
                "ycbcrRange": "LIMITED", "rgbRange": "DEFAULT"}}]})
    }

    fn live_drivers() -> DriverFiles {
        DriverFiles {
            hdmi: Some("HDMIConnected=1\nAudioData=1\nCodec=MAT\nSamplingRate=192000\n".into()),
            decoders: [None, Some("[ADEC]\nConnect=HDMI_PORT1\nStart=1\nSrcCodec=MAT_ATMOS\nSrcSamplingRate=48000\nSrcChannel=2\nOutputCodec=PCM\nOutputChannel=8\n".into())],
            vbe: None,
            sndout: Some("[SPEAKER]\nConnect=1\nOutputCodec=PCM\n[ARC]\nConnect=1\nOutputCodec=Pass-Through\n[EARC]\nEArcOnOff=On\n".into()),
        }
    }

    #[test]
    fn separates_display_output_from_hdmi_and_native_player() {
        let mut reply = main_video();
        reply["video"][0]["displayOutput"] = json!({"width":3840,"height":2160});
        let mut drivers = DriverFiles { vbe: Some("vbe_outputFrameRate=120\nvbe_displayOutput=on\nvbe_mute=off\nvbe_lockStatus=Locked\n".into()), ..Default::default() };
        let d = snapshot(Some(&reply), None, &drivers);
        assert_eq!(d.display.refresh_rate, Some(120.0));
        assert_eq!(d.display.width, Some(3840));
        assert_eq!(d.display.muted, Some(false));
        assert!(d.video.available && !d.player.available);
        reply["video"][0]["connectedSource"] = json!("VDEC");
        reply["video"][0]["width"] = json!(1920);
        reply["video"][0]["frameRate"] = json!(23.9760246);
        reply["video"][0]["videoInfo"]["hdrType"] = json!("DolbyVision");
        let d = snapshot(Some(&reply), None, &drivers);
        assert!(!d.video.available && d.player.available);
        assert_eq!(d.player.width, Some(1920));
        assert_eq!(d.player.frame_rate, Some(23.9760246));
        assert_eq!(d.player.hdr.as_deref(), Some("DolbyVision"));
        assert!(!d.transport.available);
        drivers.vbe = Some("vbe_outputFrameRate=inf\nvbe_displayOutput=unknown\n".into());
        reply["video"][0]["connected"] = json!(false);
        let d = snapshot(Some(&reply), None, &drivers);
        assert!(!d.display.available && !d.player.available);
    }

    #[test]
    fn separates_hdmi_carrier_decoder_and_actual_arc_state() {
        let result = snapshot(
            Some(&main_video()),
            Some(&json!({"returnValue": true, "arcActivation": "eARC"})),
            &live_drivers(),
        );
        assert_eq!(result.video.input.as_deref(), Some("hdmi3"));
        assert!((result.video.frame_rate.unwrap() - 59.94005994).abs() < 0.00001);
        assert_eq!(result.video.range.as_deref(), Some("LIMITED"));
        assert_eq!(result.transport.sample_rate_hz, Some(192000));
        assert_eq!(result.decoder.sample_rate_hz, Some(48000));
        assert_eq!(result.decoder.source_channels, Some(2));
        assert_eq!(result.decoder.output_channels, Some(8));
        assert_eq!(result.decoder.atmos_reported, Some(true));
        assert_eq!(result.output.arc_state.as_deref(), Some("eARC"));
        assert_eq!(result.output.passthrough, Some(true));
        assert!(
            result.video.bit_depth.is_none()
                && result.transport.bit_depth.is_none()
                && result.transport.channels.is_none()
        );
    }

    #[test]
    fn missing_inactive_mismatched_and_malformed_input_never_reuses_audio() {
        for hdmi in [
            "",
            "HDMIConnected=0\nAudioData=1\nSamplingRate=192000",
            "HDMIConnected=1\nAudioData=0\nSamplingRate=192000",
            "HDMIConnected=1\nAudioData=1\nSamplingRate=0",
            "HDMIConnected=1\nAudioData=1\nSamplingRate=not-a-rate",
        ] {
            let mut drivers = live_drivers();
            drivers.hdmi = Some(hdmi.into());
            let result = snapshot(Some(&main_video()), None, &drivers);
            assert!(!result.transport.available && !result.decoder.available);
            assert!(result.transport.codec.is_none() && result.decoder.codec.is_none());
        }
        let mut drivers = live_drivers();
        for decoder in [
            "Connect=HDMI_PORT1\nStart=0",
            "Connect=HDMI_PORT0\nStart=1",
            "Connect=TV\nStart=1",
            "Connect=HDMI_PORTbogus\nStart=1",
        ] {
            drivers.decoders[1] = Some(decoder.into());
            assert!(
                !snapshot(Some(&main_video()), None, &drivers)
                    .decoder
                    .available
            );
        }
        let mut video = main_video();
        video["video"][0]["connectedSource"] = json!("VDEC");
        assert!(
            !snapshot(Some(&video), None, &live_drivers())
                .transport
                .available
        );
        for value in [json!(0), json!(-1), json!("1/0"), json!("NaN"), json!(null)] {
            assert!(frame_rate(&value).is_none());
        }
        let result = snapshot(None, None, &DriverFiles::default());
        assert!(!result.video.available && !result.transport.available && !result.output.available);
    }

    #[test]
    fn capability_flags_and_other_output_sections_do_not_claim_active_earc_or_passthrough() {
        let mut drivers = live_drivers();
        drivers.sndout = Some("[SPEAKER]\nConnect=1\nOutputCodec=Pass-Through\n[ARC]\nConnect=0\nOutputCodec=Pass-Through\n[EARC]\nEArcOnOff=On".into());
        let result = snapshot(
            None,
            Some(
                &json!({"returnValue": true, "earcATMOS": true, "advancedEarcCapability": [{"audioCodingType":"MAT", "maxChannelNumber":8}]}),
            ),
            &drivers,
        );
        assert!(!result.output.available);
        drivers.sndout = live_drivers().sndout;
        let result = snapshot(
            None,
            Some(&json!({"returnValue": true, "arcActivation":"OFF"})),
            &drivers,
        );
        assert_eq!(result.output.arc_state.as_deref(), Some("Off"));
        assert!(result.output.passthrough.is_none());
    }

    #[test]
    fn driver_reads_are_bounded_and_require_utf8() {
        assert_eq!(read_bounded(&b"Codec=PCM\n"[..]).unwrap(), "Codec=PCM\n");
        assert!(read_bounded(&vec![b'x'; MAX_DRIVER_REPLY as usize][..]).is_ok());
        assert!(read_bounded(&vec![b'x'; MAX_DRIVER_REPLY as usize + 1][..]).is_err());
        assert!(read_bounded(&[0xff][..]).is_err());
    }
}
