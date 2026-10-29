<script lang="ts">
  import Icon from '../../lib/Icon.svelte';
  import { api } from '../../lib/ipc';
  import { t } from '../../lib/i18n.svelte';
  import { chordFromEvent, chordParts, formatChord, validateChord, type Chord, type Problem } from '../../lib/hotkeys';

  interface Props {
    value: string;
    defaultValue: string;
    others: Array<{ name: string; acc: string }>;
    onchange: (acc: string) => void;
    /** Validation / status line, rendered by the row. */
    onmessage: (m: { text: string; tone: 'error' | 'info' } | null) => void;
    labelledby?: string;
  }
  let { value, defaultValue, others, onchange, onmessage, labelledby }: Props = $props();

  let recording = $state(false);
  let held = $state<Chord | null>(null);
  let el = $state<HTMLButtonElement>();

  const parts = $derived(chordParts(value));
  const heldParts = $derived(held ? chordParts(formatChord({ ...held, key: null })) : []);

  function say(p: Problem | null, tone: 'error' | 'info' = 'error') {
    onmessage(p ? { text: t(p.key, p.vars), tone } : null);
  }

  function start() {
    recording = true;
    held = null;
    say(null);
    // While recording, the app's own global shortcuts must not fire.
    api.suspendHotkeys(true).catch(() => {});
  }

  function stop() {
    if (!recording) return;
    recording = false;
    held = null;
    api.suspendHotkeys(false).catch(() => {});
  }

  async function commit(c: Chord) {
    const acc = formatChord(c);
    const local = validateChord(c, others);
    if (local) return say(local);
    let status = 'ok';
    try {
      status = await api.checkHotkey(acc);
    } catch {
      /* backend without check_hotkey: trust local validation */
    }
    if (status === 'in_use') return say({ key: 'hk.inUse' });
    if (status === 'invalid') return say({ key: 'hk.invalid' });
    stop();
    say(null);
    if (acc !== value) onchange(acc);
  }

  function onkeydown(e: KeyboardEvent) {
    if (!recording) {
      if (e.key === 'Enter' || e.key === ' ') {
        e.preventDefault();
        start();
      }
      return;
    }
    e.preventDefault();
    e.stopPropagation();
    const plain = !e.ctrlKey && !e.altKey && !e.shiftKey && !e.metaKey;
    if (e.key === 'Escape' && plain) {
      stop();
      say(null);
      return;
    }
    if ((e.key === 'Backspace' || e.key === 'Delete') && plain) {
      stop();
      onchange('');
      say({ key: 'hk.cleared' }, 'info');
      return;
    }
    const c = chordFromEvent(e);
    held = c;
    if (c.key) void commit(c);
  }

  function onkeyup(e: KeyboardEvent) {
    if (!recording) return;
    e.preventDefault();
    // Windows only reports PrintScreen on key-up.
    if (e.code === 'PrintScreen') {
      void commit({ ...chordFromEvent(e), key: 'PrintScreen' });
      return;
    }
    held = chordFromEvent(e);
    if (!held.ctrl && !held.alt && !held.shift && !held.win) held = null;
  }
</script>

<div class="recorder-wrap">
  {#if value !== defaultValue && !recording}
    <button
      class="icon-btn"
      title={t('recorder.reset')}
      aria-label={t('recorder.reset')}
      onclick={() => {
        say(null);
        onchange(defaultValue);
      }}
    >
      <Icon name="reset" />
    </button>
  {/if}
  <button
    bind:this={el}
    class="recorder"
    class:recording
    title={t('recorder.edit')}
    aria-labelledby={labelledby}
    aria-describedby={recording ? 'rec-hint' : undefined}
    onclick={() => (recording ? null : start())}
    {onkeydown}
    {onkeyup}
    onblur={stop}
  >
    {#if recording}
      {#if heldParts.length}
        <span class="keys">{#each heldParts as p, i (i)}<kbd class="kbd">{p}</kbd>{/each}</span>
        <span class="ellipsis">…</span>
      {:else}
        <span class="placeholder">{t('recorder.recording')}</span>
      {/if}
    {:else if parts.length}
      <span class="keys">{#each parts as p, i (i)}<kbd class="kbd">{p}</kbd>{/each}</span>
    {:else}
      <span class="placeholder">{t('recorder.unset')}</span>
    {/if}
  </button>
</div>
{#if recording}
  <span id="rec-hint" class="visually-hidden">{t('recorder.hint')}</span>
{/if}

<style>
  .recorder-wrap {
    display: flex;
    align-items: center;
    gap: 4px;
  }
  .recorder {
    display: inline-flex;
    align-items: center;
    justify-content: flex-end;
    gap: 6px;
    min-width: 168px;
    height: 32px;
    padding: 0 6px 0 12px;
    border-radius: var(--r-sm);
    border: 1px solid var(--stroke-control);
    border-bottom-color: var(--stroke-control-bottom);
    background: var(--bg-control) padding-box;
    transition: background-color var(--dur-1) var(--ease-std);
  }
  .recorder:hover {
    background-color: var(--bg-control-hover);
  }
  .recorder.recording {
    justify-content: flex-start;
    background: var(--bg-control-input);
    border-bottom-color: var(--accent);
    box-shadow: inset 0 -1px 0 var(--accent);
  }
  .recorder.recording:focus-visible {
    outline: none;
  }
  .placeholder {
    flex: 1;
    text-align: left;
    color: var(--text-3);
    font-size: 13px;
  }
  .recorder:not(.recording) .placeholder {
    text-align: right;
    padding-right: 6px;
  }
  .ellipsis {
    color: var(--text-3);
  }
</style>
