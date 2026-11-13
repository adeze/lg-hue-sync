"""Expose the requested screen sync state as a switch."""

from homeassistant.components.switch import SwitchEntity
from homeassistant.core import HomeAssistant
from homeassistant.helpers.entity import EntityCategory
from homeassistant.helpers.entity_platform import AddConfigEntryEntitiesCallback
from homeassistant.helpers.update_coordinator import CoordinatorEntity

from . import HueSyncConfigEntry, HueSyncCoordinator
from .tuning import SWITCH_SETTINGS


async def async_setup_entry(
    hass: HomeAssistant,
    entry: HueSyncConfigEntry,
    async_add_entities: AddConfigEntryEntitiesCallback,
) -> None:
    async_add_entities([
        SyncSwitch(entry),
        DisplaySwitch(entry),
        *(TuningSwitch(entry, key) for key in SWITCH_SETTINGS),
    ])


class TuningSwitch(CoordinatorEntity[HueSyncCoordinator], SwitchEntity):
    """Change an individual processing or lighting setting."""

    _attr_has_entity_name = True
    _attr_entity_category = EntityCategory.CONFIG

    def __init__(self, entry: HueSyncConfigEntry, key: str) -> None:
        super().__init__(entry.runtime_data)
        self._attr_device_info = self.coordinator.device_info
        self._attr_unique_id = f"{entry.entry_id}_{key}"
        self._attr_name = SWITCH_SETTINGS[key]
        self._key = key
        self._inverted = key == "use_xy_gamut"

    @property
    def is_on(self) -> bool | None:
        value = self.coordinator.data.get("tuning", {}).get(self._key)
        return value != self._inverted if value is not None else None

    @property
    def available(self) -> bool:
        return super().available and self.is_on is not None

    async def async_turn_on(self, **kwargs) -> None:
        await self.coordinator.async_apply_tuning({self._key: not self._inverted})

    async def async_turn_off(self, **kwargs) -> None:
        await self.coordinator.async_apply_tuning({self._key: self._inverted})


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
