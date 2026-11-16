// The update store: the state of the update check and install.
//
// The Rust side does all the work: it asks the update server, downloads
// the installer, checks its signature, and starts it. The page only shows
// the state of `update://event` and sends the clicks. The page has no
// updater permission: it cannot download or install by itself.

import { writable, type Readable } from 'svelte/store'
import * as api from './api'
import type { UpdateStatus } from './api'
import type { SessionState } from './session'

/** The functions of `api.ts` that the store uses. Tests give a fake. */
export interface UpdateApi {
  updateStatus(): Promise<UpdateStatus>
  checkForUpdates(): Promise<UpdateStatus>
  installUpdate(): Promise<void>
  cancelUpdate(): Promise<void>
  hideUpdate(): Promise<void>
  onUpdateEvent(handler: (status: UpdateStatus) => void): Promise<() => void>
}

export interface UpdateState {
  /** The state of the Rust side. Null before the first read. */
  status: UpdateStatus | null
  /** True from the click on "Check for updates" until its result. */
  checking: boolean
  /** The text of the last manual check that failed or was refused. */
  checkError: string | null
  /** The text of an install or a cancel that the program refused. */
  actionError: string | null
}

export function initialUpdateState(): UpdateState {
  return { status: null, checking: false, checkError: null, actionError: null }
}

/** The text of an install while a session runs, starts, or stops. */
export const STOP_FIRST = 'Stop the session first.'
/** The text after the download and the signature check. */
export const READY_TEXT = 'The update is ready. The program closes and the installer starts.'
/** The label of the switch on the Setup screen. */
export const SWITCH_LABEL = 'Check for updates at the start and one time each day'
/** The help of the switch on the Setup screen. */
export const SWITCH_HELP =
  'The program asks the GitHub page of the project for the newest version. It sends no data about you. GitHub sees your internet address, as each web site does.'
/** The Diagnostics line when the switch is off and the program did not check. */
export const SWITCH_OFF = 'The program does not check for updates (updates.check is false).'
/** The Diagnostics line of a developer build. */
export const DEVELOPER_BUILD = 'This is a developer build. It has no updates.'
/** The Diagnostics line before the first check. */
export const NOT_CHECKED = 'The program did not check for updates yet.'
/** The Diagnostics line during a check. */
export const CHECKING = 'The program checks for updates.'

/** A size in whole megabytes (1 MB = 1,000,000 bytes). */
function megabytes(bytes: number): number {
  return Math.round(bytes / 1_000_000)
}

/** "Version 0.3.0 is available. You have version 0.2.0." */
export function availableText(s: Pick<UpdateStatus, 'version' | 'current_version'>): string {
  return `Version ${s.version ?? ''} is available. You have version ${s.current_version}.`
}

/** "Downloading the update: 3 of 12 MB." */
export function progressText(s: Pick<UpdateStatus, 'received' | 'total'>): string {
  if (s.total === null || s.total <= 0) return `Downloading the update: ${megabytes(s.received)} MB.`
  return `Downloading the update: ${megabytes(s.received)} of ${megabytes(s.total)} MB.`
}

/** True when the Status screen shows the update note. */
export function noteShown(status: UpdateStatus | null): boolean {
  if (status === null) return false
  if (status.phase === 'downloading' || status.phase === 'ready') return true
  return status.phase === 'available' && !status.hidden
}

type SessionPart = Pick<SessionState, 'running' | 'starting' | 'stopping'>

/** The text under "Install update" while a session runs, starts, or stops, else null. */
export function installBlock(session: SessionPart): string | null {
  return session.running || session.starting || session.stopping ? STOP_FIRST : null
}

/** True when "Install update" is on. */
export function canInstall(status: UpdateStatus | null, session: SessionPart): boolean {
  return status !== null && status.phase === 'available' && installBlock(session) === null
}

/** A date and a time in the form 2026-12-01 10:00, in the local time. */
export function dateTime(ms: number): string {
  const d = new Date(ms)
  const two = (n: number) => String(n).padStart(2, '0')
  return `${d.getFullYear()}-${two(d.getMonth() + 1)}-${two(d.getDate())} ${two(d.getHours())}:${two(d.getMinutes())}`
}

