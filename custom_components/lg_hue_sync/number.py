"""Expose the QSync admin faders as Home Assistant numbers."""

from homeassistant.components.number import NumberEntity, NumberMode
from homeassistant.core import HomeAssistant
from homeassistant.exceptions import HomeAssistantError
from homeassistant.helpers.entity import EntityCategory
from homeassistant.helpers.entity_platform import AddConfigEntryEntitiesCallback
from homeassistant.helpers.update_coordinator import CoordinatorEntity

from . import HueSyncConfigEntry, HueSyncCoordinator
from .tuning import NUMBER_SETTINGS


async def async_setup_entry(
    hass: HomeAssistant,
    entry: HueSyncConfigEntry,
    async_add_entities: AddConfigEntryEntitiesCallback,
) -> None:
    async_add_entities(TuningNumber(entry, key) for key in NUMBER_SETTINGS)


class TuningNumber(CoordinatorEntity[HueSyncCoordinator], NumberEntity):
    """Adjust one field without replacing settings changed by another client."""

    _attr_has_entity_name = True
    _attr_entity_category = EntityCategory.CONFIG
    _attr_mode = NumberMode.SLIDER

    def __init__(self, entry: HueSyncConfigEntry, key: str) -> None:
        super().__init__(entry.runtime_data)
        self._attr_device_info = self.coordinator.device_info
        self._attr_unique_id = f"{entry.entry_id}_{key}"
        self._key = key
        (
            self._attr_name,
            self._attr_native_min_value,
            self._attr_native_max_value,
            self._attr_native_step,
        ) = NUMBER_SETTINGS[key]
        self._integer = key == "max_color_step" or key.endswith("_ms")
        if key.endswith("_ms"):
            self._attr_native_unit_of_measurement = "ms"

    @property
    def native_value(self) -> float | None:
        return self.coordinator.data.get("tuning", {}).get(self._key)

    @property
    def available(self) -> bool:
        return super().available and self.native_value is not None

    async def async_set_native_value(self, value: float) -> None:
        if not self._attr_native_min_value <= value <= self._attr_native_max_value:
            raise HomeAssistantError("QSync setting is out of range")
        if self._integer and int(value) != value:
            raise HomeAssistantError("QSync setting requires a whole number")
        patch = {self._key: int(value) if self._integer else value}
        if self._key == "smoothing_factor":
            patch.update(rise_smoothing_factor=value, fall_smoothing_factor=value)
        await self.coordinator.async_apply_tuning(patch)
