"""Select the dashboard's colour preset."""

from homeassistant.components.select import SelectEntity
from homeassistant.core import HomeAssistant
from homeassistant.exceptions import HomeAssistantError
from homeassistant.helpers.entity_platform import AddConfigEntryEntitiesCallback
from homeassistant.helpers.update_coordinator import CoordinatorEntity

from . import VISUALIZATION_OPTIONS, HueSyncConfigEntry, HueSyncCoordinator

PRESETS = {"Soft color": "softColor", "Neutral": "neutral", "Vivid color": "vividColor"}


async def async_setup_entry(
    hass: HomeAssistant,
    entry: HueSyncConfigEntry,
    async_add_entities: AddConfigEntryEntitiesCallback,
) -> None:
    async_add_entities(
        [
            ColorPreset(entry),
            VisualizationMode(entry, "response", "Response"),
            VisualizationMode(entry, "spatial_blend", "Spatial blending"),
        ]
    )


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


class VisualizationMode(CoordinatorEntity[HueSyncCoordinator], SelectEntity):
    """Change one visualization setting at a time."""

    _attr_has_entity_name = True

    def __init__(self, entry: HueSyncConfigEntry, axis: str, name: str) -> None:
        super().__init__(entry.runtime_data)
        self._attr_device_info = self.coordinator.device_info
        self._attr_unique_id = f"{entry.entry_id}_{axis}"
        self._attr_name = name
        self._axis = axis
        self._modes = VISUALIZATION_OPTIONS[axis]
        self._attr_options = list(self._modes)

    @property
    def available(self) -> bool:
        return super().available and self._axis in self.coordinator.data.get(
            "visualization_modes", {}
        )

    @property
    def current_option(self) -> str | None:
        mode = self.coordinator.data.get("visualization_modes", {}).get(self._axis)
        return next(
            (label for label, value in self._modes.items() if mode == value), None
        )

    async def async_select_option(self, option: str) -> None:
        if option not in self._modes:
            raise HomeAssistantError("Unknown LG Hue Sync visualization mode")
        await self.coordinator.async_command(
            f"visualization/{self._axis}/{self._modes[option]}"
        )
