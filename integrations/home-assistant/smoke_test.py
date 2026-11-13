"""Test the integration against Home Assistant Core and a local fake TV.

Run from the repository root:
uv run --no-project --python 3.14 --with homeassistant==2026.11.1 python integrations/home-assistant/smoke_test.py
"""

import asyncio
import json
import shutil
from pathlib import Path
from tempfile import TemporaryDirectory
from types import MappingProxyType
from unittest.mock import Mock

from aiohttp import web
from homeassistant import bootstrap, config_entries, data_entry_flow, loader
from homeassistant.const import STATE_UNAVAILABLE
from homeassistant.core import HomeAssistant
from homeassistant.exceptions import HomeAssistantError
from homeassistant.helpers import device_registry, entity_registry


async def main() -> None:
    domain = "lg_hue_sync"
    status = {
        "requested_running": True,
        "display_enabled": True,
        "pipeline_state": "unsupported_source",
        "active_preset": "neutral",
        "fps": 30.0,
        "hue_connected": True,
        "nanoleaf_connected": False,
        "tv_power_state": "active",
        "capture_resolution": "3840x2160",
        "hue_bridge_ip": "192.0.2.1",
        "hue_zones": [
            {"x_min": 0, "y_min": 0.2, "x_max": 0.5, "y_max": 0.8, "name": "Left lamp"},
            {"x_min": -1, "y_min": 0, "x_max": 1, "y_max": 1, "name": "Invalid"},
        ],
        "nanoleaf_colors": [{"r": 1, "g": 2, "b": 3}],
        "diagnostics": {"video": {"width": 3840, "input": "private input"}},
    }
    calls = []
    command_failure = False
    status_failure = False
    subscribers = set()
    preview_frames = asyncio.Queue()
    preview_requests = 0
    preview_closed = asyncio.Event()

    def publish():
        payload = json.dumps(status)
        for subscriber in subscribers:
            subscriber.put_nowait(payload)

    async def wait_for(predicate):
        async with asyncio.timeout(5):
            while not predicate():
                await asyncio.sleep(0.01)

    async def get_status(request):
        if status_failure:
            raise web.HTTPServiceUnavailable()
        return web.json_response(status)

    async def command(request):
        calls.append(request.path)
        if command_failure:
            raise web.HTTPBadGateway()
        if request.path in ("/api/start", "/api/stop"):
            status["requested_running"] = request.path == "/api/start"
        elif request.path.startswith("/api/display/"):
            status["display_enabled"] = request.path.endswith("/on")
        elif request.path.startswith("/api/presets/"):
            status["active_preset"] = request.match_info["preset"]
        elif request.path.startswith("/api/visualization/"):
            status["visualization_modes"][request.match_info["axis"]] = (
                request.match_info["mode"]
            )
        publish()
        return web.json_response({"status": "queued"})

    async def events(request):
        if status_failure:
            raise web.HTTPServiceUnavailable()
        response = web.StreamResponse(headers={"Content-Type": "text/event-stream"})
        await response.prepare(request)
        subscriber = asyncio.Queue()
        subscribers.add(subscriber)
        try:
            await response.write(f"data: {json.dumps(status)}\n\n".encode())
            while (payload := await subscriber.get()) is not None:
                await response.write(f"data: {payload}\n\n".encode())
        except ConnectionError:
            pass
        finally:
            subscribers.remove(subscriber)
        return response

    async def preview_events(request):
        nonlocal preview_requests
        preview_requests += 1
        response = web.StreamResponse(headers={"Content-Type": "text/event-stream"})
        await response.prepare(request)
        try:
            while True:
                payload = await preview_frames.get()
                await response.write(f"data: {json.dumps(payload)}\n\n".encode())
        except ConnectionError:
            return response
        finally:
            preview_closed.set()

    server = web.Application(handler_args={"handler_cancellation": True})
    server.router.add_get("/api/status", get_status)
    server.router.add_get("/api/status/events", events)
    server.router.add_get("/api/preview/events", preview_events)
    server.router.add_post("/api/start", command)
    server.router.add_post("/api/stop", command)
    server.router.add_post("/api/display/{action}", command)
    server.router.add_post("/api/presets/{preset}", command)
    server.router.add_post("/api/restart", command)
    server.router.add_post("/api/visualization/{axis}/{mode}", command)

    with TemporaryDirectory() as config_dir:
        root = Path(__file__).resolve().parents[2]
        shutil.copytree(
            root / "custom_components", Path(config_dir) / "custom_components"
        )
        hass = HomeAssistant(config_dir)
        loader.async_setup(hass)
        assert await bootstrap.async_from_config_dict({}, hass) is hass
        runner = web.AppRunner(server)
        await runner.setup()
        try:
            site = web.TCPSite(runner, "127.0.0.1", 8088)
            await site.start()

            flow = await hass.config_entries.flow.async_init(
                domain, context={"source": config_entries.SOURCE_USER}
            )
            assert flow["type"] == data_entry_flow.FlowResultType.FORM, flow
            flow_id = flow["flow_id"]
            invalid = await hass.config_entries.flow.async_configure(
                flow_id, {"host": "http://127.0.0.1:8088"}
            )
            assert invalid["errors"] == {"host": "invalid_host"}, invalid
            status["requested_running"] = "yes"
            rejected = await hass.config_entries.flow.async_configure(
                flow_id, {"host": "127.0.0.1"}
            )
            assert rejected["errors"] == {"base": "cannot_connect"}, rejected
            status["requested_running"] = True
            result = await hass.config_entries.flow.async_configure(
                flow_id, {"host": " 127.0.0.1 "}
            )
            assert result["type"] == data_entry_flow.FlowResultType.CREATE_ENTRY, result
            entry = result["result"]
            await hass.async_block_till_done()
            assert entry.state is config_entries.ConfigEntryState.LOADED, entry.state
            assert calls == []

            duplicate = await hass.config_entries.flow.async_init(
                domain,
                context={"source": config_entries.SOURCE_USER},
                data={"host": "127.0.0.1"},
            )
            assert duplicate["reason"] == "already_configured", duplicate

            registry = entity_registry.async_get(hass)
            switch = registry.async_get_entity_id(
                "switch", domain, f"{entry.entry_id}_screen_sync"
            )
            sensor = registry.async_get_entity_id(
                "sensor", domain, f"{entry.entry_id}_pipeline"
            )
            assert hass.states.get(switch).state == "on"
            assert hass.states.get(sensor).state == "unsupported_source"

            from custom_components.lg_hue_sync.preview import subscribe_preview

            connection = Mock(subscriptions={})
            msg = {"id": 1, "entity_id": sensor}
            connection.user.permissions.check_entity.return_value = False
            subscribe_preview(hass, connection, msg)
            assert connection.send_error.call_args.args[:2] == (1, "unauthorized")
            assert not connection.subscriptions and preview_requests == 0
            connection.user.permissions.check_entity.return_value = True
            subscribe_preview(hass, connection, {**msg, "entity_id": switch})
            assert connection.send_error.call_args.args[:2] == (1, "not_found")
            assert not connection.subscriptions
            subscribe_preview(hass, connection, msg)
            frame = {"width": 128, "height": 72, "rgb": [255] * (128 * 72 * 3)}
            preview_frames.put_nowait({**frame, "captured_unix_ms": 1})
            await wait_for(lambda: connection.send_event.call_count == 1)
            connection.send_result.assert_called_once_with(1)
            connection.send_event.assert_called_with(1, {
                **frame,
                "hue": [{"bounds": [0, 0.2, 0.5, 0.8], "name": "Left lamp"}],
                "nanoleaf": [[1, 2, 3]],
            })
            preview_frames.put_nowait(None)
            await wait_for(lambda: connection.send_event.call_count == 2)
            connection.send_event.assert_called_with(1, None)
            connection.subscriptions.pop(1)()
            async with asyncio.timeout(5):
                await preview_closed.wait()
            assert preview_requests == 1

            subscribe_preview(hass, connection, msg)
            preview_frames.put_nowait({**frame, "rgb": [999, 20, 30]})
            await wait_for(lambda: connection.send_event.call_count == 3)
            connection.send_event.assert_called_with(1, None)
            connection.subscriptions.pop(1)()
            assert preview_requests == 2

            async def set_switch(service):
                await hass.services.async_call(
                    "switch", service, {"entity_id": switch}, blocking=True
                )
                expected = "on" if service == "turn_on" else "off"
                await wait_for(lambda: hass.states.get(switch).state == expected)

            await set_switch("turn_off")
            assert hass.states.get(switch).state == "off"
            command_failure = True
            try:
                await set_switch("turn_on")
            except HomeAssistantError:
                assert hass.states.get(switch).state == "off"
            else:
                raise AssertionError("Failed command was reported as successful")
            command_failure = False
            status["pipeline_state"] = "running"
            await set_switch("turn_on")
            assert calls == ["/api/stop", "/api/start", "/api/start"]
            assert hass.states.get(switch).state == "on"
            assert hass.states.get(sensor).state == "running"

            devices = device_registry.async_get(hass)
            targets = {}
            tv_mac = (device_registry.CONNECTION_NETWORK_MAC, "02:00:00:00:00:01")
            hue_mac = (device_registry.CONNECTION_NETWORK_MAC, "02:00:00:00:00:02")
            for key, integration in (
                ("tv_device", "webostv"),
                ("hue_device", "hue"),
                ("nanoleaf_device", "nanoleaf"),
            ):
                target_entry = config_entries.ConfigEntry(
                    domain=integration,
                    title=integration,
                    data={},
                    options={},
                    version=1,
                    minor_version=1,
                    source=config_entries.SOURCE_USER,
                    unique_id="020000000002" if key == "hue_device" else integration,
                    discovery_keys=MappingProxyType({}),
                    subentries_data=None,
                    disabled_by=config_entries.ConfigEntryDisabler.USER,
                )
                await hass.config_entries.async_add(target_entry)
                targets[key] = devices.async_get_or_create(
                    config_entry_id=target_entry.entry_id,
                    identifiers={
                        (integration, "020000fffe000002" if key == "hue_device" else integration)
                    },
                    connections={
                        "tv_device": {tv_mac},
                        "hue_device": {hue_mac},
                    }.get(key, set()),
                ).id

            reconfigure = await hass.config_entries.flow.async_init(
                domain,
                context={
                    "source": config_entries.SOURCE_RECONFIGURE,
                    "entry_id": entry.entry_id,
                },
            )
            assert reconfigure["step_id"] == "reconfigure", reconfigure
            reconfigure_id = reconfigure["flow_id"]
            rejected = await hass.config_entries.flow.async_configure(
                reconfigure_id, {"host": "127.0.0.2"}
            )
            assert rejected["errors"] == {"base": "cannot_connect"}, rejected
            assert entry.data["host"] == "127.0.0.1"
            rejected = await hass.config_entries.flow.async_configure(
                reconfigure_id, {"host": "localhost", "hue_device": targets["tv_device"]}
            )
            assert rejected["errors"] == {"hue_device": "invalid_device"}, rejected
            result = await hass.config_entries.flow.async_configure(
                reconfigure_id, {"host": " LOCALHOST. ", **targets}
            )
            assert result["reason"] == "reconfigure_successful", result
            await hass.async_block_till_done()
            assert (
                entry.unique_id
                == entry.data["host"]
                == entry.runtime_data.host
                == "localhost"
            )
            assert hass.states.get(switch).state == "on"
            assert hass.states.get(sensor).attributes["capture_fps"] == 30.0
            assert hass.states.get(sensor).attributes["hue_connected"] is True
            qsync_device_id = registry.async_get(sensor).device_id
            assert {device.id for device in devices.async_get_devices(connections={tv_mac})} == {
                qsync_device_id, targets["tv_device"]
            }
            for key in ("hue_device", "nanoleaf_device"):
                assert hass.states.get(sensor).attributes[key] == targets[key]

            for platform, suffix, service, extra in (
                ("switch", "display", "turn_off", {}),
                ("select", "color_preset", "select_option", {"option": "Vivid color"}),
                ("button", "restart_pipeline", "press", {}),
            ):
                entity_id = registry.async_get_entity_id(
                    platform, domain, f"{entry.entry_id}_{suffix}"
                )
                await hass.services.async_call(
                    platform, service, {"entity_id": entity_id, **extra}, blocking=True
                )
            await wait_for(
                lambda: entry.runtime_data.data.get("display_enabled") is False
            )
            assert calls[-3:] == [
                "/api/display/off",
                "/api/presets/vividColor",
                "/api/restart",
            ]
            from custom_components.lg_hue_sync.diagnostics import (
                async_get_config_entry_diagnostics,
            )

            report = await async_get_config_entry_diagnostics(hass, entry)
            assert report["last_known_status"]["signal"] == {"video": {"width": 3840}}
            assert report["last_known_status"]["display_enabled"] is False
            assert "192.0.2.1" not in str(report) and "localhost" not in str(report)
            assert all(device_id not in str(report) for device_id in targets.values())

            response = registry.async_get_entity_id(
                "select", domain, f"{entry.entry_id}_response"
            )
            blend = registry.async_get_entity_id(
                "select", domain, f"{entry.entry_id}_spatial_blend"
            )
            hue = registry.async_get_entity_id(
                "binary_sensor", domain, f"{entry.entry_id}_hue_connected"
            )
            assert hass.states.get(hue).state == "on"
            assert (
                hass.states.get(sensor).attributes["capture_resolution"] == "3840x2160"
            )
            assert hass.states.get(response).state == STATE_UNAVAILABLE
            status["visualization_modes"] = {
                "response": "default",
                "spatial_blend": None,
            }
            publish()
            await wait_for(lambda: hass.states.get(response).state == "Default")
            assert hass.states.get(response).state == "Default"
            assert hass.states.get(blend).state == "unknown"
            for entity_id, option in ((response, "Fast"), (blend, "Soft")):
                await hass.services.async_call(
                    "select",
                    "select_option",
                    {"entity_id": entity_id, "option": option},
                    blocking=True,
                )
            await wait_for(lambda: hass.states.get(blend).state == "Soft")
            assert calls[-2:] == [
                "/api/visualization/response/fast",
                "/api/visualization/spatial_blend/soft",
            ]
            assert hass.states.get(response).state == "Fast"
            assert hass.states.get(blend).state == "Soft"

            status_failure = True
            for subscriber in subscribers:
                subscriber.put_nowait(None)
            await wait_for(lambda: hass.states.get(switch).state == STATE_UNAVAILABLE)
            assert hass.states.get(switch).state == STATE_UNAVAILABLE
            assert hass.states.get(sensor).state == STATE_UNAVAILABLE
            status_failure = False
            await wait_for(lambda: hass.states.get(switch).state == "on")
            assert hass.states.get(switch).state == "on"

            cleared = await hass.config_entries.flow.async_init(
                domain,
                context={"source": config_entries.SOURCE_RECONFIGURE, "entry_id": entry.entry_id},
                data={"host": "localhost"},
            )
            assert cleared["reason"] == "reconfigure_successful", cleared
            await hass.async_block_till_done()
            assert devices.async_get(qsync_device_id).connections == set()
            assert "hue_device" not in hass.states.get(sensor).attributes
            assert "nanoleaf_device" not in hass.states.get(sensor).attributes
            assert await hass.config_entries.async_unload(entry.entry_id)
            assert entry.state is config_entries.ConfigEntryState.NOT_LOADED
            for entity_id in (switch, sensor):
                entity_state = hass.states.get(entity_id)
                assert entity_state is None or entity_state.state == STATE_UNAVAILABLE
        finally:
            await hass.async_stop(force=True)
            for subscriber in subscribers:
                subscriber.put_nowait(None)
            await runner.cleanup()
    print(
        "Home Assistant setup, reconfigure, controls, diagnostics, recovery, and unload passed"
    )


if __name__ == "__main__":
    asyncio.run(main())
