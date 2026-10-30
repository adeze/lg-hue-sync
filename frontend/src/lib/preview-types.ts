export interface Rgb {
  r: number;
  g: number;
  b: number;
}

export interface HueZone {
  channel_id: number;
  name: string;
  hue_device_id?: string | null;
  hue_positions?: number[][];
  x_min: number;
  x_max: number;
  y_min: number;
  y_max: number;
}

export interface HueDevice extends HueZone {
  key: string;
  channels: HueZone[];
  positions: number[][];
}

export interface PreviewSettings {
  brightness_multiplier: number;
  saturation_boost: number;
  gamma: number;
  noise_gate_threshold: number;
  hdr_tone_mapping: boolean;
}

export interface PreviewStatus {
  pipeline_state: string;
  capture_hardware: boolean;
  hue_zones: HueZone[];
  hue_colors: [number, Rgb][];
  nanoleaf_colors: Rgb[];
}

export interface PreviewFrame {
  width: number;
  height: number;
  rgb: number[];
  captured_unix_ms: number;
}

export interface PreviewState {
  busy: boolean;
  frame: PreviewFrame | null;
  rendered: PreviewFrame | null;
  message: string;
}
