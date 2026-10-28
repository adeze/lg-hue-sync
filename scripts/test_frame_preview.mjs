import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';

const html = readFileSync(new URL('../src/web/ui.html', import.meta.url), 'utf8');
const start = html.indexOf('    // Captured-frame preview:');
const end = html.indexOf('    // End captured-frame preview.', start);
assert.ok(start > 0 && end > start);
assert.ok(!html.includes('id="panelOverview"'));
const tuning = html.slice(html.indexOf('id="panelTuning"'), html.indexOf('id="panelDevices"'));
for (const id of ['previewColumn', 'frameEditor', 'presetSection', 'advancedSection']) assert.ok(tuning.includes(`id="${id}"`));
const devices = html.slice(html.indexOf('id="panelDevices"'), html.indexOf('id="panelDiagnostics"'));
for (const id of ['devicesSection', 'calibrationSection', 'maintenanceSection']) assert.ok(devices.includes(`id="${id}"`));
assert.ok(!html.includes('id="framePreviewBefore"') && !html.includes('id="lightLayoutSection"'));
assert.ok(tuning.indexOf('id="presetSection"') < tuning.indexOf('id="frameEditor"'));
for (const id of ['framePreviewAfter', 'hueZoneOverlay', 'topLeds', 'bottomLeds', 'leftLeds', 'rightLeds']) assert.equal([...html.matchAll(new RegExp(`id="${id}"`, 'g'))].length, 1);
const advanced = tuning.slice(tuning.indexOf('id="advancedSection"'));
const styles = tuning.slice(tuning.indexOf('id="presetSection"'), tuning.indexOf('id="frameEditor"'));
for (const id of ['saveButton', 'resetPresetButton']) {
  assert.ok(advanced.includes(`id="${id}"`) && !styles.includes(`id="${id}"`));
}
assert.ok(!html.includes('pinFramePreview') && !html.includes('is-pinned'));
assert.ok(html.includes('aria-label="About colour preview"'));
const advancedRanges = [...advanced.matchAll(/<input type="range"[^>]+>/g)].map(match => match[0]);
assert.equal(advancedRanges.length, 8);
assert.ok(advancedRanges.every(input => input.includes('aria-orientation="vertical"')));
assert.ok(!devices.includes('aria-orientation="vertical"'), 'Device ranges must stay horizontal');
assert.match(html, /#advancedSection \.fader-bank \{ display:flex; flex-wrap:nowrap;[^}]*overflow-x:auto/);
assert.match(html, /writing-mode:vertical-lr; direction:rtl; width:48px; height:160px/);
assert.match(html, /touch-action:pan-x/);
assert.match(html, /:is\(:active,:focus-visible\)::-webkit-slider-thumb \{ transform:scale\(1\.3\)/);
assert.ok(!html.includes('grid-template-columns:minmax(80px,1fr) minmax(0,4fr)'));
assert.ok(tuning.indexOf('id="framePreviewImage"') < tuning.indexOf('class="frame-editor-actions"'), 'Toolbar belongs below the image');
const source = html.slice(start, end);
const elements = new Map();
const draws = [];
const element = id => {
  if (!elements.has(id)) elements.set(id, {
    hidden: false, open: true, checked: false, disabled: false, style: {},
    addEventListener() {},
    getContext() { return {
      createImageData: (width, height) => ({ data: new Uint8ClampedArray(width * height * 4) }),
      putImageData: image => draws.push({ id, rgb: [...image.data] })
    }; }
  });
  return elements.get(id);
};
const document = { hidden: false, getElementById: element, addEventListener() {} };
const timers = new Map();
let timerId = 0;
const setTimer = (fn, delay) => { timers.set(++timerId, { fn, delay }); return timerId; };
const clearTimer = id => timers.delete(id);
const fire = delay => {
  const timer = [...timers].find(([, item]) => item.delay === delay);
  assert.ok(timer, `Expected ${delay}ms timer`);
  timers.delete(timer[0]); timer[1].fn();
};
const flush = async () => { for (let i = 0; i < 20; i++) await Promise.resolve(); };
const frame = { width: 1, height: 1, rgb: [30, 60, 90], captured_unix_ms: Date.now() };
let settings = { gamma: 1 };
let inflight = 0;
let maximumInflight = 0;
let gets = 0;
const posts = [];
const fetch = (url, options) => {
  inflight++; maximumInflight = Math.max(maximumInflight, inflight);
  if (url.endsWith('/frame')) {
    const response = ++gets === 1
      ? { status: 202, ok: true, json: async () => ({ error: 'Waiting for a captured frame.' }) }
      : { status: 200, ok: true, json: async () => frame };
    inflight--;
    return Promise.resolve(response);
  }
  return new Promise(resolve => posts.push({
    body: JSON.parse(options.body),
    resolve(rgb) {
      inflight--;
      resolve({ status: 200, ok: true, json: async () => ({ ...frame, rgb }) });
    }
  }));
};
const api = new Function('document', 'fetch', 'readLiveSettings', 'setTimeout', 'clearTimeout', `
  let lastStatus = { pipeline_state: 'running', capture_hardware: true };
  ${source}
  return { framePreview, drawFramePreview, queueFramePreviewRender, runFramePreviewWork,
    syncFramePreviewActivity, updateFramePreviewAvailability, scheduleLiveFramePreview,
    setStatus: value => { lastStatus = value; } };
`)(document, fetch, () => settings, setTimer, clearTimer);
assert.equal(posts.length + gets, 0, 'Preview must be idle by default');
assert.throws(() => api.drawFramePreview('framePreviewAfter', { ...frame, width: 129 }), /invalid preview/);
assert.throws(() => api.drawFramePreview('framePreviewAfter', { ...frame, rgb: [NaN, 1, 2] }), /invalid preview/);
api.drawFramePreview('framePreviewAfter', frame);
assert.deepEqual(draws.at(-1).rgb, [30, 60, 90, 255]);
draws.length = 0;
api.framePreview.frame = frame;
element('framePreviewStatus').textContent = 'Frozen reference';
element('advancedSection').open = false;
api.queueFramePreviewRender();
settings = { gamma: 2 };
api.queueFramePreviewRender();
fire(200);
assert.equal(posts.length, 1);
assert.equal(element('framePreviewStatus').textContent, 'Frozen reference', 'Rendering must not replace a stable preview status');
assert.equal(posts[0].body.settings.gamma, 2, 'Preview must stay available with advanced controls collapsed');
settings = { gamma: 3 };
api.queueFramePreviewRender();
fire(200);
assert.equal(posts.length, 1, 'Slider input must not overlap a render');
posts[0].resolve([100, 100, 100]);
await flush();
assert.equal(posts.length, 2);
assert.equal(posts[1].body.settings.gamma, 3);
assert.equal(draws.filter(draw => draw.id === 'framePreviewAfter').length, 0, 'Stale render must be discarded');
posts[1].resolve([10, 20, 30]);
await flush();
assert.deepEqual(draws.at(-1).rgb, [10, 20, 30, 255]);
assert.equal(api.framePreview.frame, frame, 'Frozen source must not be replaced by rendered pixels');
api.framePreview.captureRequested = true;
api.runFramePreviewWork();
await flush();
assert.equal(gets, 1);
fire(250);
await flush();
assert.equal(gets, 2);
assert.equal(posts.length, 3);
posts[2].resolve([12, 24, 36]);
await flush();
assert.equal(maximumInflight, 1, 'Frame and render requests must be serialized');
element('liveFramePreview').checked = true;
api.scheduleLiveFramePreview();
assert.ok([...timers.values()].some(timer => timer.delay > 0 && timer.delay <= 1000));
element('panelTuning').hidden = true;
api.syncFramePreviewActivity();
assert.equal(timers.size, 0, 'Hidden panel must release polling and render timers');
assert.equal(api.framePreview.captureRequested, false);
element('panelTuning').hidden = false;
api.setStatus({ pipeline_state: 'unsupported_source', capture_hardware: true });
api.updateFramePreviewAvailability();
assert.equal(element('capturePreviewFrame').disabled, true);
assert.match(element('framePreviewStatus').textContent, /Frozen frame retained/);
const presetButtons = ['neutral', 'highChroma'].map(name => ({
  dataset: { preset: name }, textContent: name, disabled: false, attributes: {},
  classList: { toggle() {} }, setAttribute(key, value) { this.attributes[key] = value; }
}));
document.querySelectorAll = () => presetButtons;
const presetSource = html.slice(html.indexOf('    function updatePresetUI('), html.indexOf('    async function updateBonjour('));
const saveSource = html.slice(html.indexOf('    async function saveSettingsToDisk('), html.indexOf('    async function restartPipeline('));
const server = { active_preset: 'neutral', preset_modified: true };
const actions = [];
let failSave = false;
let presets;
const presetFetch = async url => {
  actions.push(url);
  if (url.endsWith('/reset')) server.preset_modified = true;
  else if (url.endsWith('/save-config')) {
    if (failSave) return { ok: false, status: 503 };
    server.preset_modified = false;
  } else { server.active_preset = url.split('/').at(-1); server.preset_modified = false; }
  return { ok: true, status: 200 };
};
presets = new Function('document', 'fetch', 'pollStatus', `
  let lastStatus = { active_preset: 'neutral', preset_modified: true };
  let presetPending = false, settingsPending = 0, presetNotice = null;
  ${presetSource}
  ${saveSource}
  return { applyPreset, resetActivePreset, saveSettingsToDisk,
    setStatus(value) { lastStatus = { ...value }; updatePresetUI(lastStatus); },
    setSettingsPending(value) { settingsPending = value; updatePresetUI(lastStatus); } };
`)(document, presetFetch, async () => presets.setStatus(server));
presets.setStatus(server);
assert.equal(presetButtons[0].attributes['aria-pressed'], 'true');
await presets.applyPreset('neutral');
assert.equal(actions.length, 0, 'Clicking the active style must preserve edits');
presets.setSettingsPending(1);
await presets.applyPreset('highChroma');
await presets.resetActivePreset();
await presets.saveSettingsToDisk();
assert.equal(actions.length, 0, 'Style actions must wait for in-flight slider settings');
assert.equal(element('saveButton').disabled, true);
presets.setSettingsPending(0);
await presets.applyPreset('highChroma');
assert.equal(presetButtons[1].attributes['aria-pressed'], 'true');
await presets.resetActivePreset();
assert.equal(actions.at(-1), '/api/presets/highChroma/reset');
assert.equal(server.active_preset, 'highChroma');
assert.equal(server.preset_modified, true, 'Reset must remain live until explicitly saved');
failSave = true;
await presets.saveSettingsToDisk();
assert.match(element('presetStatus').textContent, /Could not save/);
assert.equal(element('saveButton').disabled, false, 'Failed save must release the action lock');
failSave = false;
await presets.saveSettingsToDisk();
assert.equal(server.preset_modified, false);
assert.match(element('presetStatus').textContent, /Settings saved/);
for (const [id, step] of [['sliderBrightness', '0.05'], ['sliderSaturation', '0.05'], ['sliderGamma', '0.01']]) {
  assert.match(html, new RegExp('id="' + id + '"[^>]*step="' + step.replace('.', '\\.') + '"'), 'Factory preset precision must fit the slider');
}
const helpStart = html.indexOf('    // Fader help follows');
const helpEnd = html.indexOf('    // End fader help.', helpStart);
const explain = new Function('document', html.slice(helpStart, helpEnd) + '; return showTuningHelp;')({
  getElementById: element, querySelector: () => ({ addEventListener() {} })
});
explain({ id:'sliderColorStep', value:'6', getAttribute: ()=>'Scene-Change Limiter' });
assert.match(element('tuningHelpText').textContent, /6 steps per 1\/30 second/);
assert.match(element('tuningHelpText').textContent, /not a scene count/);
explain({ id:'sliderSaturation', value:'1.5', getAttribute: ()=>'Saturation Boost' });
assert.equal(element('tuningHelpTitle').textContent, 'Saturation Boost');
console.log('Frame/preset UI: bounded RGB, latest-wins serialization, visibility, single overlay canvas, active-style preservation and explicit save/reset passed.');
