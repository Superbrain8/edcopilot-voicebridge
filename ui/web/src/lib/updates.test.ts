import { describe, expect, it } from 'vitest'
import { get } from 'svelte/store'
import type { UpdateStatus } from './api'
import { samples } from './fixtures/dto-samples'
import json from './fixtures/dto-samples.json'
import {
  availableText,
  canInstall,
  CHECKING,
  createUpdateStore,
  dateTime,
  DEVELOPER_BUILD,
  diagnosticsLine,
  installBlock,
  NOT_CHECKED,
  noteShown,
  progressText,
  READY_TEXT,
  STOP_FIRST,
  SWITCH_HELP,
  SWITCH_LABEL,
  SWITCH_OFF,
  type UpdateApi,
} from './updates'

const S = samples.update

/** A fake of the update commands. `emit` sends one state to each listener. */
function fakeApi(first: UpdateStatus = S.idle) {
  const handlers = new Set<(status: UpdateStatus) => void>()
  const calls = { check: 0, install: 0, cancel: 0, hide: 0, listen: 0 }
  let checkResult: UpdateStatus | Error = S.up_to_date
  let installError: Error | null = null
  const api: UpdateApi = {
    updateStatus: async () => structuredClone(first),
    checkForUpdates: async () => {
      calls.check += 1
      if (checkResult instanceof Error) throw checkResult
      return structuredClone(checkResult)
    },
    installUpdate: async () => {
      calls.install += 1
      if (installError) throw installError
    },
    cancelUpdate: async () => {
      calls.cancel += 1
    },
    hideUpdate: async () => {
      calls.hide += 1
    },
    onUpdateEvent: async (h) => {
      calls.listen += 1
      handlers.add(h)
      return () => handlers.delete(h)
    },
  }
  return {
    api,
    calls,
    emit: (status: UpdateStatus) => handlers.forEach((h) => h(status)),
    handlerCount: () => handlers.size,
    setCheckResult: (r: UpdateStatus | Error) => (checkResult = r),
    setInstallError: (e: Error | null) => (installError = e),
  }
}

async function ready(first: UpdateStatus = S.idle) {
  const fake = fakeApi(first)
  const store = createUpdateStore(fake.api)
  await store.init()
  return { fake, store, state: () => get(store) }
}

const IDLE_SESSION = { running: false, starting: false, stopping: false }

describe('the update samples', () => {
  it('are the JSON of the Rust DTOs', () => {
    expect(samples.update).toEqual(json.update)
    expect(samples.config.updates).toEqual({ check: true })
  })
})

describe('the update texts', () => {
  it('are exact', () => {
    expect(availableText(S.available)).toBe('Version 0.3.0 is available. You have version 0.2.0.')
    expect(progressText(S.downloading)).toBe('Downloading the update: 3 of 13 MB.')
    expect(progressText({ received: 2_400_000, total: null })).toBe('Downloading the update: 2 MB.')
    expect(READY_TEXT).toBe('The update is ready. The program closes and the installer starts.')
    expect(STOP_FIRST).toBe('Stop the session first.')
    expect(SWITCH_HELP).toBe(
      'The program asks the GitHub page of the project for the newest version. It sends no data about you. GitHub sees your internet address, as each web site does.',
    )
    expect(SWITCH_LABEL).toBe('Check for updates at the start and one time each day')
    expect(S.install_failed.error).toBe(
      'The update did not install: the signature of the installer is not correct. The program is not changed. Do not install this version by hand. Tell the project on its Issues page.',
    )
  })

  it('the Diagnostics line follows the state', () => {
    const at = S.up_to_date.checked_at as number
    expect(diagnosticsLine(S.up_to_date, true)).toBe(
      `You have the latest version. Last check: ${dateTime(at)}.`,
    )
    expect(diagnosticsLine(S.available, true)).toBe('Version 0.3.0 is available.')
    expect(diagnosticsLine(S.downloading, true)).toBe('Version 0.3.0 is available.')
    expect(diagnosticsLine(S.check_failed, true)).toBe('The last check failed: the server did not answer in time.')
    expect(diagnosticsLine(S.idle, false)).toBe(SWITCH_OFF)
    expect(SWITCH_OFF).toBe('The program does not check for updates (updates.check is false).')
    expect(diagnosticsLine(S.idle, true)).toBe(NOT_CHECKED)
    expect(diagnosticsLine(S.idle, null)).toBe(NOT_CHECKED)
    expect(diagnosticsLine(S.checking, true)).toBe(CHECKING)
    expect(diagnosticsLine(S.developer_build, true)).toBe(DEVELOPER_BUILD)
    expect(DEVELOPER_BUILD).toBe('This is a developer build. It has no updates.')
    // A manual check while the switch is off shows its result.
    expect(diagnosticsLine(S.up_to_date, false)).toContain('You have the latest version.')
  })

  it('a date has a fixed form', () => {
    const d = new Date(2026, 11, 1, 9, 5)
    expect(dateTime(d.getTime())).toBe('2026-12-01 09:05')
  })
})

