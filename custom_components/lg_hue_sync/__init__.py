"""Connect Home Assistant to the LG Hue Sync daemon on a TV."""

import asyncio
import json
import logging
import re
from datetime import UTC, datetime
from pathlib import Path
from typing import NotRequired, TypedDict

from aiohttp import ClientError, ClientSession, ClientTimeout
from homeassistant.components import frontend, websocket_api
from homeassistant.components.http import StaticPathConfig
from homeassistant.config_entries import ConfigEntry
from homeassistant.const import CONF_HOST, Platform
from homeassistant.core import HomeAssistant
from homeassistant.exceptions import HomeAssistantError
from homeassistant.helpers.aiohttp_client import async_get_clientsession
from homeassistant.helpers import device_registry as dr
from homeassistant.helpers import issue_registry as ir
from homeassistant.helpers.update_coordinator import DataUpdateCoordinator, UpdateFailed
from yarl import URL

from .tuning import parse_tuning

DOMAIN = "lg_hue_sync"
PLATFORMS = [
    Platform.SWITCH,
    Platform.SENSOR,
    Platform.BINARY_SENSOR,
    Platform.SELECT,
    Platform.BUTTON,
    Platform.NUMBER,
    Platform.EVENT,
]
LOGGER = logging.getLogger(__name__)
TIMEOUT = ClientTimeout(total=3)
CARD_URL = "/lg_hue_sync/qsync-card.js"
DEVICE_DOMAINS = {
    "tv_device": ("webostv", "bscpylgtv"),
    "hue_device": ("hue",),
    "nanoleaf_device": ("nanoleaf",),
}


def configured_device(
    hass: HomeAssistant, key: str, device_id: str | None
) -> dr.DeviceEntry | None:
    """Resolve a selected device in the expected integration."""
    if not device_id:
        return None
    device = dr.async_get(hass).async_get(device_id)
    if not isinstance(device, dr.DeviceEntry):
        return None
    entry = hass.config_entries.async_get_entry(device.config_entry_id)
    if entry is None or entry.domain not in DEVICE_DOMAINS[key]:
        return None
    if key == "hue_device" and not any(
        kind == dr.CONNECTION_NETWORK_MAC for kind, _ in device.connections
    ):
        return None
    return device


class SyncStatus(TypedDict):
    """Separate requested sync from actual capture state."""

    tuning: NotRequired[dict[str, bool | int | float]]
    daemon_version: NotRequired[str]

    requested_running: bool
    pipeline_state: str
    display_enabled: NotRequired[bool]
    active_preset: NotRequired[str]
    capture_fps: NotRequired[float]
    light_update_fps: NotRequired[float]
    hue_connected: NotRequired[bool]
    nanoleaf_connected: NotRequired[bool]
    preview_regions: NotRequired[dict]
    capture_hardware: NotRequired[bool]
    capture_resolution: NotRequired[str]
    tv_power_state: NotRequired[str]
    preset_modified: NotRequired[bool]
    visualization_modes: NotRequired[dict[str, str | None]]
    signal: NotRequired[dict[str, dict[str, bool | int | float | str]]]
    diagnostics_sampled_at: NotRequired[str]


SIGNAL_FIELDS = {
    "video": ("available", "width", "height", "frame_rate", "bit_depth", "hdr", "pixel_encoding", "range", "colourimetry"),
    "display": ("available", "width", "height", "refresh_rate", "muted", "locked", "enabled"),
    "transport": ("available", "codec", "sample_rate_hz", "channels", "bit_depth"),
    "decoder": ("available", "codec", "sample_rate_hz", "source_channels", "output_codec", "output_channels", "atmos_reported"),
    "output": ("available", "arc_state", "passthrough"),
}
SIGNAL_TEXT = {"hdr", "pixel_encoding", "range", "colourimetry", "codec", "output_codec", "arc_state"}
VISUALIZATION_OPTIONS = {
    "response": {"Slow": "slow", "Default": "default", "Fast": "fast"},
    "spatial_blend": {"Local": "local", "Balanced": "balanced", "Soft": "soft"},
}


def api_url(host: str, operation: str) -> URL:
    """Build a URL from the TV host and a fixed API path."""
    return URL.build(scheme="http", host=host, port=8088, path=f"/api/{operation}")


async def read_status(session: ClientSession, host: str) -> SyncStatus:
    """Read and validate the status fields used by the entities."""
    async with session.get(
        api_url(host, "status"), timeout=TIMEOUT, allow_redirects=False
    ) as response:
        response.raise_for_status()
        if response.status != 200:
            raise ValueError("Unexpected status response")
        data = await response.json()
    return parse_status(data)