/**
 * The result line of the Diagnostics screen. `checkOn` is `updates.check`
 * of the saved configuration, or null before the read.
 */
export function diagnosticsLine(status: UpdateStatus, checkOn: boolean | null): string {
  if (status.developer_build) return DEVELOPER_BUILD
  switch (status.phase) {
    case 'up_to_date':
      return status.checked_at === null
        ? 'You have the latest version.'
        : `You have the latest version. Last check: ${dateTime(status.checked_at)}.`
    case 'available':
    case 'downloading':
    case 'ready':
      return `Version ${status.version ?? ''} is available.`
    case 'check_failed':
      return `The last check failed: ${status.cause ?? 'an unknown error occurred'}.`
    case 'checking':
      return CHECKING
    case 'idle':
      return checkOn === false ? SWITCH_OFF : NOT_CHECKED
  }
}

export interface UpdateStore extends Readable<UpdateState> {
  /** Listens to the events and reads the state. */
  init(): Promise<void>
  /** "Check for updates". Never rejects. */
  check(): Promise<void>
  /** "Install update". Never rejects. */
  install(): Promise<void>
  /** Stops the download. Never rejects. */
  cancel(): Promise<void>
  /** "Later". Never rejects. */
  later(): Promise<void>
  /** Removes the listener. */
  destroy(): void
}

export function createUpdateStore(source: UpdateApi): UpdateStore {
  const store = writable<UpdateState>(initialUpdateState())
  let unlisten: (() => void) | null = null
  let listening: Promise<void> | null = null
  // The number of the newest `init` or `destroy`. A `listen` result of an
  // older number is removed at once.
  let epoch = 0

  function apply(status: UpdateStatus): void {
    store.update((s) => ({ ...s, status }))
  }

  function listen(): Promise<void> {
    if (unlisten) return Promise.resolve()
    if (listening) return listening
    const mine = ++epoch
    listening = source
      .onUpdateEvent(apply)
      .then((remove) => {
        if (mine !== epoch) {
          remove()
          return
        }
        unlisten = remove
      })
      .finally(() => {
        if (mine === epoch) listening = null
      })
    return listening
  }

  async function init(): Promise<void> {
    try {
      await listen()
      apply(await source.updateStatus())
    } catch {
      // The update is not important for the work of the program: the
      // screens show nothing about it.
    }
  }

  async function check(): Promise<void> {
    store.update((s) => ({ ...s, checking: true, checkError: null, actionError: null }))
    try {
      await listen()
      const status = await source.checkForUpdates()
      store.update((s) => ({ ...s, status, checking: false }))
    } catch (error) {
      store.update((s) => ({ ...s, checking: false, checkError: api.toError(error).message }))
    }
  }

  async function install(): Promise<void> {
    store.update((s) => ({ ...s, actionError: null }))
    try {
      // The listener must be in place before the first event.
      await listen()
      await source.installUpdate()
    } catch (error) {
      store.update((s) => ({ ...s, actionError: api.toError(error).message }))
    }
  }

  async function cancel(): Promise<void> {
    try {
      await source.cancelUpdate()
    } catch (error) {
      store.update((s) => ({ ...s, actionError: api.toError(error).message }))
    }
  }

  async function later(): Promise<void> {
    // The note goes at once. The event of the Rust side follows.
    store.update((s) => (s.status ? { ...s, status: { ...s.status, hidden: true }, actionError: null } : s))
    try {
      await source.hideUpdate()
    } catch {
      // The note stays hidden on this page.
    }
  }

  return {
    subscribe: store.subscribe,
    init,
    check,
    install,
    cancel,
    later,
    destroy: () => {
      epoch += 1
      listening = null
      unlisten?.()
      unlisten = null
    },
  }
}

/** The store of the window, with the real commands. */
export const updateStore: UpdateStore = createUpdateStore(api)
