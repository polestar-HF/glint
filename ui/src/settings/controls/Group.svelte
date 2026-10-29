<script lang="ts">
  import type { Snippet } from 'svelte';
  let { title, hint, children }: { title?: string; hint?: string; children: Snippet } = $props();
</script>

<section class="group">
  {#if title}
    <h2 class="group-title">
      <span>{title}</span>
      {#if hint}<span class="group-hint">{hint}</span>{/if}
    </h2>
  {/if}
  <div class="group-body">
    {@render children()}
  </div>
</section>

<style>
  .group + :global(.group) {
    margin-top: 24px;
  }
  .group-title {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    gap: 16px;
    margin: 0 0 8px;
    padding: 0 2px;
    font-family: var(--font-text);
    font-size: 14px;
    line-height: 20px;
    font-weight: 600;
    color: var(--text-1);
  }
  .group-hint {
    font-family: var(--font-small);
    font-size: var(--fs-caption);
    line-height: 16px;
    font-weight: 400;
    color: var(--text-3);
    text-align: right;
  }
  .group-body {
    border-radius: var(--r-md);
    border: 1px solid var(--stroke-card);
    background: var(--bg-card) padding-box;
    overflow: clip;
  }
  .group-body > :global(* + *) {
    border-top: 1px solid var(--stroke-divider);
  }
</style>
