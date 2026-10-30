import type { Plugin } from 'vite';
import { initialSettings, type LiveSettings } from '../src/lib/settings.ts';
import type { Status } from '../src/lib/dashboard.svelte.ts';
import type { PreviewFrame } from '../src/lib/preview-types.ts';
// Simulated channel coordinates only; production bounds come from the Rust sampler.
export function demoSamplingBounds(position: number[], depthInfluence: number) {
  const clamp = (value: number) => Math.max(0, Math.min(1, value));
  const depth = clamp((1 - position[1]) / 2);
  const span = 0.25 * (1 + 0.5 * clamp(depthInfluence) * depth * depth * (3 - 2 * depth));
  const x = clamp((position[0] + 1) / 2);
  const y = clamp((1 - position[2]) / 2);
  return {
    x_min: clamp(x - span / 2),
    x_max: clamp(x + span / 2),
    y_min: clamp(y - span / 2),
    y_max: clamp(y + span / 2),
  };
}
// Development only: simulated API; never forwards requests to the TV or lights.
export function mockApi(): Plugin {
  let settings: LiveSettings = structuredClone(initialSettings),
    active = 'neutral',
    modified = false,
    running = true,
    display = true,
    bonjour = false;
  let saved = structuredClone(settings);
  const overrides = new Map<string, Partial<LiveSettings>>();
  const zones = [
    {
      channel_id: 0,
      name: 'Left lightstrip',
      demo_position: [-1, 0.1, 0],
      hue_device_id: 'demo-left',
      x_min: 0,
      x_max: 0.2,
      y_min: 0.15,
      y_max: 0.85,
      hue_positions: [[-1, 0.1, 0]],
    },
    {
      channel_id: 1,
      name: 'Top lightstrip',
      demo_position: [0, 0.5, 0.8],
      hue_device_id: 'demo-top',
      x_min: 0.15,
      x_max: 0.85,
      y_min: 0,
      y_max: 0.2,
      hue_positions: [
        [-0.7, 0.5, 0.8],
        [0.7, 0.5, 0.8],
      ],
    },
    {
      channel_id: 4,
      name: 'Gradient strip · Segments 1–2',
      demo_position: [1, -0.6, 0],
      hue_device_id: 'demo-gradient',
      x_min: 0.7,
      x_max: 1,
      y_min: 0.2,
      y_max: 0.8,
      hue_positions: [
        [1, -0.7, 0],
        [1, 0.65, 0],
      ],
    },
    {
      channel_id: 5,
      name: 'Gradient strip · Segments 3–7',
      demo_position: [1, 0.55, 0],
      hue_device_id: 'demo-gradient',
      x_min: 0.65,
      x_max: 0.9,
      y_min: 0.4,
      y_max: 0.6,
      hue_positions: [
        [1, -0.7, 0],
        [1, 0.65, 0],
      ],
    },
  ];
  const color = (i: number) => ({
    r: Math.round(140 + 100 * Math.sin(i * 0.6)),
    g: Math.round(140 + 100 * Math.sin(i * 0.6 + 2)),
    b: Math.round(140 + 100 * Math.sin(i * 0.6 + 4)),
  });
  const status = (): Status => ({
    settings,
    active_preset: active,
    preset_modified: modified,
    bonjour_enabled: bonjour,
    pipeline_state: running && display ? 'running' : display ? 'paused' : 'paused',
    requested_running: running,
    is_syncing: running && display,
    capture_hardware: running && display,
    hue_connected: running && display,
    nanoleaf_connected: running && display,
    hue_bridge_ip: 'demo.bridge',
    nanoleaf_ip: 'demo.nanoleaf',
    fps: 60,
    light_update_fps: 50,
    capture_resolution: '320x180',
    tv_power_state: 'active',
    last_error: null,
    hue_zones: zones.map(({ demo_position, ...zone }) => ({
      ...zone,
      ...demoSamplingBounds(demo_position, settings.depth_influence),
    })),
    hue_colors: zones.map((zone) => [zone.channel_id, color(zone.channel_id)]),
    nanoleaf_colors: Array.from({ length: 40 }, (_, i) => color(i)),
    diagnostics: {
      sampled_unix_ms: Date.now(),
      display: {
        available: true,
        source: 'HDMI',
        width: 3840,
        height: 2160,
        refresh_rate: 120,
        enabled: display,
        muted: !display,
        locked: true,
      },
      video: {
        available: true,
        input: 'hdmi3',
        width: 3840,
        height: 2160,
        frame_rate: 24,
        scan_type: 'progressive',
        hdr: 'SDR',
        pixel_encoding: 'YCbCr444',
        range: 'LIMITED',
        colourimetry: 'ITU709',
        bit_depth: null,
      },
      player: { available: false },
      output: { available: true, arc_state: 'eARC', passthrough: true },
    },
  });
  const frame = (): PreviewFrame => ({
    width: 128,
    height: 72,
    captured_unix_ms: Date.now(),
    rgb: Array.from({ length: 128 * 72 }, (_, i) => {
      const x = (i % 128) / 127,
        y = Math.floor(i / 128) / 71;
      return [Math.round(x * 255), Math.round(y * 255), Math.round((1 - x) * 180)];
    }).flat(),
  });
  return {
    name: 'qsync-isolated-preview',
    configureServer(server) {
      server.middlewares.use(async (req, res, next) => {
        const path = req.url?.split('?')[0] || '';
        if (!path.startsWith('/api/')) {
          if (path === '/capture-patterns') {
            res.setHeader('Content-Type', 'text/html');
            res.end(
              '<title>Local capture patterns</title><body style="margin:0;background:linear-gradient(90deg,red,lime,blue);height:100vh"><p style="color:white">Simulated calibration pattern · no TV connection</p>',
            );
            return;
          }
          next();
          return;
        }
        res.setHeader('Content-Type', 'application/json');
        res.setHeader('Cache-Control', 'no-store');
        try {
          let raw = '';
          for await (const chunk of req) {
            raw += chunk;
            if (raw.length > 512 * 1024) throw new Error('Request too large');
          }
          const body = raw ? JSON.parse(raw) : undefined;
          let result: unknown = { status: 'ok' };
          if (path === '/api/status') result = status();
          else if (path === '/api/display') result = { enabled: display };
          else if (path === '/api/display/toggle') {
            display = !display;
            result = { enabled: display };
          } else if (path === '/api/start') running = true;
          else if (path === '/api/stop') running = false;
          else if (path === '/api/settings') {
            settings = body;
            modified = true;
          } else if (path === '/api/save-config') {
            saved = structuredClone(settings);
            overrides.set(active, {
              brightness_multiplier: saved.brightness_multiplier,
              saturation_boost: saved.saturation_boost,
              peak_weight: saved.peak_weight,
              gamma: saved.gamma,
              noise_gate_threshold: saved.noise_gate_threshold,
              strict_blackout: saved.strict_blackout,
            });
            modified = false;
          } else if (path.startsWith('/api/presets/')) {
            const name = path.split('/')[3];
            const reset = path.endsWith('/reset');
            const values =
              name === 'softColor'
                ? [0.85, 0.75, 0.1, 0.95, 0.02]
                : name === 'vividColor'
                  ? [1.1, 1.8, 0.4, 1.1, 0.025]
                  : [1, 1, 0.15, 1, 0.02];
            settings = {
              ...settings,
              ...(!reset ? overrides.get(name) : {}),
              brightness_multiplier: values[0],
              saturation_boost: values[1],
              peak_weight: values[2],
              gamma: values[3],
              noise_gate_threshold: values[4],
              strict_blackout: name === 'vividColor',
              ...(!reset ? overrides.get(name) : {}),
            };
            active = name;
            modified = false;
          } else if (path === '/api/bonjour') {
            bonjour = body.enabled;
            result = { enabled: bonjour };
          } else if (path === '/api/hue/areas')
            result = {
              areas: [{ id: 'demo-area', name: 'Preview Entertainment Area' }],
              selected_area_id: 'demo-area',
            };
          else if (path === '/api/nanoleaf/alignment')
            settings = { ...settings, nanoleaf_alignment: body };
          else if (path === '/api/preview/frame') {
            if (!running || !display) {
              res.statusCode = 409;
              result = { error: 'Capture is paused.' };
            } else result = frame();
          } else if (path === '/api/preview/render') {
            const f = body.frame as PreviewFrame,
              s = body.settings as LiveSettings;
            result = {
              ...f,
              rgb: f.rgb.map((v, i) => {
                const offset = i - (i % 3);
                const rgb = f.rgb.slice(offset, offset + 3).map((v) => v / 255);
                const max = Math.max(...rgb),
                  min = Math.min(...rgb);
                const lum = 0.2126 * rgb[0] + 0.7152 * rgb[1] + 0.0722 * rgb[2];
                if (lum < s.noise_gate_threshold) return 0;
                const sat = max ? Math.min(1, ((max - min) / max) * s.saturation_boost) : 0;
                const channel = max === min ? max : max * (1 - sat * (1 - rgb[i % 3] / max));
                let n = Math.pow(channel, s.gamma) * s.brightness_multiplier;
                if (s.hdr_tone_mapping) n = Math.sqrt(Math.max(0, n));
                return Math.round(Math.min(1, Math.max(0, n)) * 255);
              }),
            };
          } else if (
            !['/api/restart', '/api/sync-bridge', '/api/hue/area'].includes(path) &&
            !path.startsWith('/api/pair/') &&
            !path.startsWith('/api/test-pattern/')
          ) {
            res.statusCode = 404;
            result = { error: 'Unknown preview endpoint' };
          }
          res.end(JSON.stringify(result));
        } catch (error) {
          res.statusCode = 400;
          res.end(
            JSON.stringify({ error: error instanceof Error ? error.message : String(error) }),
          );
        }
      });
    },
  };
}
