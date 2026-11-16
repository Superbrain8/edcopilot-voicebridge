import { describe, expect, it } from 'vitest'
import { get } from 'svelte/store'
import type { CheckResult, DiagnosticsPart, SaveResult } from './api'
import {
  AUDIO_NOTE,
  OFF_NOTE,
  UNINSTALL_NOTE,
  checkOutput,
  createDiagnosticsStore,
  partOn,
  type DiagnosticsApi,
  type DiagnosticsConfig,
} from './diagnostics'
import { listenTrayStart, screenForStart } from './nav'
import { startupLines } from './startup'
import { samples } from './fixtures/dto-samples'

const WRITTEN: SaveResult = { problems: [], written: true, restart_needed: false }

function fake(next: SaveResult | Error = WRITTEN) {
  const calls = { set: [] as [DiagnosticsPart, boolean][], open: [] as DiagnosticsPart[] }
  let openError: Error | null = null
  const api: DiagnosticsApi = {
    setDiagnostics: async (part, on) => {
      calls.set.push([part, on])
      if (next instanceof Error) throw next
      return next
    },
    openFolder: async (part) => {
      calls.open.push(part)
      if (openError) throw openError
    },
  }
  const config = { loads: 0, checks: 0, restarts: 0 }
  const host: DiagnosticsConfig = {
    load: async () => {
      config.loads += 1
    },
    runCheck: async () => {
      config.checks += 1
    },
    markRestartNeeded: () => {
      config.restarts += 1
    },
  }
  const store = createDiagnosticsStore(api, host)
  return {
    store,
    calls,
    config,
    state: () => get(store),
    setOpenError: (e: Error) => (openError = e),
  }
}

describe('check output', () => {
  it('has the lines of voicebridge check in the same order', () => {
    const check: CheckResult = {
      ...samples.check.loaded,
      problems: ['edcopilot.dir is empty, set the EDCoPilot folder'],
    }
    const lines = checkOutput(check)
    expect(lines[0]).toBe(samples.check.loaded.report?.line)
    expect(lines.slice(1, 4)).toEqual(samples.check.loaded.profile?.lines)
    expect(lines.slice(4)).toEqual([
      'warning: w',
      'push-to-talk: key "F13"',
      'problem: edcopilot.dir is empty, set the EDCoPilot folder',
      '1 problem(s) found',
    ])
  })

  it('ends with "configuration is valid" without a problem, and has no report line for a file that does not load', () => {
    const valid: CheckResult = { problems: [], warnings: [], report: null, profile: null, edcopilot_problems: [], info: [] }
    expect(checkOutput(valid)).toEqual(['configuration is valid'])
    expect(checkOutput(samples.check.not_loaded)).toEqual(['problem: bad', '1 problem(s) found'])
  })
})

describe('diagnostics store', () => {
  it('a switch sends only the part name, then reads the configuration and the check again', async () => {
    const { store, calls, config, state } = fake()
    await store.setPart('log', true)
    await store.setPart('audio', false)
    expect(calls.set).toEqual([
      ['log', true],
      ['audio', false],
    ])
    expect(config.loads).toBe(2)
    expect(config.checks).toBe(2)
    expect(state().problems).toEqual([])
    expect(state().busy).toBeNull()
  })

  it('a refused switch shows the problems', async () => {
    const refused: SaveResult = { problems: ['diagnostics.log_file: no'], written: false, restart_needed: false }
    const { store, state } = fake(refused)
    await store.setPart('log', true)
    expect(state().problems).toEqual(['diagnostics.log_file: no'])
  })

  it('a switch while a session runs marks restart needed', async () => {
    const { store, config } = fake({ ...WRITTEN, restart_needed: true })
    await store.setPart('audio', true)
    expect(config.restarts).toBe(1)
  })

  it('a failed call shows its error and never rejects', async () => {
    const { store, state } = fake(new Error('cannot read configuration file'))
    await store.setPart('log', true)
    expect(state().error).toBe('cannot read configuration file')
    expect(state().busy).toBeNull()
  })

  it('open folder sends only the part name and shows an error', async () => {
    const { store, calls, state, setOpenError } = fake()
    await store.open('log')
    expect(calls.open).toEqual(['log'])
    expect(state().error).toBeNull()
    setOpenError(new Error('the audio capture is off'))
    await store.open('audio')
    expect(state().error).toBe('the audio capture is off')
  })

  it('a part is on when the saved file has a path', () => {
    const config = structuredClone(samples.config)
    expect(partOn(config, 'log')).toBe(false)
    config.diagnostics.save_audio_dir = 'D:\\vb\\logs\\audio'
    expect(partOn(config, 'audio')).toBe(true)
    expect(AUDIO_NOTE).toBe('The files hold your voice and stay on this computer.')
  })
})

