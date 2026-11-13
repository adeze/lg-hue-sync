"""Test the integration against Home Assistant Core and a local fake TV.

Run from the repository root:
uv run --no-project --python 3.14 --with homeassistant==2026.11.1 python integrations/home-assistant/smoke_test.py
"""

import asyncio
import shutil
from pathlib import Path
from tempfile import TemporaryDirectory

from aiohttp import web
from homeassistant import bootstrap, config_entries, data_entry_flow, loader
from homeassistant.const import STATE_UNAVAILABLE
from homeassistant.core import HomeAssistant
from homeassistant.exceptions import HomeAssistantError
from homeassistant.helpers import entity_registry


async def main() -> None:
    domain = "lg_hue_sync"
    status = {"requested_running": True, "pipeline_state": "unsupported_source"}
    calls = []
    command_failure = False
    status_failure = False

    async def get_status(request):
        if status_failure:
            raise web.HTTPServiceUnavailable()
        return web.json_response(status)

    async def command(request):
        calls.append(request.path)
        if command_failure:
            raise web.HTTPBadGateway()
        status["requested_running"] = request.path == "/api/start"
        return web.json_response({"status": "queued"})

    server = web.Application()
    server.router.add_get("/api/status", get_status)
    server.router.add_post("/api/start", command)
    server.router.add_post("/api/stop", command)

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

            async def set_switch(service):
                await hass.services.async_call(
                    "switch", service, {"entity_id": switch}, blocking=True
                )
                await entry.runtime_data.async_refresh()

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

            status_failure = True
            await entry.runtime_data.async_refresh()
            assert hass.states.get(switch).state == STATE_UNAVAILABLE
            assert hass.states.get(sensor).state == STATE_UNAVAILABLE
            status_failure = False
            await entry.runtime_data.async_refresh()
            assert hass.states.get(switch).state == "on"
            assert await hass.config_entries.async_unload(entry.entry_id)
            assert entry.state is config_entries.ConfigEntryState.NOT_LOADED
            for entity_id in (switch, sensor):
                entity_state = hass.states.get(entity_id)
                assert entity_state is None or entity_state.state == STATE_UNAVAILABLE
        finally:
            await hass.async_stop(force=True)
            await runner.cleanup()
    print("Home Assistant setup, start/stop, failure, recovery, and unload passed")


if __name__ == "__main__":
    asyncio.run(main())
