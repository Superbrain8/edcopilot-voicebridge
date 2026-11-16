// The session store: the state of the voice session for all screens.
//
// The store reads `session_status` when the window opens, because a session
// can already run or start when the page loads again. Then it follows the
// events of `session://event`. After each stop and each failed start it
// reads the status again, so that a missed event does not leave the store
// wrong.

import { writable, type Readable } from 'svelte/store'
import * as api from './api'
import type { SessionEvent, SessionStatus, StartInfo, StopReason, Utterance } from './api'

/** The number of rows that the result list keeps. */
export const MAX_RESULTS = 200
/** The number of warnings that the store keeps. */
export const MAX_WARNINGS = 50
/** The error text of a start that a stop cancelled. */
export const CANCELLED = 'the start was cancelled'

/** The functions of `api.ts` that the store uses. Tests give a fake. */
export interface SessionApi {
  sessionStatus(): Promise<SessionStatus>
  startSession(dryRun: boolean): Promise<void>
  stopSession(): Promise<void>
  onSessionEvent(handler: (event: SessionEvent) => void): Promise<() => void>
}

/** One row of the result list. The newest row is the first. */
export type ResultRow =
  | { id: number; at: number; kind: 'utterance'; utterance: Utterance }
  | { id: number; at: number; kind: 'dropped'; reason: string }

export interface Warning {
  id: number
  at: number
  text: string
}

export interface SessionState {
  /** True after the first read of the status. */
  loaded: boolean
  running: boolean
  dryRun: boolean
  talking: boolean
  /** A start is in work: the model loads. */
  starting: boolean
  /**
   * The mode of the start in work, when this window started it: true for
   * the dry run. Null when no start is in work, or when the mode is not
   * known (a start of an earlier page load).
   */
  startingDryRun: boolean | null
  /** A stop is in work: the utterance in work completes. */
  stopping: boolean
  /** The start lines of the running session, or of the last session. */
  startInfo: StartInfo | null
  /** Newest first, at most `MAX_RESULTS`. */
  results: ResultRow[]
  /** The problems of the last failed start, one for each line. */
  problems: string[]
  /** Newest first, at most `MAX_WARNINGS`. */
  warnings: Warning[]
  /** Why the last session stopped. Null while a session runs. */
  stopReason: StopReason | null
  /** The audio glitches of the last session that stopped. */
  glitches: number
  /** The counts of the last reload of the vocabulary. */
  reload: { files: number; names: number } | null
  /** The text of a command that failed, other than a start. */
  error: string | null
}

export interface SessionStore extends Readable<SessionState> {
  /** Listens to the events, then reads the status. A second call does nothing. */
  init(): Promise<void>
  /** Starts a session. Does nothing while a start or a session is in work. */
  start(dryRun: boolean): Promise<void>
  /** Stops the session or the start in work. */
  stop(): Promise<void>
  /** Applies one event. The listener of `init` calls it. */
  handle(event: SessionEvent): void
  /** Removes the listener. A later `init` listens again. */
  destroy(): void
}

export function initialState(): SessionState {
  return {
    loaded: false,
    running: false,
    dryRun: false,
    talking: false,
    starting: false,
    startingDryRun: null,
    stopping: false,
    startInfo: null,
    results: [],
    problems: [],
    warnings: [],
    stopReason: null,
    glitches: 0,
    reload: null,
    error: null,
  }
}

/** Splits the error text of a failed start into one problem for each line. */
export function problemLines(error: unknown): string[] {
  const text = api.toError(error).message
  return text
    .split(/\r?\n/)
    .map((line) => line.trim())
    .filter((line) => line.length > 0)
}

