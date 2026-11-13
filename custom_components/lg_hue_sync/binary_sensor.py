"""Expose output connectivity for Home Assistant automations."""

from homeassistant.components.binary_sensor import (
    BinarySensorDeviceClass,
    BinarySensorEntity,
)
from homeassistant.core import HomeAssistant
from homeassistant.helpers.entity import EntityCategory
from homeassistant.helpers.entity_platform import AddConfigEntryEntitiesCallback
from homeassistant.helpers.update_coordinator import CoordinatorEntity

from . import HueSyncConfigEntry, HueSyncCoordinator


async def async_setup_entry(
    hass: HomeAssistant,
    entry: HueSyncConfigEntry,
    async_add_entities: AddConfigEntryEntitiesCallback,
) -> None:
    async_add_entities(
        [
            OutputConnection(entry, "hue_connected", "Hue connection"),
            OutputConnection(entry, "nanoleaf_connected", "Nanoleaf connection"),
        ]
    )


class OutputConnection(CoordinatorEntity[HueSyncCoordinator], BinarySensorEntity):
    """Report whether the daemon has connected an output."""

    _attr_has_entity_name = True
    _attr_device_class = BinarySensorDeviceClass.CONNECTIVITY
    _attr_entity_category = EntityCategory.DIAGNOSTIC

    def __init__(self, entry: HueSyncConfigEntry, key: str, name: str) -> None:
        super().__init__(entry.runtime_data)
        self._attr_device_info = self.coordinator.device_info
        self._attr_unique_id = f"{entry.entry_id}_{key}"
        self._attr_name = name
        self._key = key

    @property
    def is_on(self) -> bool | None:
        return self.coordinator.data.get(self._key)