def parse_status(data: object) -> SyncStatus:
    if (
        not isinstance(data, dict)
        or not isinstance(data.get("requested_running"), bool)
        or not isinstance(data.get("pipeline_state"), str)
        or not data["pipeline_state"]
        or len(data["pipeline_state"]) > 255
    ):
        raise ValueError("Unexpected LG Hue Sync status format")
    status: SyncStatus = {
        "requested_running": data["requested_running"],
        "pipeline_state": data["pipeline_state"],
    }
    if isinstance(version := data.get("version"), str) and re.fullmatch(
        r"[0-9][0-9A-Za-z.+-]{0,63}", version
    ):
        status["daemon_version"] = version
    if data.get("settings_patch_supported") is True and isinstance(
        raw_settings := data.get("settings"), dict
    ):
        status["tuning"] = parse_tuning(raw_settings)
    if isinstance(preset := data.get("active_preset"), str) and len(preset) <= 64:
        status["active_preset"] = preset
    if isinstance(enabled := data.get("display_enabled"), bool):
        status["display_enabled"] = enabled
    if (power := data.get("tv_power_state")) in ("active", "standby", "unknown"):
        status["tv_power_state"] = power
    if isinstance(resolution := data.get("capture_resolution"), str) and re.fullmatch(
        r"[0-9]{1,5}\s*[x×]\s*[0-9]{1,5}", resolution
    ):
        status["capture_resolution"] = resolution
    if isinstance(modes := data.get("visualization_modes"), dict):
        status["visualization_modes"] = {
            axis: mode if mode in options.values() else None
            for axis, options in VISUALIZATION_OPTIONS.items()
            if axis in modes
            and ((mode := modes[axis]) is None or isinstance(mode, str))
        }
    diagnostics = data.get("diagnostics")
    if isinstance(diagnostics, dict):
        if type(sampled := diagnostics.get("sampled_unix_ms")) is int and 0 < sampled < 253402300799000:
            status["diagnostics_sampled_at"] = datetime.fromtimestamp(sampled / 1000, UTC).isoformat()
        status["signal"] = {
            section: {
                field: sample[field]
                for field in fields
                if type(sample.get(field)) in (bool, int, float)
                or (field in SIGNAL_TEXT and isinstance(sample.get(field), str)
                    and len(sample[field]) <= 80)
            }
            for section, fields in SIGNAL_FIELDS.items()
            if isinstance(sample := diagnostics.get(section), dict)
        }
    for key in (
        "hue_connected",
        "nanoleaf_connected",
        "capture_hardware",
        "preset_modified",
    ):
        if isinstance(value := data.get(key), bool):
            status[key] = value
    for api_key, key in (
        ("fps", "capture_fps"),
        ("light_update_fps", "light_update_fps"),
    ):
        value = data.get(api_key)
        if type(value) in (int, float) and 0 <= value <= 1000:
            status[key] = value
    regions = {}
    if isinstance(zones := data.get("hue_zones"), list):
        regions["hue"] = []
        for zone in zones[:256]:
            if not isinstance(zone, dict):
                continue
            bounds = [zone.get(key) for key in ("x_min", "y_min", "x_max", "y_max")]
            if (
                all(type(value) in (int, float) and 0 <= value <= 1 for value in bounds)
                and bounds[0] < bounds[2] and bounds[1] < bounds[3]
            ):
                name = zone.get("name")
                regions["hue"].append({
                    "bounds": bounds,
                    "name": name[:80] if isinstance(name, str) else "Hue",
                })
    if isinstance(colors := data.get("nanoleaf_colors"), list):
        regions["nanoleaf"] = [
            [color["r"], color["g"], color["b"]]
            if isinstance(color, dict) and all(
                type(color.get(key)) is int and 0 <= color[key] <= 255 for key in "rgb"
            ) else None
            for color in colors[:50]
        ]
    if regions:
        status["preview_regions"] = regions
    return status


type HueSyncConfigEntry = ConfigEntry[HueSyncCoordinator]


