<script lang="ts">
  // The Diagnostics screen: the utterance log, the audio capture, their
  // folders, the report of the EDCoPilot profile with the left-out
  // commands, and the full output of `voicebridge check`.
  import * as api from '../lib/api'
  import { configStore as config } from '../lib/config'
  import StartupNotes from '../lib/components/StartupNotes.svelte'
  import {
    AUDIO_NOTE,
    OFF_NOTE,
    UNINSTALL_NOTE,
    checkOutput,
    createDiagnosticsStore,
    partOn,
    type SwitchPart,
  } from '../lib/diagnostics'
  import { startupStore } from '../lib/startup'
  import { MAX_FILTER_CHARS } from '../lib/commands'
  import { createLeftOutStore, followChecks, leftOutListShown, profileView } from '../lib/profile'
  import { SWITCH_OFF, diagnosticsLine, updateStore as updates } from '../lib/updates'

  const diagnostics = createDiagnosticsStore(api, config)

  let saved = $derived($config.saved)
  let check = $derived($config.check)
  let lines = $derived(check ? checkOutput(check) : [])
  let dataDir = $derived($startupStore?.data_dir ?? null)
  let profile = $derived(profileView(check, $config.checkError))
  let update = $derived($updates.status)
  let checkOn = $derived(saved ? saved.updates.check : null)
  let checkBusy = $derived(
    $updates.checking || (update !== null && ['checking', 'downloading', 'ready'].includes(update.phase)),
  )

  // The full list of the left-out commands loads when the section opens,
  // and again after each new check while it is open.
  const leftOut = createLeftOutStore(api)
  let leftOutOpen = $state(false)
  function toggleLeftOut(event: Event & { currentTarget: HTMLDetailsElement }) {
    leftOutOpen = event.currentTarget.open
    if (leftOutOpen) void leftOut.open()
    else leftOut.close()
  }
  $effect(() => followChecks(config, leftOut))
  // The list goes away when the profile did not load: close it, so that a
  // later check does not load it again unseen.
  $effect(() => {
    if (!leftOutListShown(profile.kind) && leftOutOpen) {
      leftOutOpen = false
      leftOut.close()
    }
  })

  /**
   * Sends the switch. After the call the box shows the saved state again,
   * also after a refusal or an error.
   */
  async function toggle(part: SwitchPart, box: HTMLInputElement): Promise<void> {
    await diagnostics.setPart(part, box.checked)
    const now = $config.saved
    box.checked = now ? partOn(now, part) : false
  }

  interface Part {
    id: SwitchPart
    label: string
    help: string
    path: (c: api.Config) => string
  }

  const PARTS: Part[] = [
    {
      id: 'log',
      label: 'Utterance log',
      help: 'One line for each utterance: the time, the result, the heard text, the token, the confidence, the times, and the levels.',
      path: (c) => c.diagnostics.log_file,
    },
    {
      id: 'audio',
      label: 'Audio capture',
      help: `The audio of each utterance as a WAV file. The program keeps the newest 500 files. ${AUDIO_NOTE}`,
      path: (c) => c.diagnostics.save_audio_dir,
    },
  ]
</script>

