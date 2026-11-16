import { describe, expect, it } from 'vitest'
import { get } from 'svelte/store'
import type { SessionEvent, SessionStatus } from './api'
import { CANCELLED, createSessionStore, MAX_RESULTS, type SessionApi } from './session'
import { samples } from './fixtures/dto-samples'
import json from './fixtures/dto-samples.json'

const events = samples.events
const startedInfo = events.started.info

/** A promise that the test resolves or rejects by hand. */
function deferred<T>() {
  let resolve!: (value: T) => void
  let reject!: (error: unknown) => void
  const promise = new Promise<T>((res, rej) => {
    resolve = res
    reject = rej
  })
  return { promise, resolve, reject }
}

const STOPPED: SessionStatus = samples.status.never_started
const RUNNING_DRY: SessionStatus = {
  running: true,
  starting: false,
  dry_run: true,
  start_info: startedInfo,
  stop_reason: null,
}

/** A fake of the API. The test sends events with `emit`. */
function fakeApi(status: SessionStatus = STOPPED) {
  const handlers: ((event: SessionEvent) => void)[] = []
  const calls = { start: [] as boolean[], stop: 0, status: 0, listen: 0 }
  let nextStart: Promise<void> = Promise.resolve()
  const api: SessionApi = {
    sessionStatus: async () => {
      calls.status += 1
      return status
    },
    startSession: (dryRun) => {
      calls.start.push(dryRun)
      return nextStart
    },
    stopSession: async () => {
      calls.stop += 1
    },
    onSessionEvent: async (handler) => {
      calls.listen += 1
      handlers.push(handler)
      return () => {
        const i = handlers.indexOf(handler)
        if (i >= 0) handlers.splice(i, 1)
      }
    },
  }
  return {
    api,
    calls,
    emit: (event: SessionEvent) => [...handlers].forEach((h) => h(event)),
    setStatus: (s: SessionStatus) => (status = s),
    setNextStart: (p: Promise<void>) => (nextStart = p),
    handlerCount: () => handlers.length,
  }
}

function utterance(text: string): SessionEvent {
  const base = events.utterance_dry_run
  return { ...base, outcome: { ...base.outcome, text } }
}

async function ready(status: SessionStatus = STOPPED) {
  const fake = fakeApi(status)
  const store = createSessionStore(fake.api)
  await store.init()
  return { fake, store, state: () => get(store) }
}

