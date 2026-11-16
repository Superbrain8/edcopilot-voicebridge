<script lang="ts">
  import Glyph from '../lib/components/Glyph.svelte'
  import ResultItem from '../lib/components/ResultItem.svelte'
  import StartupNotes from '../lib/components/StartupNotes.svelte'
  import { configStore as config, savedWakeWordWarning, startBlock } from '../lib/config'
  import { sessionStore as session } from '../lib/session'
  import { clock } from '../lib/format'
  import { currentScreen } from '../lib/nav'

  // A problem of `check` turns the Start buttons off. A warning does not.
  // A session that runs is not affected: Stop stays on.
  let block = $derived(startBlock($config))
  let checkProblems = $derived($config.check?.problems.length ?? 0)
  // Always-listening without a wake word: shown before and during a session.
  let wakeWarning = $derived(savedWakeWordWarning($config))

  let canStart = $derived(
    $session.loaded &&
      !$session.starting &&
      !$session.stopping &&
      !$session.running &&
      block === null,
  )
  let canStop = $derived(($session.running || $session.starting) && !$session.stopping)

  let lamp = $derived(
    !$session.running ? 'off' : $session.talking ? 'down' : 'up',
  )
</script>

<div class="status">
  <StartupNotes />
  <section class="controls" aria-label="Session controls">
    <div class="buttons">
      <button
        type="button"
        class="btn primary"
        data-action="start"
        disabled={!canStart}
        onclick={() => session.start(false)}>Start</button
      >
      <button
        type="button"
        class="btn"
        data-action="start-dry"
        disabled={!canStart}
        onclick={() => session.start(true)}>Start as dry run</button
      >
      <button
        type="button"
        class="btn"
        data-action="stop"
        disabled={!canStop}
        onclick={() => session.stop()}>Stop</button
      >
    </div>
    {#if $session.starting}
      <p class="hint" role="status">Loading the speech model. This can take some seconds.</p>
    {:else if $session.error}
      <p class="hint error" role="alert">{$session.error}</p>
    {/if}
    {#if block !== null && !$session.running && !$session.starting}
      <p class="hint" class:error={checkProblems > 0} data-start-block>
        {#if checkProblems > 0}
          <Glyph shape="diamond" />
          {block}
          <button type="button" class="link" data-action="open-setup" onclick={() => currentScreen.set('setup')}
            >Open Setup</button
          >
        {:else}
          {block}
        {/if}
      </p>
    {/if}
    <!-- The session uses the saved file, so the warning follows the file. -->
    <div role="status" aria-live="polite">
      {#if wakeWarning}
        <p class="hint warn" data-wake-warning>
          <Glyph shape="triangle" /> <strong>Warning:</strong> {wakeWarning}
          <button type="button" class="link" data-action="open-microphone" onclick={() => currentScreen.set('microphone')}
            >Open Microphone</button
          >
        </p>
      {/if}
    </div>
  </section>

  <!-- Not a live region: a screen reader must not read each press. -->
  <div class="lamp lamp-{lamp}" data-talk={lamp} aria-live="off">
    <Glyph shape={lamp === 'down' ? 'dot' : lamp === 'up' ? 'ring' : 'dash'} size={14} />
    <span class="lamp-word">
      {#if lamp === 'down'}
        Talk button down: listening
      {:else if lamp === 'up'}
        Talk button up
      {:else}
        Talk button: no session
      {/if}
    </span>
  </div>

  {#if $session.problems.length > 0}
    <section class="problems" role="alert" aria-labelledby="problems-title">
      <h2 id="problems-title">
        <Glyph shape="diamond" />
        The session did not start
      </h2>
      <ul data-list="problems">
        {#each $session.problems as problem, i (i)}
          <li>{problem}</li>
        {/each}
      </ul>
    </section>
  {/if}

  <div class="body">
    <section class="results" aria-labelledby="results-title">
      <h2 id="results-title" class="section-title">
        Results <span class="count">{$session.results.length}</span>
      </h2>
      {#if $session.results.length === 0}
        <p class="empty">
          {#if $session.running}
            Hold the talk button and speak a command. Each result shows here, the newest at the top.
          {:else}
            No results yet. Start a session, then hold the talk button and speak a command.
          {/if}
        </p>
      {:else}
        <ol class="list" data-list="results">
          {#each $session.results as row (row.id)}
            <ResultItem {row} />
          {/each}
        </ol>
      {/if}
    </section>

    <aside class="side">
      <section class="info" aria-labelledby="info-title">
        <h2 id="info-title" class="section-title">
          {$session.running ? 'This session' : 'Last session'}
        </h2>
        {#if $session.startInfo}
          <ul class="lines" data-list="start-lines">
            {#each $session.startInfo.lines as line, i (i)}
              <li>{line}</li>
            {/each}
          </ul>
        {:else}
          <p class="empty">No session in this run of the program.</p>
        {/if}
        {#if $session.reload}
          <p class="note">
            Vocabulary reloaded: {$session.reload.files} files, {$session.reload.names} names
          </p>
        {/if}
        {#if !$session.running && $session.glitches > 0}
          <p class="note">Audio glitches in the last session: {$session.glitches}</p>
        {/if}
      </section>

      <details class="warnings" data-list="warnings">
        <summary>
          Warnings <span class="count">{$session.warnings.length}</span>
        </summary>
        {#if $session.warnings.length === 0}
          <p class="empty">No warnings.</p>
        {:else}
          <ul class="lines">
            {#each $session.warnings as warning (warning.id)}
              <li><time>{clock(warning.at)}</time> {warning.text}</li>
            {/each}
          </ul>
        {/if}
      </details>
    </aside>
  </div>
</div>

<style>
  .status {
    display: flex;
    flex-direction: column;
    gap: var(--gap);
  }

  .buttons {
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
  }

  .hint {
    margin-top: 6px;
    font-size: var(--step-0);
    color: var(--muted);
  }

  .hint.error {
    color: var(--tone-error);
  }

  .hint.warn {
    padding: 8px 10px;
    border-left: 3px solid var(--tone-dry);
    background: var(--raised);
    color: var(--text);
    overflow-wrap: anywhere;
  }

  /* The talk lamp: the one element that changes at a glance. The change is
     immediate, without a transition. */
  .lamp {
    display: flex;
    align-items: center;
    gap: 10px;
    min-height: 48px;
    padding: 0 14px;
    border: 2px solid var(--line);
    border-radius: var(--radius-m);
    font-size: var(--step-1);
    font-weight: 600;
  }

  .lamp-off {
    border-style: dashed;
    color: var(--faint);
  }

  .lamp-up {
    color: var(--signal);
    border-color: var(--signal);
  }

  .lamp-down {
    background: var(--signal);
    border-color: var(--signal);
    color: var(--on-signal);
  }

  .problems {
    padding: 10px 14px;
    border: 1px solid var(--tone-error);
    border-left-width: 4px;
    border-radius: var(--radius-s);
    background: var(--panel);
  }

  .problems h2 {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: var(--step-1);
    color: var(--tone-error);
  }

  .problems li {
    padding: 4px 0;
    border-top: 1px solid var(--raised);
    overflow-wrap: anywhere;
  }

  .problems li:first-child {
    border-top: 0;
  }

  .body {
    display: grid;
    gap: var(--gap);
  }

  @media (min-width: 1000px) {
    .body {
      grid-template-columns: minmax(0, 1fr) 20rem;
      align-items: start;
    }
  }

  .section-title {
    display: flex;
    align-items: baseline;
    gap: 8px;
    margin-bottom: 6px;
    font-size: var(--step-0);
    font-weight: 600;
    color: var(--muted);
  }

  .count {
    font-weight: 400;
    font-variant-numeric: tabular-nums;
    color: var(--faint);
  }

  .empty {
    color: var(--muted);
    font-size: var(--step-0);
  }

  .side {
    display: flex;
    flex-direction: column;
    gap: var(--gap);
    min-width: 0;
  }

  .info,
  .warnings {
    padding: 10px 12px;
    background: var(--panel);
    border-radius: var(--radius-s);
  }

  .lines {
    font-size: var(--step--1);
    color: var(--muted);
  }

  .lines li {
    padding: 2px 0;
    overflow-wrap: anywhere;
  }

  .note {
    margin-top: 8px;
    font-size: var(--step--1);
    color: var(--muted);
  }

  .warnings summary {
    font-size: var(--step-0);
    font-weight: 600;
    color: var(--muted);
    cursor: pointer;
  }

  .warnings[open] summary {
    margin-bottom: 6px;
  }

  .warnings time {
    color: var(--faint);
    font-variant-numeric: tabular-nums;
  }
</style>
