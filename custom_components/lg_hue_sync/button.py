"""Expose QSync pipeline restart."""

from homeassistant.components.button import ButtonDeviceClass, ButtonEntity
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
    async_add_entities([RestartPipeline(entry)])


class RestartPipeline(CoordinatorEntity[HueSyncCoordinator], ButtonEntity):
    """Restart the capture and lighting pipeline."""

    _attr_has_entity_name = True
    _attr_name = "Restart pipeline"
    _attr_device_class = ButtonDeviceClass.RESTART
    _attr_entity_category = EntityCategory.CONFIG

    def __init__(self, entry: HueSyncConfigEntry) -> None:
        super().__init__(entry.runtime_data)
        self._attr_device_info = self.coordinator.device_info
        self._attr_unique_id = f"{entry.entry_id}_restart_pipeline"

    async def async_press(self) -> None:
        await self.coordinator.async_command("restart")
