<script lang="ts">
  import { onMount } from 'svelte';
  import { Download } from '@lucide/svelte';
  import { Button } from './components/ui/button';

  interface InstallPrompt extends Event {
    prompt(): Promise<void>;
    userChoice: Promise<{ outcome: 'accepted' | 'dismissed' }>;
  }
  let prompt = $state<InstallPrompt | null>(null);
  let installed = $state(false);
  let busy = $state(false);
  let message = $state('');
  onMount(() => {
    const standalone = window.matchMedia('(display-mode: standalone)');
    const update = () => {
      installed =
        standalone.matches ||
        Boolean((navigator as Navigator & { standalone?: boolean }).standalone);
    };
    const available = (event: Event) => {
      event.preventDefault();
      prompt = event as InstallPrompt;
    };
    const complete = () => {
      installed = true;
      prompt = null;
    };
    update();
    window.addEventListener('beforeinstallprompt', available);
    window.addEventListener('appinstalled', complete);
    standalone.addEventListener('change', update);
    return () => {
      window.removeEventListener('beforeinstallprompt', available);
      window.removeEventListener('appinstalled', complete);
      standalone.removeEventListener('change', update);
    };
  });
  async function install() {
    if (!prompt) return;
    const pending = prompt;
    prompt = null;
    busy = true;
    try {
      await pending.prompt();
      const choice = await pending.userChoice;
      message =
        choice.outcome === 'accepted'
          ? 'Installation requested.'
          : 'Installation dismissed. You can still use the browser menu.';
    } catch {
      message = 'Use your browser menu to add QSync.';
    } finally {
      busy = false;
    }
  }
</script>

<section class="card grid gap-3" aria-labelledby="install-heading">
  <div class="flex items-center gap-3">
    <img src="/icon-192.png" alt="" width="44" height="44" />
    <h2 id="install-heading">QSync app</h2>
  </div>
  {#if installed}
    <p>QSync is running as an installed app.</p>
  {:else}
    <p>Add this dashboard to your Home Screen or Dock for quick access.</p>
    {#if prompt || busy}
      <Button onclick={install} disabled={busy}><Download /> Install QSync</Button>
    {/if}
    <p>
      iPhone / iPad: Safari → Share → Add to Home Screen → Open as Web App. Mac: Safari → File → Add
      to Dock.
    </p>
    <p>
      Chrome / Edge: use Install app when offered. Automatic installation requires HTTPS; a plain
      HTTP TV address may only offer a shortcut.
    </p>
    <p>
      The TV must be reachable on your network. Controls and captured frames are not cached for
      offline use.
    </p>
    {#if message}<p role="status">{message}</p>{/if}
  {/if}
</section>
