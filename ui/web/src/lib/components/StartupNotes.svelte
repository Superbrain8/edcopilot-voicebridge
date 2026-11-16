<script lang="ts">
  // The texts of the start as plain text: a problem that stops the use of
  // a configuration, a failed first start, and the notes of the update.
  import { startupLines, startupStore } from '../startup'

  let lines = $derived($startupStore ? startupLines($startupStore) : [])
</script>

{#if lines.length > 0}
  <ul class="startup" data-startup-notes>
    {#each lines as line, i (i)}
      <li class={line.kind} role={line.kind === 'problem' ? 'alert' : undefined}>{line.text}</li>
    {/each}
  </ul>
{/if}

<style>
  .startup {
    display: grid;
    gap: 4px;
    padding: 10px 14px;
    background: var(--panel);
    border-radius: var(--radius-s);
    font-size: var(--step--1);
    overflow-wrap: anywhere;
  }

  .problem {
    color: var(--tone-error);
  }

  .note {
    color: var(--tone-dry);
  }
</style>
