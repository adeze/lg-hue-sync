"""Expose QSync pipeline restart."""

from aiohttp import ClientTimeout
from homeassistant.components.button import ButtonDeviceClass, ButtonEntity
from homeassistant.core import HomeAssistant
from homeassistant.exceptions import HomeAssistantError
from homeassistant.helpers.entity import EntityCategory
from homeassistant.helpers.entity_platform import AddConfigEntryEntitiesCallback
from homeassistant.helpers.update_coordinator import CoordinatorEntity

from . import HueSyncConfigEntry, HueSyncCoordinator
from .select import PRESETS


async def async_setup_entry(
    hass: HomeAssistant,
    entry: HueSyncConfigEntry,
    async_add_entities: AddConfigEntryEntitiesCallback,
) -> None:
    async_add_entities([
        RestartPipeline(entry),
        DashboardButton(entry, "refresh_diagnostics", "Refresh diagnostics", "diagnostics/sample"),
        DashboardButton(entry, "save_settings", "Save settings", "save-config"),
        DashboardButton(entry, "reset_colour", "Reset colour"),
    ])


class DashboardButton(CoordinatorEntity[HueSyncCoordinator], ButtonEntity):
    """Run an explicit operation from the QSync admin dashboard."""

    _attr_has_entity_name = True
    _attr_entity_category = EntityCategory.CONFIG

    def __init__(
        self, entry: HueSyncConfigEntry, key: str, name: str, command: str | None = None
    ) -> None:
        super().__init__(entry.runtime_data)
        self._attr_device_info = self.coordinator.device_info
        self._attr_unique_id = f"{entry.entry_id}_{key}"
        self._attr_name = name
        self._command = command

    async def async_press(self) -> None:
        if self._command is not None:
            await self.coordinator.async_command(self._command, timeout=ClientTimeout(total=10))
            return
        preset = self.coordinator.data.get("active_preset")
        if preset not in PRESETS.values():
            raise HomeAssistantError("Select a QSync colour preset first")
        await self.coordinator.async_command(f"presets/{preset}/reset")
        await self.coordinator.async_command("save-config", timeout=ClientTimeout(total=7))


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
