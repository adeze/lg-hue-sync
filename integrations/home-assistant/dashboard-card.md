# QSync dashboard card

The card follows your Home Assistant theme. Use **Ambient style** for Colour, Response and Spatial blending. **Advanced tuning** contains the native number and switch entities for QSync's sliders, processing settings and per-output screen sync. Those entities require a daemon with partial settings support. **Diagnostics** includes the latest HDMI, display and ARC sample and the pipeline activity event entity. Use **Refresh diagnostics** to request a fresh sample.

Open **Live HDMI preview** to see a thumbnail updated up to once per second while HDMI sync is running. Home Assistant relays the TV's existing preview stream only while the panel is open and visible.
Use **Light regions** to show or hide the Hue sampling boxes and Nanoleaf perimeter segments.

1. Install or update LG Hue Sync to 1.3.0 or later through HACS and configure the TV if needed.
2. Restart Home Assistant, then refresh the browser. The integration serves and loads the card automatically. If you previously registered the standalone card, remove its `/local/qsync-card.js` resource.
3. Edit a dashboard, select **Add card**, and choose **QSync**. For a manual card, use:

   ```yaml
   type: custom:qsync-card
   ```

To use the full layout, create a dashboard and paste [the exported Sections YAML](qsync-dashboard.yaml) into its **Raw configuration editor**. The card editor can select the QSync device and show or hide the preview and advanced tuning. The YAML uses automatic device discovery and can be shared without device IDs.

The card finds the controls for a single LG Hue Sync device automatically. If you have several, set `device_id` to the ID at the end of the Home Assistant device page URL. If you've renamed entity IDs, specify those IDs under `entities`. You can also supply `dashboard_url` to override the admin link.

```yaml
type: custom:qsync-card
title: Living room QSync
entities:
  screen_sync: switch.tv_screen_sync
  spatial_blending: select.tv_spatial_blending
```

Supported entity override keys are `screen_sync`, `display`, `color_preset`, `response`, `spatial_blending`, `pipeline`, `hue_connection`, `nanoleaf_connection`, and `restart_pipeline`.

In LG Hue Sync's **Reconfigure** screen, select the same TV from the built-in LG webOS TV integration or **LG WebOS TV (bscpylgtv)**. The card uses the selected device's media player for TV controls. Home Assistant lists the TV under **Linked devices** when the selected integration has registered the TV's MAC address. Select the Hue Bridge and Nanoleaf controller to show shortcuts under **Status**. Pairing for screen sync and alignment still live in QSync admin.

In the automation editor, use the **Pipeline activity** entity as a state trigger to react to a new event. Check its `event_type` attribute in a condition to distinguish `faulted`, `disconnected`, and `reconnected`. For a watching scene, call `select.select_option` on the QSync preset entities and `switch.turn_on` on Screen sync. The linked TV's media player provides its own triggers and actions.
