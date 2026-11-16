<script lang="ts">
  import { onDestroy, onMount } from 'svelte'
  import Glyph from '../lib/components/Glyph.svelte'
  import {
    absentDeviceNote,
    fieldNotes,
    bindingChanged,
    bindingLabel,
    changedScreens,
    configStore as config,
    deviceLabel,
    deviceStatus,
    fieldErrors,
    logicalCores,
    MAX_PTT_EXTRA_MS,
    MAX_VAD_DBFS,
    MIN_VAD_DBFS,
    modeOf,
    parseDecimal,
    parseWhole,
    wakeWordProblem,
    wakeWordWarning,
    type FieldKey,
  } from '../lib/config'
  import { sessionStore as session } from '../lib/session'

  const cores = logicalCores()

  let draft = $derived($config.draft)
  let saved = $derived($config.saved)
  let errors = $derived(fieldErrors($config, cores))
  let notes = $derived(fieldNotes($config))
  let absentNote = $derived(absentDeviceNote($config))
  // While a capture runs, each control except Cancel is off: a key that the
  // user types goes to the capture.
  let capturing = $derived($config.capturing)
  let mode = $derived(modeOf(draft))
  let devices = $derived($config.devices ?? [])
  let warning = $derived(wakeWordWarning(draft))
  let otherChanges = $derived(changedScreens($config).includes('setup'))
  let canSave = $derived($config.dirty && !$config.saving && !$config.capturing)
  let sessionBusy = $derived($session.running || $session.starting)

  // The device names of the file that the list does not have: the saved
  // name and the draft name. They stay in the list with their state, so
  // that an unplugged device does not drop out of the file silently.
  let extraNames = $derived(
    [saved?.microphone.device ?? '', draft?.microphone.device ?? ''].filter(
      (name, i, all) => name.trim() !== '' && !devices.includes(name) && all.indexOf(name) === i,
    ),
  )
  let device = $derived(deviceStatus(draft?.microphone.device ?? '', devices))

  /** The problems of a refused save that name one of these keys. */
  function rustProblems(...keys: string[]): string[] {
    if ($config.saveOutcome !== 'refused') return []
    return $config.problems.filter((p) => keys.some((key) => p === key || p.startsWith(`${key} `)))
  }

  function problemsOf(key: FieldKey): string[] {
    const page = errors[key]
    return page ? [page, ...rustProblems(key)] : rustProblems(key)
  }

  function describedBy(key: FieldKey, help = `${key}-help`): string {
    const ids = [help]
    if (problemsOf(key).length > 0) ids.push(`${key}-error`)
    if (notes[key]) ids.push(`${key}-note`)
    return ids.join(' ')
  }

  // When a capture ends, the focus goes back to "Press to set".
  let captureButton = $state<HTMLButtonElement | null>(null)
  let wasCapturing = false
  $effect(() => {
    const now = $config.capturing
    if (wasCapturing && !now) captureButton?.focus()
    wasCapturing = now
  })

  // The number fields keep the text as typed, as on the Setup screen.
  const numberText = (value: number | undefined) =>
    value === undefined || Number.isNaN(value) ? '' : String(value)
  const sameNumber = (a: number, b: number) => Object.is(a, b) || a === b

  let preRollText = $state('')
  let tailText = $state('')
  let vadText = $state('')

  $effect(() => {
    const value = draft?.push_to_talk.pre_roll_ms
    if (value !== undefined && !sameNumber(parseWhole(preRollText), value)) preRollText = numberText(value)
  })
  $effect(() => {
    const value = draft?.push_to_talk.release_tail_ms
    if (value !== undefined && !sameNumber(parseWhole(tailText), value)) tailText = numberText(value)
  })
  $effect(() => {
    const value = draft?.microphone.vad_threshold_dbfs
    if (value !== undefined && !sameNumber(parseDecimal(vadText), value)) vadText = numberText(value)
  })

  // The wake word that the user types before Add.
  let newWord = $state('')
  let wordProblem = $state<string | null>(null)

  function addWord() {
    const list = draft?.always_listening.wake_words ?? []
    wordProblem = wakeWordProblem(newWord, list)
    if (wordProblem !== null) return
    config.set('always_listening.wake_words', [...list, newWord.trim()])
    newWord = ''
  }

  function wordKey(event: KeyboardEvent) {
    if (event.key === 'Enter') {
      event.preventDefault()
      addWord()
    }
  }

  function removeWord(index: number) {
    const list = draft?.always_listening.wake_words ?? []
    config.set(
      'always_listening.wake_words',
      list.filter((_, i) => i !== index),
    )
  }

  onMount(() => {
    void config.loadDevices()
  })

  // A capture must not stay after the screen is gone: it blocks a start.
  onDestroy(() => {
    void config.cancelCapture()
  })