class HueSyncCoordinator(DataUpdateCoordinator[SyncStatus]):
    """Share one TV subscription across the entities."""

    def __init__(self, hass: HomeAssistant, entry: HueSyncConfigEntry) -> None:
        super().__init__(
            hass,
            LOGGER,
            config_entry=entry,
            name=DOMAIN,
            always_update=False,
        )
        self.host = entry.data[CONF_HOST]
        self.tuning_lock = asyncio.Lock()
        self.session = async_get_clientsession(hass)
        self.device_info = dr.DeviceInfo(
            identifiers={(DOMAIN, entry.entry_id)},
            name=entry.title,
            configuration_url=str(api_url(self.host, "status").origin()),
        )

    async def _async_update_data(self) -> SyncStatus:
        try:
            status = await read_status(self.session, self.host)
        except (ClientError, TimeoutError, ValueError) as err:
            raise UpdateFailed("Could not read LG Hue Sync status") from err
        return status

    async def async_listen(self) -> None:
        """Reconnect when the TV restarts and mark stale entities unavailable."""
        delay = 1
        while True:
            try:
                async with self.session.get(
                    api_url(self.host, "status/events"),
                    timeout=ClientTimeout(sock_connect=3, sock_read=35),
                    allow_redirects=False,
                ) as response:
                    response.raise_for_status()
                    if response.content_type != "text/event-stream":
                        raise ValueError("Expected an LG Hue Sync status stream")
                    async for line in response.content:
                        if line.startswith(b"data:"):
                            data = parse_status(json.loads(line[5:]))
                            delay = 1
                            if not self.last_update_success:
                                LOGGER.info("LG Hue Sync status stream restored")
                            if data != self.data or not self.last_update_success:
                                self.async_set_updated_data(data)
                raise UpdateFailed("LG Hue Sync status stream closed")
            except (ClientError, TimeoutError, ValueError, UpdateFailed):
                self.async_set_update_error(
                    UpdateFailed("LG Hue Sync status stream unavailable")
                )
                await asyncio.sleep(delay)
                delay = min(delay * 2, 30)

    async def async_apply_tuning(self, patch: dict[str, bool | int | float]) -> None:
        """Apply a partial change and save using the same API as the admin dashboard."""
        async with self.tuning_lock:
            if not self.last_update_success or not self.data.get("tuning"):
                raise HomeAssistantError("The TV daemon does not offer live tuning")
            await self.async_command("settings", method="PATCH", payload=patch)
            try:
                await self.async_command("save-config", timeout=ClientTimeout(total=7))
            except HomeAssistantError as err:
                raise HomeAssistantError(
                    "QSync applied the live change but could not confirm it was saved"
                ) from err

    async def async_command(
        self, command: str, *, method: str = "POST", payload: dict | None = None,
        timeout: ClientTimeout = TIMEOUT,
    ) -> None:
        """Send an explicit dashboard command. The status stream reports its result."""
        try:
            async with self.session.request(
                method, api_url(self.host, command), json=payload, timeout=timeout,
                allow_redirects=False,
            ) as response:
                response.raise_for_status()
                if response.status != 200:
                    raise ValueError("Unexpected command response")
        except (ClientError, TimeoutError, ValueError) as err:
            raise HomeAssistantError(
                "LG Hue Sync did not acknowledge the command"
            ) from err


async def async_setup(hass: HomeAssistant, config: dict) -> bool:
    """Serve and load the card once per Home Assistant process."""
    from .preview import subscribe_preview

    websocket_api.async_register_command(hass, subscribe_preview)
    if "frontend" in hass.config.components:
        await hass.http.async_register_static_paths(
            [
                StaticPathConfig(
                    CARD_URL, str(Path(__file__).with_name("qsync-card.js")), False
                )
            ]
        )
        frontend.add_extra_js_url(hass, CARD_URL)
    return True


async def async_setup_entry(hass: HomeAssistant, entry: HueSyncConfigEntry) -> bool:
    """Verify the TV and set up the entities."""
    coordinator = HueSyncCoordinator(hass, entry)
    await coordinator.async_config_entry_first_refresh()
    entry.runtime_data = coordinator
    await hass.config_entries.async_forward_entry_setups(entry, PLATFORMS)
    for key in DEVICE_DOMAINS:
        device_id = entry.data.get(key)
        issue_id = f"{entry.entry_id}_{key}"
        if device_id and dr.async_get(hass).async_get(device_id) is None:
            ir.async_create_issue(
                hass, DOMAIN, issue_id, is_fixable=False,
                severity=ir.IssueSeverity.WARNING, translation_key="missing_device",
                learn_more_url="https://github.com/adeze/lg-hue-sync/blob/main/integrations/home-assistant/dashboard-card.md",
            )
        else:
            ir.async_delete_issue(hass, DOMAIN, issue_id)
    registry = dr.async_get(hass)
    if device := registry.async_get_device_by_identifier(
        (DOMAIN, entry.entry_id), entry.entry_id
    ):
        tv = configured_device(hass, "tv_device", entry.data.get("tv_device"))
        registry.async_update_device(
            device.id,
            new_connections={
                connection
                for connection in (tv.connections if tv else ())
                if connection[0] == dr.CONNECTION_NETWORK_MAC
            },
        )
    entry.async_create_background_task(
        hass, coordinator.async_listen(), "LG Hue Sync status"
    )
    return True


async def async_unload_entry(hass: HomeAssistant, entry: HueSyncConfigEntry) -> bool:
    """Unload all entity platforms."""
    return await hass.config_entries.async_unload_platforms(entry, PLATFORMS)


async def async_remove_entry(hass: HomeAssistant, entry: HueSyncConfigEntry) -> None:
    """Clear device link repairs when the integration entry is removed."""
    for key in DEVICE_DOMAINS:
        ir.async_delete_issue(hass, DOMAIN, f"{entry.entry_id}_{key}")
