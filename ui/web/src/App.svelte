<script lang="ts">
  import { onMount } from 'svelte'
  import { get } from 'svelte/store'
  import Glyph, { type Shape } from './lib/components/Glyph.svelte'
  import * as api from './lib/api'
  import { appInfo, cancelCapture, yesNo, type AppInfo } from './lib/api'
  import { changedScreens, configStore as config } from './lib/config'
  import { phaseLabel, phaseOf, stopText, type Phase } from './lib/format'
  import { modelStore as model } from './lib/model'
  import { SCREENS, currentScreen, listenTrayStart, screenForStart } from './lib/nav'
  import { loadStartupNotes } from './lib/startup'
  import { sessionStore as session } from './lib/session'
  import Commands from './routes/Commands.svelte'
  import Diagnostics from './routes/Diagnostics.svelte'
  import Microphone from './routes/Microphone.svelte'
  import Setup from './routes/Setup.svelte'
  import Status from './routes/Status.svelte'
  import Words from './routes/Words.svelte'
  import { isDirty, tablesStore as tables } from './lib/tables'
  import { updateStore as updates } from './lib/updates'

  const PHASE_SHAPE: Record<Phase, Shape> = {
    loading: 'half',
    starting: 'half',
    stopping: 'half',
    running: 'dot',
    stopped: 'square',
    failed: 'diamond',
  }

  let info = $state<AppInfo | null>(null)
  // The window shows at once. When the first check has problems, Setup
  // replaces Status, but only if the user did not select a screen before.
  let userSelected = false

  let phase = $derived(phaseOf($session))
  let reason = $derived(phase === 'failed' ? stopText($session.stopReason) : null)
  let screen = $derived(SCREENS.find((s) => s.id === $currentScreen) ?? SCREENS[0])
  let unsaved = $derived([
    ...(changedScreens($config) as string[]),
    ...(Object.values($tables.tabs).some(isDirty) ? ['words'] : []),
  ])

  onMount(() => {
    // A capture of the talk button from an earlier load of the page ends
    // now. Else it blocks a start until its time limit.
    cancelCapture().catch(() => {})
    void session.init()
    void config.init().finally(() => {
      const problems = get(config).check?.problems.length ?? 0
      if (problems > 0 && !userSelected && get(currentScreen) === 'status') currentScreen.set('setup')
    })
    // The download can run while the user is on another screen.
    void model.init()
    // The update state: the Rust side checks by itself after the start.
    void updates.init()
    appInfo()
      .then((result) => (info = result))
      .catch(() => (info = null))
    // "Start..." in the tray menu: the check runs again, then the window
    // shows Setup for a problem, else Status. The tray starts no session.
    // A click before this page listened is not lost: the program keeps a
    // mark until the page takes it.
    void loadStartupNotes()
    let removeTray: (() => void) | null = null
    let gone = false
    listenTrayStart(api, () => {
      userSelected = true
      void config.runCheck().finally(() => {
        const checked = get(config)
        currentScreen.set(screenForStart(checked.check, checked.checkError))
      })
    })
      .then((remove) => (gone ? remove() : (removeTray = remove)))
      .catch(() => {})
    return () => {
      gone = true
      removeTray?.()
      session.destroy()
      model.destroy()
      updates.destroy()
    }
  })
</script>

