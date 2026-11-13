"""Relay QSync thumbnails to authorized Home Assistant dashboard viewers."""

import asyncio
import json

import probatio
from aiohttp import ClientError, ClientTimeout
from homeassistant.auth.permissions.const import POLICY_READ
from homeassistant.components import websocket_api
from homeassistant.config_entries import ConfigEntryState
from homeassistant.core import HomeAssistant, callback
from homeassistant.helpers import config_validation as cv
from homeassistant.helpers import entity_registry

from . import DOMAIN, api_url


def parse_frame(payload: bytes) -> dict | None:
    """Keep only bounded RGB thumbnails in the relay."""
    frame = json.loads(payload)
    if frame is None:
        return None
    if isinstance(frame, dict):
        width, height, rgb = (frame.get(key) for key in ("width", "height", "rgb"))
        if (
            type(width) is int
            and 1 <= width <= 128
            and type(height) is int
            and 1 <= height <= 72
            and isinstance(rgb, list)
            and len(rgb) == width * height * 3
            and all(type(value) is int and 0 <= value <= 255 for value in rgb)
        ):
            return {"width": width, "height": height, "rgb": rgb}
    raise ValueError("Invalid QSync thumbnail")


@callback
@websocket_api.websocket_command(
    {
        probatio.Required("type"): "lg_hue_sync/preview",
        probatio.Required("entity_id"): cv.entity_id,
    }
)
def subscribe_preview(
    hass: HomeAssistant, connection: websocket_api.ActiveConnection, msg: dict
) -> None:
    """Relay frames until the card unsubscribes or the integration unloads."""
    entity_id = msg["entity_id"]
    subscription = msg["id"]

    def can_read() -> bool:
        return connection.user.permissions.check_entity(entity_id, POLICY_READ)

    if not can_read():
        connection.send_error(
            subscription, websocket_api.ERR_UNAUTHORIZED, "Preview access denied"
        )
        return
    entity = entity_registry.async_get(hass).async_get(entity_id)
    entry = (
        hass.config_entries.async_get_entry(entity.config_entry_id)
        if entity and entity.config_entry_id
        else None
    )
    if (
        entry is None
        or entry.domain != DOMAIN
        or entry.state is not ConfigEntryState.LOADED
        or entity.unique_id != f"{entry.entry_id}_pipeline"
    ):
        connection.send_error(
            subscription, websocket_api.ERR_NOT_FOUND, "QSync pipeline unavailable"
        )
        return
    coordinator = entry.runtime_data

    async def relay() -> None:
        delay = 1
        while can_read():
            try:
                async with coordinator.session.get(
                    api_url(coordinator.host, "preview/events"),
                    timeout=ClientTimeout(sock_connect=3, sock_read=35),
                    allow_redirects=False,
                ) as response:
                    if (
                        response.status != 200
                        or response.content_type != "text/event-stream"
                    ):
                        raise ValueError("QSync preview unavailable")
                    async for line in response.content:
                        if not can_read():
                            return
                        if line.startswith(b"data:"):
                            frame = parse_frame(line[5:])
                            connection.send_event(subscription, frame)
                            delay = 1
            except (ClientError, TimeoutError, ValueError):
                pass
            if can_read():
                connection.send_event(subscription, None)
                await asyncio.sleep(delay)
                delay = min(delay * 2, 30)

    task = entry.async_create_background_task(hass, relay(), "QSync preview")
    connection.subscriptions[subscription] = task.cancel
    connection.send_result(subscription)
