"""Expose actual capture state separately from the sync switch."""

from homeassistant.components.sensor import SensorEntity
from homeassistant.core import HomeAssistant
from homeassistant.helpers.entity import EntityCategory
from homeassistant.helpers.entity_platform import AddConfigEntryEntitiesCallback
from homeassistant.helpers.update_coordinator import CoordinatorEntity

from . import HueSyncConfigEntry, HueSyncCoordinator, configured_device


async def async_setup_entry(
    hass: HomeAssistant,
    entry: HueSyncConfigEntry,
    async_add_entities: AddConfigEntryEntitiesCallback,
) -> None:
    async_add_entities([
        PipelineSensor(entry),
        SignalSensor(entry, "video", "frame_rate", "HDMI frame rate", "Hz"),
        SignalSensor(entry, "video", "hdr", "HDMI HDR format"),
        SignalSensor(entry, "display", "refresh_rate", "Display refresh rate", "Hz"),
        SignalSensor(entry, "output", "arc_state", "ARC status"),
        SignalSensor(entry, "transport", "codec", "Audio transport codec"),
        SignalSensor(entry, "transport", "sample_rate_hz", "Audio transport rate", "Hz"),
        SignalSensor(entry, "decoder", "output_codec", "Audio output codec"),
    ])


class SignalSensor(CoordinatorEntity[HueSyncCoordinator], SensorEntity):
    """Report a sampled signal measurement with its companion fields."""

    _attr_has_entity_name = True
    _attr_entity_category = EntityCategory.DIAGNOSTIC

    def __init__(
        self, entry: HueSyncConfigEntry, section: str, field: str, name: str, unit: str | None = None
    ) -> None:
        super().__init__(entry.runtime_data)
        self._attr_device_info = self.coordinator.device_info
        self._attr_unique_id = f"{entry.entry_id}_{section}_{field}"
        self._attr_name = name
        self._attr_native_unit_of_measurement = unit
        self._section = section
        self._field = field

    @property
    def native_value(self) -> str | int | float | None:
        sample = self.coordinator.data.get("signal", {}).get(self._section, {})
        return sample.get(self._field) if sample.get("available") else None


class PipelineSensor(CoordinatorEntity[HueSyncCoordinator], SensorEntity):
    """Report the daemon's current capture state."""

    _attr_has_entity_name = True
    _attr_name = "Pipeline"

    def __init__(self, entry: HueSyncConfigEntry) -> None:
        super().__init__(entry.runtime_data)
        self._attr_device_info = self.coordinator.device_info
        self._attr_unique_id = f"{entry.entry_id}_pipeline"
        self._entry = entry

    @property
    def native_value(self) -> str:
        return self.coordinator.data["pipeline_state"]

    @property
    def extra_state_attributes(self) -> dict:
        attributes = {
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
                "daemon_version",
                "diagnostics_sampled_at",
            )
        }
        for section, sample in self.coordinator.data.get("signal", {}).items():
            if sample.get("available"):
                attributes.update({
                    f"signal_{section}_{key}": value
                    for key, value in sample.items() if key != "available"
                })
        for key in ("tv_device", "hue_device", "nanoleaf_device"):
            if device := configured_device(self.hass, key, self._entry.data.get(key)):
                attributes[key] = device.id
        return attributes