<div class="shell">
  <header class="bar">
    <h1 class="name">VoiceBridge</h1>
    <!-- One live region for the state and the stop reason, so that a screen
         reader reads an error stop one time. It has no box of its own. -->
    <div class="live" role="status" aria-live="polite" aria-label="Session" data-live="session">
      <div class="plate phase-{phase}" data-phase={phase}>
        <span class="phase">
          <Glyph shape={PHASE_SHAPE[phase]} size={14} />
          <span class="phase-word">{phaseLabel($session)}</span>
        </span>
        {#if $session.running && $session.dryRun}
          <span class="dry" data-dry-run="true">
            <Glyph shape="ring" />
            Dry run: nothing is sent
          </span>
        {/if}
      </div>
      {#if reason}
        <p class="reason" data-stop-reason>{reason}</p>
      {/if}
    </div>
  </header>

  <nav class="nav" aria-label="Screens">
    <ul>
      {#each SCREENS as item (item.id)}
        <li>
          <button
            type="button"
            class="nav-item"
            data-screen={item.id}
            aria-current={item.id === $currentScreen ? 'page' : undefined}
            onclick={() => {
              userSelected = true
              currentScreen.set(item.id)
            }}
            >{item.label}
            {#if unsaved.includes(item.id)}
              <span class="unsaved" data-unsaved>unsaved</span>
            {/if}</button
          >
        </li>
      {/each}
    </ul>
    {#if info}
      <p class="about">
        Version {info.version}<br />Local engine: {yesNo(info.engine_local)}
      </p>
    {/if}
  </nav>

  <main class="main" data-screen-view={screen.id}>
    {#if screen.id === 'status'}
      <Status />
    {:else if screen.id === 'setup'}
      <Setup />
    {:else if screen.id === 'microphone'}
      <Microphone />
    {:else if screen.id === 'commands'}
      <Commands />
    {:else if screen.id === 'words'}
      <Words />
    {:else if screen.id === 'diagnostics'}
      <Diagnostics />
    {/if}
  </main>
</div>

<style>
  .shell {
    display: grid;
    grid-template-areas:
      'bar'
      'nav'
      'main';
    min-height: 100vh;
  }

  .bar {
    grid-area: bar;
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 6px 16px;
    padding: 10px 16px;
    background: var(--panel);
    border-bottom: 1px solid var(--line);
  }

  .name {
    font-family: var(--font-display);
    font-size: var(--step-1);
    font-weight: 600;
    letter-spacing: 0.01em;
    color: var(--muted);
  }

  .live {
    display: contents;
  }

  .plate {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 6px 14px;
    margin-left: auto;
  }

  .phase {
    display: inline-flex;
    align-items: center;
    gap: 8px;
    font-size: var(--step-2);
    font-weight: 700;
  }

  .phase-running .phase {
    color: var(--tone-sent);
  }

  .phase-failed .phase {
    color: var(--tone-error);
  }

  .phase-stopped .phase,
  .phase-loading .phase {
    color: var(--muted);
  }

  .phase-starting .phase,
  .phase-stopping .phase {
    color: var(--text);
  }

  .dry {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 2px 8px;
    border: 1px solid var(--tone-dry);
    border-radius: var(--radius-s);
    font-size: var(--step-0);
    font-weight: 600;
    color: var(--tone-dry);
  }

  .reason {
    flex-basis: 100%;
    text-align: right;
    font-size: var(--step-0);
    color: var(--tone-error);
    overflow-wrap: anywhere;
  }

  .nav {
    grid-area: nav;
    padding: 8px 16px;
    border-bottom: 1px solid var(--line);
  }

  .nav ul {
    display: grid;
    grid-template-columns: repeat(3, minmax(0, 1fr));
    gap: 4px;
  }

  .nav-item {
    width: 100%;
    min-height: 36px;
    padding: 0 10px;
    border: 1px solid transparent;
    border-radius: var(--radius-s);
    background: transparent;
    text-align: left;
    font-size: var(--step-0);
    color: var(--muted);
    cursor: pointer;
  }

  .nav-item:hover {
    color: var(--text);
    border-color: var(--line);
  }

  .nav-item[aria-current='page'] {
    background: var(--raised);
    color: var(--text);
    font-weight: 600;
    box-shadow: inset 3px 0 0 var(--text);
  }

  /* The mark of a draft that is not saved. A word, not only a color. */
  .unsaved {
    margin-left: 6px;
    padding: 0 5px;
    border: 1px solid var(--tone-dry);
    border-radius: var(--radius-s);
    font-size: var(--step--1);
    font-weight: 600;
    color: var(--tone-dry);
  }

  .about {
    display: none;
  }

  .main {
    grid-area: main;
    min-width: 0;
    padding: 14px 16px 24px;
  }

  /* A wide window: the screens are a rail on the left. */
  @media (min-width: 760px) {
    .shell {
      grid-template-columns: 11rem minmax(0, 1fr);
      grid-template-rows: auto 1fr;
      grid-template-areas:
        'bar bar'
        'nav main';
    }

    .nav {
      display: flex;
      flex-direction: column;
      gap: 20px;
      padding: 14px 10px;
      border-bottom: 0;
      border-right: 1px solid var(--line);
    }

    .nav ul {
      grid-template-columns: 1fr;
    }

    .about {
      display: block;
      padding: 0 10px;
      font-size: var(--step--1);
      color: var(--faint);
    }
  }
</style>
