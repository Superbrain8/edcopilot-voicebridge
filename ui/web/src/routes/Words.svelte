<script lang="ts">
  import { onDestroy, onMount } from 'svelte'
  import Glyph from '../lib/components/Glyph.svelte'
  import type { TableKind } from '../lib/api'
  import { configStore as config } from '../lib/config'
  import { currentScreen } from '../lib/nav'
  import { sessionStore as session } from '../lib/session'
  import {
    MAY_BE_LAST,
    NOT_LAST,
    TABLES,
    canSave,
    checkIsCurrent,
    isDirty,
    meantFromText,
    meantText,
    needsConfirmation,
    tabId,
    tablesStore as tables,
  } from '../lib/tables'

  /** The longest entry. The Rust side refuses a longer one. */
  const MAX_ENTRY_CHARS = 300

  let kind = $state<TableKind>('synonyms')
  let language = $state('')
  let newName = $state('')

  let info = $derived(TABLES.find((t) => t.kind === kind) ?? TABLES[1])
  let id = $derived(tabId(kind, kind === 'aliases' ? language : null))
  let tab = $derived($tables.tabs[id] ?? null)
  let busy = $derived(tab === null || tab.loading || tab.saving)
  let changed = $derived(Object.keys($tables.tabs).filter((key) => isDirty($tables.tabs[key])))
  let sessionBusy = $derived($session.running || $session.starting)

  function show(next: TableKind) {
    kind = next
    if (next === 'aliases') {
      if (language === '') language = $tables.languages[0] ?? ''
      if (language === '') return
      void tables.open('aliases', language)
    } else {
      void tables.open(next, null)
    }
  }

  /** The program can give the language in the case of its file ("de" for "DE"). */
  function syncLanguage() {
    const active = $tables.tabs[$tables.active]
    if (kind === 'aliases' && active?.kind === 'aliases' && active.language) language = active.language
  }

  function selectLanguage(next: string) {
    language = next
    if (next !== '') void tables.open('aliases', next).then(syncLanguage)
  }

  function addLanguage(event: SubmitEvent) {
    event.preventDefault()
    const name = newName.trim()
    if (name === '') return
    language = name
    newName = ''
    void tables.newLanguage(name).then(syncLanguage)
  }

  function tabLabel(k: TableKind): string {
    return TABLES.find((t) => t.kind === k)?.label ?? k
  }

  onMount(() => {
    // The tab of the last visit shows again.
    const last = $tables.tabs[$tables.active]
    if (last) {
      kind = last.kind
      language = last.language ?? ''
    }
    show(kind)
  })

  onDestroy(() => {
    tables.destroy()
  })
</script>

