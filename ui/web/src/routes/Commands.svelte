<script lang="ts">
  import { onDestroy, onMount } from 'svelte'
  import Glyph, { type Shape } from '../lib/components/Glyph.svelte'
  import {
    categoryLabel,
    commandsStore as commands,
    groupRows,
    listState,
    MAX_FILTER_CHARS,
    MAX_PHRASE_CHARS,
    PAGE_ROWS,
  } from '../lib/commands'
  import { configStore as config } from '../lib/config'
  import { MARK_LABEL, OUTCOME_LABEL, OUTCOME_TONE, outcomeMeaning, percent, type Tone } from '../lib/format'

  const SHAPE: Record<Tone, Shape> = {
    sent: 'dot',
    dry: 'ring',
    miss: 'dash',
    rejected: 'cross',
    unread: 'triangle',
    error: 'diamond',
    dropped: 'square',
  }

  // Only the loaded pages are in the page. The Rust side filters the list.
  let page = $derived(groupRows($commands.list ?? { rows: [], total: 0 }))
  let state = $derived(listState($commands))
  let outcome = $derived($commands.result?.outcome ?? null)
  let tone: Tone = $derived(outcome ? OUTCOME_TONE[outcome.kind] : 'miss')
  let wakeWords = $derived($commands.result?.wake_words ?? [])

  function submit(event: SubmitEvent) {
    event.preventDefault()
    void commands.test()
  }

  onMount(() => {
    void commands.init()
  })

  onDestroy(() => {
    commands.destroy()
  })
</script>

