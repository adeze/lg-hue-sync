"""Expose actual capture state separately from the sync switch."""

from homeassistant.components.sensor import SensorEntity
from homeassistant.core import HomeAssistant
from homeassistant.helpers.entity_platform import AddConfigEntryEntitiesCallback
from homeassistant.helpers.update_coordinator import CoordinatorEntity

from . import HueSyncConfigEntry, HueSyncCoordinator


async def async_setup_entry(
    hass: HomeAssistant,
    entry: HueSyncConfigEntry,
    async_add_entities: AddConfigEntryEntitiesCallback,
) -> None:
    async_add_entities([PipelineSensor(entry)])


class PipelineSensor(CoordinatorEntity[HueSyncCoordinator], SensorEntity):
    """Report the daemon's current capture state."""

    _attr_has_entity_name = True
    _attr_name = "Pipeline"

    def __init__(self, entry: HueSyncConfigEntry) -> None:
        super().__init__(entry.runtime_data)
        self._attr_device_info = self.coordinator.device_info
        self._attr_unique_id = f"{entry.entry_id}_pipeline"

    @property
    def native_value(self) -> str:
        return self.coordinator.data["pipeline_state"]

    @property
    def extra_state_attributes(self) -> dict:
        return {
            key: value
            for key, value in self.coordinator.data.items()
            if key
            in (
                "capture_fps",
                "light_update_fps",
                "hue_connected",
                "nanoleaf_connected",
                "capture_hardware",
                "capture_resolution",
                "tv_power_state",
                "preset_modified",
                "visualization_modes",
            )
        }
