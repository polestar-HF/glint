<script lang="ts">
  import type { Snippet } from 'svelte';
  import Icon from '../../lib/Icon.svelte';

  interface Props {
    title: string;
    desc?: string;
    id?: string;
    message?: { text: string; tone: 'error' | 'info' } | null;
    compact?: boolean;
    children?: Snippet;
  }
  let { title, desc, id, message = null, compact = false, children }: Props = $props();
</script>

<div class="row" class:compact>
  <div class="row-text">
    <div class="row-title" {id}>{title}</div>
    {#if desc}<div class="row-desc">{desc}</div>{/if}
    {#if message}
      <div class="row-msg" class:error={message.tone === 'error'} role={message.tone === 'error' ? 'alert' : 'status'}>
        {#if message.tone === 'error'}<Icon name="alert" />{/if}
        <span>{message.text}</span>
      </div>
    {/if}
  </div>
  {#if children}
    <div class="row-control">{@render children()}</div>
  {/if}
</div>

<style>
  .row {
    display: flex;
    align-items: center;
    gap: 24px;
    min-height: 56px;
    padding: 10px 16px;
  }
  .row.compact {
    min-height: 44px;
    padding-block: 6px;
  }
  .row-text {
    flex: 1 1 auto;
    min-width: 0;
  }
  .row-title {
    font-size: 14px;
    line-height: 20px;
    color: var(--text-1);
  }
  .row-desc {
    margin-top: 1px;
    font-family: var(--font-small);
    font-size: var(--fs-caption);
    line-height: 16px;
    color: var(--text-2);
  }
  .row-msg {
    display: flex;
    align-items: center;
    gap: 6px;
    margin-top: 6px;
    font-family: var(--font-small);
    font-size: var(--fs-caption);
    line-height: 16px;
    color: var(--text-2);
  }
  .row-msg.error {
    color: var(--critical);
  }
  .row-control {
    flex: none;
    display: flex;
    align-items: center;
    gap: 8px;
  }
</style>
