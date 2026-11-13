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
        errors = {}
        if user_input is not None:
            try:
                host = normalize_host(user_input[CONF_HOST])
            except ValueError:
                errors[CONF_HOST] = "invalid_host"
            else:
                await self.async_set_unique_id(host)
                self._abort_if_unique_id_configured()
                try:
                    await read_status(async_get_clientsession(self.hass), host)
                except (ClientError, TimeoutError, ValueError):
                    errors["base"] = "cannot_connect"
                else:
                    return self.async_create_entry(
                        title=f"LG Hue Sync ({host})", data={CONF_HOST: host}
                    )
        return self.async_show_form(
            step_id="user",
            data_schema=vol.Schema({vol.Required(CONF_HOST): str}),
            errors=errors,
        )
