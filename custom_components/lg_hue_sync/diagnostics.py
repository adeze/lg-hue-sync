"""Download the latest polled LG Hue Sync diagnostics."""

from homeassistant.core import HomeAssistant

from . import HueSyncConfigEntry


async def async_get_config_entry_diagnostics(
    hass: HomeAssistant, entry: HueSyncConfigEntry
) -> dict:
    """Export the coordinator's allowlisted state without addresses or credentials."""
    coordinator = entry.runtime_data
    status = dict(coordinator.data)
    status.pop("active_preset", None)
    status.pop("preview_regions", None)
    status["pipeline_state"] = (
        pipeline
        if (pipeline := status["pipeline_state"])
        in {
            "starting",
            "running",
            "stopping",
            "paused",
            "recovering",
            "faulted",
            "unsupported_source",
        }
        else "unknown"
    )
    return {
        "reachable": coordinator.last_update_success,
        "last_known_status": status,
    }
