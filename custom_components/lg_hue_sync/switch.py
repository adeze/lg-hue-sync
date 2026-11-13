"""Expose the requested screen sync state as a switch."""

from homeassistant.components.switch import SwitchEntity
from homeassistant.core import HomeAssistant
from homeassistant.helpers.entity_platform import AddConfigEntryEntitiesCallback
from homeassistant.helpers.update_coordinator import CoordinatorEntity

from . import HueSyncConfigEntry, HueSyncCoordinator


async def async_setup_entry(
    hass: HomeAssistant,
    entry: HueSyncConfigEntry,
    async_add_entities: AddConfigEntryEntitiesCallback,
) -> None:
    async_add_entities([SyncSwitch(entry), DisplaySwitch(entry)])


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
        await self.coordinator.async_command("start")

    async def async_turn_off(self, **kwargs) -> None:
        await self.coordinator.async_command("stop")


class DisplaySwitch(CoordinatorEntity[HueSyncCoordinator], SwitchEntity):
    """Control the TV panel using its reported display state."""

    _attr_has_entity_name = True
    _attr_name = "Display"

    def __init__(self, entry: HueSyncConfigEntry) -> None:
        super().__init__(entry.runtime_data)
        self._attr_device_info = self.coordinator.device_info
        self._attr_unique_id = f"{entry.entry_id}_display"

    @property
    def available(self) -> bool:
        return super().available and "display_enabled" in self.coordinator.data

    @property
    def is_on(self) -> bool | None:
        return self.coordinator.data.get("display_enabled")

    async def async_turn_on(self, **kwargs) -> None:
        await self.coordinator.async_command("display/on")

    async def async_turn_off(self, **kwargs) -> None:
        await self.coordinator.async_command("display/off")
