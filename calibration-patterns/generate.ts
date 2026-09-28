import { mkdir } from 'node:fs/promises';
import { join } from 'node:path';

type Zone = {
  channel_id: number;
  name: string;
  x_min: number;
  x_max: number;
  y_min: number;
  y_max: number;
};

type Rgb = readonly [number, number, number];
const primaries: Rgb[] = [[255, 0, 0], [0, 255, 0], [0, 0, 255]];
const isolated: Rgb[] = [[255, 0, 255], [0, 255, 255], [255, 255, 0], [255, 128, 0], [128, 0, 255], [255, 255, 255]];
const outputDir = join(import.meta.dir, 'local');

export function overlaps(a: Zone, b: Zone): boolean {
  return Math.min(a.x_max, b.x_max) > Math.max(a.x_min, b.x_min)
    && Math.min(a.y_max, b.y_max) > Math.max(a.y_min, b.y_min);
}

function rgbHex(color: Rgb): string {
  return `#${color.map(component => component.toString(16).padStart(2, '0')).join('')}`;
}

function validateZones(value: unknown): Zone[] {
  if (!Array.isArray(value) || value.length === 0 || value.length > 64) throw new Error('No valid Hue channels in /api/status');
  const ids = new Set<number>();
  for (const zone of value) {
    if (!Number.isInteger(zone.channel_id) || ids.has(zone.channel_id) || typeof zone.name !== 'string') {
      throw new Error('Hue channel ID or name is invalid');
    }
    ids.add(zone.channel_id);
    for (const key of ['x_min', 'x_max', 'y_min', 'y_max'] as const) {
      if (!Number.isFinite(zone[key]) || zone[key] < 0 || zone[key] > 1) throw new Error(`Invalid ${key} for channel ${zone.channel_id}`);
    }
    if (zone.x_min >= zone.x_max || zone.y_min >= zone.y_max) throw new Error(`Empty sampling zone for channel ${zone.channel_id}`);
  }
  return [...value].sort((a, b) => a.channel_id - b.channel_id);
}

export function assignColors(zones: Zone[]): Map<number, Rgb> {
  const colors = new Map<number, Rgb>();
  const visited = new Set<number>();
  let soloIndex = 0;
  for (const zone of zones) {
    if (visited.has(zone.channel_id)) continue;
    const component: Zone[] = [];
    const queue = [zone];
    visited.add(zone.channel_id);
    while (queue.length) {
      const current = queue.shift()!;
      component.push(current);
      for (const other of zones) {
        if (!visited.has(other.channel_id) && overlaps(current, other)) {
          visited.add(other.channel_id);
          queue.push(other);
        }
      }
    }
    component.sort((a, b) => a.channel_id - b.channel_id);
    if (component.length === 1) {
      colors.set(zone.channel_id, isolated[soloIndex++ % isolated.length]);
      continue;
    }
    // Distinct RGB primaries make shared pixels visibly additive; >3 overlapping
    // channels can reuse a primary only when those two rectangles do not overlap.
    const paint = (index: number): boolean => {
      if (index === component.length) return true;
      const current = component[index];
      const used = new Set(component.slice(0, index).map(item => colors.get(item.channel_id)));
      for (const color of [...primaries].sort((a, b) => Number(used.has(a)) - Number(used.has(b)))) {
        if (component.slice(0, index).some(item => overlaps(current, item) && colors.get(item.channel_id) === color)) continue;
        colors.set(current.channel_id, color);
        if (paint(index + 1)) return true;
      }
      colors.delete(current.channel_id);
      return false;
    };
    if (!paint(0)) throw new Error('Overlap graph needs more than three primary colors; use individual channel masks');
  }
  return colors;
}

function rect(zone: Zone, color: Rgb): string {
  return `<rect x="${zone.x_min * 3840}" y="${zone.y_min * 2160}" width="${(zone.x_max - zone.x_min) * 3840}" height="${(zone.y_max - zone.y_min) * 2160}" fill="${rgbHex(color)}"/>`;
}

function svg(body: string): string {
  return `<svg xmlns="http://www.w3.org/2000/svg" width="3840" height="2160" viewBox="0 0 3840 2160" shape-rendering="crispEdges"><rect width="3840" height="2160" fill="#000000"/>${body}</svg>\n`;
}

export function compositeSvg(zones: Zone[], colors: Map<number, Rgb>): string {
  const xs = [...new Set([0, 1, ...zones.flatMap(zone => [zone.x_min, zone.x_max])])].sort((a, b) => a - b);
  const ys = [...new Set([0, 1, ...zones.flatMap(zone => [zone.y_min, zone.y_max])])].sort((a, b) => a - b);
  const tiles: string[] = [];
  for (let yi = 0; yi < ys.length - 1; yi++) {
    for (let xi = 0; xi < xs.length - 1; xi++) {
      const centerX = (xs[xi] + xs[xi + 1]) / 2;
      const centerY = (ys[yi] + ys[yi + 1]) / 2;
      const covering = zones.filter(zone => centerX >= zone.x_min && centerX < zone.x_max && centerY >= zone.y_min && centerY < zone.y_max);
      if (!covering.length) continue;
      const color: Rgb = covering.reduce<Rgb>((sum, zone) => {
        const next = colors.get(zone.channel_id)!;
        return [Math.min(255, sum[0] + next[0]), Math.min(255, sum[1] + next[1]), Math.min(255, sum[2] + next[2])];
      }, [0, 0, 0]);
      tiles.push(rect({ ...covering[0], x_min: xs[xi], x_max: xs[xi + 1], y_min: ys[yi], y_max: ys[yi + 1] }, color));
    }
  }
  return svg(tiles.join(''));
}

async function writeImage(name: string, content: string): Promise<void> {
  const source = join(outputDir, `${name}.svg`);
  const destination = join(outputDir, `${name}.png`);
  await Bun.write(source, content);
  const process = Bun.spawn(['rsvg-convert', '-w', '3840', '-h', '2160', '-o', destination, source], { stdout: 'ignore', stderr: 'inherit' });
  if (await process.exited !== 0) throw new Error(`Could not render ${name}.png`);
}

if (import.meta.main) {
  const base = process.argv[2];
  if (!base || !/^https?:\/\//.test(base)) throw new Error('Usage: bun run calibration-patterns/generate.ts http://<tv-ip>:8088');
  const response = await fetch(new URL('/api/status', base));
  if (!response.ok) throw new Error(`Dashboard status returned HTTP ${response.status}`);
  const zones = validateZones((await response.json()).hue_zones);
  const colors = assignColors(zones);
  await mkdir(outputDir, { recursive: true });
  await writeImage('00-current-area-composite', compositeSvg(zones, colors));
  for (const zone of zones) {
    await writeImage(`channel-${String(zone.channel_id).padStart(2, '0')}`, svg(rect(zone, colors.get(zone.channel_id)!)));
  }
  await Bun.write(join(outputDir, 'mapping.json'), JSON.stringify({ generated_at: new Date().toISOString(), channels: zones.map(zone => ({ ...zone, color: rgbHex(colors.get(zone.channel_id)!), overlaps: zones.filter(other => other !== zone && overlaps(zone, other)).map(other => other.channel_id) })) }, null, 2));
  console.log(`Rendered current Hue layout: ${zones.length} channel masks plus composite in ${outputDir}`);
}
