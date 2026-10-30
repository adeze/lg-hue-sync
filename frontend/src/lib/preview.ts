import type {
  HueDevice,
  HueZone,
  PreviewFrame,
  PreviewSettings,
  PreviewState,
  Rgb,
} from './preview-types';

export function validatePreviewFrame(value: unknown): asserts value is PreviewFrame {
  const frame = value as PreviewFrame | null;
  if (
    !frame ||
    !Number.isInteger(frame.width) ||
    !Number.isInteger(frame.height) ||
    frame.width < 1 ||
    frame.height < 1 ||
    frame.width > 128 ||
    frame.height > 72 ||
    !Array.isArray(frame.rgb) ||
    frame.rgb.length !== frame.width * frame.height * 3 ||
    !frame.rgb.every((value) => Number.isInteger(value) && value >= 0 && value <= 255) ||
    !Number.isFinite(frame.captured_unix_ms) ||
    frame.captured_unix_ms <= 0
  ) {
    throw new Error('The TV returned an invalid preview frame.');
  }
}

export function drawPreview(canvas: HTMLCanvasElement, frame: PreviewFrame): void {
  validatePreviewFrame(frame);
  const context = canvas.getContext('2d');
  if (!context) throw new Error('Image preview is unavailable in this browser.');
  const image = context.createImageData(frame.width, frame.height);
  for (let pixel = 0; pixel < frame.width * frame.height; pixel++) {
    image.data.set([...frame.rgb.slice(pixel * 3, pixel * 3 + 3), 255], pixel * 4);
  }
  canvas.width = frame.width;
  canvas.height = frame.height;
  context.putImageData(image, 0, 0);
}

export function groupHueZones(zones: HueZone[]): HueDevice[] {
  const devices = new Map<string, HueDevice>();
  for (const zone of zones) {
    const key = zone.hue_device_id || `channel:${zone.channel_id}`;
    if (!devices.has(key)) {
      devices.set(key, {
        ...zone,
        key,
        name: zone.name.replace(/ · Segments? [\d,\s–]+$/, ''),
        channels: [],
        positions: [],
      });
    }
    const device = devices.get(key)!;
    device.channels.push(zone);
    for (const point of zone.hue_positions || []) {
      if (
        !device.positions.some((existing) => existing.every((value, axis) => value === point[axis]))
      )
        device.positions.push(point);
    }
    device.x_min = Math.min(device.x_min, zone.x_min);
    device.x_max = Math.max(device.x_max, zone.x_max);
    device.y_min = Math.min(device.y_min, zone.y_min);
    device.y_max = Math.max(device.y_max, zone.y_max);
  }
  return [...devices.values()];
}

export function hueOrientation(positions: number[][]): string {
  if (positions.length < 2) return '';
  if (positions.length > 2) return 'Multi-point layout';
  const delta = positions[0].map((value, axis) => Math.abs(positions[1][axis] - value));
  const largest = Math.max(...delta);
  if (largest < 0.001) return '';
  const dominant = delta.indexOf(largest);
  if (delta.some((value, axis) => axis !== dominant && value > largest * 0.1)) return 'Angled';
  return ['Left ↔ right', 'Front ↔ back', 'Up ↔ down'][dominant];
}

export function orientationDescription(positions: number[][]): string {
  const orientation = hueOrientation(positions);
  return orientation === 'Front ↔ back'
    ? 'Along room depth; monitor end is front, toward the TV wall. Does not indicate LED facing.'
    : orientation;
}

export function orientationPath(positions: number[][]): string {
  const paths: Record<string, string> = {
    'Front ↔ back':
      'M5 3h22v12H5ZM12 18h8m-4-3v3m0 5v10m-5-5 5-5 5 5m-5 5a2 2 0 1 0 0 4 2 2 0 0 0 0-4',
    'Left ↔ right': 'M4 20h24M10 14l-6 6 6 6m12-12 6 6-6 6',
    'Up ↔ down': 'M16 5v30M10 11l6-6 6 6m-12 18 6 6 6-6',
    Angled: 'M5 31 27 9M5 23v8h8M19 9h8v8',
    'Multi-point layout': 'M5 30V11h22v19',
  };
  return paths[hueOrientation(positions)] || '';
}

export function deviceColor(device: HueDevice, colors: [number, Rgb][]): Rgb {
  const byChannel = new Map(colors);
  const samples = device.channels
    .map((zone) => byChannel.get(zone.channel_id))
    .filter((color): color is Rgb => Boolean(color));
  const average = (component: keyof Rgb) =>
    samples.length
      ? Math.round(samples.reduce((sum, color) => sum + color[component], 0) / samples.length)
      : 0;
  return { r: average('r'), g: average('g'), b: average('b') };
}

export function rgbCss(color?: Rgb, alpha?: number): string {
  const rgb = [color?.r || 0, color?.g || 0, color?.b || 0].map((value) =>
    Math.max(0, Math.min(255, value)),
  );
  return alpha === undefined ? `rgb(${rgb.join(',')})` : `rgba(${rgb.join(',')},${alpha})`;
}

async function readResponse(response: Response): Promise<unknown> {
  let data: unknown;
  try {
    data = await response.json();
  } catch {
    throw new Error('The TV returned an unreadable preview response.');
  }
  if (!response.ok) {
    const error = (data as { error?: unknown } | null)?.error;
    throw new Error(typeof error === 'string' ? error.slice(0, 240) : 'Preview request failed.');
  }
  return data;
}

