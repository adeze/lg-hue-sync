"""Set up LG Hue Sync from a TV address."""

import re
from ipaddress import ip_address

import voluptuous as vol
from aiohttp import ClientError
from homeassistant.config_entries import ConfigFlow, ConfigFlowResult
from homeassistant.const import CONF_HOST
from homeassistant.helpers.aiohttp_client import async_get_clientsession

from . import DOMAIN, read_status

HOST_LABEL = re.compile(r"[a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?")


def normalize_host(value: str) -> str:
    """Accept a bare IPv4 address or DNS hostname for the TV's IPv4 listener."""
    host = value.strip().lower().removesuffix(".")
    try:
        address = ip_address(host)
    except ValueError:
        if (
            len(host) <= 253
            and not all(label.isdigit() for label in host.split("."))
            and all(HOST_LABEL.fullmatch(label) for label in host.split("."))
        ):
            return host
    else:
        if address.version == 4:
            return str(address)
    raise ValueError("Expected an IPv4 address or hostname")


class HueSyncConfigFlow(ConfigFlow, domain=DOMAIN):
    """Confirm the TV responds before creating a config entry."""

    VERSION = 1

    async def async_step_user(self, user_input=None) -> ConfigFlowResult:
        return await self._async_step_address("user", user_input)

    async def async_step_reconfigure(self, user_input=None) -> ConfigFlowResult:
        return await self._async_step_address("reconfigure", user_input)

    async def _async_step_address(
        self, step_id: str, user_input: dict[str, str] | None
    ) -> ConfigFlowResult:
        entry = self._get_reconfigure_entry() if step_id == "reconfigure" else None
        errors = {}
        if user_input is not None:
            try:
                host = normalize_host(user_input[CONF_HOST])
            except ValueError:
                errors[CONF_HOST] = "invalid_host"
            else:
                existing = await self.async_set_unique_id(host)
                if entry is None:
                    self._abort_if_unique_id_configured()
                if existing is not None and entry is not None and existing is not entry:
                    errors[CONF_HOST] = "already_configured"
                else:
                    try:
                        await read_status(async_get_clientsession(self.hass), host)
                    except (ClientError, TimeoutError, ValueError):
                        errors["base"] = "cannot_connect"
                    else:
                        if entry is not None:
                            return self.async_update_reload_and_abort(
                                entry,
                                unique_id=host,
                                title=f"LG Hue Sync ({host})",
                                data_updates={CONF_HOST: host},
                                reload_even_if_entry_is_unchanged=False,
                            )
                        return self.async_create_entry(
                            title=f"LG Hue Sync ({host})", data={CONF_HOST: host}
                        )
        schema = vol.Schema({vol.Required(CONF_HOST): str})
        return self.async_show_form(
            step_id=step_id,
            data_schema=(
                self.add_suggested_values_to_schema(schema, user_input or entry.data)
                if entry is not None
                else schema
            ),
            errors=errors,
        )
