<script lang="ts">
  import { onMount } from 'svelte';
  import { Button } from './components/ui/button';
  import { companionOrigin } from './streaming';

  let { active = false }: { active?: boolean } = $props();
  let address = $state('');
  let origin = $state('');
  let visible = $state(false);
  let message = $state('Disconnected.');
  let error = $state('');
  let playlist = $derived(origin ? `${origin}/stream.m3u8` : '');

  onMount(() => {
    const visibility = () => {
      visible = !document.hidden;
    };
    visibility();
    document.addEventListener('visibilitychange', visibility);
    try {
      address = localStorage.getItem('qsync-companion-origin') ?? '';
    } catch {
      /* Storage may be disabled. */
    }
    return () => document.removeEventListener('visibilitychange', visibility);
  });

  function connect(event: SubmitEvent) {
    event.preventDefault();
    error = '';
    try {
      origin = companionOrigin(address);
      address = origin;
      message = 'Connecting to companion viewer…';
      try {
        localStorage.setItem('qsync-companion-origin', origin);
      } catch {
        /* Playback does not require storage. */
      }
    } catch {
      error =
        'Enter the companion’s HTTP or HTTPS origin, without credentials, a path, query or fragment.';
    }
  }

  async function copyPlaylist() {
    try {
      await navigator.clipboard.writeText(playlist);
      message = 'Playlist URL copied.';
    } catch {
      error = 'Copy unavailable. Use the playlist link to copy its address.';
    }
  }
</script>

<section class="card streaming" aria-labelledby="streaming-title">
  <h2 id="streaming-title">Screen streaming</h2>
  <p>
    View QSync’s existing HDMI capture through the companion running on your computer. Lighting
    settings stay unchanged.
  </p>
  <form onsubmit={connect}>
    <label class="field" for="companion-address"
      >Companion address
      <input
        id="companion-address"
        type="url"
        bind:value={address}
        required
        autocomplete="off"
        spellcheck="false"
        aria-describedby="companion-help"
      />
    </label>
    <p id="companion-help">
      Use an address reachable from this browser. A companion bound to localhost works only on that
      computer.
    </p>
    <div class="actions">
      <Button type="submit">Connect</Button>
      <Button
        variant="outline"
        disabled={!origin}
        onclick={() => {
          origin = '';
          message = 'Disconnected.';
          error = '';
        }}>Disconnect</Button
      >
    </div>
  </form>
  {#if error}<p class="notice error" role="alert">{error}</p>{/if}
  {#if origin}
    {#if active && visible}
      <iframe
        src={`${origin}/?embedded=1`}
        title="QSync companion stream viewer"
        allow="autoplay; fullscreen"
        referrerpolicy="no-referrer"
        onload={() => {
          message = 'Companion viewer loaded; playback status is shown inside.';
        }}
      >
      </iframe>
    {/if}
    <div class="actions">
      <a href={`${origin}/`} target="_blank" rel="noopener noreferrer">Open companion viewer</a>
      <a href={playlist} target="_blank" rel="noopener noreferrer">HLS playlist</a>
      <Button variant="outline" onclick={copyPlaylist}>Copy playlist URL</Button>
    </div>
  {/if}
  <p role="status">
    {origin && (!active || !visible) ? 'Playback suspended while this view is hidden.' : message}
  </p>
  <p class="limitations">
    Low-resolution diagnostic video. Encoded playback FPS can include repeated captures; it is not
    the fresh-frame rate. Audio is not connected yet.
  </p>
</section>

<style>
  .streaming {
    display: grid;
    gap: 1rem;
  }
  .streaming p {
    margin: 0;
  }
  form {
    display: grid;
    gap: 0.75rem;
  }
  input {
    width: 100%;
    min-width: 0;
  }
  iframe {
    width: 100%;
    height: 480px;
    border: 0;
    background: black;
    border-radius: 0.75rem;
  }
  .actions {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 0.75rem;
  }
  .actions a {
    overflow-wrap: anywhere;
  }
  .limitations,
  #companion-help {
    color: var(--text-muted);
    font-size: 0.875rem;
  }
</style>
