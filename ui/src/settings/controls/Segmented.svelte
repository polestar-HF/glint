<script lang="ts" generics="T extends string">
  interface Props {
    value: T;
    options: Array<{ value: T; label: string }>;
    onchange: (v: T) => void;
    label?: string;
  }
  let { value, options, onchange, label }: Props = $props();

  function onkey(e: KeyboardEvent) {
    const i = options.findIndex((o) => o.value === value);
    let n = -1;
    if (e.key === 'ArrowRight' || e.key === 'ArrowDown') n = (i + 1) % options.length;
    else if (e.key === 'ArrowLeft' || e.key === 'ArrowUp') n = (i - 1 + options.length) % options.length;
    else return;
    e.preventDefault();
    onchange(options[n].value);
    ((e.currentTarget as HTMLElement).querySelectorAll('button')[n] as HTMLElement)?.focus();
  }
</script>

<div class="seg" role="radiogroup" aria-label={label} tabindex="-1" onkeydown={onkey}>
  {#each options as o (o.value)}
    <button
      role="radio"
      aria-checked={o.value === value}
      tabindex={o.value === value ? 0 : -1}
      onclick={() => onchange(o.value)}
    >
      {o.label}
    </button>
  {/each}
</div>

<style>
  .seg {
    display: inline-flex;
    gap: 4px;
    padding: 4px;
    border-radius: var(--r-sm);
    border: 1px solid var(--stroke-control);
    background: var(--bg-subtle-track);
  }
  button {
    height: 22px;
    min-width: 64px;
    padding: 0 12px;
    border-radius: var(--r-sm);
    font-size: 13px;
    color: var(--text-2);
    border: 1px solid transparent;
    transition:
      background-color var(--dur-1) var(--ease-std),
      color var(--dur-1) var(--ease-std);
  }
  button:hover {
    color: var(--text-1);
    background: var(--bg-subtle-hover);
  }
  button[aria-checked='true'] {
    color: var(--text-1);
    background: light-dark(#ffffff, rgb(255 255 255 / 0.09));
    border-color: var(--stroke-control);
    box-shadow: var(--shadow-control);
  }
  button:focus-visible {
    outline-offset: 0;
  }
</style>
