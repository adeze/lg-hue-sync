# QSync dashboard card

The card uses native Home Assistant controls and follows your active theme. It shows screen sync, display power, color preset, response, spatial blending, pipeline state, and output connections. It also offers a pipeline restart and a link to QSync for pairing, calibration, and detailed tuning.

Open **Live HDMI preview** to see a thumbnail updated up to once per second while HDMI sync is running. Home Assistant relays the TV's existing preview stream only while the panel is open and visible.

1. Install or update LG Hue Sync to 1.3.0 or later through HACS and configure the TV if needed.
2. Restart Home Assistant, then refresh the browser. The integration serves and loads the card automatically. If you previously registered the standalone card, remove its `/local/qsync-card.js` resource.
3. Edit a dashboard, select **Add card**, and choose **QSync**. For a manual card, use:

   ```yaml
   type: custom:qsync-card
   ```

The card finds the controls for a single LG Hue Sync device automatically. If you have several, set `device_id` to the ID at the end of the Home Assistant device page URL. If you've renamed entity IDs, specify those IDs under `entities`. You can also supply `dashboard_url` to override the admin link.

```yaml
type: custom:qsync-card
title: Living room QSync
entities:
  screen_sync: switch.tv_screen_sync
  spatial_blending: select.tv_spatial_blending
```

Supported entity override keys are `screen_sync`, `display`, `color_preset`, `response`, `spatial_blending`, `pipeline`, `hue_connection`, `nanoleaf_connection`, and `restart_pipeline`.

In LG Hue Sync's **Reconfigure** screen, select the existing LG webOS TV at the configured address. Home Assistant lists it under **Linked devices** once the LG integration has registered the TV's MAC address. Select your paired Philips Hue Bridge and Nanoleaf controller after adding their own Home Assistant integrations to show shortcuts under **Status**. The selections are optional and can be cleared in **Reconfigure**.
