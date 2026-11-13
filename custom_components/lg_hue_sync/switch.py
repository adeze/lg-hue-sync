"""Expose the requested screen sync state as a switch."""

from aiohttp import ClientError
from homeassistant.components.switch import SwitchEntity
from homeassistant.core import HomeAssistant
from homeassistant.exceptions import HomeAssistantError
from homeassistant.helpers.entity_platform import AddConfigEntryEntitiesCallback
from homeassistant.helpers.update_coordinator import CoordinatorEntity

from . import TIMEOUT, HueSyncConfigEntry, HueSyncCoordinator, api_url


async def async_setup_entry(
    hass: HomeAssistant,
    entry: HueSyncConfigEntry,
    async_add_entities: AddConfigEntryEntitiesCallback,
) -> None:
    async_add_entities([SyncSwitch(entry)])


class SyncSwitch(CoordinatorEntity[HueSyncCoordinator], SwitchEntity):
    """Start and stop sync through the same controls as the TV dashboard."""

    _attr_has_entity_name = True
    _attr_name = "Screen sync"

    def __init__(self, entry: HueSyncConfigEntry) -> None:
        super().__init__(entry.runtime_data)
        self._attr_device_info = self.coordinator.device_info
        self._attr_unique_id = f"{entry.entry_id}_screen_sync"

    @property
    def is_on(self) -> bool:
        return self.coordinator.data["requested_running"]

    async def async_turn_on(self, **kwargs) -> None:
        await self._async_command("start")

    async def async_turn_off(self, **kwargs) -> None:
        await self._async_command("stop")

    async def _async_command(self, command: str) -> None:
        coordinator = self.coordinator
        try:
            async with coordinator.session.post(
                api_url(coordinator.host, command),
                timeout=TIMEOUT,
                allow_redirects=False,
            ) as response:
                response.raise_for_status()
                if response.status != 200:
                    raise ValueError("Unexpected command response")
        except (ClientError, TimeoutError, ValueError) as err:
            raise HomeAssistantError(
                "LG Hue Sync did not acknowledge the command"
            ) from err
        await coordinator.async_request_refresh()
