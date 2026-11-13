"""Expose pipeline transitions for Home Assistant history and automations."""

from homeassistant.components.event import EventEntity
from homeassistant.core import HomeAssistant, callback
from homeassistant.helpers.entity import EntityCategory
from homeassistant.helpers.entity_platform import AddConfigEntryEntitiesCallback
from homeassistant.helpers.update_coordinator import CoordinatorEntity

from . import HueSyncConfigEntry, HueSyncCoordinator


async def async_setup_entry(
    hass: HomeAssistant,
    entry: HueSyncConfigEntry,
    async_add_entities: AddConfigEntryEntitiesCallback,
) -> None:
    async_add_entities([PipelineEvent(entry)])


class PipelineEvent(CoordinatorEntity[HueSyncCoordinator], EventEntity):
    """Record a transition once, including loss and recovery of the status stream."""

    _attr_has_entity_name = True
    _attr_name = "Pipeline activity"
    _attr_entity_category = EntityCategory.DIAGNOSTIC
    _attr_event_types = [
        "starting", "running", "stopping", "paused", "recovering", "faulted",
        "unsupported_source", "disconnected", "reconnected",
    ]

    def __init__(self, entry: HueSyncConfigEntry) -> None:
        super().__init__(entry.runtime_data)
        self._attr_device_info = self.coordinator.device_info
        self._attr_unique_id = f"{entry.entry_id}_pipeline_activity"
        self._connected = self.coordinator.last_update_success
        self._pipeline = self.coordinator.data["pipeline_state"]

    @property
    def available(self) -> bool:
        return True

    @callback
    def _handle_coordinator_update(self) -> None:
        connected = self.coordinator.last_update_success
        pipeline = self.coordinator.data["pipeline_state"]
        if connected != self._connected:
            self._trigger_event("reconnected" if connected else "disconnected")
        elif connected and pipeline != self._pipeline and pipeline in self._attr_event_types:
            self._trigger_event(pipeline)
        else:
            return
        self._connected = connected
        self._pipeline = pipeline
        self.async_write_ha_state()
