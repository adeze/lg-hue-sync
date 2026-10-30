export interface LiveSettings {
  brightness_multiplier: number;
  hue_output_brightness: number;
  nanoleaf_output_brightness: number;
  saturation_boost: number;
  peak_weight: number;
  spatial_blend: number;
  depth_influence: number;
  gamma: number;
  noise_gate_threshold: number;
  smoothing_factor: number;
  rise_smoothing_factor: number;
  fall_smoothing_factor: number;
  strict_blackout: boolean;
  use_xy_gamut: boolean;
  letterbox_detection: boolean;
  hdr_tone_mapping: boolean;
  hue_sync_enabled: boolean;
  nanoleaf_sync_enabled: boolean;
  auto_tv_power: boolean;
  nanoleaf_alignment: {
    start_corner: string;
    reverse_direction: boolean;
    perimeter_offset: number;
  };
  max_color_step: number;
}
export const initialSettings: LiveSettings = {
  brightness_multiplier: 1,
  hue_output_brightness: 1,
  nanoleaf_output_brightness: 1,
  saturation_boost: 1,
  peak_weight: 0.15,
  spatial_blend: 0.15,
  depth_influence: 0.5,
  gamma: 1,
  noise_gate_threshold: 0.02,
  smoothing_factor: 0.35,
  rise_smoothing_factor: 0.35,
  fall_smoothing_factor: 0.35,
  strict_blackout: false,
  use_xy_gamut: false,
  letterbox_detection: true,
  hdr_tone_mapping: false,
  hue_sync_enabled: true,
  nanoleaf_sync_enabled: true,
  auto_tv_power: true,
  nanoleaf_alignment: {
    start_corner: 'bottom-center',
    reverse_direction: false,
    perimeter_offset: 0,
  },
  max_color_step: 12,
};
export type NumericKey =
  | 'brightness_multiplier'
  | 'saturation_boost'
  | 'peak_weight'
  | 'spatial_blend'
  | 'gamma'
  | 'smoothing_factor'
  | 'max_color_step'
  | 'noise_gate_threshold'
  | 'hue_output_brightness'
  | 'nanoleaf_output_brightness';
export const faders: {
  key: NumericKey;
  label: string;
  stops: number[];
  format: (v: number) => string;
  help: string;
}[] = [
  {
    key: 'brightness_multiplier',
    label: 'Brightness',
    stops: [0.5, 1, 1.5, 2, 2.5],
    format: (v) => `${Number(v.toFixed(2))}×`,
    help: 'Scales shared lighting brightness; device output trims remain independent.',
  },
  {
    key: 'saturation_boost',
    label: 'Saturation',
    stops: [0.5, 1, 1.5, 2, 2.5],
    format: (v) => `${Number(v.toFixed(2))}×`,
    help: 'Below 1 reduces colour intensity; 1 is unchanged. Higher values push colours towards full saturation.',
  },
  {
    key: 'peak_weight',
    label: 'Highlights',
    stops: [0, 0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8, 0.9, 1],
    format: (v) => `${Math.round(v * 100)}%`,
    help: 'Weights bright highlights more strongly than average screen colour. The frozen colour preview does not simulate peak sampling.',
  },
  {
    key: 'spatial_blend',
    label: 'Blend',
    stops: [0, 0.05, 0.1, 0.15, 0.2, 0.25, 0.3, 0.35, 0.4, 0.45, 0.5],
    format: (v) => `${Math.round(v * 100)}%`,
    help: 'Mixes neighbouring screen regions with local colour. Black regions remain off. The preview overlay shows sampling geometry, not this mixing.',
  },
  {
    key: 'gamma',
    label: 'Gamma',
    stops: [0.5, 0.6, 0.7, 0.85, 1, 1.2, 1.4, 1.7, 2],
    format: (v) => v.toFixed(2),
    help: 'Above 1 darkens midtones; below 1 lifts them. Neutral is 1.',
  },
  {
    key: 'smoothing_factor',
    label: 'Response',
    stops: [0.05, 0.08, 0.12, 0.18, 0.25, 0.35, 0.5, 0.65, 0.8, 1],
    format: (v) => `${Math.round(v * 100)}%`,
    help: 'Higher values follow colour changes faster; 100% removes temporal smoothing. Moving this fader makes rise and fall response equal.',
  },
  {
    key: 'max_color_step',
    label: 'Scene limit',
    stops: [4, 6, 8, 12, 16, 24, 32, 48, 64],
    format: (v) => `${v} steps`,
    help: 'Maximum RGB component change per 1/30 second, including scene cuts. Lower values produce gentler transitions.',
  },
  {
    key: 'noise_gate_threshold',
    label: 'Black gate',
    stops: [0, 0.005, 0.01, 0.015, 0.02, 0.03, 0.04, 0.06],
    format: (v) => `${Number((v * 100).toFixed(1))}%`,
    help: 'Treats very dark screen regions as black. Strict black turns those lights fully off.',
  },
];
export const colourModes = [
  {
    value: 'softColor',
    label: 'Soft',
    help: 'Subdued saturation, reduced intensity and gentle highlights.',
  },
  {
    value: 'neutral',
    label: 'Default',
    help: 'Neutral saturation, brightness and gamma with balanced highlights.',
  },
  {
    value: 'vividColor',
    label: 'Vivid',
    help: 'Richer saturation, stronger highlights and deeper contrast.',
  },
];
export const responseModes = {
  low: [0.2, 0.18, 0.22, 6],
  default: [0.35, 0.35, 0.35, 12],
  fast: [0.65, 0.65, 0.7, 14],
} as const;
export const blendModes = { local: 0, balanced: 0.15, soft: 0.3 } as const;
export function nearestStop(value: number, stops: readonly number[]) {
  return stops.reduce(
    (best, stop, i) => (Math.abs(stop - value) < Math.abs(stops[best] - value) ? i : best),
    0,
  );
}
export function responseMode(settings: LiveSettings | null) {
  return (
    Object.entries(responseModes).find(
      ([, v]) =>
        settings &&
        [
          settings.smoothing_factor,
          settings.rise_smoothing_factor,
          settings.fall_smoothing_factor,
          settings.max_color_step,
        ].every((n, i) => Math.abs(n - v[i]) < 0.00001),
    )?.[0] ?? ''
  );
}
export function blendMode(settings: LiveSettings | null) {
  return (
    Object.entries(blendModes).find(
      ([, v]) => settings && Math.abs(settings.spatial_blend - v) < 0.00001,
    )?.[0] ?? ''
  );
}
