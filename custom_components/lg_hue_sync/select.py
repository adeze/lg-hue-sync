"""Select the dashboard's colour preset."""

from homeassistant.components.select import SelectEntity
from homeassistant.core import HomeAssistant
from homeassistant.exceptions import HomeAssistantError
from homeassistant.helpers.entity_platform import AddConfigEntryEntitiesCallback
from homeassistant.helpers.update_coordinator import CoordinatorEntity

from . import HueSyncConfigEntry, HueSyncCoordinator

PRESETS = {"Soft color": "softColor", "Neutral": "neutral", "Vivid color": "vividColor"}


async def async_setup_entry(
    hass: HomeAssistant,
    entry: HueSyncConfigEntry,
    async_add_entities: AddConfigEntryEntitiesCallback,
) -> None:
    async_add_entities([ColorPreset(entry)])


class ColorPreset(CoordinatorEntity[HueSyncCoordinator], SelectEntity):
    """Apply a live colour preset without saving TV configuration."""

    _attr_has_entity_name = True
    _attr_name = "Color preset"

    def __init__(self, entry: HueSyncConfigEntry) -> None:
        super().__init__(entry.runtime_data)
        self._attr_device_info = self.coordinator.device_info
        self._attr_unique_id = f"{entry.entry_id}_color_preset"
        self._attr_options = list(PRESETS)

    @property
    def current_option(self) -> str | None:
        current = self.coordinator.data.get("active_preset")
        return next(
            (label for label, preset in PRESETS.items() if preset == current), None
        )

    async def async_select_option(self, option: str) -> None:
        if option not in PRESETS:
            raise HomeAssistantError("Unknown LG Hue Sync color preset")
        await self.coordinator.async_command(f"presets/{PRESETS[option]}")