<!-- All values from the tables are text. Svelte escapes them. -->
<div class="words">
  <div class="tabs" role="tablist" aria-label="Word tables">
    {#each TABLES as table (table.kind)}
      <button
        type="button"
        role="tab"
        class="tab"
        data-tab={table.kind}
        aria-selected={table.kind === kind}
        onclick={() => show(table.kind)}
        >{table.label}{#if changed.some((key) => key === table.kind || (table.kind === 'aliases' && key.startsWith('aliases:')))}
          <span class="unsaved" data-tab-unsaved>unsaved</span>{/if}</button
      >
    {/each}
  </div>

  <section class="group" aria-labelledby="table-title" data-table={kind}>
    <h2 id="table-title">{info.label}</h2>
    <p class="help">{info.help}</p>

    {#if kind === 'aliases'}
      <div class="row">
        <label class="inline" for="language">Language</label>
        <select
          id="language"
          data-language
          value={language}
          disabled={tab?.saving ?? false}
          onchange={(e) => selectLanguage(e.currentTarget.value)}
        >
          {#if language === ''}<option value="">(no language)</option>{/if}
          {#each $tables.languages as name (name)}
            <option value={name}>{name}</option>
          {/each}
        </select>
        <form class="row" onsubmit={addLanguage}>
          <label class="visually-hidden" for="new-language">New language</label>
          <input
            id="new-language"
            type="text"
            data-new-language
            autocomplete="off"
            spellcheck="false"
            maxlength="32"
            placeholder="For example: fr"
            bind:value={newName}
          />
          <button type="submit" class="btn" data-action="new-language" disabled={newName.trim() === ''}
            >New language</button
          >
        </form>
      </div>
      <p class="help">A new language gets its file only when you save it.</p>
    {/if}

    {#if kind === 'aliases' && language === ''}
      <p class="help" data-no-language>The alias folder has no language file. Add a language.</p>
    {:else if tab === null || (tab.loading && tab.table === null)}
      <p class="help">Reading the table.</p>
    {:else if tab.loadError !== null}
      <p class="field-error" data-load-error>
        <Glyph shape="diamond" /> <span class="word">Problem:</span>
        {tab.loadError}
      </p>
    {:else if tab.table}
      <p class="help path" data-path>File: {tab.table.path}</p>
      {#if !tab.table.exists}
        <p class="note" data-new-file>The file does not exist yet. Save makes it.</p>
      {/if}
      {#if kind === 'hotwords' && tab.table.hotwords_off}
        <p class="note" data-hotwords-off>
          <Glyph shape="triangle" /> The hotwords are off. This tab edits the shipped list
          <span class="path" data-hotwords-path>{tab.table.path}</span>. The program does not use it until you switch
          the hotwords on in Setup.
          <button type="button" class="link" data-action="go-setup" onclick={() => currentScreen.set('setup')}
            >Open Setup</button
          >
        </p>
      {/if}
      {#if kind === 'hotwords' && $config.saved && $config.saved.stt.engine !== 'parakeet'}
        <p class="help" data-cloud-engine>Only the local engine uses hotwords. The cloud engine ignores them.</p>
      {/if}

      <table class="rows" data-rows>
        <thead>
          <tr>
            <th scope="col">{info.spoken}</th>
            {#if info.meant !== null}<th scope="col">{info.meant}</th>{/if}
            <th scope="col"><span class="visually-hidden">Remove</span></th>
          </tr>
        </thead>
        <tbody>
          {#each tab.rows as row, i (i)}
            <tr data-row={i}>
              <td>
                <input
                  type="text"
                  data-spoken
                  aria-label="{info.spoken}, row {i + 1}"
                  autocomplete="off"
                  spellcheck="false"
                  maxlength={MAX_ENTRY_CHARS}
                  value={row.spoken}
                  disabled={busy}
                  oninput={(e) => tables.setSpoken(id, i, e.currentTarget.value)}
                />
              </td>
              {#if info.meantForm === 'list'}
                <td>
                  <textarea
                    data-meant
                    aria-label="{info.meant}, row {i + 1}"
                    spellcheck="false"
                    rows={Math.max(1, row.meant.length)}
                    value={meantText(row)}
                    disabled={busy}
                    oninput={(e) => tables.setMeant(id, i, meantFromText(e.currentTarget.value))}
                  ></textarea>
                </td>
              {:else if info.meantForm === 'word'}
                <td>
                  <input
                    type="text"
                    data-meant
                    aria-label="{info.meant}, row {i + 1}"
                    autocomplete="off"
                    spellcheck="false"
                    maxlength={MAX_ENTRY_CHARS}
                    value={row.meant[0] ?? ''}
                    disabled={busy}
                    oninput={(e) => tables.setMeant(id, i, [e.currentTarget.value])}
                  />
                </td>
              {:else if info.meantForm === 'filler'}
                <td>
                  <select
                    data-meant
                    aria-label="{info.meant}, row {i + 1}"
                    value={row.meant[0] ?? MAY_BE_LAST}
                    disabled={busy}
                    onchange={(e) => tables.setMeant(id, i, [e.currentTarget.value])}
                  >
                    <option value={NOT_LAST}>Never the last word (not_last)</option>
                    <option value={MAY_BE_LAST}>Can be the last word (may_be_last)</option>
                  </select>
                </td>
              {/if}
              <td>
                <button
                  type="button"
                  class="link"
                  data-action="remove-row"
                  aria-label="Remove row {i + 1}"
                  disabled={busy}
                  onclick={() => tables.removeRow(id, i)}>Remove</button
                >
              </td>
            </tr>
          {/each}
        </tbody>
      </table>
      {#if tab.rows.length === 0}
        <p class="help" data-empty>The table has no rows.</p>
      {/if}
      <div class="row">
        <button type="button" class="btn" data-action="add-row" disabled={busy} onclick={() => tables.addRow(id)}
          >Add</button
        >
      </div>

      <div class="check" aria-live="polite" data-check>
        <h3>Check</h3>
        {#if !checkIsCurrent(tab)}
          <p class="help" data-checking>Checking the rows as <code>voicebridge check</code> does.</p>
        {:else if tab.checkError !== null}
          <p class="field-error"><Glyph shape="diamond" /> <span class="word">Problem:</span> The check did not run: {tab.checkError}</p>
        {:else if tab.check}
          {#each tab.check.problems as problem, i (i)}
            <p class="field-error" data-problem>
              <Glyph shape="diamond" /> <span class="word">Problem:</span>
              {problem}
            </p>
          {/each}
          {#each tab.check.warnings as warning, i (i)}
            <p class="note" data-warning>
              <Glyph shape="triangle" /> <span class="word">Warning:</span>
              {warning}
            </p>
          {/each}
          {#if tab.check.problems.length === 0 && tab.check.warnings.length === 0}
            <p class="help" data-check-clean>No problem and no warning.</p>
          {:else if tab.check.problems.length > 0}
            <p class="help">A problem stops the save. Change the rows.</p>
          {/if}
          {#if needsConfirmation(tab) && isDirty(tab)}
            <button type="button" class="btn" data-action="confirm" onclick={() => tables.confirm(id)}
              >I read the warnings: save is permitted</button
            >
          {:else if tab.check.problems.length === 0 && tab.check.warnings.length > 0 && isDirty(tab)}
            <p class="help" data-confirmed>The warnings are confirmed for these rows.</p>
          {/if}
        {/if}
      </div>

      <div class="row actions">
        <button
          type="button"
          class="btn primary"
          data-action="save-table"
          disabled={!canSave(tab)}
          onclick={() => tables.save(id)}>{tab.saving ? 'Saving' : 'Save'}</button
        >
        <button
          type="button"
          class="btn"
          data-action="discard-table"
          disabled={tab.saving}
          onclick={() => tables.discard(id)}>Discard changes and read the file again</button
        >
      </div>
      <div aria-live="polite">
        {#if tab.saveMessage}<p class="help" data-save-message>{tab.saveMessage}</p>{/if}
        {#if tab.saveError}
          <p class="field-error" data-save-error><Glyph shape="diamond" /> {tab.saveError}</p>
        {/if}
        {#if tab.fileChanged}
          <p class="note" data-file-changed>
            Another program changed the file after this screen read it.
            <button type="button" class="link" data-action="read-again-keep-rows" onclick={() => tables.readAgainKeepRows(id)}
              >Read the file again and keep my rows</button
            >
          </p>
        {/if}
        {#if $config.restartNeeded && sessionBusy}
          <p class="note" data-restart-note>
            <Glyph shape="triangle" /> The running session keeps the tables of its start. Stop the session and start
            it again to use the saved tables.
          </p>
        {/if}
      </div>
    {/if}
  </section>

  {#if changed.length > 0}
    <p class="help" data-changed-tabs>
      Not saved: {changed.map((key) => (key.startsWith('aliases:') ? `Aliases (${key.slice(8)})` : tabLabel(key as TableKind))).join(', ')}.
    </p>
  {/if}
</div>

<style>
  .words {
    display: flex;
    flex-direction: column;
    gap: var(--gap);
    max-width: 60rem;
  }

  .tabs {
    display: flex;
    flex-wrap: wrap;
    gap: 4px;
  }

  .tab {
    min-height: 36px;
    padding: 0 12px;
    border: 1px solid var(--line);
    border-radius: var(--radius-s);
    background: transparent;
    font-size: var(--step-0);
    color: var(--muted);
    cursor: pointer;
  }

  .tab[aria-selected='true'] {
    background: var(--raised);
    border-color: var(--text);
    color: var(--text);
    font-weight: 600;
  }

  .unsaved {
    margin-left: 6px;
    padding: 0 5px;
    border: 1px solid var(--tone-dry);
    border-radius: var(--radius-s);
    font-size: var(--step--1);
    font-weight: 600;
    color: var(--tone-dry);
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

  .check h3 {
    font-size: var(--step-0);
    font-weight: 600;
    margin-bottom: 4px;
  }

  .check {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  .row {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 8px;
  }

  .inline {
    font-size: var(--step-0);
  }

  input[type='text'],
  select,
  textarea {
    width: 100%;
    min-width: 0;
    min-height: 36px;
    padding: 6px 8px;
    border: 1px solid var(--line);
    border-radius: var(--radius-m);
    background: var(--ground);
    color: var(--text);
    font: inherit;
    font-size: var(--step-0);
  }

  .row input[type='text'],
  .row select {
    width: auto;
    flex: 1 1 10rem;
  }

  textarea {
    resize: vertical;
    line-height: 1.4;
  }

  .rows {
    width: 100%;
    border-collapse: collapse;
    table-layout: fixed;
  }

  .rows th {
    text-align: left;
    font-size: var(--step--1);
    font-weight: 600;
    color: var(--muted);
    padding: 0 6px 4px 0;
  }

  .rows th:last-child {
    width: 5.5rem;
  }

  .rows td {
    padding: 3px 6px 3px 0;
    vertical-align: top;
  }

  .help {
    font-size: var(--step--1);
    color: var(--muted);
    overflow-wrap: anywhere;
  }

  .path {
    font-family: var(--font-mono, monospace);
  }

  .note {
    font-size: var(--step--1);
    color: var(--tone-dry);
    overflow-wrap: anywhere;
  }

  .field-error {
    font-size: var(--step--1);
    color: var(--tone-error);
    overflow-wrap: anywhere;
  }

  .word {
    font-weight: 600;
  }

  .actions {
    margin-top: 4px;
  }
</style>
