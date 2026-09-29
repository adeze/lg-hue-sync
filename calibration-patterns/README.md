# Screen-to-light calibration images

Open `/capture-patterns` from the dashboard for selectable, full-screen solid patches. The page is self-contained after loading, so it remains usable while the daemon is stopped for a hardware capture probe. It may also be opened directly from `capture.html` on a Mac or phone and mirrored to an Apple TV HDMI input. These are nominal SDR browser colours; an Apple TV set to always output Dolby Vision maps them into its DV signal, so they test that **output path**, not authored Dolby Vision colour values. For a source-authored HDR or Dolby Vision test, use a verified encoded video file and confirm the TV's reported mode.

The read-only probe `lg-hue-sync test-capture --raw-nv12-stats --config <config>` reports five central 16×16 NV12 Y/Cb/Cr summaries and the TV's reported mode. It requires the webOS vtCapture hardware and an available capture scaler; it does not save pixels or send light output. Keep the pattern steady and centred while probing. A displayed mode label does not establish the captured pixels' primaries, transfer, or matrix.

The 3840×2160 PNGs are nominal sRGB test stimuli, not light-output measurements. Show each full-screen on the same HDMI input and TV picture mode used for viewing, at 1:1 aspect ratio without player controls. Use the dashboard's live mapping preview to compare a Hue channel's sampling rectangle with the displayed patches; keep the Hue app's physical placements authoritative. Do not edit Hue positions merely to make the preview visually neat.

1. `01-position-grid.png`: identify which screen tiles influence each Hue channel and Nanoleaf edge segment.
2. `02-high-chroma.png`: compare red, green, blue, cyan, magenta, and yellow response. This makes green/yellow confusion easy to spot.
3. `03-black-and-highlights.png`: check black gating, spill into the left half, near-black response, and isolated highlights. Avoid this pattern if sensitive to high contrast.
4. `04-neutral-ramp.png`: check visible tint across equal-RGB white/gray levels. White is not a verified 6500 K measurement.

Use the **same image** under Neutral Reference, High Chroma, Neon Contrast, and Dark-Scene Detail to isolate the preset effect. Let the lights settle before each photo. Keep iPhone position, exposure, and white balance fixed across comparisons if possible; automatic camera processing makes photos diagnostic, not colorimetric evidence. Do not use the dashboard's direct light test-pattern buttons at the same time: those bypass video sampling.

Editable SVG sources accompany the PNGs. Regenerate with `rsvg-convert -w 3840 -h 2160 -o output.png input.svg`.

## Current Hue Entertainment Area

For images aligned to the selected area's **actual API sampling rectangles**, run `bun run calibration-patterns/generate.ts http://<tv-ip>:8088` while the TV dashboard is reachable. This reads `/api/status` only; it does not start/stop lights or change settings. Requires `rsvg-convert`. Outputs stay in ignored `calibration-patterns/local/` because the geometry and light names are specific to your room.

- `00-current-area-composite.png` gives each channel a distinct pure color where possible. Regions shared by channels add their colors (red + green = yellow, green + blue = cyan, red + blue = magenta).
- `channel-NN.png` fills only that channel's exact sampling rectangle; use these sequentially to identify channels that respond together because their rectangles overlap.
- `mapping.json` records the area snapshot, expected channel color, and overlapping channel IDs. Regenerate after changing placements or the selected area in the Hue app.

An image pixel in two channels' rectangles cannot simultaneously have two different pure colors. Both channels sample that pixel, so their *final light colors* can still blend even when the composite shows an explicit overlap color. These images test the app's video-derived mapping; they do not override the Bridge's Entertainment channel grouping or validate physical color accuracy.