</script>

{#snippet fieldError(key: FieldKey)}
  {#if problemsOf(key).length > 0}
    <div class="field-error" id="{key}-error">
      {#each problemsOf(key) as problem, i (i)}
        <p><Glyph shape="diamond" /> <span class="word">Problem:</span> {problem}</p>
      {/each}
    </div>
  {/if}
{/snippet}

{#snippet fieldNote(key: FieldKey)}
  {#if notes[key]}
    <p class="help" id="{key}-note" data-field-note={key}><span class="word">Note:</span> {notes[key]}</p>
  {/if}
{/snippet}

{#snippet noWakeWord()}
  {#if warning}
    <p class="warn" data-wake-warning><Glyph shape="triangle" /> <span class="word">Warning:</span> {warning}</p>
  {/if}
{/snippet}

<div class="microphone">
  {#if $config.loadError !== null}
    <section class="panel alarm" role="alert" aria-labelledby="mic-load-title" data-load-error>
      <h2 id="mic-load-title"><Glyph shape="diamond" /> The settings did not load</h2>
      <p class="plain">{$config.loadError}</p>
      <p class="help">Open the Setup screen to see the problem.</p>
    </section>
  {:else if draft === null || saved === null}
    <p class="help">Reading the settings.</p>
  {:else}
    <fieldset class="form" disabled={$config.saving}>
      <legend class="visually-hidden">Microphone</legend>

      <fieldset class="inert" disabled={capturing}>
      <section class="group" aria-labelledby="g-device">
        <h2 id="g-device">Input device</h2>
        <div class="field">
          <label for="microphone.device">Device</label>
          <div class="row">
            <select
              id="microphone.device"
              value={draft.microphone.device}
              onchange={(e) => config.set('microphone.device', e.currentTarget.value)}
              aria-invalid={problemsOf('microphone.device').length > 0}
              aria-describedby={describedBy('microphone.device')}
            >
              <option value="">Default device</option>
              {#each devices as name, i (i)}
                <option value={name}>{name}</option>
              {/each}
              {#each extraNames as name (name)}
                <option value={name}>{deviceLabel(name, devices)}</option>
              {/each}
            </select>
            <button type="button" class="btn" data-action="list-devices" onclick={() => config.loadDevices()}
              >Refresh</button
            >
          </div>
          <p class="help" id="microphone.device-help">
            {#if $config.devices === null && $config.devicesError !== null}
              The connected devices are not known now. Use Refresh to try again.
            {:else if $config.devices === null}
              Reading the devices.
            {:else if device.kind === 'default'}
              The default input device of Windows.
            {:else if device.kind === 'listed'}
              A connected device.
            {:else if device.kind === 'matched'}
              The file names "{draft.microphone.device}". The program uses the connected device
              "{device.device}".
            {/if}
          </p>
          {#if absentNote}
            <p class="note" data-device-absent><Glyph shape="triangle" /> {absentNote}</p>
          {/if}
          {#if $config.devicesError}
            <p class="field-error">
              <Glyph shape="diamond" /> <span class="word">Problem:</span> The device list did not load: {$config.devicesError}
            </p>
          {/if}
          {@render fieldError('microphone.device')}
        </div>
      </section>

      <section class="group" aria-labelledby="g-mode">
        <h2 id="g-mode">Mode</h2>
        <div class="field" role="radiogroup" aria-labelledby="mode-label" aria-describedby="mode-help">
          <span class="label" id="mode-label">How the program listens</span>
          <div class="choice">
            <label class="option">
              <input
                type="radio"
                name="mode"
                value="push_to_talk"
                checked={mode === 'push_to_talk'}
                onchange={() => config.set('microphone.mode', 'push_to_talk')}
              />
              Push-to-talk
            </label>
            <label class="option">
              <input
                type="radio"
                name="mode"
                value="always_listening"
                checked={mode === 'always_listening'}
                onchange={() => config.set('microphone.mode', 'always_listening')}
              />
              Always listening
            </label>
          </div>
          {#if mode === null}
            <p class="field-error" data-unknown-mode>
              <Glyph shape="diamond" /> <span class="word">Problem:</span> Unknown mode "{draft.microphone.mode}",
              choose one.
            </p>
          {/if}
          <p class="help" id="mode-help">
            Push-to-talk records only while you hold the talk button. Always listening records each
            phrase that the microphone hears.
          </p>
          {@render fieldError('microphone.mode')}
        </div>
      </section>
      </fieldset>

      {#if mode === 'push_to_talk'}
        <section class="group" aria-labelledby="g-ptt">
          <h2 id="g-ptt">Push-to-talk</h2>
          <div class="field">
            <span class="label" id="binding-label">Talk button</span>
            <div class="row binding-row">
              <output class="binding" aria-labelledby="binding-label" data-binding>{bindingLabel(draft.push_to_talk)}</output>
              {#if bindingChanged($config)}
                <span class="pending" data-binding-pending><Glyph shape="half" /> Not saved</span>
              {/if}
            </div>
            <div class="buttons">
              <button
                type="button"
                class="btn"
                data-action="capture"
                disabled={sessionBusy}
                aria-describedby="binding-help"
                bind:this={captureButton}
                onclick={() => config.captureBinding()}
                >{$config.capturing ? 'Waiting for a press' : 'Press to set'}</button
              >
              {#if $config.capturing}
                <button type="button" class="btn" data-action="cancel-capture" onclick={() => config.cancelCapture()}
                  >Cancel</button
                >
              {/if}
            </div>
            <div role="status" aria-live="polite">
              {#if $config.capturing}
                <p class="note" data-capture="waiting">
                  <Glyph shape="half" /> Press and release one key or one joystick button now. Each key counts,
                  also Tab. Esc cancels.
                </p>
              {:else if $config.captureMessage}
                <p class="help" data-capture="message">{$config.captureMessage}</p>
              {/if}
            </div>
            {#if $config.captureError}
              <p class="field-error" role="alert" data-capture="error">
                <Glyph shape="diamond" /> <span class="word">Not set:</span> {$config.captureError}
              </p>
            {/if}
            <p class="help" id="binding-help">
              {#if sessionBusy}
                Stop the session to set the talk button. The session reads the talk button.
              {:else}
                Click "Press to set", then press and release the key or the joystick button. Each key
                counts, also Tab. Esc cancels. Only one input counts: a key combination is not a talk
                button. The left, right and middle mouse buttons cannot be the talk button. The other
                fields are off until the capture ends.
              {/if}
            </p>
            {#each rustProblems('push_to_talk', 'push_to_talk.key', 'push_to_talk.joystick') as problem, i (i)}
              <p class="field-error"><Glyph shape="diamond" /> <span class="word">Problem:</span> {problem}</p>
            {/each}
          </div>

          <fieldset class="inert" disabled={capturing}>
          <div class="pair">
            <div class="field">
              <label for="push_to_talk.pre_roll_ms">Pre-roll (ms)</label>
              <input
                id="push_to_talk.pre_roll_ms"
                type="text"
                inputmode="numeric"
                spellcheck="false"
                autocomplete="off"
                bind:value={preRollText}
                oninput={() => config.set('push_to_talk.pre_roll_ms', parseWhole(preRollText))}
                aria-invalid={problemsOf('push_to_talk.pre_roll_ms').length > 0}
                aria-describedby={describedBy('push_to_talk.pre_roll_ms')}
              />
              <p class="help" id="push_to_talk.pre_roll_ms-help">
                Audio from before the press, so that the first word is complete. From 0 to {MAX_PTT_EXTRA_MS}.
                0 is off.
              </p>
              {@render fieldError('push_to_talk.pre_roll_ms')}
            </div>
            <div class="field">
              <label for="push_to_talk.release_tail_ms">Release tail (ms)</label>
              <input
                id="push_to_talk.release_tail_ms"
                type="text"
                inputmode="numeric"
                spellcheck="false"
                autocomplete="off"
                bind:value={tailText}
                oninput={() => config.set('push_to_talk.release_tail_ms', parseWhole(tailText))}
                aria-invalid={problemsOf('push_to_talk.release_tail_ms').length > 0}
                aria-describedby={describedBy('push_to_talk.release_tail_ms')}
              />
              <p class="help" id="push_to_talk.release_tail_ms-help">
                Audio after the release, so that the last word is complete. From 0 to {MAX_PTT_EXTRA_MS}.
                0 is off.
              </p>
              {@render fieldError('push_to_talk.release_tail_ms')}
            </div>
          </div>
          </fieldset>
        </section>
      {:else if mode === 'always_listening'}
        <fieldset class="inert" disabled={capturing}>
        <section class="group" aria-labelledby="g-always">
          <h2 id="g-always">Always listening</h2>
          <div class="field">
            <span class="label" id="words-label">Wake words</span>
            {#if draft.always_listening.wake_words.length > 0}
              <ul class="words" aria-labelledby="words-label" data-wake-words>
                {#each draft.always_listening.wake_words as word, i (i)}
                  <li>
                    <span class="entry">{word}</span>
                    <button
                      type="button"
                      class="link"
                      aria-label="Remove the wake word {word}"
                      onclick={() => removeWord(i)}>Remove</button
                    >
                  </li>
                {/each}
              </ul>
            {:else}
              <p class="help" data-wake-words-empty>No wake word.</p>
            {/if}
            <div class="row">
              <input
                id="new-wake-word"
                type="text"
                spellcheck="false"
                autocomplete="off"
                aria-label="New wake word"
                bind:value={newWord}
                oninput={() => (wordProblem = null)}
                onkeydown={wordKey}
                aria-invalid={wordProblem !== null}
                aria-describedby={wordProblem !== null
                  ? 'always_listening.wake_words-help new-wake-word-error'
                  : 'always_listening.wake_words-help'}
              />
              <button type="button" class="btn" data-action="add-wake-word" onclick={addWord}>Add</button>
            </div>
            {#if wordProblem}
              <p class="field-error" id="new-wake-word-error" role="alert"><Glyph shape="diamond" /> <span class="word">Not added:</span> {wordProblem}</p>
            {/if}
            <p class="help" id="always_listening.wake_words-help">
              A phrase must start with a wake word, for example "copilot" and then the command. The program
              removes the wake word before it matches the phrase. Case and punctuation do not count.
            </p>
            {@render noWakeWord()}
            {@render fieldError('always_listening.wake_words')}
            {@render fieldNote('always_listening.wake_words')}
          </div>

          <div class="field narrow">
            <label for="microphone.vad_threshold_dbfs">Voice threshold (dBFS)</label>
            <input
              id="microphone.vad_threshold_dbfs"
              type="text"
              inputmode="decimal"
              spellcheck="false"
              autocomplete="off"
              bind:value={vadText}
              oninput={() => config.set('microphone.vad_threshold_dbfs', parseDecimal(vadText))}
              aria-invalid={problemsOf('microphone.vad_threshold_dbfs').length > 0}
              aria-describedby={describedBy('microphone.vad_threshold_dbfs')}
            />
            <p class="help" id="microphone.vad_threshold_dbfs-help">
              Sound above this level counts as voice. A higher value (for example -30) ignores more
              background noise. This screen accepts values from {MIN_VAD_DBFS} to {MAX_VAD_DBFS}.
            </p>
            {@render fieldError('microphone.vad_threshold_dbfs')}
            {@render fieldNote('microphone.vad_threshold_dbfs')}
          </div>
        </section>
        </fieldset>
      {/if}
    </fieldset>

    <section class="actions" aria-label="Save">
      {@render noWakeWord()}
      <div class="buttons">
        <button
          type="button"
          class="btn primary"
          data-action="save"
          disabled={!canSave}
          onclick={() => config.save('microphone')}>{$config.saving ? 'Saving' : 'Save'}</button
        >
        <button
          type="button"
          class="btn"
          data-action="discard"
          disabled={!$config.dirty || $config.saving || capturing}
          onclick={() => config.discard()}>Discard changes</button
        >
        {#if $config.dirty}
          <span class="pending" data-pending><Glyph shape="half" /> Unsaved changes</span>
        {/if}
      </div>
      {#if otherChanges}
        <p class="help" data-other-changes>
          The Setup screen has unsaved changes too. Save writes them, and Discard removes them.
        </p>
      {/if}
      <p class="help">The settings go to <span class="path">{saved.path}</span>.</p>

      <div class="result" role="status" aria-live="polite" data-save-result={$config.saveOutcome ?? 'none'}>
        {#if $config.saveError}
          <p class="field-error"><Glyph shape="diamond" /> <span class="word">Not saved:</span> {$config.saveError}</p>
        {:else if $config.saveOutcome === 'refused'}
          <div class="field-error">
            <p><Glyph shape="diamond" /> <span class="word">Not saved.</span> The file did not change. Fix these problems:</p>
            <ul class="lines">
              {#each $config.problems as problem, i (i)}
                <li>{problem}</li>
              {/each}
            </ul>
            {#if $config.fileChanged}
              <p data-file-changed>
                Another program changed the file after this screen read it.
                <button type="button" class="link" data-action="read-again-keep-changes" onclick={() => config.readAgainKeepingChanges()}
                  >Read the file again and keep my changes</button
                >
              </p>
            {/if}
          </div>
        {:else if $config.fileChanged}
          <p class="field-error" data-file-changed>
            <Glyph shape="diamond" /> Another program changed the file after this screen read it. Your changes are not
            saved.
            <button type="button" class="link" data-action="read-again-keep-changes" onclick={() => config.readAgainKeepingChanges()}
              >Read the file again and keep my changes</button
            >
          </p>
        {:else if $config.saveOutcome === 'saved'}
          <p class="done"><Glyph shape="dot" /> Saved.</p>
          {#if $config.problems.length > 0}
            <div class="todo">
              <p>Still to fix (these problems were in the file before):</p>
              <ul class="lines">
                {#each $config.problems as problem, i (i)}
                  <li>{problem}</li>
                {/each}
              </ul>
            </div>
          {/if}
        {:else if $config.saveOutcome === 'unchanged'}
          <p class="help">Nothing to save: the file already has these values.</p>
        {/if}
        {#if $config.restartNeeded && sessionBusy}
          <p class="note" data-restart><Glyph shape="triangle" /> The running session keeps its old settings until you stop and start it.</p>
        {/if}
      </div>
    </section>
  {/if}
</div>

<style>
  .microphone {
    display: flex;
    flex-direction: column;
    gap: var(--gap);
    max-width: 46rem;
  }

  .panel {
    padding: 10px 14px;
    background: var(--panel);
    border-radius: var(--radius-s);
  }

  .alarm {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: 8px;
    border: 1px solid var(--tone-error);
    border-left-width: 4px;
  }

  .alarm h2 {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: var(--step-1);
    color: var(--tone-error);
  }

  .plain {
    overflow-wrap: anywhere;
    white-space: pre-wrap;
  }

  .lines {
    font-size: var(--step--1);
    align-self: stretch;
  }

  .lines li {
    padding: 3px 0;
    border-top: 1px solid var(--raised);
    overflow-wrap: anywhere;
  }

  .lines li:first-child {
    border-top: 0;
  }

  .form {
    display: flex;
    flex-direction: column;
    gap: var(--gap);
    min-width: 0;
    margin: 0;
    padding: 0;
    border: 0;
  }

  /* A group of controls that a capture turns off. It adds no box. */
  .inert {
    display: contents;
  }

  .group {
    display: flex;
    flex-direction: column;
    gap: 14px;
    padding: 12px 14px 14px;
    background: var(--panel);
    border-radius: var(--radius-s);
  }

  .group h2 {
    font-size: var(--step-1);
    font-weight: 600;
  }

  .field {
    display: flex;
    flex-direction: column;
    gap: 4px;
    min-width: 0;
  }

  .field.narrow {
    max-width: 14rem;
  }

  .pair {
    display: grid;
    gap: 14px;
  }

  @media (min-width: 560px) {
    .pair {
      grid-template-columns: minmax(0, 1fr) minmax(0, 1fr);
    }
  }

  label,
  .label {
    font-size: var(--step-0);
    font-weight: 600;
    color: var(--text);
  }

  .row {
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
  }

  .row input,
  .row select {
    flex: 1 1 14rem;
  }

  input[type='text'],
  select {
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

  input[type='text']:hover:not(:disabled),
  select:hover:not(:disabled) {
    border-color: var(--muted);
  }

  input[aria-invalid='true'],
  select[aria-invalid='true'] {
    border-color: var(--tone-error);
    border-width: 2px;
  }

  input:disabled,
  select:disabled {
    border-style: dashed;
    background: transparent;
    color: var(--faint);
    cursor: not-allowed;
  }

  .choice {
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
  }

  .option {
    display: inline-flex;
    align-items: center;
    gap: 10px;
    min-height: 40px;
    padding: 0 12px;
    border: 1px solid var(--line);
    border-radius: var(--radius-m);
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

  .binding-row {
    align-items: center;
  }

  .binding {
    min-width: 6rem;
    padding: 6px 12px;
    border: 1px solid var(--line);
    border-radius: var(--radius-m);
    background: var(--ground);
    font-family: ui-monospace, 'Cascadia Mono', Consolas, monospace;
    font-size: var(--step-1);
    font-weight: 600;
    overflow-wrap: anywhere;
  }

  .words {
    display: flex;
    flex-direction: column;
    align-self: stretch;
  }

  .words li {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 10px;
    padding: 4px 0;
    border-top: 1px solid var(--raised);
  }

  .words li:first-child {
    border-top: 0;
  }

  .entry {
    overflow-wrap: anywhere;
  }

  .help {
    font-size: var(--step--1);
    color: var(--muted);
    overflow-wrap: anywhere;
  }

  .path {
    font-family: ui-monospace, 'Cascadia Mono', Consolas, monospace;
    color: var(--text);
    overflow-wrap: anywhere;
  }

  .field-error {
    font-size: var(--step--1);
    color: var(--tone-error);
    overflow-wrap: anywhere;
  }

  .field-error p + p {
    margin-top: 2px;
  }

  .word {
    font-weight: 700;
  }

  .note {
    font-size: var(--step--1);
    color: var(--tone-dry);
    overflow-wrap: anywhere;
  }

  .warn {
    padding: 8px 10px;
    border-left: 3px solid var(--tone-dry);
    background: var(--raised);
    font-size: var(--step-0);
    color: var(--text);
    overflow-wrap: anywhere;
  }

  .buttons {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 8px;
  }

  .actions {
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: 12px 14px;
    background: var(--panel);
    border-radius: var(--radius-s);
  }

  .pending {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    font-size: var(--step--1);
    font-weight: 600;
    color: var(--tone-dry);
  }

  .result {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  .result:empty {
    display: none;
  }

  .done {
    font-weight: 600;
    color: var(--tone-sent);
  }

  .todo {
    font-size: var(--step--1);
    color: var(--tone-dry);
  }
</style>
