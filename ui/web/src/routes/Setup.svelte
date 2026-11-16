<script lang="ts">
  import Glyph from '../lib/components/Glyph.svelte'
  import { pickFolder } from '../lib/api'
  import {
    changedScreens,
    clearWhenBlocked,
    configStore as config,
    engineOf,
    fieldErrors,
    BASE_URL_NOTE,
    checkOkText,
    hostWarning,
    keyFieldBlock,
    keyStatus,
    logicalCores,
    NAME_PROBLEM,
    parseDecimal,
    parseWhole,
    type FieldKey,
  } from '../lib/config'
  import {
    busy,
    downloadBlock,
    megabytes,
    modelStore as model,
    NO_DATA_DIR,
    percent,
    progressText,
    type ModelState,
  } from '../lib/model'
  import { sessionStore as session } from '../lib/session'
  import { profileField } from '../lib/profile'
  import { sentence } from '../lib/format'
  import { SWITCH_HELP } from '../lib/updates'

  const cores = logicalCores()

  /** The text of the download step in work. */
  function stepText(s: ModelState): string {
    switch (s.phase) {
      case 'starting':
        return 'Starting the download.'
      case 'downloading':
        return `Downloading: ${progressText(s)}.`
      case 'verifying':
        return 'Checking the downloaded file.'
      case 'unpacking':
        return 'Unpacking the model.'
      default:
        return ''
    }
  }

  let draft = $derived($config.draft)
  let saved = $derived($config.saved)
  let errors = $derived(fieldErrors($config, cores))
  let engine = $derived(engineOf(draft))
  let local = $derived(engine === 'parakeet')
  let cloud = $derived(engine === 'openai_compatible')
  let keyBlock = $derived(keyFieldBlock($config))
  let hostNote = $derived(hostWarning($config))
  let canSave = $derived($config.dirty && !$config.saving)
  let profile = $derived(profileField(saved, $config.check, $config.checkError))

  /** The problems of a refused save that name this key. */
  function rustProblems(key: FieldKey): string[] {
    if ($config.saveOutcome !== 'refused') return []
    return $config.problems.filter((p) => p === key || p.startsWith(`${key} `))
  }

  /** The text of a field error: the page check first, then the Rust problems. */
  function problemsOf(key: FieldKey): string[] {
    const page = errors[key]
    return page ? [page, ...rustProblems(key)] : rustProblems(key)
  }

  function describedBy(key: FieldKey, help = `${key}-help`): string {
    const ids = [help]
    if (problemsOf(key).length > 0) ids.push(`${key}-error`)
    return ids.join(' ')
  }

  // The number fields keep the text as typed, so that a text that is not a
  // number stays visible with its error. Only plain decimal text counts. A
  // comma is a decimal mark too.
  const numberText = (value: number | undefined) =>
    value === undefined || Number.isNaN(value) ? '' : String(value)
  const sameNumber = (a: number, b: number) => Object.is(a, b) || a === b

  let thresholdText = $state('')
  let threadsText = $state('')
  let timeoutText = $state('')

  // A load, a discard or a save gives the draft new values: show them.
  $effect(() => {
    const value = draft?.matching.threshold
    if (value !== undefined && !sameNumber(parseDecimal(thresholdText), value)) {
      thresholdText = numberText(value)
    }
  })
  $effect(() => {
    const value = draft?.stt.parakeet.threads
    if (value !== undefined && !sameNumber(parseWhole(threadsText), value)) {
      threadsText = numberText(value)
    }
  })
  $effect(() => {
    const value = draft?.stt.openai_compatible.timeout_seconds
    if (value !== undefined && !sameNumber(parseWhole(timeoutText), value)) {
      timeoutText = numberText(value)
    }
  })

  function text(key: FieldKey) {
    return (event: Event & { currentTarget: HTMLInputElement }) =>
      config.set(key, event.currentTarget.value)
  }

  let dialogError = $state<string | null>(null)

  async function browse(key: 'edcopilot.dir' | 'stt.parakeet.model_dir', title: string) {
    dialogError = null
    const start = key === 'edcopilot.dir' ? draft?.edcopilot.dir : draft?.stt.parakeet.model_dir
    try {
      const folder = await pickFolder(title, start ?? '')
      if (folder !== null) config.set(key, folder)
    } catch (error) {
      dialogError = error instanceof Error ? error.message : String(error)
    }
  }

  // The key field. The key is only in the input element until the user
  // sends it. Then the input is cleared at once, before the call.
  let keyInput = $state<HTMLInputElement | null>(null)
  let confirmRemove = $state(false)

  // When the key field turns off (the server or the name has a draft
  // change), a typed key is removed. It must not go to another server later.
  $effect(() => {
    clearWhenBlocked(keyInput, keyBlock)
  })

  // A refused text in the variable name field can be a key. When the user
  // leaves the field, the saved name comes back, so the text does not stay
  // in the store.
  function leaveName() {
    config.dropInvalidName()
  }

  function sendKey() {
    if (!keyInput || keyBlock !== null) return
    const value = keyInput.value
    keyInput.value = ''
    void config.saveApiKey(value)
  }

  function keyDown(event: KeyboardEvent) {
    if (event.key === 'Enter') {
      event.preventDefault()
      sendKey()
    }
  }

  async function removeKey() {
    confirmRemove = false
    await config.removeApiKey()
  }

  async function tryAgain() {
    await config.checkAgain()
  }
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

