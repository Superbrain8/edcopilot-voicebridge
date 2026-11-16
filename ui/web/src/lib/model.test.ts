import { describe, expect, it } from 'vitest'
import { get } from 'svelte/store'
import type { CheckResult, Config, ModelEvent, ModelInfo } from './api'
import { createConfigStore, type ConfigApi } from './config'
import { samples } from './fixtures/dto-samples'
import {
  busy,
  createModelStore,
  downloadBlock,
  NO_DATA_DIR,
  percent,
  progressText,
  type ModelApi,
} from './model'

const INFO: ModelInfo = samples.model_info.idle
const OK_CHECK: CheckResult = { problems: [], warnings: [], report: null, profile: null, edcopilot_problems: [], info: [] }

/** A fake of the model commands. `emit` sends one event to each listener. */
function fakeApi(info: ModelInfo = INFO) {
  const handlers = new Set<(event: ModelEvent) => void>()
  const calls = { download: 0, cancel: 0, listen: 0 }
  let downloadError: Error | null = null
  let cancelError: Error | null = null
  let infoError: Error | null = null
  const api: ModelApi = {
    modelInfo: async () => {
      if (infoError) throw infoError
      return structuredClone(info)
    },
    downloadModel: async () => {
      calls.download += 1
      if (downloadError) throw downloadError
    },
    cancelModelDownload: async () => {
      calls.cancel += 1
      if (cancelError) throw cancelError
    },
    onModelEvent: async (h) => {
      calls.listen += 1
      handlers.add(h)
      return () => handlers.delete(h)
    },
  }
  return {
    api,
    calls,
    emit: (event: ModelEvent) => handlers.forEach((h) => h(event)),
    handlerCount: () => handlers.size,
    setDownloadError: (e: Error | null) => (downloadError = e),
    setCancelError: (e: Error | null) => (cancelError = e),
    setInfoError: (e: Error | null) => (infoError = e),
  }
}

function deferred<T>() {
  let resolve!: (value: T) => void
  const promise = new Promise<T>((r) => (resolve = r))
  return { promise, resolve }
}

async function ready(onDone: (dir: string, restartNeeded: boolean) => Promise<void> = async () => {}) {
  const fake = fakeApi()
  const store = createModelStore(fake.api, onDone)
  await store.init()
  return { fake, store, state: () => get(store) }
}

