# Home Assistant

This [Home Assistant package](https://www.home-assistant.io/docs/configuration/packages/) adds a switch and pipeline sensor for one LG Hue Sync TV using the daemon's existing LAN API. It uses built-in [REST](https://www.home-assistant.io/integrations/rest/), [RESTful Command](https://www.home-assistant.io/integrations/rest_command/), and [Template](https://www.home-assistant.io/integrations/template/#switch) integrations.

1. Copy `lg_hue_sync.yaml` to `packages/lg_hue_sync.yaml` in your Home Assistant configuration directory. If you already load packages from a different directory, use that one.
2. If packages are new to your setup, add this to `configuration.yaml`. Add the `packages:` line under your existing `homeassistant:` key if present.

   ```yaml
   homeassistant:
     packages: !include_dir_named packages
   ```

3. Add these entries to Home Assistant's `secrets.yaml`. Replace `TV_HOST` with the TV's reserved LAN address or a hostname that Home Assistant can resolve.

   ```yaml
   lg_hue_sync_status_url: "http://TV_HOST:8088/api/status"
   lg_hue_sync_start_url: "http://TV_HOST:8088/api/start"
   lg_hue_sync_stop_url: "http://TV_HOST:8088/api/stop"
   ```

4. In **Developer Tools → YAML**, check configuration, then restart Home Assistant. Look for `switch.lg_hue_sync` and `sensor.lg_hue_sync_pipeline`. If the pipeline sensor's entity ID differs, update both references in the package.

The switch follows `requested_running`. It can stay on when the pipeline reports `unsupported_source`; turning it off sends an explicit Stop and cancels automatic resume. Use the pipeline sensor's `running` state to detect active capture rather than using the switch. Status refreshes every 10 seconds, and the switch becomes unavailable when the status request fails.

From Home Assistant's network, `GET http://TV_HOST:8088/api/status` should return `requested_running` and `pipeline_state`. The API is unauthenticated; keep port 8088 on the trusted LAN. To remove the integration, remove the package and its three secret entries, then restart Home Assistant.
