import type { Dashboard } from './dashboard.svelte';
type Tool = {
  name: string;
  title: string;
  description: string;
  inputSchema: object;
  annotations: { readOnlyHint: boolean; consequentialHint?: boolean };
  execute: (input: Record<string, unknown>) => Promise<unknown>;
};
type ModelContext = {
  registerTool: (tool: Tool) => void | Promise<void>;
  unregisterTool?: (name: string) => void;
};
export function registerAgentTools(model: Dashboard) {
  const context = (document as Document & { modelContext?: ModelContext }).modelContext;
  if (!context) return () => {};
  const tools: Tool[] = [
    {
      name: 'get_ambient_sync_status',
      title: 'Get ambient sync status',
      description: 'Read current capture and lighting connection status.',
      inputSchema: { type: 'object', properties: {} },
      annotations: { readOnlyHint: true },
      execute: async () => {
        await model.refresh();
        const s = model.status;
        return {
          syncing: s?.is_syncing,
          captureFps: s?.fps,
          lightUpdateFps: s?.light_update_fps,
          hueConnected: s?.hue_connected,
          nanoleafConnected: s?.nanoleaf_connected,
        };
      },
    },
    {
      name: 'set_ambient_style',
      title: 'Set ambient colour style',
      description:
        'Recall a colour preset while preserving response and blending. Requires user confirmation.',
      inputSchema: {
        type: 'object',
        properties: { profile: { type: 'string', enum: ['softColor', 'neutral', 'vividColor'] } },
        required: ['profile'],
      },
      annotations: { readOnlyHint: false, consequentialHint: true },
      execute: async ({ profile }) => {
        if (
          typeof profile !== 'string' ||
          !['softColor', 'neutral', 'vividColor'].includes(profile)
        )
          throw Error('Unknown colour profile');
        if (!confirm(`Apply ${profile} to enabled lights?`))
          return { applied: false, cancelled: true };
        await model.preset(profile);
        if (model.error) throw Error(model.error);
        return { applied: true, profile };
      },
    },
    {
      name: 'set_sync_output',
      title: 'Enable or disable a sync output',
      description: 'Change screen, Hue or Nanoleaf output after user confirmation.',
      inputSchema: {
        type: 'object',
        properties: {
          output: { type: 'string', enum: ['screen', 'hue', 'nanoleaf'] },
          enabled: { type: 'boolean' },
        },
        required: ['output', 'enabled'],
      },
      annotations: { readOnlyHint: false, consequentialHint: true },
      execute: async ({ output, enabled }) => {
        if (!['screen', 'hue', 'nanoleaf'].includes(String(output)) || typeof enabled !== 'boolean')
          throw Error('Invalid output');
        if (!confirm(`${enabled ? 'Enable' : 'Disable'} ${output} sync?`))
          return { changed: false, cancelled: true };
        if (output === 'screen') await model.command(enabled ? '/api/start' : '/api/stop');
        else {
          if (!model.settings) throw Error('Settings unavailable');
          if (model.busy) throw Error('Another command is in progress');
          model.patch({
            [output === 'hue' ? 'hue_sync_enabled' : 'nanoleaf_sync_enabled']: enabled,
          });
          await model.save();
          if (model.error) throw Error(model.error);
        }
        return { changed: true, output, enabled };
      },
    },
  ];
  for (const tool of tools)
    Promise.resolve(context.registerTool(tool)).catch((error) =>
      console.warn('Optional agent tools unavailable', error),
    );
  return () => {
    for (const tool of tools) context.unregisterTool?.(tool.name);
  };
}