<!-- All values from the program are text. Svelte escapes them. -->
<div class="commands">
  <section class="group" aria-labelledby="try-title">
    <h2 id="try-title">Try a phrase</h2>
    <form class="row" onsubmit={submit}>
      <label class="visually-hidden" for="phrase">Phrase</label>
      <input
        id="phrase"
        type="text"
        data-phrase
        autocomplete="off"
        spellcheck="false"
        placeholder="Type a phrase that you speak"
        maxlength={MAX_PHRASE_CHARS}
        value={$commands.phrase}
        oninput={(e) => commands.setPhrase(e.currentTarget.value)}
        aria-describedby="phrase-help"
      />
      <button type="submit" class="btn" data-action="test-phrase" disabled={$commands.phrase.trim() === ''}
        >Test</button
      >
    </form>
    <p class="help" id="phrase-help">
      <Glyph shape="ring" /> Dry run: the test sends nothing to EDCoPilot, also while a session runs. The test
      treats the phrase as heard speech, with the saved settings.
    </p>
    <p class="help" class:note={$config.restartNeeded} data-saved-settings>
      The test uses the saved settings. A running session keeps the settings that it started with{#if $config.restartNeeded}:
        restart the session to use the saved settings{/if}.
    </p>

    <div class="result" aria-live="polite" data-phrase-result>
      {#if $commands.testing}
        <p class="help">Testing the phrase.</p>
      {:else if $commands.testError !== null}
        <p class="field-error" data-test-error>
          <Glyph shape="diamond" /> <span class="word">Problem:</span> {$commands.testError}
        </p>
      {:else if $commands.result && outcome}
        <div class="outcome tone-{tone}" data-kind={outcome.kind}>
          <p class="kind">
            <Glyph shape={SHAPE[tone]} />
            <span class="kind-word">{OUTCOME_LABEL[outcome.kind]}</span>
            {#if outcome.confidence !== null}<span class="conf">{percent(outcome.confidence)}</span>{/if}
          </p>
          {#if outcome.token}<p class="token" data-token>{outcome.token}</p>{/if}
          {#if outcome.reason}
            <p class="reason" data-reason><span class="word">Reason:</span> {outcome.reason}</p>
          {/if}
          {#if outcome.marks.length > 0}
            <p class="marks">
              {#each outcome.marks as mark (mark)}<span class="mark">{MARK_LABEL[mark] ?? mark}</span>{/each}
            </p>
          {/if}
          <p class="help" data-meaning>{outcomeMeaning(outcome)}</p>
          {#if outcome.kind === 'no_wake_word' && wakeWords.length > 0}
            <p class="wake" data-wake-words>
              In always-listening mode the program acts only after a wake word: {wakeWords.join(', ')}. Start the
              phrase with one of them.
            </p>
          {/if}
          <p class="line" data-line>{$commands.result.line}</p>
        </div>
        {#each $commands.result.warnings as warning, i (i)}
          <p class="note"><Glyph shape="triangle" /> <span class="word">Warning:</span> {warning}</p>
        {/each}
      {/if}
    </div>
  </section>

  <section class="group" aria-labelledby="list-title">
    <h2 id="list-title">Commands</h2>
    <div class="row filter-row">
      <label class="visually-hidden" for="filter">Filter</label>
      <input
        id="filter"
        type="search"
        data-filter
        autocomplete="off"
        spellcheck="false"
        placeholder="Filter: each word must be in the phrase, the token, or the category"
        maxlength={MAX_FILTER_CHARS}
        value={$commands.filter}
        oninput={(e) => commands.setFilter(e.currentTarget.value)}
      />
      <label class="option">
        <input
          type="checkbox"
          data-all
          checked={$commands.all}
          onchange={(e) => commands.setAll(e.currentTarget.checked)}
        />
        Show commands that do not work
      </label>
    </div>

    {#if state === 'failed'}
      <div class="panel alarm" role="alert" data-list-problems>
        <h3><Glyph shape="diamond" /> The command list did not load</h3>
        <p class="plain">{$commands.error}</p>
        <p class="help">Open the Setup screen to correct the settings.</p>
      </div>
    {/if}

    {#if $commands.list}
      {#if state === 'empty'}
        <div class="panel alarm" role="alert" data-list-empty>
          <h3><Glyph shape="diamond" /> No commands were found</h3>
          <p class="help">The command data loaded, but it has no command that works. Open the Setup screen.</p>
        </div>
      {/if}
      {#if $commands.list.problems.length > 0}
        <div class="panel warn-panel" data-data-problems>
          <h3><Glyph shape="triangle" /> Problems with the command data</h3>
          <ul class="lines">
            {#each $commands.list.problems as problem, i (i)}<li>{problem}</li>{/each}
          </ul>
        </div>
      {/if}
      <p class="summary" data-summary aria-live="polite">
        {$commands.list.summary}{#if $commands.loading}<span class="help"> Filtering.</span>{/if}
      </p>
      {#if $commands.notice}<p class="note" data-notice>{$commands.notice}</p>{/if}
      {#each $commands.list.warnings as warning, i (i)}
        <p class="note"><Glyph shape="triangle" /> <span class="word">Warning:</span> {warning}</p>
      {/each}

      {#each page.groups as group, g (g)}
        <section class="category" data-category={group.category}>
          <h3>{categoryLabel(group.category)}</h3>
          <ul class="cmd-rows">
            {#each group.rows as row, r (r)}
              <li class="cmd" class:broken={!row.works} data-row data-works={row.works}>
                <span class="phrase">{row.phrase}</span>
                <span class="token">{row.token}</span>
                {#if row.alias}<span class="mark">alias</span>{/if}
                {#if row.reason}
                  <span class="cmd-reason"><Glyph shape="cross" /> {row.reason}</span>
                {/if}
              </li>
            {/each}
          </ul>
        </section>
      {/each}

      {#if page.hidden > 0}
        <div class="more" data-hidden={page.hidden}>
          <p class="help">
            {$commands.list.rows.length} rows shown. {page.hidden} more rows are not shown. Type a filter to find a
            command.
          </p>
          <button
            type="button"
            class="btn"
            data-action="show-more"
            disabled={$commands.loadingMore}
            onclick={() => commands.showMore()}>Show {Math.min(PAGE_ROWS, page.hidden)} more</button
          >
        </div>
      {/if}

      {#if $commands.list.composite.length > 0}
        {#each $commands.list.composite as group, g (g)}
          <section class="category composite" data-composite>
            <h3>{group.heading}</h3>
            <ul class="lines">
              {#each group.lines as line, i (i)}<li>{line}</li>{/each}
            </ul>
          </section>
        {/each}
      {/if}
    {:else if $commands.loading}
      <p class="help">Reading the commands.</p>
    {/if}
  </section>
</div>

<style>
  .commands {
    display: flex;
    flex-direction: column;
    gap: var(--gap);
    max-width: 60rem;
  }

  .group {
    display: flex;
    flex-direction: column;
    gap: 10px;
    padding: 12px 14px 14px;
    background: var(--panel);
    border-radius: var(--radius-s);
    min-width: 0;
  }

  .group h2 {
    font-size: var(--step-1);
    font-weight: 600;
  }

  .row {
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
  }

  .row input[type='text'],
  .row input[type='search'] {
    flex: 1 1 14rem;
  }

  input[type='text'],
  input[type='search'] {
    min-width: 0;
    min-height: 40px;
    padding: 0 10px;
    border: 1px solid var(--line);
    border-radius: var(--radius-m);
    background: var(--ground);
    color: var(--text);
    font: inherit;
    font-size: var(--step-0);
  }

  input[type='text']:hover,
  input[type='search']:hover {
    border-color: var(--muted);
  }

  .option {
    display: inline-flex;
    align-items: center;
    gap: 10px;
    min-height: 40px;
    padding: 0 12px;
    border: 1px solid var(--line);
    border-radius: var(--radius-m);
    font-size: var(--step-0);
    cursor: pointer;
  }

  .option:has(input:checked) {
    background: var(--raised);
    border-color: var(--text);
  }

  .option input {
    width: 18px;
    height: 18px;
    margin: 0;
    accent-color: var(--signal);
  }

  .help {
    font-size: var(--step--1);
    color: var(--muted);
    overflow-wrap: anywhere;
  }

  .path {
    font-family: ui-monospace, 'Cascadia Mono', Consolas, monospace;
    color: var(--text);
  }

  .word {
    font-weight: 700;
  }

  .field-error {
    font-size: var(--step-0);
    color: var(--tone-error);
    overflow-wrap: anywhere;
  }

  .note {
    font-size: var(--step--1);
    color: var(--tone-dry);
    overflow-wrap: anywhere;
  }

  .result:empty {
    display: none;
  }

  .outcome {
    display: flex;
    flex-direction: column;
    gap: 3px;
    padding: 8px 12px 8px 10px;
    border-left: 4px solid var(--tone);
    background: var(--ground);
  }

  .tone-sent { --tone: var(--tone-sent); }
  .tone-dry { --tone: var(--tone-dry); }
  .tone-miss { --tone: var(--line); }
  .tone-rejected { --tone: var(--tone-rejected); }
  .tone-unread { --tone: var(--tone-unread); }
  .tone-error { --tone: var(--tone-error); }
  .tone-dropped { --tone: transparent; }

  .kind {
    display: flex;
    align-items: center;
    gap: 8px;
    font-weight: 700;
    color: var(--tone);
  }

  .tone-miss .kind {
    color: var(--tone-miss);
  }

  .conf {
    margin-left: auto;
    font-variant-numeric: tabular-nums;
    color: var(--text);
  }

  .token {
    font-weight: 600;
    color: var(--text);
    overflow-wrap: anywhere;
  }

  .reason {
    color: var(--tone);
    overflow-wrap: anywhere;
  }

  .marks {
    display: flex;
    flex-wrap: wrap;
    gap: 4px;
  }

  .mark {
    padding: 0 6px;
    border: 1px solid var(--line);
    border-radius: var(--radius-s);
    font-size: var(--step--1);
    color: var(--muted);
  }

  .line {
    font-family: ui-monospace, 'Cascadia Mono', Consolas, monospace;
    font-size: var(--step--1);
    color: var(--muted);
    overflow-wrap: anywhere;
    white-space: pre-wrap;
  }

  .panel {
    padding: 10px 14px;
    background: var(--ground);
    border-radius: var(--radius-s);
  }

  .alarm {
    display: flex;
    flex-direction: column;
    gap: 8px;
    border: 1px solid var(--tone-error);
    border-left-width: 4px;
  }

  .alarm h3 {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: var(--step-1);
    color: var(--tone-error);
  }

  .warn-panel {
    display: flex;
    flex-direction: column;
    gap: 8px;
    border-left: 4px solid var(--tone-dry);
  }

  .warn-panel h3 {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: var(--step-0);
    color: var(--tone-dry);
  }

  .wake {
    padding: 6px 8px;
    border-left: 3px solid var(--tone-dry);
    background: var(--raised);
    overflow-wrap: anywhere;
  }

  .plain {
    overflow-wrap: anywhere;
    white-space: pre-wrap;
  }

  .lines {
    font-size: var(--step--1);
  }

  .lines li {
    padding: 3px 0;
    border-top: 1px solid var(--raised);
    overflow-wrap: anywhere;
  }

  .lines li:first-child {
    border-top: 0;
  }

  .summary {
    font-size: var(--step--1);
    color: var(--muted);
  }

  .category h3 {
    margin: 8px 0 2px;
    font-size: var(--step-0);
    font-weight: 600;
    color: var(--muted);
  }

  .cmd {
    display: flex;
    flex-wrap: wrap;
    align-items: baseline;
    gap: 2px 12px;
    padding: 4px 0;
    border-top: 1px solid var(--raised);
  }

  .phrase {
    flex: 1 1 18rem;
    min-width: 0;
    overflow-wrap: anywhere;
  }

  .cmd .token {
    font-family: ui-monospace, 'Cascadia Mono', Consolas, monospace;
    font-size: var(--step--1);
    font-weight: 400;
    color: var(--muted);
  }

  .broken .phrase {
    color: var(--muted);
  }

  .cmd-reason {
    flex-basis: 100%;
    display: inline-flex;
    align-items: center;
    gap: 6px;
    font-size: var(--step--1);
    color: var(--tone-rejected);
    overflow-wrap: anywhere;
  }

  .more {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 8px 14px;
    padding-top: 6px;
  }
</style>