function waitRetry(signal: AbortSignal, delay: number): Promise<void> {
  return new Promise((resolve, reject) => {
    const stop = () => {
      clearTimeout(timer);
      reject(new DOMException('Aborted', 'AbortError'));
    };
    const timer = setTimeout(() => {
      signal.removeEventListener('abort', stop);
      resolve();
    }, delay);
    signal.addEventListener('abort', stop, { once: true });
    if (signal.aborted) stop();
  });
}

export class FrozenPreview {
  private state: PreviewState = {
    busy: false,
    frame: null,
    rendered: null,
    message: 'Capture an HDMI frame to preview colour adjustments.',
  };
  private settings: PreviewSettings | null = null;
  private active = false;
  private supported = false;
  private captureRequested = false;
  private renderRequested = false;
  private timer: ReturnType<typeof setTimeout> | undefined;
  private controller: AbortController | undefined;
  private generation = 0;
  private settingsKey = '';
  private retries = 0;

  constructor(
    private publish: (state: PreviewState) => void,
    private fetcher: (input: RequestInfo | URL, init?: RequestInit) => Promise<Response> = (
      ...args
    ) => fetch(...args),
  ) {}

  update(settings: PreviewSettings | null, active: boolean, supported: boolean): void {
    const settingsKey = JSON.stringify(settings);
    const changed = this.settingsKey !== settingsKey;
    const resumed = !this.active && active;
    const becameSupported = !this.supported && supported;
    this.settings = settings;
    this.settingsKey = settingsKey;
    this.active = active;
    this.supported = supported;
    if (!active) this.cancel();
    else {
      if ((changed || resumed) && this.state.frame && settings) this.queueRender();
      if (becameSupported && !this.state.frame && !this.state.busy)
        this.state.message = 'Capture an HDMI frame to preview colour adjustments.';
      if (!supported) {
        this.captureRequested = false;
        if (!this.state.busy)
          this.state.message = this.state.frame
            ? 'Frozen frame retained. New frames need active HDMI screen sync.'
            : 'Play an HDMI source and start screen sync to capture a frame. Built-in apps are not supported.';
      }
    }
    this.emit();
  }

  capture(): void {
    if (!this.active || !this.supported || this.state.busy || !this.settings) return;
    this.retries = 0;
    this.captureRequested = true;
    void this.run();
  }

  cancel(): void {
    this.generation++;
    this.controller?.abort();
    clearTimeout(this.timer);
    this.timer = undefined;
    this.captureRequested = this.renderRequested = false;
  }

  private emit(): void {
    this.publish({ ...this.state });
  }

  private queueRender(): void {
    this.renderRequested = true;
    this.retries = 0;
    clearTimeout(this.timer);
    this.timer = setTimeout(() => {
      this.timer = undefined;
      void this.run();
    }, 200);
  }

  private async run(): Promise<void> {
    if (this.state.busy || !this.active || !this.settings) return;
    const capture = this.captureRequested && this.supported;
    if (!capture && (!this.renderRequested || this.timer || !this.state.frame)) return;
    this.state.busy = true;
    this.emit();
    const generation = this.generation;
    const controller = new AbortController();
    this.controller = controller;
    const deadlineTimer = setTimeout(() => controller.abort(), 5000);
    try {
      if (capture) {
        this.captureRequested = false;
        if (!this.state.frame) this.state.message = 'Waiting for a captured HDMI frame…';
        this.emit();
        const deadline = Date.now() + 4000;
        let response: Response;
        do {
          response = await this.fetcher('/api/preview/frame', {
            cache: 'no-store',
            signal: controller.signal,
          });
          if (response.status !== 202) break;
          if (Date.now() >= deadline)
            throw new Error('No HDMI frame arrived. Check that screen sync is running.');
          await waitRetry(controller.signal, 250);
        } while (!controller.signal.aborted);
        const frame = await readResponse(response);
        if (generation !== this.generation || !this.active) return;
        validatePreviewFrame(frame);
        this.state.frame = frame;
        this.renderRequested = true;
      } else {
        this.renderRequested = false;
        const frame = this.state.frame;
        const settings = this.settings;
        const key = JSON.stringify(settings);
        const response = await this.fetcher('/api/preview/render', {
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify({ frame, settings }),
          signal: controller.signal,
        });
        if (response.status === 429 && this.retries++ < 2) {
          this.renderRequested = true;
          this.timer = setTimeout(() => {
            this.timer = undefined;
            void this.run();
          }, 300);
          return;
        }
        const rendered = await readResponse(response);
        if (generation !== this.generation || !this.active) return;
        if (key !== this.settingsKey) {
          this.renderRequested = true;
          return;
        }
        validatePreviewFrame(rendered);
        this.state.rendered = rendered;
        this.state.message = `Frozen frame captured at ${new Date(frame!.captured_unix_ms).toLocaleTimeString()}.`;
      }
    } catch (error) {
      if (generation === this.generation && this.active) {
        this.state.message = controller.signal.aborted
          ? 'Preview timed out. Try capturing another frame.'
          : error instanceof Error
            ? error.message
            : 'Preview request failed.';
      }
    } finally {
      clearTimeout(deadlineTimer);
      this.state.busy = false;
      this.controller = undefined;
      this.emit();
      if (this.active && (this.captureRequested || (this.renderRequested && !this.timer)))
        void this.run();
    }
  }
}
