"""Shared bounds and labels for QSync's native Home Assistant controls."""

NUMBER_SETTINGS = {
    "brightness_multiplier": ("Brightness", 0.5, 2.5, 0.05),
    "saturation_boost": ("Saturation", 0.5, 2.5, 0.05),
    "peak_weight": ("Highlights", 0, 1, 0.1),
    "spatial_blend": ("Blend", 0, 0.5, 0.05),
    "gamma": ("Gamma", 0.5, 2, 0.05),
    "smoothing_factor": ("Response strength", 0.05, 1, 0.01),
    "max_color_step": ("Scene limit", 4, 64, 1),
    "noise_gate_threshold": ("Black gate", 0, 0.06, 0.005),
    "depth_influence": ("Depth influence", 0, 1, 0.5),
    "black_frame_hold_ms": ("Black-screen hold", 0, 5000, 250),
    "black_frame_fade_ms": ("Black-screen fade", 0, 10000, 250),
    "hue_output_brightness": ("Hue output brightness", 0, 2.5, 0.05),
    "nanoleaf_output_brightness": ("Nanoleaf output brightness", 0, 2.5, 0.05),
}

SWITCH_SETTINGS = {
    "strict_blackout": "Strict black",
    "use_xy_gamut": "Direct RGB mode",
    "letterbox_detection": "Letterbox compensation",
    "hdr_tone_mapping": "Midtone lift (legacy)",
    "hue_sync_enabled": "Hue screen sync",
    "nanoleaf_sync_enabled": "Nanoleaf screen sync",
    "auto_tv_power": "Follow TV power",
}


def parse_tuning(raw: dict) -> dict[str, bool | int | float]:
    """Accept only the settings surfaced by the integration."""
    values = {}
    for key, (_, minimum, maximum, _) in NUMBER_SETTINGS.items():
        value = raw.get(key)
        if type(value) in (int, float) and minimum <= value <= maximum:
            if not (key == "max_color_step" or key.endswith("_ms")) or type(value) is int:
                values[key] = value
    for key in SWITCH_SETTINGS:
        if type(value := raw.get(key)) is bool:
            values[key] = value
    return values
