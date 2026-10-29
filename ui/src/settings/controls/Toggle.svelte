<script lang="ts">
  interface Props {
    checked: boolean;
    onchange: (v: boolean) => void;
    label?: string;
    labelledby?: string;
    disabled?: boolean;
  }
  let { checked, onchange, label, labelledby, disabled = false }: Props = $props();
</script>

<button
  class="toggle"
  role="switch"
  aria-checked={checked}
  aria-label={label}
  aria-labelledby={labelledby}
  {disabled}
  onclick={() => onchange(!checked)}
>
  <span class="toggle-knob"></span>
</button>

<style>
  /* Fluent ToggleSwitch: 40 × 20, knob 12 → 14 on hover, slides 20px. */
  .toggle {
    position: relative;
    flex: none;
    width: 40px;
    height: 20px;
    border-radius: var(--r-pill);
    corner-shape: round;
    border: 1px solid var(--stroke-strong);
    background: var(--bg-subtle-track);
    transition:
      background-color var(--dur-1) var(--ease-std),
      border-color var(--dur-1) var(--ease-std);
  }
  .toggle:hover {
    background: var(--bg-subtle-hover);
  }
  .toggle-knob {
    position: absolute;
    top: 50%;
    left: 3px;
    width: 12px;
    height: 12px;
    margin-top: -6px;
    border-radius: var(--r-pill);
    corner-shape: round;
    background: var(--text-2);
    transition:
      transform var(--dur-2) var(--ease-out),
      width var(--dur-1) var(--ease-std),
      height var(--dur-1) var(--ease-std),
      margin var(--dur-1) var(--ease-std),
      background-color var(--dur-1) var(--ease-std);
  }
  .toggle:hover .toggle-knob {
    width: 14px;
    height: 14px;
    margin-top: -7px;
    margin-left: -1px;
  }
  .toggle:active .toggle-knob {
    width: 17px;
  }
  .toggle[aria-checked='true'] {
    border-color: var(--accent);
    background: var(--accent);
  }
  .toggle[aria-checked='true']:hover {
    border-color: var(--accent-hover);
    background: var(--accent-hover);
  }
  .toggle[aria-checked='true'] .toggle-knob {
    transform: translateX(20px);
    background: var(--on-accent);
  }
  .toggle[aria-checked='true']:active .toggle-knob {
    margin-left: -4px;
  }
  .toggle:disabled {
    opacity: 0.4;
  }
</style>