export function createSessionStore(source: SessionApi): SessionStore {
  const { subscribe, update } = writable<SessionState>(initialState())
  let nextId = 1
  let unlisten: (() => void) | null = null
  // The number of the newest `init` or `destroy`. A `listen` result of an
  // older number is removed at once.
  let epoch = 0
  // True from the call of `init` until its listener is in place.
  let listening = false
  // Counts the `started` and `stopped` events. A status read that began
  // before such an event is older than the event, and is not applied.
  let changes = 0
  // A stop was asked for during the start in work.
  let stopAsked = false
  // Plain flags beside the store, so that two quick clicks in the same
  // tick cannot both pass the check.
  let busyStarting = false
  let busyRunning = false
  let busyStartingFromStatus = false
  subscribe((s) => {
    busyRunning = s.running
    busyStartingFromStatus = s.starting
  })

  function applyStatus(status: SessionStatus, s: SessionState): SessionState {
    // While a session runs, start lines of another mode are of the last
    // session. They are not shown as the lines of this session.
    const info =
      status.start_info && status.running && status.start_info.dry_run !== status.dry_run
        ? null
        : status.start_info
    // The own start in work keeps its flag until its command returns.
    const starting = busyStarting || (status.starting && !status.running)
    return {
      ...s,
      running: status.running,
      starting,
      startingDryRun: starting ? s.startingDryRun : null,
      dryRun: status.running ? status.dry_run : s.dryRun,
      startInfo: info ?? s.startInfo,
      stopReason: status.running || status.starting ? null : status.stop_reason,
      talking: status.running ? s.talking : false,
    }
  }

  async function readStatus(): Promise<void> {
    const before = changes
    try {
      const status = await source.sessionStatus()
      update((s) => {
        const next = before === changes ? applyStatus(status, s) : s
        return { ...next, loaded: true }
      })
    } catch (error) {
      update((s) => ({ ...s, loaded: true, error: api.toError(error).message }))
    }
  }

  function handle(event: SessionEvent): void {
    const at = Date.now()
    switch (event.type) {
      case 'started':
        changes += 1
        update((s) => ({
          ...s,
          running: true,
          // The own start in work keeps its flag until its command returns.
          starting: busyStarting,
          startingDryRun: busyStarting ? s.startingDryRun : null,
          dryRun: event.info.dry_run,
          talking: false,
          startInfo: event.info,
          stopReason: null,
          problems: [],
          error: null,
        }))
        break
      case 'stopped':
        changes += 1
        update((s) => ({
          ...s,
          running: false,
          starting: busyStarting,
          startingDryRun: busyStarting ? s.startingDryRun : null,
          talking: false,
          stopReason: event.reason,
          glitches: event.glitches,
        }))
        break
      case 'talk':
        update((s) => ({ ...s, talking: event.down }))
        break
      case 'utterance': {
        const { type: _type, ...utterance } = event
        const row: ResultRow = { id: nextId++, at, kind: 'utterance', utterance }
        update((s) => ({ ...s, results: [row, ...s.results].slice(0, MAX_RESULTS) }))
        break
      }
      case 'dropped': {
        const row: ResultRow = { id: nextId++, at, kind: 'dropped', reason: event.reason }
        update((s) => ({ ...s, results: [row, ...s.results].slice(0, MAX_RESULTS) }))
        break
      }
      case 'warning': {
        const warning: Warning = { id: nextId++, at, text: event.text }
        update((s) => ({ ...s, warnings: [warning, ...s.warnings].slice(0, MAX_WARNINGS) }))
        break
      }
      case 'vocabulary_reloaded':
        update((s) => ({ ...s, reload: { files: event.files, names: event.names } }))
        break
    }
  }

  async function start(dryRun: boolean): Promise<void> {
    if (busyStarting || busyRunning || busyStartingFromStatus) return
    busyStarting = true
    stopAsked = false
    update((s) => ({
      ...s,
      starting: true,
      startingDryRun: dryRun,
      problems: [],
      error: null,
      stopReason: null,
    }))
    try {
      await source.startSession(dryRun)
      // The `started` event normally comes first. The status covers a late event.
      await readStatus()
    } catch (error) {
      // A stop cancels the start: that text is no problem. Other texts are.
      const problems = problemLines(error).filter((line) => !(stopAsked && line === CANCELLED))
      // The status covers a missed event, for example a session that runs.
      await readStatus()
      // The reason of an earlier stop does not belong to this start.
      update((s) => ({
        ...s,
        problems: s.running ? [] : problems,
        stopReason: s.running ? s.stopReason : null,
      }))
    } finally {
      busyStarting = false
      update((s) => ({ ...s, starting: false, startingDryRun: null }))
    }
  }

  async function stop(): Promise<void> {
    if (busyStarting) stopAsked = true
    update((s) => ({ ...s, stopping: true, error: null }))
    try {
      await source.stopSession()
    } catch (error) {
      update((s) => ({ ...s, error: api.toError(error).message }))
    } finally {
      // The status covers a missed `stopped` event.
      await readStatus()
      update((s) => ({ ...s, stopping: false }))
    }
  }

  async function init(): Promise<void> {
    if (unlisten || listening) return
    listening = true
    const mine = ++epoch
    try {
      const remove = await source.onSessionEvent(handle)
      if (mine !== epoch) {
        // A `destroy` came during the wait.
        remove()
        return
      }
      unlisten = remove
    } catch (error) {
      if (mine !== epoch) return
      update((s) => ({ ...s, error: api.toError(error).message }))
    } finally {
      if (mine === epoch) listening = false
    }
    await readStatus()
  }

  function destroy(): void {
    epoch += 1
    listening = false
    unlisten?.()
    unlisten = null
  }

  return { subscribe, init, start, stop, handle, destroy }
}

/** The store of the window, with the real commands. */
export const sessionStore: SessionStore = createSessionStore(api)
