<script lang="ts" module>
  /** The shapes of the window. Each state has its own shape, so that the state is not shown by color only. */
  export type Shape = 'dot' | 'ring' | 'dash' | 'cross' | 'triangle' | 'diamond' | 'square' | 'half'
</script>

<script lang="ts">
  let { shape, size = 12 }: { shape: Shape; size?: number } = $props()
</script>

<svg
  class="glyph"
  width={size}
  height={size}
  viewBox="0 0 12 12"
  aria-hidden="true"
  focusable="false"
>
  {#if shape === 'dot'}
    <circle cx="6" cy="6" r="5" fill="currentColor" />
  {:else if shape === 'ring'}
    <circle cx="6" cy="6" r="4.25" fill="none" stroke="currentColor" stroke-width="1.5" />
  {:else if shape === 'dash'}
    <rect x="1" y="5" width="10" height="2" fill="currentColor" />
  {:else if shape === 'cross'}
    <path d="M2 2 L10 10 M10 2 L2 10" stroke="currentColor" stroke-width="2" />
  {:else if shape === 'triangle'}
    <path d="M6 1.5 L10.75 10.25 H1.25 Z" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linejoin="round" />
  {:else if shape === 'diamond'}
    <path d="M6 0.5 L11.5 6 L6 11.5 L0.5 6 Z" fill="currentColor" />
  {:else if shape === 'square'}
    <rect x="1.75" y="1.75" width="8.5" height="8.5" fill="none" stroke="currentColor" stroke-width="1.5" />
  {:else if shape === 'half'}
    <circle cx="6" cy="6" r="4.25" fill="none" stroke="currentColor" stroke-width="1.5" />
    <path d="M6 1.75 A4.25 4.25 0 0 1 6 10.25 Z" fill="currentColor" />
  {/if}
</svg>

<style>
  .glyph {
    flex: none;
    display: inline-block;
    vertical-align: -1px;
  }
</style>