<section class="diagnostics" aria-labelledby="diagnostics-title">
  <h2 id="diagnostics-title">Diagnostics</h2>
  <p class="intro">Data for tuning. A change applies at the next start of the session.</p>
  <StartupNotes />

  {#if $config.loadError}
    <p class="error" role="alert">{$config.loadError}</p>
  {:else if saved}
    <div class="panel">
      {#each PARTS as part (part.id)}
        {@const on = partOn(saved, part.id)}
        <div class="field" data-part={part.id}>
          <label class="option check">
            {#key $diagnostics.calls}
              <input
                type="checkbox"
                checked={on}
                disabled={$diagnostics.busy !== null}
                onchange={(e) => void toggle(part.id, e.currentTarget)}
                aria-describedby="{part.id}-help"
              />
            {/key}
            {part.label}
          </label>
          <p class="help" id="{part.id}-help">{part.help}</p>
          {#if on}
            <p class="where">
              <span class="path" data-path>{part.path(saved)}</span>
              <button type="button" class="btn" data-open={part.id} onclick={() => diagnostics.open(part.id)}
                >Open folder</button
              >
            </p>
            <p class="help" data-off-note>{OFF_NOTE}</p>
          {/if}
        </div>
      {/each}
      {#if $diagnostics.problems.length > 0}
        <ul class="problems" role="alert">
          {#each $diagnostics.problems as problem, i (i)}
            <li>{problem}</li>
          {/each}
        </ul>
      {/if}
      {#if $diagnostics.error}
        <p class="error" role="alert" data-error>{$diagnostics.error}</p>
      {/if}
      {#if $config.restartNeeded}
        <p class="note">Restart the session to use the change.</p>
      {/if}
    </div>
  {/if}

  <div class="panel" data-part="data">
    <h3>Data folder</h3>
    {#if dataDir}
      <p class="where">
        <span class="path" data-path>{dataDir}</span>
        <button type="button" class="btn" data-open="data" onclick={() => diagnostics.open('data')}
          >Open folder</button
        >
      </p>
    {:else}
      <p class="help">This start of the program has no data folder.</p>
    {/if}
    <p class="help" data-uninstall-note>{UNINSTALL_NOTE}</p>
    {#if !saved && $diagnostics.error}
      <p class="error" role="alert" data-error>{$diagnostics.error}</p>
    {/if}
  </div>

  <div class="panel" data-part="updates">
    <h3>Updates</h3>
    {#if update}
      <p class="help" data-update-version>You have version {update.current_version}.</p>
      <p data-update-line>{diagnosticsLine(update, checkOn)}</p>
      {#if checkOn === false && update.phase !== 'idle' && !update.developer_build}
        <p class="help" data-update-off>{SWITCH_OFF}</p>
      {/if}
    {/if}
    <p>
      <button type="button" class="btn" data-action="check-updates" disabled={checkBusy} onclick={() => updates.check()}
        >Check for updates</button
      >
    </p>
    {#if $updates.checkError}
      <p class="error" role="alert" data-update-error>{$updates.checkError}</p>
    {/if}
  </div>

  <div class="panel" data-profile-report={profile.kind}>
    <h3>Commands from the EDCoPilot profile</h3>
    <p class="help">
      The program reads the commands from the EDCoPilot profile at each start. A command that the program does not
      fully understand is left out: it gives no match. After an update of EDCoPilot, stop and start the session.
    </p>
    {#if profile.kind === 'waiting'}
      <p class="help">Checking.</p>
    {/if}
    {#if profile.lines.length > 0}
      <ul class="lines" data-profile-lines>
        {#each profile.lines as line, i (i)}<li>{line}</li>{/each}
      </ul>
    {/if}
    {#if profile.problems.length > 0}
      <ul class="problems" role="alert" data-profile-problems>
        {#each profile.problems as problem, i (i)}<li>{problem}</li>{/each}
      </ul>
    {/if}
    {#if leftOutListShown(profile.kind)}
      <details data-left-out ontoggle={toggleLeftOut}>
        <summary>Every left-out command, with its reason</summary>
        <!-- After a check that did not run, the panel shows NOT_CHECKED above: the list shows nothing. -->
        {#if leftOutOpen && !$leftOut.notChecked}
          <label class="visually-hidden" for="left-out-filter">Filter</label>
          <input
            id="left-out-filter"
            type="search"
            autocomplete="off"
            spellcheck="false"
            placeholder="Filter: each word must be in the line"
            maxlength={MAX_FILTER_CHARS}
            value={$leftOut.filter}
            oninput={(e) => void leftOut.setFilter(e.currentTarget.value)}
          />
          {#if $leftOut.error}
            <p class="error" role="alert">{$leftOut.error}</p>
          {:else if $leftOut.list}
            <p class="help" aria-live="polite">
              {$leftOut.list.total} commands. {$leftOut.list.rows.length} shown.
            </p>
            {#if $leftOut.notice}<p class="note">{$leftOut.notice}</p>{/if}
            <ul class="lines output" data-left-out-rows>
              {#each $leftOut.list.rows as row, i (i)}<li>{row.line}</li>{/each}
            </ul>
            {#if $leftOut.list.rows.length < $leftOut.list.total}
              <button type="button" class="btn" disabled={$leftOut.loading} onclick={() => leftOut.showMore()}
                >Show more</button
              >
            {/if}
          {:else}
            <p class="help">Reading the list.</p>
          {/if}
        {/if}
      </details>
    {/if}
  </div>

  <div class="panel">
    <div class="head">
      <h3>Output of the check</h3>
      <button type="button" class="btn" disabled={$config.checking} onclick={() => config.checkAgain()}
        >Check again</button
      >
    </div>
    <p class="help">The same lines as <span class="path">voicebridge check</span> for {saved?.path ?? 'the configuration file'}.</p>
    {#if $config.checkError}
      <p class="error" role="alert">{$config.checkError}</p>
    {:else if check}
      <pre class="output" data-check-output>{lines.join('\n')}</pre>
    {:else}
      <p class="help">Checking.</p>
    {/if}
  </div>
</section>

<style>
  .diagnostics {
    display: grid;
    gap: var(--gap);
    max-width: 52rem;
  }

  h2 {
    font-size: var(--step-1);
  }

  h3 {
    font-size: var(--step-0);
  }

  .intro,
  .help {
    color: var(--muted);
    font-size: var(--step--1);
  }

  .panel {
    display: grid;
    gap: var(--gap);
    padding: 14px 16px;
    background: var(--panel);
    border-radius: var(--radius-s);
  }

  .field {
    display: grid;
    gap: 4px;
  }

  .option {
    display: inline-flex;
    align-items: center;
    gap: 8px;
    font-weight: 600;
  }

  .where {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 8px;
  }

  .path {
    font-family: Consolas, 'Cascadia Mono', monospace;
    font-size: var(--step--1);
    overflow-wrap: anywhere;
  }

  .head {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
  }

  .output {
    margin: 0;
    padding: 10px 12px;
    background: var(--ground);
    border: 1px solid var(--line);
    border-radius: var(--radius-s);
    font-family: Consolas, 'Cascadia Mono', monospace;
    font-size: var(--step--1);
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }

  .lines {
    display: grid;
    gap: 2px;
    margin: 0;
    padding-left: 1.2em;
    font-size: var(--step--1);
    overflow-wrap: anywhere;
  }

  input[type='search'] {
    width: 100%;
    min-height: 36px;
    margin: 8px 0;
    padding: 0 10px;
    border: 1px solid var(--line);
    border-radius: var(--radius-m);
    background: var(--ground);
    color: var(--text);
    font: inherit;
  }

  .problems,
  .error {
    color: var(--tone-error);
    font-size: var(--step--1);
  }

  .note {
    color: var(--tone-dry);
    font-size: var(--step--1);
  }
</style>