<div class="setup">
  {#if $config.loadError !== null}
    <section class="panel alarm" role="alert" aria-labelledby="load-title" data-load-error>
      <h2 id="load-title"><Glyph shape="diamond" /> The settings did not load</h2>
      <p class="plain">{$config.loadError}</p>
      <p class="help">
        Correct the file in a text editor, or put a copy of voicebridge.example.toml in its place. The
        program does not change the file while it does not load.
      </p>
      <button type="button" class="btn" data-action="load-again" disabled={$config.loading} onclick={tryAgain}
        >Try again</button
      >
    </section>
  {:else if draft === null || saved === null}
    <p class="help">Reading the settings.</p>
  {:else}
    {#if $config.readError}
      <p class="field-error" role="alert" data-read-error>
        <Glyph shape="diamond" /> <span class="word">Problem:</span> The settings file did not read again: {sentence($config.readError)}
        The screen shows the settings of the last read.
      </p>
    {/if}
    {#if $config.checkError}
      <p class="field-error" role="alert" data-check="error">
        <Glyph shape="diamond" /> <span class="word">Problem:</span> The check did not run: {sentence($config.checkError)}
        <button type="button" class="link" data-action="check" disabled={$config.checking} onclick={() => config.checkAgain()}
          >Check again</button
        >
      </p>
    {:else if $config.check && $config.check.problems.length > 0}
      <section class="panel alarm" aria-labelledby="check-title" data-check="problems">
        <h2 id="check-title"><Glyph shape="diamond" /> Start is off until you fix these problems</h2>
        <ul class="lines">
          {#each $config.check.problems as problem, i (i)}
            <li>{problem}</li>
          {/each}
        </ul>
        <button type="button" class="btn" data-action="check" disabled={$config.checking} onclick={() => config.checkAgain()}
          >Check again</button
        >
      </section>
    {:else if $config.check}
      <p class="check-ok" data-check="ok">
        <Glyph shape="ring" /> {checkOkText($config)}
        <button type="button" class="link" data-action="check" disabled={$config.checking} onclick={() => config.checkAgain()}
          >Check again</button
        >
      </p>
    {/if}

    {#if !$config.checkError && $config.check && $config.check.warnings.length > 0}
      <details class="panel" data-check="warnings">
        <summary>Warnings <span class="count">{$config.check.warnings.length}</span></summary>
        <p class="help">Warnings do not stop the start.</p>
        <ul class="lines">
          {#each $config.check.warnings as warning, i (i)}
            <li>{warning}</li>
          {/each}
        </ul>
      </details>
    {/if}

    <fieldset class="form" disabled={$config.saving}>
      <legend class="visually-hidden">Settings</legend>

      <section class="group" aria-labelledby="g-edcopilot">
        <h2 id="g-edcopilot">EDCoPilot</h2>
        <div class="field">
          <label for="edcopilot.dir">EDCoPilot folder</label>
          <div class="row">
            <input
              id="edcopilot.dir"
              type="text"
              spellcheck="false"
              autocomplete="off"
              value={draft.edcopilot.dir}
              oninput={text('edcopilot.dir')}
              aria-invalid={problemsOf('edcopilot.dir').length > 0}
              aria-describedby={describedBy('edcopilot.dir')}
            />
            <button type="button" class="btn" onclick={() => browse('edcopilot.dir', 'EDCoPilot folder')}
              >Browse</button
            >
          </div>
          <p class="help" id="edcopilot.dir-help">The EDCoPilot folder. Required.</p>
          {@render fieldError('edcopilot.dir')}
          <p class="help" data-profile-file>
            {#if profile.file}
              The program reads the commands from this profile: <span class="path">{profile.file}</span>
            {:else}
              The program reads the commands from the EDCoPilot profile in the EDCoPilot folder.
            {/if}
          </p>
          {#if profile.problems.length > 0}
            <div class="field-error" role="alert" data-profile-problems>
              {#each profile.problems as problem, i (i)}
                <p><Glyph shape="diamond" /> <span class="word">Problem:</span> {problem}</p>
              {/each}
            </div>
          {/if}
        </div>
      </section>

      <section class="group" aria-labelledby="g-engine">
        <h2 id="g-engine">Speech engine</h2>
        <div class="field" role="radiogroup" aria-labelledby="engine-label" aria-describedby="engine-help">
          <span class="label" id="engine-label">Engine</span>
          <div class="choice">
            <label class="option">
              <input
                type="radio"
                name="engine"
                value="parakeet"
                checked={local}
                onchange={() => config.set('stt.engine', 'parakeet')}
              />
              Local (Parakeet)
            </label>
            <label class="option">
              <input
                type="radio"
                name="engine"
                value="openai_compatible"
                checked={cloud}
                onchange={() => config.set('stt.engine', 'openai_compatible')}
              />
              Cloud (OpenAI-compatible)
            </label>
          </div>
          {#if engine === null}
            <p class="field-error" data-unknown-engine>
              <Glyph shape="diamond" /> <span class="word">Problem:</span> Unknown engine "{draft.stt.engine}", choose
              one.
            </p>
          {/if}
          <p class="help" id="engine-help">
            The local model runs on this computer. A cloud service needs an API key and gets your voice
            over the network. The settings of the other engine stay in the file.
          </p>
        </div>

        {#if local}
          <div class="field">
            <label for="stt.parakeet.model_dir">Model folder</label>
            <div class="row">
              <input
                id="stt.parakeet.model_dir"
                type="text"
                spellcheck="false"
                autocomplete="off"
                value={draft.stt.parakeet.model_dir}
                oninput={text('stt.parakeet.model_dir')}
                aria-invalid={problemsOf('stt.parakeet.model_dir').length > 0}
                aria-describedby={describedBy('stt.parakeet.model_dir')}
              />
              <button type="button" class="btn" onclick={() => browse('stt.parakeet.model_dir', 'Model folder')}
                >Browse</button
              >
            </div>
            <div class="download" data-model-download={$model.phase}>
              {#if busy($model)}
                <div class="row progress-row">
                  <progress
                    max="100"
                    value={$model.phase === 'downloading' || $model.phase === 'starting' ? percent($model) : undefined}
                    aria-label="Model download"
                  ></progress>
                  <button
                    type="button"
                    class="btn"
                    data-action="cancel-download"
                    disabled={$model.cancelling}
                    onclick={() => model.cancel()}>{$model.cancelling ? 'Cancelling' : 'Cancel'}</button
                  >
                </div>
                <p class="help" data-download-step>{stepText($model)}</p>
              {:else}
                <div class="buttons">
                  <button
                    type="button"
                    class="btn"
                    data-action="download-model"
                    disabled={downloadBlock($model) !== null}
                    onclick={() => model.start()}>Download the model</button
                  >
                  {#if $model.info}
                    <span class="help">
                      {megabytes($model.info.archive_bytes)} MB download, {megabytes($model.info.unpacked_bytes)} MB
                      on the disk
                    </span>
                  {/if}
                </div>
                {#if $model.info && $model.info.models_dir === null}
                  <p class="field-error" data-download-block>
                    <Glyph shape="diamond" /> <span class="word">Problem:</span> {NO_DATA_DIR}
                  </p>
                {:else if $model.info?.models_dir}
                  <p class="help">
                    The program gets the Parakeet model from the sherpa-onnx release on GitHub, checks it, and
                    puts it in <span class="path">{$model.info.models_dir}</span>.
                  </p>
                {/if}
              {/if}
              <div role="status" aria-live="polite">
                {#if $model.phase === 'done'}
                  <p class="done" data-download-result="done"><Glyph shape="dot" /> The model is in {$model.modelDir}.</p>
                  {#if $model.configError}
                    <p class="field-error">
                      <Glyph shape="diamond" /> <span class="word">Not saved:</span> The settings file did not get the
                      new folder: {$model.configError} Use Save to write it.
                    </p>
                  {/if}
                {:else if $model.phase === 'cancelled'}
                  <p class="help" data-download-result="cancelled">
                    The download stopped. A new download continues from the same point.
                  </p>
                {/if}
              </div>
              {#if $model.message}
                <p class="field-error" role="alert" data-download-result="failed">
                  <Glyph shape="diamond" /> <span class="word">{$model.phase === 'failed' ? 'Download failed:' : 'Problem:'}</span>
                  {$model.message}
                </p>
              {/if}
            </div>
            <p class="help" id="stt.parakeet.model_dir-help">
              The folder of the local model. Required for the local engine.
            </p>
            {@render fieldError('stt.parakeet.model_dir')}
          </div>

          <div class="field narrow">
            <label for="stt.parakeet.threads">Threads</label>
            <input
              id="stt.parakeet.threads"
              type="text"
              inputmode="numeric"
              spellcheck="false"
              autocomplete="off"
              bind:value={threadsText}
              oninput={() => config.set('stt.parakeet.threads', parseWhole(threadsText))}
              aria-invalid={problemsOf('stt.parakeet.threads').length > 0}
              aria-describedby={describedBy('stt.parakeet.threads')}
            />
            <p class="help" id="stt.parakeet.threads-help">
              Processor threads for the local model, from 1 to {cores}. The game needs the other cores.
            </p>
            {@render fieldError('stt.parakeet.threads')}
          </div>

          <div class="field">
            <label class="option check">
              <input
                type="checkbox"
                checked={draft.stt.parakeet.hotwords_file !== ''}
                onchange={(e) => config.setHotwords(e.currentTarget.checked)}
                aria-invalid={problemsOf('stt.parakeet.hotwords_file').length > 0}
                aria-describedby={describedBy('stt.parakeet.hotwords_file', 'hotwords-help')}
              />
              Hotwords
            </label>
            <p class="help" id="hotwords-help">
              A second pass for names that the model often hears wrong ("Sol" as "toll"). Costs: about
              700 MB more memory, about 1.2 s longer start, and a second pass of about 0.1 s for each
              utterance that gives no command.
              {#if draft.stt.parakeet.hotwords_file !== ''}
                List: <span class="path">{draft.stt.parakeet.hotwords_file}</span>
              {/if}
            </p>
            {@render fieldError('stt.parakeet.hotwords_file')}
          </div>
        {:else if cloud}
          <div class="field">
            <label for="stt.openai_compatible.base_url">Server address</label>
            <input
              id="stt.openai_compatible.base_url"
              type="text"
              inputmode="url"
              spellcheck="false"
              autocomplete="off"
              value={draft.stt.openai_compatible.base_url}
              oninput={text('stt.openai_compatible.base_url')}
              aria-invalid={problemsOf('stt.openai_compatible.base_url').length > 0}
              aria-describedby={describedBy('stt.openai_compatible.base_url')}
            />
            <p class="help" id="stt.openai_compatible.base_url-help">The address of the service.</p>
            {#if saved.stt.openai_compatible.base_url_problem !== null}
              <p class="note" data-base-url-problem><Glyph shape="triangle" /> {BASE_URL_NOTE}</p>
            {/if}
            {@render fieldError('stt.openai_compatible.base_url')}
            {#if hostNote}
              <p class="note" data-host-warning><Glyph shape="triangle" /> {hostNote}</p>
            {/if}
          </div>

          <div class="field">
            <label for="stt.openai_compatible.model">Model</label>
            <input
              id="stt.openai_compatible.model"
              type="text"
              spellcheck="false"
              autocomplete="off"
              value={draft.stt.openai_compatible.model}
              oninput={text('stt.openai_compatible.model')}
              aria-invalid={problemsOf('stt.openai_compatible.model').length > 0}
              aria-describedby={describedBy('stt.openai_compatible.model')}
            />
            <p class="help" id="stt.openai_compatible.model-help">The model name.</p>
            {@render fieldError('stt.openai_compatible.model')}
          </div>

          <div class="pair">
            <div class="field">
              <label for="stt.openai_compatible.language">Language</label>
              <input
                id="stt.openai_compatible.language"
                type="text"
                spellcheck="false"
                autocomplete="off"
                value={draft.stt.openai_compatible.language}
                oninput={text('stt.openai_compatible.language')}
                aria-invalid={problemsOf('stt.openai_compatible.language').length > 0}
                aria-describedby={describedBy('stt.openai_compatible.language')}
              />
              <p class="help" id="stt.openai_compatible.language-help">
                Empty means the service detects the language.
              </p>
              {@render fieldError('stt.openai_compatible.language')}
            </div>

            <div class="field">
              <label for="stt.openai_compatible.timeout_seconds">Time limit (seconds)</label>
              <input
                id="stt.openai_compatible.timeout_seconds"
                type="text"
                inputmode="numeric"
                spellcheck="false"
                autocomplete="off"
                bind:value={timeoutText}
                oninput={() => config.set('stt.openai_compatible.timeout_seconds', parseWhole(timeoutText))}
                aria-invalid={problemsOf('stt.openai_compatible.timeout_seconds').length > 0}
                aria-describedby={describedBy('stt.openai_compatible.timeout_seconds')}
              />
              <p class="help" id="stt.openai_compatible.timeout_seconds-help">
                The time limit for one request, from 1 to 600.
              </p>
              {@render fieldError('stt.openai_compatible.timeout_seconds')}
            </div>
          </div>

          <section class="key" aria-labelledby="key-title">
            <h3 id="key-title">API key</h3>
            <p class="key-state" data-key-state>{keyStatus($config)}</p>
            <label for="api-key">{saved.api_key_host !== null ? 'New key' : 'Key'}</label>
            <div class="row">
              <input
                id="api-key"
                type="password"
                autocomplete="off"
                spellcheck="false"
                autocapitalize="off"
                bind:this={keyInput}
                disabled={keyBlock !== null || $config.keyBusy}
                onkeydown={keyDown}
                aria-invalid={$config.keyError !== null}
                aria-describedby={$config.keyError !== null ? 'api-key-help api-key-error' : 'api-key-help'}
              />
              <button
                type="button"
                class="btn"
                data-action="save-key"
                disabled={keyBlock !== null || $config.keyBusy}
                onclick={sendKey}>{saved.api_key_host !== null ? 'Replace key' : 'Save key'}</button
              >
            </div>
            <p class="help" id="api-key-help">
              {#if keyBlock !== null}
                {keyBlock}. The key belongs to the saved server.
              {:else}
                The key goes to the Windows Credential Manager, for {saved.stt.openai_compatible.base_url}. It
                never goes into the file, and this window never shows it.
              {/if}
            </p>
            {#if saved.api_key_host !== null}
              {#if confirmRemove}
                <div class="confirm" role="group" aria-label="Remove the key">
                  <p>Remove the saved key from this computer?</p>
                  <div class="buttons">
                    <button type="button" class="btn" data-action="remove-key-yes" onclick={removeKey}
                      >Remove</button
                    >
                    <button type="button" class="btn" onclick={() => (confirmRemove = false)}>Keep</button>
                  </div>
                </div>
              {:else}
                <button
                  type="button"
                  class="btn"
                  data-action="remove-key"
                  disabled={$config.keyBusy}
                  onclick={() => (confirmRemove = true)}>Remove key</button
                >
              {/if}
            {/if}
            <div role="status" aria-live="polite">
              {#if $config.keyMessage}
                <p class="done"><Glyph shape="dot" /> {$config.keyMessage}</p>
              {/if}
            </div>
            {#if $config.keyError}
              <p class="field-error" id="api-key-error" role="alert"><Glyph shape="diamond" /> <span class="word">Problem:</span> {$config.keyError}</p>
            {/if}
          </section>

          <details class="advanced">
            <summary>Advanced</summary>
            <div class="field">
              <label for="stt.openai_compatible.api_key_env">Name of the environment variable, not the key</label>
              <input
                id="stt.openai_compatible.api_key_env"
                type="text"
                spellcheck="false"
                autocomplete="off"
                value={draft.stt.openai_compatible.api_key_env}
                oninput={text('stt.openai_compatible.api_key_env')}
                onblur={leaveName}
                aria-invalid={problemsOf('stt.openai_compatible.api_key_env').length > 0}
                aria-describedby={describedBy('stt.openai_compatible.api_key_env')}
              />
              <p class="help" id="stt.openai_compatible.api_key_env-help">
                Letters, digits and _ only, for example OPENAI_API_KEY. The program also reads the key from
                this variable. Enter the key itself in the key field above.
              </p>
              {@render fieldError('stt.openai_compatible.api_key_env')}
              {#if $config.nameDropped}
                <p class="field-error" data-name-dropped>
                  <Glyph shape="diamond" /> <span class="word">Not kept:</span> {NAME_PROBLEM} The saved name is back.
                </p>
              {/if}
            </div>
          </details>
        {/if}
      </section>

      <section class="group" aria-labelledby="g-matching">
        <h2 id="g-matching">Matching</h2>
        <div class="field narrow">
          <label for="matching.threshold">Threshold</label>
          <input
            id="matching.threshold"
            type="text"
            inputmode="decimal"
            spellcheck="false"
            autocomplete="off"
            bind:value={thresholdText}
            oninput={() => config.set('matching.threshold', parseDecimal(thresholdText))}
            aria-invalid={problemsOf('matching.threshold').length > 0}
            aria-describedby={describedBy('matching.threshold')}
          />
          <p class="help" id="matching.threshold-help">
            Minimum confidence, from 0.0 to 1.0. A higher value gives fewer wrong actions.
          </p>
          {@render fieldError('matching.threshold')}
        </div>
        <div class="field">
          <label class="option check">
            <input
              type="checkbox"
              checked={draft.matching.exact_single_word}
              onchange={(e) => config.set('matching.exact_single_word', e.currentTarget.checked)}
              aria-invalid={problemsOf('matching.exact_single_word').length > 0}
              aria-describedby={describedBy('matching.exact_single_word', 'exact-help')}
            />
            One-word phrases must be exact
          </label>
          <p class="help" id="exact-help">A phrase of one word needs confidence 1.0.</p>
          {@render fieldError('matching.exact_single_word')}
        </div>
      </section>

      <section class="group" aria-labelledby="g-updates">
        <h2 id="g-updates">Updates</h2>
        <div class="field">
          <label class="option check">
            <input
              type="checkbox"
              checked={draft.updates.check}
              onchange={(e) => config.set('updates.check', e.currentTarget.checked)}
              aria-invalid={problemsOf('updates.check').length > 0}
              aria-describedby={describedBy('updates.check', 'updates-help')}
            />
            Check for updates at the start
          </label>
          <p class="help" id="updates-help">{SWITCH_HELP}</p>
          {@render fieldError('updates.check')}
        </div>
      </section>
    </fieldset>

    {#if dialogError}
      <p class="field-error" role="alert">
        <Glyph shape="diamond" /> <span class="word">Problem:</span> The folder dialog did not open: {dialogError}
      </p>
    {/if}

    <section class="actions" aria-label="Save">
      <div class="buttons">
        <button type="button" class="btn primary" data-action="save" disabled={!canSave} onclick={() => config.save()}
          >{$config.saving ? 'Saving' : 'Save'}</button
        >
        <button type="button" class="btn" data-action="discard" disabled={!$config.dirty || $config.saving} onclick={() => config.discard()}
          >Discard changes</button
        >
        {#if $config.dirty}
          <span class="pending" data-pending><Glyph shape="half" /> Unsaved changes</span>
        {/if}
      </div>
      {#if changedScreens($config).includes('microphone')}
        <p class="help" data-other-changes>
          The Microphone screen has unsaved changes too. Save writes them, and Discard removes them.
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
        {#if $config.restartNeeded && ($session.running || $session.starting)}
          <p class="note" data-restart><Glyph shape="triangle" /> The running session keeps its old settings until you stop and start it.</p>
        {/if}
      </div>
    </section>
  {/if}
</div>

<style>
  .setup {
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

  .check-ok {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 6px 10px;
    font-size: var(--step-0);
    color: var(--tone-sent);
  }

  details summary {
    font-size: var(--step-0);
    font-weight: 600;
    color: var(--muted);
    cursor: pointer;
  }

  details[open] summary {
    margin-bottom: 6px;
  }

  .count {
    font-weight: 400;
    color: var(--faint);
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
      grid-template-columns: minmax(0, 1fr) 12rem;
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

  .row input {
    flex: 1 1 14rem;
  }

  input[type='text'],
  input[type='password'] {
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
  input[type='password']:hover:not(:disabled) {
    border-color: var(--muted);
  }

  input[aria-invalid='true'] {
    border-color: var(--tone-error);
    border-width: 2px;
  }

  input:disabled {
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

  .option.check {
    align-self: flex-start;
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

  .key {
    display: flex;
    flex-direction: column;
    align-items: stretch;
    gap: 6px;
    padding: 10px 12px;
    border: 1px solid var(--line);
    border-radius: var(--radius-s);
  }

  .key h3 {
    font-size: var(--step-0);
    font-weight: 600;
  }

  .key > .btn {
    align-self: flex-start;
  }

  .key-state {
    font-weight: 600;
    overflow-wrap: anywhere;
  }

  .confirm {
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 8px 10px;
    border-left: 3px solid var(--tone-dry);
    background: var(--raised);
  }

  .buttons {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 8px;
  }

  .advanced {
    padding-top: 2px;
  }

  .advanced .field {
    margin-top: 6px;
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

  .download {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }

  .progress-row {
    align-items: center;
  }

  progress {
    flex: 1 1 14rem;
    height: 10px;
    accent-color: var(--signal);
  }
</style>