describe('the update note', () => {
  it('shows for an offer, a download, and the install, not after Later', () => {
    expect(noteShown(null)).toBe(false)
    expect(noteShown(S.idle)).toBe(false)
    expect(noteShown(S.up_to_date)).toBe(false)
    expect(noteShown(S.check_failed)).toBe(false)
    expect(noteShown(S.available)).toBe(true)
    expect(noteShown(S.available_hidden)).toBe(false)
    expect(noteShown(S.install_failed)).toBe(true)
    expect(noteShown(S.downloading)).toBe(true)
    expect(noteShown(S.ready)).toBe(true)
  })

  it('Install update is off while a session runs, starts, or stops', () => {
    expect(installBlock(IDLE_SESSION)).toBeNull()
    expect(canInstall(S.available, IDLE_SESSION)).toBe(true)
    for (const busy of [
      { ...IDLE_SESSION, running: true },
      { ...IDLE_SESSION, starting: true },
      { ...IDLE_SESSION, stopping: true },
    ]) {
      expect(installBlock(busy)).toBe(STOP_FIRST)
      expect(canInstall(S.available, busy)).toBe(false)
    }
    // Only an offer can install.
    expect(canInstall(S.downloading, IDLE_SESSION)).toBe(false)
    expect(canInstall(S.ready, IDLE_SESSION)).toBe(false)
    expect(canInstall(S.up_to_date, IDLE_SESSION)).toBe(false)
    expect(canInstall(null, IDLE_SESSION)).toBe(false)
  })
})

describe('the update store', () => {
  it('reads the state and follows the events', async () => {
    const { fake, state } = await ready()
    expect(state().status).toEqual(S.idle)
    expect(fake.calls.listen).toBe(1)
    fake.emit(S.available)
    expect(state().status).toEqual(S.available)
    fake.emit(S.downloading)
    expect(state().status?.phase).toBe('downloading')
  })

  it('a manual check shows its result, or its error text', async () => {
    const { fake, store, state } = await ready()
    fake.setCheckResult(S.available)
    await store.check()
    expect(fake.calls.check).toBe(1)
    expect(state().status).toEqual(S.available)
    expect(state().checking).toBe(false)
    const text = 'The program could not check for updates: the server did not answer in time. Try again later.'
    fake.setCheckResult(new Error(text))
    await store.check()
    expect(state().checkError).toBe(text)
    // The next check clears the text.
    fake.setCheckResult(S.up_to_date)
    await store.check()
    expect(state().checkError).toBeNull()
  })

  it('a refused install shows the text of the program', async () => {
    const { fake, store, state } = await ready(S.available)
    fake.setInstallError(new Error(STOP_FIRST))
    await store.install()
    expect(fake.calls.install).toBe(1)
    expect(state().actionError).toBe(STOP_FIRST)
    fake.setInstallError(null)
    await store.install()
    expect(state().actionError).toBeNull()
  })

  it('Later hides the note at once', async () => {
    const { fake, store, state } = await ready(S.available)
    expect(noteShown(state().status)).toBe(true)
    await store.later()
    expect(fake.calls.hide).toBe(1)
    expect(noteShown(state().status)).toBe(false)
    // The program finds a newer version later: the note shows again.
    fake.emit({ ...S.available, version: '0.4.0', release_page: null, hidden: false })
    expect(noteShown(state().status)).toBe(true)
  })

  it('cancel asks the program', async () => {
    const { fake, store } = await ready(S.downloading)
    await store.cancel()
    expect(fake.calls.cancel).toBe(1)
  })

  it('destroy removes the listener', async () => {
    const { fake, store } = await ready()
    expect(fake.handlerCount()).toBe(1)
    store.destroy()
    expect(fake.handlerCount()).toBe(0)
  })
})