describe('session store', () => {
  it('started event sets running and startInfo', async () => {
    const { fake, state } = await ready()
    fake.emit(events.started)
    expect(state().running).toBe(true)
    expect(state().dryRun).toBe(true)
    expect(state().startInfo?.lines).toEqual(startedInfo.lines)
    expect(state().stopReason).toBeNull()
  })

  it('talk events set and clear talking', async () => {
    const { fake, state } = await ready()
    fake.emit(events.started)
    fake.emit(events.talk_down)
    expect(state().talking).toBe(true)
    fake.emit(events.talk_up)
    expect(state().talking).toBe(false)
  })

  it('utterance events go to the front of results, and row 201 is dropped', async () => {
    const { fake, state } = await ready()
    fake.emit(events.started)
    for (let i = 1; i <= MAX_RESULTS + 1; i++) fake.emit(utterance(`phrase ${i}`))
    const rows = state().results
    expect(MAX_RESULTS).toBe(200)
    expect(rows).toHaveLength(200)
    const first = rows[0]
    const last = rows[rows.length - 1]
    expect(first.kind === 'utterance' && first.utterance.outcome.text).toBe('phrase 201')
    expect(last.kind === 'utterance' && last.utterance.outcome.text).toBe('phrase 2')
    // Each row has its own key for the list.
    expect(new Set(rows.map((r) => r.id)).size).toBe(200)
  })

  it('stopped event with a device error sets running to false and shows the reason', async () => {
    const { fake, state } = await ready()
    fake.emit(events.started)
    fake.emit(events.talk_down)
    fake.emit(events.stopped_device_error)
    expect(state().running).toBe(false)
    expect(state().talking).toBe(false)
    expect(state().stopReason).toEqual({ kind: 'device_error', message: 'device removed' })
    expect(state().glitches).toBe(2)
  })

  it('status on load restores a running session', async () => {
    const { fake, state } = await ready(RUNNING_DRY)
    expect(fake.calls.status).toBe(1)
    expect(state().loaded).toBe(true)
    expect(state().running).toBe(true)
    expect(state().dryRun).toBe(true)
    expect(state().startInfo?.engine).toBe('parakeet')
    // The store follows the events after the load.
    expect(fake.handlerCount()).toBe(1)
    fake.emit(events.stopped_requested)
    expect(state().running).toBe(false)
  })

  it('status on load restores the reason of an error stop', async () => {
    const { state } = await ready(samples.status.stopped_after_device_error)
    expect(state().running).toBe(false)
    expect(state().stopReason).toEqual({ kind: 'device_error', message: 'gone' })
  })

  it('status on load restores a start in work', async () => {
    const { fake, store, state } = await ready(samples.status.starting)
    expect(state().starting).toBe(true)
    expect(state().running).toBe(false)
    // The mode of a start that this window did not make is not known.
    expect(state().startingDryRun).toBeNull()
    // A second start is not sent while the first one loads.
    await store.start(true)
    expect(fake.calls.start).toEqual([])
    fake.emit(events.started)
    expect(state().starting).toBe(false)
    expect(state().running).toBe(true)
  })

  it('a start in work from the status ends with a stopped event', async () => {
    const { fake, state } = await ready(samples.status.starting)
    fake.emit(events.stopped_requested)
    expect(state().starting).toBe(false)
  })

  it('a start in work from the status ends with a status that says neither', async () => {
    const { fake, store, state } = await ready(samples.status.starting)
    fake.setStatus(STOPPED)
    await store.stop()
    expect(state().starting).toBe(false)
    expect(state().running).toBe(false)
  })

  it('an event during the load wins over the older status', async () => {
    const fake = fakeApi()
    const gate = deferred<SessionStatus>()
    let asked = false
    fake.api.sessionStatus = () => {
      asked = true
      return gate.promise
    }
    const store = createSessionStore(fake.api)
    const loading = store.init()
    // The listener is in place before the status is read.
    while (!asked) await Promise.resolve()
    expect(fake.handlerCount()).toBe(1)
    fake.emit(events.started)
    gate.resolve(STOPPED)
    await loading
    expect(get(store).running).toBe(true)
  })

  it('a failed start shows each problem on its own line and clears starting', async () => {
    const { fake, store, state } = await ready()
    fake.setNextStart(
      Promise.reject(new Error('matching.threshold must be between 0 and 1\nstt.parakeet.threads must be 1 or more\n')),
    )
    const starting = store.start(false)
    expect(state().starting).toBe(true)
    await starting
    expect(state().starting).toBe(false)
    expect(state().running).toBe(false)
    expect(state().problems).toEqual([
      'matching.threshold must be between 0 and 1',
      'stt.parakeet.threads must be 1 or more',
    ])
    // The next start clears the old problems.
    fake.setNextStart(Promise.resolve())
    const again = store.start(true)
    expect(state().problems).toEqual([])
    await again
  })

  it('start cannot be triggered twice while starting', async () => {
    const { fake, store, state } = await ready()
    const gate = deferred<void>()
    fake.setNextStart(gate.promise)
    const first = store.start(true)
    const second = store.start(false)
    expect(fake.calls.start).toEqual([true])
    expect(state().starting).toBe(true)
    fake.emit(events.started)
    // The own start is still in work: the buttons stay off.
    expect(state().starting).toBe(true)
    fake.setStatus(RUNNING_DRY)
    gate.resolve()
    await Promise.all([first, second])
    expect(state().starting).toBe(false)
    expect(state().running).toBe(true)
    // A start while the session runs is not sent either.
    await store.start(true)
    expect(fake.calls.start).toEqual([true])
  })

  it('the store records which start button was pressed', async () => {
    const { fake, store, state } = await ready()
    const gate = deferred<void>()
    fake.setNextStart(gate.promise)
    const starting = store.start(true)
    expect(state().startingDryRun).toBe(true)
    gate.resolve()
    await starting
    expect(state().startingDryRun).toBeNull()
    fake.setNextStart(new Promise(() => {}))
    void store.start(false)
    expect(state().startingDryRun).toBe(false)
  })

  it('a successful start reads the status when the started event is late', async () => {
    const { fake, store, state } = await ready()
    fake.setStatus({ ...RUNNING_DRY, dry_run: false, start_info: { ...startedInfo, dry_run: false } })
    await store.start(false)
    expect(state().running).toBe(true)
    expect(state().dryRun).toBe(false)
    expect(state().startInfo?.engine).toBe('parakeet')
  })

  it('a status does not take the start lines of another mode while running', async () => {
    // The start lines are of the last session, a dry run. The session that
    // runs now is not a dry run.
    const { state } = await ready({ ...RUNNING_DRY, dry_run: false })
    expect(state().running).toBe(true)
    expect(state().dryRun).toBe(false)
    expect(state().startInfo).toBeNull()
  })

  it('a stop during the start does not show the cancel text as a problem', async () => {
    const { fake, store, state } = await ready()
    const gate = deferred<void>()
    fake.setNextStart(gate.promise)
    const starting = store.start(true)
    const stopping = store.stop()
    expect(fake.calls.stop).toBe(1)
    gate.reject(CANCELLED)
    await Promise.all([starting, stopping])
    expect(state().problems).toEqual([])
    expect(state().starting).toBe(false)
    expect(state().stopping).toBe(false)
  })

  it('a stop during the start still shows a real problem', async () => {
    const { fake, store, state } = await ready()
    const gate = deferred<void>()
    fake.setNextStart(gate.promise)
    const starting = store.start(true)
    const stopping = store.stop()
    gate.reject('a voice session for x is already running')
    await Promise.all([starting, stopping])
    expect(state().problems).toEqual(['a voice session for x is already running'])
  })

  it('a start clears the reason of the last error stop', async () => {
    // The status keeps the reason of the last session after a failed start.
    const { fake, store, state } = await ready(samples.status.stopped_after_device_error)
    fake.setNextStart(Promise.reject('bad'))
    const starting = store.start(false)
    expect(state().stopReason).toBeNull()
    await starting
    expect(state().stopReason).toBeNull()
    expect(state().problems).toEqual(['bad'])
  })

  it('an error stop keeps the result list', async () => {
    const { fake, store, state } = await ready()
    fake.emit(events.started)
    fake.emit(utterance('one'))
    fake.emit(utterance('two'))
    fake.emit({ type: 'stopped', reason: { kind: 'engine_error', message: 'the engine thread failed' }, glitches: 0 })
    expect(state().results).toHaveLength(2)
    expect(state().stopReason?.message).toBe('the engine thread failed')
    // Start is possible again.
    await store.start(true)
    expect(fake.calls.start).toEqual([true])
  })

  it('a missed stopped event is recovered by the status read after stop', async () => {
    const { fake, store, state } = await ready()
    fake.emit(events.started)
    expect(state().running).toBe(true)
    // The stop works, but the window misses the event.
    fake.setStatus({ ...STOPPED, dry_run: true, start_info: startedInfo, stop_reason: { kind: 'requested', message: null } })
    await store.stop()
    expect(state().running).toBe(false)
    expect(state().stopping).toBe(false)
  })

  it('an already-running error leaves the store running', async () => {
    const { fake, store, state } = await ready()
    // The session runs, but the window missed the started event.
    fake.setStatus(RUNNING_DRY)
    fake.setNextStart(Promise.reject('a session is already running'))
    await store.start(false)
    expect(state().running).toBe(true)
    expect(state().dryRun).toBe(true)
    // The problem is not shown while the session runs.
    expect(state().problems).toEqual([])
  })

  it('a started event clears the problems of a failed second start', async () => {
    const { fake, store, state } = await ready()
    fake.setNextStart(Promise.reject('a session is already running'))
    await store.start(false)
    expect(state().problems).toEqual(['a session is already running'])
    fake.emit(events.started)
    expect(state().problems).toEqual([])
  })

  it('a warning event is kept and does not change running', async () => {
    const { fake, state } = await ready()
    fake.emit(events.started)
    fake.emit(events.warning)
    expect(state().running).toBe(true)
    expect(state().warnings.map((w) => w.text)).toEqual(['worker busy'])
    fake.emit(events.stopped_requested)
    fake.emit(events.warning_with_prefix_removed)
    expect(state().running).toBe(false)
    expect(state().warnings.map((w) => w.text)).toEqual([
      'vocabulary directory not found: x',
      'worker busy',
    ])
  })

  it('a dropped event is shown as a row or note with its reason', async () => {
    const { fake, state } = await ready()
    fake.emit(events.started)
    fake.emit(events.dropped)
    const row = state().results[0]
    expect(row.kind).toBe('dropped')
    expect(row.kind === 'dropped' && row.reason).toBe('waited 5000 ms')
  })

  it('a reload of the vocabulary is kept with its counts', async () => {
    const { fake, state } = await ready()
    fake.emit(events.vocabulary_reloaded)
    expect(state().reload).toEqual({ files: 3, names: 12 })
  })

  it('a stop error is shown and clears stopping', async () => {
    const { fake, store, state } = await ready()
    fake.emit(events.started)
    fake.api.stopSession = async () => {
      throw new Error('the command failed')
    }
    await store.stop()
    expect(state().stopping).toBe(false)
    expect(state().error).toBe('the command failed')
  })

  it('a status that does not load is shown as an error', async () => {
    const fake = fakeApi()
    fake.api.sessionStatus = async () => {
      throw new Error('no state')
    }
    const store = createSessionStore(fake.api)
    await store.init()
    expect(get(store).error).toBe('no state')
  })

  it('destroy removes the listener', async () => {
    const { fake, store } = await ready()
    store.destroy()
    expect(fake.handlerCount()).toBe(0)
  })

  it('destroy then init listens again', async () => {
    const { fake, store, state } = await ready()
    store.destroy()
    await store.init()
    expect(fake.handlerCount()).toBe(1)
    fake.emit(events.started)
    expect(state().running).toBe(true)
  })

  it('init twice registers one listener', async () => {
    const fake = fakeApi()
    const store = createSessionStore(fake.api)
    await Promise.all([store.init(), store.init()])
    await store.init()
    expect(fake.calls.listen).toBe(1)
    expect(fake.handlerCount()).toBe(1)
    fake.emit(events.started)
    fake.emit(utterance('once'))
    expect(get(store).results).toHaveLength(1)
  })

  it('a late listen result after destroy is removed', async () => {
    const fake = fakeApi()
    const gate = deferred<void>()
    const realListen = fake.api.onSessionEvent
    fake.api.onSessionEvent = async (handler) => {
      await gate.promise
      return realListen(handler)
    }
    const store = createSessionStore(fake.api)
    const loading = store.init()
    store.destroy()
    gate.resolve()
    await loading
    expect(fake.handlerCount()).toBe(0)
    // A new init after that listens one time.
    fake.api.onSessionEvent = realListen
    await store.init()
    expect(fake.handlerCount()).toBe(1)
  })
})

describe('the JSON samples of ui/src/dto.rs', () => {
  it('the typed samples are equal to the JSON file that the Rust test reads', () => {
    expect(samples).toEqual(json)
  })

  it('each event sample is accepted by the store', async () => {
    const { fake, state } = await ready()
    const all: SessionEvent[] = Object.values(samples.events)
    const types = new Set(all.map((e) => e.type))
    // Each case of EventDto has a sample.
    expect([...types].sort()).toEqual(
      ['dropped', 'started', 'stopped', 'talk', 'utterance', 'vocabulary_reloaded', 'warning'],
    )
    for (const event of all) {
      expect(() => fake.emit(event)).not.toThrow()
    }
    const row = state().results.find((r) => r.kind === 'utterance')
    expect(row?.kind === 'utterance' && row.utterance.outcome.marks).toEqual([
      'sound_alike',
      'filler',
      'hotwords',
      'glitch',
      'clipping',
    ])
    expect(row?.kind === 'utterance' && row.utterance.outcome.confidence).toBeCloseTo(0.95)
    // The keys of the start info are the 10 keys that the Rust test counts.
    expect(Object.keys(startedInfo)).toHaveLength(10)
  })
})