describe('tray start', () => {
  it('opens Setup when the check has problems, else Status', () => {
    expect(screenForStart({ problems: ['x'], warnings: [], report: null, profile: null, edcopilot_problems: [], info: [] })).toBe('setup')
    expect(screenForStart({ problems: [], warnings: ['w'], report: null, profile: null, edcopilot_problems: [], info: [] })).toBe('status')
    // No check result: Status, where the Start button waits for the check.
    expect(screenForStart(null)).toBe('status')
  })

  it('opens Setup when the check did not run, also after an older clean result', () => {
    const clean = { problems: [], warnings: [], report: null, profile: null, edcopilot_problems: [], info: [] }
    expect(screenForStart(clean, 'the command failed')).toBe('setup')
    expect(screenForStart(null, 'the command failed')).toBe('setup')
    expect(screenForStart(clean, null)).toBe('status')
  })
})

describe('the switches, the data folder, and the problem lines', () => {
  it('each call ends with a new call number, so the screen shows the saved state again', async () => {
    const refused: SaveResult = { problems: ['no'], written: false, restart_needed: false }
    const { store, state } = fake(refused)
    expect(state().calls).toBe(0)
    await store.setPart('log', true)
    expect(state().calls).toBe(1)
    // The saved file did not change: the switch goes back to off.
    expect(partOn(structuredClone(samples.config), 'log')).toBe(false)
    const failed = fake(new Error('x'))
    await failed.store.setPart('audio', true)
    expect(failed.state().calls).toBe(1)
  })

  it('tells the user that off removes the own path, and that an uninstall keeps the data folder', () => {
    expect(OFF_NOTE).toBe(
      'Off removes this path from the configuration. On again uses the default folder below the configuration file.',
    )
    expect(UNINSTALL_NOTE).toBe(
      'An uninstall keeps this folder. Delete it by hand to remove your settings, the model and the recordings.',
    )
  })

  it('open folder of the data folder sends the part name data', async () => {
    const { store, calls } = fake()
    await store.open('data')
    expect(calls.open).toEqual(['data'])
  })
})

describe('startup notes', () => {
  it('gives the problems first and then the notes, each as plain text', () => {
    expect(startupLines(samples.startup.dev_build)).toEqual([
      { kind: 'problem', text: samples.startup.dev_build.problem },
    ])
    expect(startupLines(samples.startup.updated)).toEqual([
      { kind: 'note', text: samples.startup.updated.notes[0] },
    ])
    expect(
      startupLines({
        problem: null,
        first_start_error: 'the program could not make its data folder D:\\x: no space',
        notes: ['a note of the update'],
        update_problems: ['cannot replace configuration file D:\\x\\voicebridge.toml'],
        data_dir: null,
      }),
    ).toEqual([
      { kind: 'problem', text: 'the program could not make its data folder D:\\x: no space' },
      { kind: 'problem', text: 'cannot replace configuration file D:\\x\\voicebridge.toml' },
      { kind: 'note', text: 'a note of the update' },
    ])
  })
})

describe('tray start when the page loads', () => {
  it('a click before the page listened acts one time, and an event acts one time', async () => {
    let pending = true
    let handler: (() => void) | null = null
    let acts = 0
    const source = {
      takeTrayStart: async () => {
        const was = pending
        pending = false
        return was
      },
      onTrayStart: async (h: () => void) => {
        handler = h
        return () => (handler = null)
      },
    }
    const remove = await listenTrayStart(source, () => (acts += 1))
    expect(acts).toBe(1)
    pending = true
    handler!()
    await new Promise((r) => setTimeout(r, 0))
    expect(acts).toBe(2)
    // An event after the page took the mark does nothing more.
    handler!()
    await new Promise((r) => setTimeout(r, 0))
    expect(acts).toBe(2)
    remove()
    expect(handler).toBeNull()
  })
})