describe('the model store', () => {
  it('reads the model info and listens before a download', async () => {
    const { fake, state } = await ready()
    expect(state().info).toEqual(INFO)
    expect(fake.calls.listen).toBe(1)
    expect(busy(state())).toBe(false)
    expect(state().phase).toBe('idle')
  })

  it('progress events move the bar', async () => {
    const { fake, store, state } = await ready()
    await store.start()
    expect(fake.calls.download).toBe(1)
    expect(busy(state())).toBe(true)
    fake.emit({ type: 'progress', done: 0, total: 400_000_000 })
    expect(percent(state())).toBe(0)
    fake.emit({ type: 'progress', done: 100_000_000, total: 400_000_000 })
    expect(state().phase).toBe('downloading')
    expect(percent(state())).toBe(25)
    expect(progressText(state())).toBe('100 of 400 MB (25 %)')
    fake.emit({ type: 'progress', done: 400_000_000, total: 400_000_000 })
    expect(percent(state())).toBe(100)
    fake.emit({ type: 'verifying' })
    expect(state().phase).toBe('verifying')
    fake.emit({ type: 'unpacking' })
    expect(state().phase).toBe('unpacking')
    expect(busy(state())).toBe(true)
    // A second click while the download runs sends nothing.
    await store.start()
    expect(fake.calls.download).toBe(1)
  })

  it('done sets the model folder in the draft and saved configuration', async () => {
    let file: Config = structuredClone(samples.config)
    file.stt.parakeet.model_dir = ''
    let checks = 0
    const configApi: ConfigApi = {
      getConfig: async () => structuredClone(file),
      saveConfig: async () => ({ problems: [], written: true, restart_needed: false }),
      check: async () => {
        checks += 1
        return OK_CHECK
      },
      setApiKey: async () => ({ restart_needed: false }),
      clearApiKey: async () => ({ restart_needed: false }),
    }
    const config = createConfigStore(configApi, { cores: 8 })
    await config.init()
    // The user has an unsaved change of another field.
    config.set('matching.threshold', 0.9)
    const dir = 'C:\\Users\\x\\AppData\\Roaming\\VoiceBridge\\models\\m'
    const { fake, store, state } = await ready((d, r) => config.useModelFolder(d, r))
    await store.start()
    // The Rust side wrote the file before the event.
    file = structuredClone(file)
    file.stt.parakeet.model_dir = dir
    const before = checks
    fake.emit({ type: 'done', model_dir: dir, config_error: null, restart_needed: false })
    await store.settled()
    expect(state().phase).toBe('done')
    expect(state().modelDir).toBe(dir)
    expect(busy(state())).toBe(false)
    const c = get(config)
    expect(c.saved?.stt.parakeet.model_dir).toBe(dir)
    expect(c.draft?.stt.parakeet.model_dir).toBe(dir)
    // The other change stays, and the check ran again.
    expect(c.draft?.matching.threshold).toBe(0.9)
    expect(checks).toBe(before + 1)
    expect(c.restartNeeded).toBe(false)
  })

  it('done while a session runs sets the restart note of the configuration store', async () => {
    const file: Config = structuredClone(samples.config)
    const configApi: ConfigApi = {
      getConfig: async () => structuredClone(file),
      saveConfig: async () => ({ problems: [], written: true, restart_needed: false }),
      check: async () => OK_CHECK,
      setApiKey: async () => ({ restart_needed: false }),
      clearApiKey: async () => ({ restart_needed: false }),
    }
    const config = createConfigStore(configApi, { cores: 8 })
    await config.init()
    const { fake, store } = await ready((d, r) => config.useModelFolder(d, r))
    await store.start()
    fake.emit(samples.model_events.done)
    await store.settled()
    expect(get(config).restartNeeded).toBe(true)
  })

  it('done without a save puts the folder in the draft, so that Save writes it', async () => {
    const file: Config = structuredClone(samples.config)
    const configApi: ConfigApi = {
      getConfig: async () => structuredClone(file),
      saveConfig: async () => ({ problems: [], written: true, restart_needed: false }),
      check: async () => OK_CHECK,
      setApiKey: async () => ({ restart_needed: false }),
      clearApiKey: async () => ({ restart_needed: false }),
    }
    const config = createConfigStore(configApi, { cores: 8 })
    await config.init()
    const { fake, store, state } = await ready((d, r) => config.useModelFolder(d, r))
    await store.start()
    fake.emit(samples.model_events.done_not_saved)
    await store.settled()
    expect(state().configError).toBe('the file does not load')
    const c = get(config)
    expect(c.draft?.stt.parakeet.model_dir).toBe('D:\\models\\m')
    expect(c.dirty).toBe(true)
  })

  it('failed shows the message and enables the button again', async () => {
    const { fake, store, state } = await ready()
    await store.start()
    fake.emit({ type: 'progress', done: 5, total: 10 })
    fake.emit({ type: 'failed', message: 'the downloaded file is damaged' })
    expect(state().phase).toBe('failed')
    expect(state().message).toBe('the downloaded file is damaged')
    expect(busy(state())).toBe(false)
    await store.start()
    expect(fake.calls.download).toBe(2)
    expect(state().message).toBeNull()
  })

  it('a refused start shows the message and enables the button again', async () => {
    const { fake, store, state } = await ready()
    fake.setDownloadError(new Error('a model download is already running'))
    await store.start()
    expect(state().phase).toBe('failed')
    expect(state().message).toBe('a model download is already running')
    expect(busy(state())).toBe(false)
  })

  it('cancel asks the program to stop, and cancelled enables the button again', async () => {
    const { fake, store, state } = await ready()
    await store.start()
    fake.emit({ type: 'progress', done: 5, total: 10 })
    await store.cancel()
    expect(fake.calls.cancel).toBe(1)
    expect(state().cancelling).toBe(true)
    expect(busy(state())).toBe(true)
    fake.emit({ type: 'cancelled' })
    expect(state().phase).toBe('cancelled')
    expect(state().cancelling).toBe(false)
    expect(busy(state())).toBe(false)
  })

  it('a download of an earlier page load shows as running', async () => {
    const fake = fakeApi({ ...INFO, running: true })
    const store = createModelStore(fake.api, async () => {})
    await store.init()
    expect(busy(get(store))).toBe(true)
    expect(get(store).phase).toBe('starting')
    fake.emit({ type: 'progress', done: 1, total: 2 })
    expect(percent(get(store))).toBe(50)
  })

  it('a page load during the unpack shows the unpack step, not 0 of 0 MB', async () => {
    const fake = fakeApi(samples.model_info.unpacking)
    const store = createModelStore(fake.api, async () => {})
    await store.init()
    expect(get(store).phase).toBe('unpacking')
    expect(busy(get(store))).toBe(true)
  })

  it('a page load during the download shows the last progress', async () => {
    const fake = fakeApi({
      ...INFO,
      running: true,
      step: { type: 'progress', done: 200_000_000, total: 400_000_000 },
    })
    const store = createModelStore(fake.api, async () => {})
    await store.init()
    expect(get(store).phase).toBe('downloading')
    expect(progressText(get(store))).toBe('200 of 400 MB (50 %)')
  })

  it('an error of the model info is shown', async () => {
    const fake = fakeApi()
    fake.setInfoError(new Error('the command failed'))
    const store = createModelStore(fake.api, async () => {})
    await store.init()
    expect(get(store).message).toBe('the command failed')
    expect(get(store).phase).toBe('idle')
  })

  it('a cancel error is shown and the download stays busy', async () => {
    const { fake, store, state } = await ready()
    await store.start()
    fake.setCancelError(new Error('the command failed'))
    await store.cancel()
    expect(state().message).toBe('the command failed')
    expect(state().cancelling).toBe(false)
    expect(busy(state())).toBe(true)
  })

  it('without a data folder the button is off with its own text', async () => {
    expect(downloadBlock({ info: { ...INFO, models_dir: null } })).toBe(NO_DATA_DIR)
    expect(downloadBlock({ info: INFO })).toBeNull()
    expect(downloadBlock({ info: null })).toBe('Reading the model information.')
  })

  it('destroy removes the listener, and init after it listens one time', async () => {
    const { fake, store, state } = await ready()
    store.destroy()
    expect(fake.handlerCount()).toBe(0)
    await store.init()
    expect(fake.handlerCount()).toBe(1)
    await store.start()
    fake.emit({ type: 'verifying' })
    expect(state().phase).toBe('verifying')
  })

  it('init twice registers one listener', async () => {
    const fake = fakeApi()
    const store = createModelStore(fake.api, async () => {})
    await Promise.all([store.init(), store.init()])
    expect(fake.calls.listen).toBe(1)
    expect(fake.handlerCount()).toBe(1)
  })

  it('a late listen result after destroy is removed', async () => {
    const fake = fakeApi()
    const gate = deferred<void>()
    const realListen = fake.api.onModelEvent
    fake.api.onModelEvent = async (handler) => {
      await gate.promise
      return realListen(handler)
    }
    const store = createModelStore(fake.api, async () => {})
    const loading = store.init()
    store.destroy()
    gate.resolve()
    await loading
    expect(fake.handlerCount()).toBe(0)
    fake.api.onModelEvent = realListen
    await store.init()
    expect(fake.handlerCount()).toBe(1)
  })

  it('each model event sample is accepted', async () => {
    const all: ModelEvent[] = Object.values(samples.model_events)
    expect([...new Set(all.map((e) => e.type))].sort()).toEqual(
      ['cancelled', 'done', 'failed', 'progress', 'unpacking', 'verifying'],
    )
    const { fake, store } = await ready()
    await store.start()
    for (const event of all) expect(() => fake.emit(event)).not.toThrow()
    await store.settled()
  })
})
