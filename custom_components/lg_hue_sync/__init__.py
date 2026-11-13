"""Connect Home Assistant to the LG Hue Sync daemon on a TV."""

import logging
from datetime import timedelta
from typing import TypedDict

from aiohttp import ClientError, ClientSession, ClientTimeout
from homeassistant.config_entries import ConfigEntry
from homeassistant.const import CONF_HOST, Platform
from homeassistant.core import HomeAssistant
from homeassistant.helpers.aiohttp_client import async_get_clientsession
from homeassistant.helpers.device_registry import DeviceInfo
from homeassistant.helpers.update_coordinator import DataUpdateCoordinator, UpdateFailed
from yarl import URL

DOMAIN = "lg_hue_sync"
PLATFORMS = [Platform.SWITCH, Platform.SENSOR]
LOGGER = logging.getLogger(__name__)
TIMEOUT = ClientTimeout(total=3)


class SyncStatus(TypedDict):
    """Separate requested sync from actual capture state."""

    requested_running: bool
    pipeline_state: str


def api_url(host: str, operation: str) -> URL:
    """Build a URL from the TV host and a fixed API path."""
    return URL.build(scheme="http", host=host, port=8088, path=f"/api/{operation}")


async def read_status(session: ClientSession, host: str) -> SyncStatus:
    """Read and validate the two status fields used by the entities."""
    async with session.get(
        api_url(host, "status"), timeout=TIMEOUT, allow_redirects=False
    ) as response:
        response.raise_for_status()
        if response.status != 200:
            raise ValueError("Unexpected status response")
        data = await response.json()
    if (
        not isinstance(data, dict)
        or not isinstance(data.get("requested_running"), bool)
        or not isinstance(data.get("pipeline_state"), str)
        or not data["pipeline_state"]
        or len(data["pipeline_state"]) > 255
    ):
        raise ValueError("Unexpected LG Hue Sync status format")
    return {
        "requested_running": data["requested_running"],
        "pipeline_state": data["pipeline_state"],
    }


type HueSyncConfigEntry = ConfigEntry[HueSyncCoordinator]


class HueSyncCoordinator(DataUpdateCoordinator[SyncStatus]):
    """Share one polling request across the switch and sensor."""

    def __init__(self, hass: HomeAssistant, entry: HueSyncConfigEntry) -> None:
        super().__init__(
            hass,
            LOGGER,
            config_entry=entry,
            name=DOMAIN,
            update_interval=timedelta(seconds=10),
            always_update=False,
        )
        self.host = entry.data[CONF_HOST]
        self.session = async_get_clientsession(hass)
        self.device_info = DeviceInfo(
            identifiers={(DOMAIN, entry.entry_id)},
            name=entry.title,
            configuration_url=str(api_url(self.host, "status").origin()),
        )

    async def _async_update_data(self) -> SyncStatus:
        try:
            return await read_status(self.session, self.host)
        except (ClientError, TimeoutError, ValueError) as err:
            raise UpdateFailed("Could not read LG Hue Sync status") from err


async def async_setup_entry(hass: HomeAssistant, entry: HueSyncConfigEntry) -> bool:
    """Verify the TV and forward the entry to the two platforms."""
    coordinator = HueSyncCoordinator(hass, entry)
    await coordinator.async_config_entry_first_refresh()
    entry.runtime_data = coordinator
    await hass.config_entries.async_forward_entry_setups(entry, PLATFORMS)
    return True


async def async_unload_entry(hass: HomeAssistant, entry: HueSyncConfigEntry) -> bool:
    """Unload both platforms."""
    return await hass.config_entries.async_unload_platforms(entry, PLATFORMS)
