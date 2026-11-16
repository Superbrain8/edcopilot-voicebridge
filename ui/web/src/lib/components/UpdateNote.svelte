<script lang="ts">
  // The update note at the top of the Status screen. It shows only when a
  // newer version is available, and during its download and install.
  import Glyph from './Glyph.svelte'
  import { sessionStore as session } from '../session'
  import {
    READY_TEXT,
    availableText,
    canInstall,
    installBlock,
    noteShown,
    progressText,
    updateStore as updates,
  } from '../updates'

  let status = $derived($updates.status)
  let block = $derived(installBlock($session))
  let installOn = $derived(canInstall(status, $session))
</script>

{#if status && noteShown(status)}
  <section class="update" aria-labelledby="update-title" data-update={status.phase}>
    <h2 id="update-title" class="visually-hidden">Update</h2>
    {#if status.phase === 'available'}
      <p class="lead"><Glyph shape="ring" /> {availableText(status)}</p>
      {#if status.error}
        <p class="error" role="alert" data-update-error>{status.error}</p>
      {/if}
      <div class="buttons">
        <button type="button" class="btn primary" data-action="install-update" disabled={!installOn} onclick={() => updates.install()}
          >Install update</button
        >
        <button type="button" class="btn" data-action="later" onclick={() => updates.later()}>Later</button>
      </div>
      {#if block}
        <p class="hint" data-install-block>{block}</p>
      {/if}
      {#if $updates.actionError}
        <p class="error" role="alert" data-action-error>{$updates.actionError}</p>
      {/if}
      <details class="notes">
        <summary>What is new</summary>
        {#if status.notes}
          <pre class="text" data-notes>{status.notes}</pre>
        {/if}
        <p class="hint">
          The release page: <span class="address" data-release-page>{status.release_page}</span>
        </p>
      </details>
    {:else if status.phase === 'downloading'}
      <p class="lead" role="status" data-progress><Glyph shape="half" /> {progressText(status)}</p>
      <div class="buttons">
        <button type="button" class="btn" data-action="cancel-update" onclick={() => updates.cancel()}>Cancel</button>
      </div>
      {#if $updates.actionError}
        <p class="error" role="alert" data-action-error>{$updates.actionError}</p>
      {/if}
    {:else if status.phase === 'ready'}
      <p class="lead" role="status" data-ready><Glyph shape="dot" /> {READY_TEXT}</p>
    {/if}
  </section>
{/if}

<style>
  .update {
    display: grid;
    gap: 8px;
    padding: 10px 14px;
    border: 1px solid var(--signal);
    border-left-width: 4px;
    border-radius: var(--radius-s);
    background: var(--panel);
  }

  .lead {
    display: flex;
    align-items: center;
    gap: 8px;
    font-weight: 600;
  }

  .buttons {
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
  }

  .hint {
    font-size: var(--step--1);
    color: var(--muted);
  }

  .error {
    font-size: var(--step--1);
    color: var(--tone-error);
    overflow-wrap: anywhere;
  }

  .notes summary {
    cursor: pointer;
    font-size: var(--step-0);
    color: var(--muted);
  }

  .text {
    margin: 6px 0;
    padding: 8px 10px;
    background: var(--ground);
    border: 1px solid var(--line);
    border-radius: var(--radius-s);
    font-family: inherit;
    font-size: var(--step--1);
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }

  .address {
    font-family: Consolas, 'Cascadia Mono', monospace;
    user-select: all;
    overflow-wrap: anywhere;
  }
</style>
