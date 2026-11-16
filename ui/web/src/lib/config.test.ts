import { describe, expect, it, vi } from 'vitest'
import { get } from 'svelte/store'
import type { Binding, CheckResult, Config, ConfigEdit, KeyResult, SaveResult } from './api'
import { writable } from 'svelte/store'
import {
  absentDeviceNote,
  savedWakeWordWarning,
  NO_WAKE_WORD_WARNING,
  fieldNotes,
  bindingLabel,
  changedScreens,
  deviceLabel,
  deviceStatus,
  wakeWordProblem,
  wakeWordWarning,
  type MicrophoneApi,
  clearWhenBlocked,
  createConfigStore,
  engineOf,
  fieldErrors,
  isVariableName,
  NAME_PROBLEM,
  parseDecimal,
  parseWhole,
  hostWarning,
  BASE_URL_BLOCK,
  keyFieldBlock,
  keyStatus,
  KEY_RESTART,
  startBlock,
  checkOkText,
  FILE_CHANGED_BLOCK,
  CHECK_FAILED_BLOCK,
  CHECKING_BLOCK,
  SAVE_DURING_CAPTURE,
  CONFIG_CHANGED,
  type ConfigApi,
} from './config'
import { samples } from './fixtures/dto-samples'

const CORES = 8
const KEY = 'sk-test-DUMMY-NOT-REAL'

function config(change: (c: Config) => void = () => {}): Config {
  const c = structuredClone(samples.config)
  change(c)
  return c
}

const OK_CHECK: CheckResult = { problems: [], warnings: [], report: null, profile: null, edcopilot_problems: [], info: [] }
const WRITTEN: SaveResult = { problems: [], written: true, restart_needed: false }

/** A fake of the API. Each call is recorded. */
function fakeApi(start: Config = config()) {
  let file = structuredClone(start)
  const calls = {
    get: 0,
    save: [] as ConfigEdit[][],
    saveVersions: [] as string[],
    check: 0,
    setKey: [] as string[],
    keyTarget: [] as [string, string][],
    clearKey: 0,
  }
  let nextSave: SaveResult | Error = WRITTEN
  let nextKey: KeyResult = samples.key_result.no_restart
  let nextCheck: CheckResult = OK_CHECK
  let loadError: Error | null = null
  const api: ConfigApi = {
    getConfig: async () => {
      calls.get += 1
      if (loadError) throw loadError
      return structuredClone(file)
    },
    saveConfig: async (edits, version) => {
      calls.save.push(edits)
      calls.saveVersions.push(version)
      if (nextSave instanceof Error) throw nextSave
      return nextSave
    },
    check: async () => {
      calls.check += 1
      return nextCheck
    },
    setApiKey: async (key, expectedOrigin, expectedName) => {
      calls.setKey.push(key)
      calls.keyTarget.push([expectedOrigin, expectedName])
      file.api_key_set = true
      file.api_key_host = 'https://example.invalid'
      return nextKey
    },
    clearApiKey: async () => {
      calls.clearKey += 1
      file.api_key_set = false
      file.api_key_host = null
      return nextKey
    },
  }
  return {
    api,
    calls,
    setFile: (c: Config) => (file = structuredClone(c)),
    setNextSave: (r: SaveResult | Error) => (nextSave = r),
    setNextKey: (r: KeyResult) => (nextKey = r),
    setNextCheck: (r: CheckResult) => (nextCheck = r),
    setLoadError: (e: Error | null) => (loadError = e),
  }
}

async function ready(start: Config = config()) {
  const fake = fakeApi(start)
  const store = createConfigStore(fake.api, { cores: CORES })
  await store.init()
  return { fake, store, state: () => get(store) }
}

describe('config store', () => {
  it('edits holds only the changed keys', async () => {
    const { store, state } = await ready()
    expect(store.edits()).toEqual([])
    expect(state().dirty).toBe(false)
    store.set('matching.threshold', 0.85)
    expect(store.edits()).toEqual([
      { key: 'matching.threshold', value: { kind: 'number', value: 0.85 } },
    ])
    expect(state().dirty).toBe(true)
    // Back to the saved value: no edit.
    store.set('matching.threshold', 0.8)
    expect(store.edits()).toEqual([])
    expect(state().dirty).toBe(false)
  })

  it('edits holds one edit per changed key and uses the right value kind', async () => {
    const { store } = await ready()
    store.set('edcopilot.dir', 'D:\\Games\\ED CoPilot')
    store.set('matching.threshold', 0.9)
    store.set('stt.parakeet.threads', 2)
    store.set('matching.exact_single_word', false)
    store.set('stt.parakeet.threads', 3)
    const edits = store.edits()
    expect(edits).toHaveLength(4)
    const byKey = Object.fromEntries(edits.map((e) => [e.key, e.value]))
    expect(byKey).toEqual({
      'edcopilot.dir': { kind: 'text', value: 'D:\\Games\\ED CoPilot' },
      'matching.threshold': { kind: 'number', value: 0.9 },
      'stt.parakeet.threads': { kind: 'integer', value: 3 },
      'matching.exact_single_word': { kind: 'flag', value: false },
    })
  })

  it('switching the engine sends only stt.engine', async () => {
    const { store, fake } = await ready()
    store.set('stt.engine', 'openai_compatible')
    await store.save()
    expect(fake.calls.save).toEqual([
      [{ key: 'stt.engine', value: { kind: 'text', value: 'openai_compatible' } }],
    ])
  })

  it('hotwords on writes the list path that the program gives, and off sets it to empty', async () => {
    const copy = 'C:\\Users\\x\\AppData\\Roaming\\VoiceBridge\\hotwords.txt'
    expect(samples.config.default_hotwords_file).toBe(copy)
    const { store, state } = await ready()
    store.setHotwords(true)
    expect(state().draft?.stt.parakeet.hotwords_file).toBe(copy)
    expect(store.edits()).toEqual([
      { key: 'stt.parakeet.hotwords_file', value: { kind: 'text', value: copy } },
    ])
    store.setHotwords(false)
    expect(store.edits()).toEqual([])

    // A file that already names a list: on keeps that list, off empties it.
    const own = await ready(config((c) => (c.stt.parakeet.hotwords_file = 'D:\\My Words\\hot.txt')))
    own.store.setHotwords(false)
    expect(own.store.edits()).toEqual([
      { key: 'stt.parakeet.hotwords_file', value: { kind: 'text', value: '' } },
    ])
    own.store.setHotwords(true)
    expect(own.state().draft?.stt.parakeet.hotwords_file).toBe('D:\\My Words\\hot.txt')
    expect(own.store.edits()).toEqual([])
  })

  it('a save with problems keeps the draft and shows the problems', async () => {
    const { store, state, fake } = await ready()
    fake.setNextSave(samples.save.refused)
    store.set('matching.threshold', 0.95)
    await store.save()
    expect(state().saveOutcome).toBe('refused')
    expect(state().problems).toEqual(['matching.threshold must be between 0.0 and 1.0'])
    expect(state().draft?.matching.threshold).toBe(0.95)
    expect(state().dirty).toBe(true)
    // A refused save does not read the file again.
    expect(fake.calls.get).toBe(1)
  })

  it('a refused save keeps the draft', async () => {
    const { store, state, fake } = await ready()
    fake.setNextSave({
      problems: ['stt.parakeet.model_dir is not a directory: D:\\Nowhere'],
      written: false,
      restart_needed: false,
    })
    store.set('stt.parakeet.model_dir', 'D:\\Nowhere')
    store.set('stt.parakeet.threads', 2)
    await store.save()
    expect(state().draft?.stt.parakeet.model_dir).toBe('D:\\Nowhere')
    expect(state().draft?.stt.parakeet.threads).toBe(2)
    expect(state().saved?.stt.parakeet.model_dir).toBe('')
    expect(store.edits()).toHaveLength(2)
    expect(state().saveOutcome).toBe('refused')
  })

  it('a save with old problems counts as saved', async () => {
    const { store, state, fake } = await ready()
    fake.setNextSave(samples.save.written_with_old_problem)
    store.set('matching.threshold', 0.85)
    fake.setFile(config((c) => (c.matching.threshold = 0.85)))
    await store.save()
    expect(state().saveOutcome).toBe('saved')
    expect(state().problems).toEqual([
      'stt.parakeet.model_dir is empty, the parakeet engine needs it',
    ])
    expect(state().dirty).toBe(false)
    expect(state().saved?.matching.threshold).toBe(0.85)
    // After the save the store reads the file and runs the check again.
    expect(fake.calls.get).toBe(2)
    expect(fake.calls.check).toBe(2)
  })

  it('a save that changes nothing is reported as such', async () => {
    const { store, state, fake } = await ready()
    fake.setNextSave({ problems: [], written: false, restart_needed: false })
    store.set('matching.threshold', 0.85)
    await store.save()
    expect(state().saveOutcome).toBe('unchanged')
  })

  it('a save refused because the file changed offers a read that keeps the draft', async () => {
    const { store, state, fake } = await ready()
    store.set('matching.threshold', 0.9)
    fake.setNextSave({ problems: [CONFIG_CHANGED], written: false, restart_needed: false })
    fake.setFile(
      config((c) => {
        c.matching.threshold = 0.7
        c.stt.parakeet.threads = 2
        c.version = 'v2'
      }),
    )
    await store.save()
    expect(fake.calls.saveVersions).toEqual([samples.config.version])
    expect(state().saveOutcome).toBe('refused')
    expect(state().fileChanged).toBe(true)
    // The refusal does not read the file: the draft and the saved values stay.
    expect(state().saved?.version).toBe(samples.config.version)
    await store.readAgainKeepingChanges()
    expect(state().fileChanged).toBe(false)
    expect(state().saveOutcome).toBeNull()
    expect(state().problems).toEqual([])
    expect(state().saved?.version).toBe('v2')
    expect(state().saved?.matching.threshold).toBe(0.7)
    expect(state().draft?.matching.threshold).toBe(0.9)
    expect(state().draft?.stt.parakeet.threads).toBe(2)
    // The next save sends the new version.
    fake.setNextSave(WRITTEN)
    await store.save()
    expect(fake.calls.saveVersions).toEqual([samples.config.version, 'v2'])
  })

  it('another refusal does not offer the read again', async () => {
    const { store, state, fake } = await ready()
    fake.setNextSave(samples.save.refused)
    store.set('matching.threshold', 0.95)
    await store.save()
    expect(state().fileChanged).toBe(false)
  })

  it('a save while the session runs sets restartNeeded', async () => {
    const { store, state, fake } = await ready()
    fake.setNextSave({ problems: [], written: true, restart_needed: true })
    store.set('matching.threshold', 0.85)
    await store.save()
    expect(state().restartNeeded).toBe(true)
    expect(state().saveOutcome).toBe('saved')
  })

  it('discard restores the saved values', async () => {
    const { store, state } = await ready()
    store.set('edcopilot.dir', 'E:\\Other')
    store.set('stt.engine', 'openai_compatible')
    store.set('stt.parakeet.threads', Number.NaN)
    expect(state().dirty).toBe(true)
    store.discard()
    expect(state().dirty).toBe(false)
    expect(state().draft).toEqual(state().saved)
    expect(store.edits()).toEqual([])
  })

  it('load failure shows the error and no form state', async () => {
    const fake = fakeApi()
    fake.setLoadError(new Error('cannot read voicebridge.toml: the file does not exist'))
    const store = createConfigStore(fake.api, { cores: CORES })
    await store.init()
    const s = get(store)
    expect(s.loadError).toBe('cannot read voicebridge.toml: the file does not exist')
    expect(s.saved).toBeNull()
    expect(s.draft).toBeNull()
    expect(s.dirty).toBe(false)
    expect(store.edits()).toEqual([])
    // A save in this state writes nothing.
    await store.save()
    expect(fake.calls.save).toEqual([])
    // Try again works when the file is back.
    fake.setLoadError(null)
    await store.load()
    expect(get(store).loadError).toBeNull()
    expect(get(store).saved).not.toBeNull()
  })

  it('a page error stops the save and sends nothing', async () => {
    const { store, state, fake } = await ready()
    store.set('stt.parakeet.threads', 0)
    expect(fieldErrors(state(), CORES)['stt.parakeet.threads']).toMatch(/from 1 to 8/)
    store.set('stt.parakeet.threads', 2.5)
    expect(fieldErrors(state(), CORES)['stt.parakeet.threads']).toMatch(/whole number/)
    await store.save()
    expect(fake.calls.save).toEqual([])
    expect(state().saveError).toMatch(/marked fields/)
  })

  it('a variable name that looks like a key is refused in the page', async () => {
    const { store, state, fake } = await ready(config((c) => (c.stt.engine = 'openai_compatible')))
    store.set('stt.openai_compatible.api_key_env', KEY)
    expect(fieldErrors(state(), CORES)['stt.openai_compatible.api_key_env']).toMatch(/not the key/)
    await store.save()
    expect(fake.calls.save).toEqual([])
    store.set('stt.openai_compatible.api_key_env', 'VOICEBRIDGE_UI_TEST_KEY')
    expect(fieldErrors(state(), CORES)['stt.openai_compatible.api_key_env']).toBeUndefined()
    store.set('stt.openai_compatible.api_key_env', '')
    expect(fieldErrors(state(), CORES)['stt.openai_compatible.api_key_env']).toBeUndefined()
  })

  it('the base URL must be an http or https address', async () => {
    const { store, state } = await ready(config((c) => (c.stt.engine = 'openai_compatible')))
    for (const bad of ['', 'not a url', 'ftp://x.example/v1']) {
      store.set('stt.openai_compatible.base_url', bad)
      expect(fieldErrors(state(), CORES)['stt.openai_compatible.base_url'], bad).toBeDefined()
    }
    store.set('stt.openai_compatible.base_url', 'http://localhost:8000/v1')
    expect(fieldErrors(state(), CORES)['stt.openai_compatible.base_url']).toBeUndefined()
  })
})

describe('the API key', () => {
  it('the key field is write-only: the store never holds a key after the save call', async () => {
    const { store, state, fake } = await ready(config((c) => (c.stt.engine = 'openai_compatible')))
    const ok = await store.saveApiKey(KEY)
    expect(ok).toBe(true)
    expect(fake.calls.setKey).toEqual([KEY])
    expect(JSON.stringify(state())).not.toContain(KEY)
    expect(JSON.stringify(store.edits())).not.toContain(KEY)
  })

  it('the key is passed to set_api_key and is nowhere in the store afterwards', async () => {
    const { store, state, fake } = await ready(config((c) => (c.stt.engine = 'openai_compatible')))
    store.set('matching.threshold', 0.9)
    await store.saveApiKey(KEY)
    expect(fake.calls.setKey).toEqual([KEY])
    // The key state comes from the file again, the draft stays.
    expect(state().saved?.api_key_host).toBe('https://example.invalid')
    expect(state().draft?.matching.threshold).toBe(0.9)
    expect(fake.calls.check).toBe(2)
    const text = JSON.stringify(state())
    expect(text).not.toContain(KEY)
    expect(text).not.toContain('DUMMY')
    // A key store error that repeats the key is not shown with the key.
    fake.api.setApiKey = async (key) => {
      throw new Error(`cannot save ${key}`)
    }
    expect(await store.saveApiKey(KEY)).toBe(false)
    expect(JSON.stringify(state())).not.toContain(KEY)
    expect(state().keyError).toBeTruthy()
    // An empty key is not sent.
    const before = fake.calls.setKey.length
    expect(await store.saveApiKey('   ')).toBe(false)
    expect(fake.calls.setKey.length).toBe(before)
  })

  it('a key save or a key removal during a cloud session sets restartNeeded', async () => {
    const { store, state, fake } = await ready(config((c) => (c.stt.engine = 'openai_compatible')))
    expect(await store.saveApiKey(KEY)).toBe(true)
    expect(state().restartNeeded).toBe(false)
    fake.setNextKey(samples.key_result.restart_needed)
    expect(await store.saveApiKey(KEY)).toBe(true)
    expect(state().restartNeeded).toBe(true)
    expect(state().keyMessage).toBe(`The key is saved. ${KEY_RESTART}`)
    expect(JSON.stringify(state())).not.toContain(KEY)

    const other = await ready(config((c) => (c.stt.engine = 'openai_compatible')))
    other.fake.setNextKey(samples.key_result.restart_needed)
    await other.store.removeApiKey()
    expect(other.state().restartNeeded).toBe(true)
    expect(other.state().keyMessage).toBe(`The key is removed. ${KEY_RESTART}`)
  })

  it('remove key calls clear_api_key and reads the key state again', async () => {
    const { store, state, fake } = await ready()
    await store.removeApiKey()
    expect(fake.calls.clearKey).toBe(1)
    expect(state().saved?.api_key_set).toBe(false)
    expect(keyStatus(state())).toBe('No key is saved')
  })

  it('the key field is disabled while base_url is dirty', async () => {
    const { store, state } = await ready(config((c) => (c.stt.engine = 'openai_compatible')))
    expect(keyFieldBlock(state())).toBeNull()
    store.set('stt.openai_compatible.base_url', 'https://example.invalid/v1')
    expect(keyFieldBlock(state())).toBe('Save the server address first')
    store.discard()
    store.set('stt.openai_compatible.api_key_env', 'OTHER_KEY')
    expect(keyFieldBlock(state())).toBe('Save the variable name first')
    // The store refuses the call too.
    store.set('stt.openai_compatible.base_url', 'https://example.invalid/v1')
    expect(await store.saveApiKey(KEY)).toBe(false)
  })

  it('a base URL with a secret part in the file blocks the key field', async () => {
    const { store, state, fake } = await ready(
      config((c) => {
        c.stt.engine = 'openai_compatible'
        c.stt.openai_compatible.base_url = 'https://host.example/v1'
        c.stt.openai_compatible.base_url_problem = 'the server address in the file holds a secret or a query'
      }),
    )
    expect(keyFieldBlock(state())).toBe(BASE_URL_BLOCK)
    expect(await store.saveApiKey(KEY)).toBe(false)
    expect(fake.calls.setKey).toEqual([])
    expect(state().keyError).toBe(BASE_URL_BLOCK)
  })

  it('a key from the environment variable is described as such', async () => {
    const env = await ready(
      config((c) => {
        c.api_key_set = true
        c.api_key_host = null
        c.stt.openai_compatible.api_key_env = 'VOICEBRIDGE_UI_TEST_KEY'
      }),
    )
    expect(keyStatus(env.state())).toBe(
      'The key comes from the environment variable VOICEBRIDGE_UI_TEST_KEY',
    )
    const stored = await ready()
    expect(keyStatus(stored.state())).toBe('A key is saved for https://api.openai.com')
    const none = await ready(
      config((c) => {
        c.api_key_set = false
        c.api_key_host = null
      }),
    )
    expect(keyStatus(none.state())).toBe('No key is saved')
  })

  it('a draft base_url on another host warns that the key must be entered again', async () => {
    const { store, state } = await ready()
    expect(hostWarning(state())).toBeNull()
    // Another path on the same host: no warning.
    store.set('stt.openai_compatible.base_url', 'https://api.openai.com/v2')
    expect(hostWarning(state())).toBeNull()
    store.set('stt.openai_compatible.base_url', 'https://example.invalid/v1')
    expect(hostWarning(state())).toBe(
      'The saved key belongs to https://api.openai.com. After the save, enter the key again for https://example.invalid.',
    )
  })
})

describe('the check', () => {
  it('check problems disable start and check warnings do not', async () => {
    const fake = fakeApi()
    const store = createConfigStore(fake.api, { cores: CORES })
    // Before the first check ends, the start waits.
    expect(startBlock(get(store))).toBe('Checking the setup.')
    fake.setNextCheck({ ...OK_CHECK, warnings: ['the hotword file has no hotword'] })
    await store.init()
    expect(startBlock(get(store))).toBeNull()
    fake.setNextCheck({
      ...OK_CHECK,
      problems: ['stt.parakeet.model_dir is empty, the parakeet engine needs it'],
    })
    await store.runCheck()
    expect(startBlock(get(store))).toBe('Fix the setup first: 1 problem.')
    fake.setNextCheck({ ...OK_CHECK, problems: ['a', 'b'] })
    await store.runCheck()
    expect(startBlock(get(store))).toBe('Fix the setup first: 2 problems.')
  })

  it('a check that fails to run blocks the start, also after an older clean check', async () => {
    const fake = fakeApi()
    const store = createConfigStore(fake.api, { cores: CORES })
    await store.init()
    expect(startBlock(get(store))).toBeNull()
    const working = fake.api.check
    fake.api.check = async () => {
      throw new Error('the command failed')
    }
    await store.runCheck()
    expect(get(store).checkError).toBe('the command failed')
    // The older clean result is still in the state, but it does not count.
    expect(get(store).check).not.toBeNull()
    expect(startBlock(get(store))).toBe(CHECK_FAILED_BLOCK)
    const fresh = createConfigStore(fake.api, { cores: CORES })
    await fresh.init()
    expect(startBlock(get(fresh))).toBe(CHECK_FAILED_BLOCK)
    // A check that runs again frees the start.
    fake.api.check = working
    await store.runCheck()
    expect(startBlock(get(store))).toBeNull()
  })

  it('check again reads the file first, so the shown profile is the checked one', async () => {
    const fake = fakeApi()
    const store = createConfigStore(fake.api, { cores: CORES })
    await store.init()
    const order: string[] = []
    const getConfig = fake.api.getConfig
    const check = fake.api.check
    fake.api.getConfig = async () => {
      order.push('get')
      return getConfig()
    }
    fake.api.check = async () => {
      order.push('check')
      return check()
    }
    const moved = config()
    moved.edcopilot.profile_in_use = 'D:\\Other\\EDCoPilot-Profile.vap'
    fake.setFile(moved)
    await store.checkAgain()
    expect(order).toEqual(['get', 'check'])
    expect(get(store).saved?.edcopilot.profile_in_use).toBe('D:\\Other\\EDCoPilot-Profile.vap')
  })

  it('a check that runs blocks the start, also when the older result is clean', async () => {
    const fake = fakeApi()
    const store = createConfigStore(fake.api, { cores: CORES })
    await store.init()
    expect(startBlock(get(store))).toBeNull()
    const gate = deferred<CheckResult>()
    fake.api.check = () => gate.promise
    const running = store.runCheck()
    expect(get(store).checking).toBe(true)
    expect(startBlock(get(store))).toBe(CHECKING_BLOCK)
    expect(CHECKING_BLOCK).toBe('Checking the setup.')
    gate.resolve(OK_CHECK)
    await running
    expect(startBlock(get(store))).toBeNull()
  })

  it('the Setup screen says "Checking the setup." while a check runs, not the older clean result', async () => {
    const fake = fakeApi()
    const store = createConfigStore(fake.api, { cores: CORES })
    await store.init()
    expect(checkOkText(get(store))).toBe('The check found no problem.')
    const gate = deferred<CheckResult>()
    fake.api.check = () => gate.promise
    const running = store.runCheck()
    expect(checkOkText(get(store))).toBe(CHECKING_BLOCK)
    gate.resolve(OK_CHECK)
    await running
    expect(checkOkText(get(store))).toBe('The check found no problem.')
  })
})

describe('check again keeps the work of the user', () => {
  it('a read that fails keeps the saved settings and the draft, shows the error, and still checks', async () => {
    const { store, state, fake } = await ready()
    store.set('matching.threshold', 0.9)
    const saved = state().saved
    const draft = state().draft
    const checks = fake.calls.check
    fake.setLoadError(new Error('the file is locked'))
    await store.checkAgain()
    expect(state().saved).toEqual(saved)
    expect(state().draft).toEqual(draft)
    expect(state().dirty).toBe(true)
    expect(state().readError).toBe('the file is locked')
    expect(state().loadError).toBeNull()
    expect(fake.calls.check).toBe(checks + 1)
    expect(state().checking).toBe(false)
    // The next read that works clears the error.
    fake.setLoadError(null)
    await store.checkAgain()
    expect(state().readError).toBeNull()
    expect(state().draft?.matching.threshold).toBe(0.9)
  })

  it('a draft with changes and a new file version: no silent merge, the user chooses', async () => {
    const { store, state, fake } = await ready()
    store.set('matching.threshold', 0.9)
    const saved = state().saved
    const draft = state().draft
    const checks = fake.calls.check
    fake.setFile(
      config((c) => {
        c.version = 'v-other'
        c.microphone.vad_threshold_dbfs = -30
      }),
    )
    await store.checkAgain()
    expect(state().fileChanged).toBe(true)
    // The draft and its base stay: a save still sends the old version.
    expect(state().saved).toEqual(saved)
    expect(state().draft).toEqual(draft)
    // No check of a file that the screen does not show: the user chooses
    // first. The start stays off until then.
    expect(fake.calls.check).toBe(checks)
    expect(state().checking).toBe(false)
    expect(startBlock(state())).toBe(FILE_CHANGED_BLOCK)
    // The user chooses to keep the changes on the new file.
    await store.readAgainKeepingChanges()
    expect(fake.calls.check).toBe(checks + 1)
    expect(startBlock(state())).toBeNull()
    expect(state().fileChanged).toBe(false)
    expect(state().saved?.version).toBe('v-other')
    expect(state().draft?.matching.threshold).toBe(0.9)
    expect(state().draft?.microphone.vad_threshold_dbfs).toBe(-30)
  })

  it('a save that a field error refuses keeps the start off after a change of the file', async () => {
    const { store, state, fake } = await ready()
    store.set('matching.threshold', 0.9)
    fake.setFile(config((c) => (c.version = 'v-other')))
    await store.checkAgain()
    expect(state().fileChanged).toBe(true)
    // A local refusal sends nothing: the screen still shows the old file.
    store.set('stt.parakeet.threads', 0)
    await store.save()
    expect(fake.calls.save).toEqual([])
    expect(state().saveError).toMatch(/marked fields/)
    expect(state().fileChanged).toBe(true)
    expect(startBlock(state())).toBe(FILE_CHANGED_BLOCK)
  })

  it('a discard after a change of the file reads the new file and checks it', async () => {
    const { store, state, fake } = await ready()
    store.set('matching.threshold', 0.9)
    const checks = fake.calls.check
    fake.setFile(config((c) => (c.version = 'v-other')))
    await store.checkAgain()
    expect(startBlock(state())).toBe(FILE_CHANGED_BLOCK)
    store.discard()
    await vi.waitFor(() => expect(fake.calls.check).toBe(checks + 1))
    await vi.waitFor(() => expect(state().checking).toBe(false))
    expect(state().saved?.version).toBe('v-other')
    expect(state().dirty).toBe(false)
    expect(startBlock(state())).toBeNull()
  })

  it('a draft without changes takes the new file', async () => {
    const { store, state, fake } = await ready()
    fake.setFile(
      config((c) => {
        c.version = 'v-other'
        c.matching.threshold = 0.7
      }),
    )
    await store.checkAgain()
    expect(state().fileChanged).toBe(false)
    expect(state().saved?.version).toBe('v-other')
    expect(state().draft?.matching.threshold).toBe(0.7)
    expect(state().dirty).toBe(false)
  })

  it('does nothing while a save runs, and the check is on before the read', async () => {
    const { store, state, fake } = await ready()
    store.set('matching.threshold', 0.9)
    const gate = deferred<SaveResult>()
    fake.api.saveConfig = () => gate.promise
    const saving = store.save()
    expect(state().saving).toBe(true)
    const gets = fake.calls.get
    const checks = fake.calls.check
    await store.checkAgain()
    expect(fake.calls.get).toBe(gets)
    expect(fake.calls.check).toBe(checks)
    gate.resolve(WRITTEN)
    await saving
    // The flag is on before the read starts, so the button is off.
    const read = deferred<Config>()
    fake.api.getConfig = () => read.promise
    const again = store.checkAgain()
    expect(state().checking).toBe(true)
    expect(startBlock(state())).toBe(CHECKING_BLOCK)
    read.resolve(config())
    await again
    expect(state().checking).toBe(false)
  })
})

/** A promise that the test resolves by hand. */
function deferred<T>() {
  let resolve!: (value: T) => void
  const promise = new Promise<T>((r) => (resolve = r))
  return { promise, resolve }
}

describe('save rules, field rules, and the talk button capture', () => {
  it('a hidden-engine field out of range is refused and names its engine', async () => {
    const { store, state, fake } = await ready()
    store.set('stt.parakeet.threads', 0)
    store.set('stt.engine', 'openai_compatible')
    await store.save()
    expect(fake.calls.save).toEqual([])
    expect(state().saveError).toContain('Local engine: Threads')
    expect(state().saveError).toContain('from 1 to 8')
    expect(state().saveError).toMatch(/switch the engine/i)
    expect(state().saving).toBe(false)
  })

  it('a hidden-engine field that is not a number is refused, and the save does not reject', async () => {
    const { store, state, fake } = await ready()
    store.set('stt.parakeet.threads', Number.NaN)
    store.set('stt.engine', 'openai_compatible')
    await expect(store.save()).resolves.toBeUndefined()
    expect(fake.calls.save).toEqual([])
    expect(state().saveError).toContain('Local engine: Threads')
    expect(state().saving).toBe(false)
  })

  it('a hidden cloud field is refused with the name of the cloud engine', async () => {
    const { store, state, fake } = await ready()
    store.set('stt.openai_compatible.timeout_seconds', 0)
    await store.save()
    expect(fake.calls.save).toEqual([])
    expect(state().saveError).toContain('Cloud engine: Time limit')
  })

  it('the name rule refuses keys made of name characters and keeps real names', () => {
    const run = (n: number) => 'aB3dE5gH7jK9mN1pQ2sT4vW6xY8z0Ac'.repeat(4).slice(0, n)
    const keys = [
      `gsk_${run(52)}`,
      `sk_${run(48)}`,
      '3f9a0c4e1b7d2a6f8e0c3b5d7a9f1e2c',
      run(40),
      'A'.repeat(65),
      'pk_live_abc',
      'XAI_abc',
    ]
    for (const key of keys) expect(isVariableName(key), key).toBe(false)
    for (const name of ['OPENAI_API_KEY', 'GROQ_API_KEY', 'VOICEBRIDGE_UI_TEST_KEY', 'MY_KEY_2', 'API_KEY_OPENAI', 'A'.repeat(64)]) {
      expect(isVariableName(name), name).toBe(true)
    }
  })

  it('a refused name shows the fixed text and is dropped from the draft on blur', async () => {
    const key = `gsk_${'aB3dE5gH7j'.repeat(5)}12`
    const { store, state, fake } = await ready(config((c) => (c.stt.engine = 'openai_compatible')))
    store.set('stt.openai_compatible.api_key_env', key)
    expect(fieldErrors(state(), CORES)['stt.openai_compatible.api_key_env']).toBe(NAME_PROBLEM)
    expect(NAME_PROBLEM).not.toContain(key)
    store.dropInvalidName()
    expect(state().draft?.stt.openai_compatible.api_key_env).toBe('OPENAI_API_KEY')
    expect(state().nameDropped).toBe(true)
    expect(JSON.stringify(state())).not.toContain(key)
    await store.save()
    expect(fake.calls.save).toEqual([])
    // A valid name is not dropped.
    store.set('stt.openai_compatible.api_key_env', 'GROQ_API_KEY')
    expect(state().nameDropped).toBe(false)
    store.dropInvalidName()
    expect(state().draft?.stt.openai_compatible.api_key_env).toBe('GROQ_API_KEY')
  })

  it('the form stays in the saving state until the reload and the check are done', async () => {
    const fake = fakeApi()
    const store = createConfigStore(fake.api, { cores: CORES })
    await store.init()
    const gate = deferred<void>()
    const getConfig = fake.api.getConfig
    fake.api.getConfig = async () => {
      await gate.promise
      return getConfig()
    }
    store.set('matching.threshold', 0.85)
    fake.setFile(config((c) => (c.matching.threshold = 0.85)))
    const saving = store.save()
    for (let i = 0; i < 10; i++) await Promise.resolve()
    expect(fake.calls.save).toHaveLength(1)
    expect(get(store).saving).toBe(true)
    gate.resolve()
    await saving
    expect(get(store).saving).toBe(false)
    expect(get(store).dirty).toBe(false)
  })

  it('a field typed during the save survives the reload and stays dirty', async () => {
    const fake = fakeApi()
    const store = createConfigStore(fake.api, { cores: CORES })
    await store.init()
    const gate = deferred<void>()
    const getConfig = fake.api.getConfig
    fake.api.getConfig = async () => {
      await gate.promise
      return getConfig()
    }
    store.set('matching.threshold', 0.85)
    fake.setFile(config((c) => (c.matching.threshold = 0.85)))
    const saving = store.save()
    for (let i = 0; i < 10; i++) await Promise.resolve()
    store.set('edcopilot.dir', 'TYPED')
    gate.resolve()
    await saving
    const s = get(store)
    expect(s.draft?.edcopilot.dir).toBe('TYPED')
    expect(s.saved?.matching.threshold).toBe(0.85)
    expect(s.draft?.matching.threshold).toBe(0.85)
    expect(s.dirty).toBe(true)
    expect(store.edits()).toEqual([{ key: 'edcopilot.dir', value: { kind: 'text', value: 'TYPED' } }])
  })

  it('a second save during the first is ignored', async () => {
    const fake = fakeApi()
    const store = createConfigStore(fake.api, { cores: CORES })
    await store.init()
    const gate = deferred<void>()
    const getConfig = fake.api.getConfig
    fake.api.getConfig = async () => {
      await gate.promise
      return getConfig()
    }
    store.set('matching.threshold', 0.85)
    const first = store.save()
    for (let i = 0; i < 10; i++) await Promise.resolve()
    await store.save()
    gate.resolve()
    await first
    expect(fake.calls.save).toHaveLength(1)
  })

  it('the key goes with the server and the name that the page shows', async () => {
    const { store, fake } = await ready(
      config((c) => {
        c.stt.engine = 'openai_compatible'
        c.stt.openai_compatible.api_key_env = 'VOICEBRIDGE_UI_TEST_KEY'
      }),
    )
    await store.saveApiKey(KEY)
    expect(fake.calls.keyTarget).toEqual([['https://api.openai.com', 'VOICEBRIDGE_UI_TEST_KEY']])
  })

  it('a key refused because the file changed reloads the settings', async () => {
    const { store, state, fake } = await ready(config((c) => (c.stt.engine = 'openai_compatible')))
    fake.setFile(
      config((c) => {
        c.stt.engine = 'openai_compatible'
        c.stt.openai_compatible.base_url = 'https://other.example.invalid/v1'
      }),
    )
    fake.api.setApiKey = async () => {
      throw new Error('the server address in the file changed, check it and enter the key again')
    }
    expect(await store.saveApiKey(KEY)).toBe(false)
    expect(state().keyError).toContain('server address in the file changed')
    // The page now shows the server of the file. It is not a draft change.
    expect(state().saved?.stt.openai_compatible.base_url).toBe('https://other.example.invalid/v1')
    expect(state().draft?.stt.openai_compatible.base_url).toBe('https://other.example.invalid/v1')
    expect(state().dirty).toBe(false)
  })

  it('restartNeeded is cleared when the session stops', async () => {
    const fake = fakeApi()
    const session = writable({ running: true, starting: false })
    const store = createConfigStore(fake.api, { cores: CORES, session })
    await store.init()
    fake.setNextSave({ problems: [], written: true, restart_needed: true })
    store.set('matching.threshold', 0.85)
    await store.save()
    expect(get(store).restartNeeded).toBe(true)
    session.set({ running: false, starting: false })
    expect(get(store).restartNeeded).toBe(false)
    // A new session does not bring the note back.
    session.set({ running: true, starting: false })
    expect(get(store).restartNeeded).toBe(false)
  })

  it('number fields accept plain decimal text and a comma as the decimal mark', () => {
    expect(parseDecimal('0,8')).toBe(0.8)
    expect(parseDecimal(' 0.85 ')).toBe(0.85)
    expect(parseDecimal('1')).toBe(1)
    for (const bad of ['0x10', '1e3', 'Infinity', '', ' ', '.5', '0.8.1', 'abc', '1,000.5']) {
      expect(parseDecimal(bad), bad).toBeNaN()
    }
    expect(parseWhole('4')).toBe(4)
    expect(parseWhole(' 12 ')).toBe(12)
    for (const bad of ['4.0', '4,5', '-1', '0x10', '1e3', 'Infinity', '']) {
      expect(parseWhole(bad), bad).toBeNaN()
    }
  })

  it('the messages of a number field show an example', async () => {
    const { store, state } = await ready()
    store.set('matching.threshold', Number.NaN)
    expect(fieldErrors(state(), CORES)['matching.threshold']).toMatch(/0\.8 or 0,8/)
    store.set('stt.parakeet.threads', Number.NaN)
    expect(fieldErrors(state(), CORES)['stt.parakeet.threads']).toMatch(/such as 4/)
  })

  it('text fields are trimmed before they go to the edits, inner spaces stay', async () => {
    const { store, state } = await ready()
    store.set('stt.parakeet.model_dir', '  D:\\My Models\\parakeet v3  ')
    expect(store.edits()).toEqual([
      { key: 'stt.parakeet.model_dir', value: { kind: 'text', value: 'D:\\My Models\\parakeet v3' } },
    ])
    // Spaces around the saved value are no change.
    store.set('stt.parakeet.model_dir', '')
    store.set('edcopilot.dir', ' C:\\EDCoPilot ')
    expect(store.edits()).toEqual([])
    expect(state().dirty).toBe(false)
  })

  it('an unknown engine value selects no engine', async () => {
    const { state } = await ready(config((c) => (c.stt.engine = 'whisper')))
    expect(engineOf(state().draft)).toBeNull()
    const local = await ready()
    expect(engineOf(local.state().draft)).toBe('parakeet')
  })

  it('the key input is cleared when it becomes disabled', () => {
    const input = { value: 'sk-typed' }
    clearWhenBlocked(input, null)
    expect(input.value).toBe('sk-typed')
    clearWhenBlocked(input, 'Save the server address first')
    expect(input.value).toBe('')
    clearWhenBlocked(null, 'x')
  })
})

/** A fake of the microphone commands. The test ends each capture by hand. */
function fakeMicrophone(devices: string[] = ['Microphone (RODE NT-USB)', 'Headset (Arctis 7)']) {
  const calls = { list: 0, capture: 0, cancel: 0 }
  const tokens = { capture: [] as number[], cancel: [] as (number | undefined)[] }
  let pending: { resolve: (b: Binding) => void; reject: (e: Error) => void } | null = null
  let cancelEnds = true
  let listError: Error | null = null
  const api: MicrophoneApi = {
    listDevices: async () => {
      calls.list += 1
      if (listError) throw listError
      return [...devices]
    },
    captureBinding: (token) => {
      calls.capture += 1
      tokens.capture.push(token)
      return new Promise<Binding>((resolve, reject) => (pending = { resolve, reject }))
    },
    cancelCapture: async (token) => {
      calls.cancel += 1
      tokens.cancel.push(token)
      if (cancelEnds) end(new Error('the capture was cancelled'))
    },
  }
  function end(result: Binding | Error) {
    const p = pending
    pending = null
    if (!p) return
    if (result instanceof Error) p.reject(result)
    else p.resolve(result)
  }
  return {
    api,
    calls,
    tokens,
    end,
    /** False: a cancel call does not reach the capture (it comes too early). */
    setCancelEnds: (on: boolean) => (cancelEnds = on),
    setListError: (e: Error | null) => (listError = e),
  }
}

async function readyMic(start: Config = config(), devices?: string[]) {
  const fake = fakeApi(start)
  const mic = fakeMicrophone(devices)
  let clock = 1_000
  const store = createConfigStore(fake.api, {
    cores: CORES,
    microphone: mic.api,
    now: () => clock,
  })
  await store.init()
  return {
    fake,
    mic,
    store,
    state: () => get(store),
    advance: (ms: number) => (clock += ms),
  }
}

/** Lets the pending promise callbacks run. */
async function settle() {
  for (let i = 0; i < 10; i++) await Promise.resolve()
}

const withJoystick = (c: Config) => {
  c.push_to_talk.key = null
  c.push_to_talk.joystick = { device: 'MOZA AB9', button: 3 }
}

describe('the microphone screen', () => {
  it('a captured key sets key and removes joystick', async () => {
    const { store, state, mic } = await readyMic(config(withJoystick))
    expect(bindingLabel(state().draft!.push_to_talk)).toBe('MOZA AB9 button 3')
    const capture = store.captureBinding()
    expect(state().capturing).toBe(true)
    mic.end({ kind: 'key', label: 'key "<"', key: '<', joystick: null })
    await capture
    expect(state().capturing).toBe(false)
    expect(state().draft?.push_to_talk.key).toBe('<')
    expect(state().draft?.push_to_talk.joystick).toBeNull()
    expect(bindingLabel(state().draft!.push_to_talk)).toBe('<')
    expect(state().dirty).toBe(true)
    // The same edits as `voicebridge bind`: remove the joystick, set the key.
    expect(store.edits()).toEqual([
      { key: 'push_to_talk.joystick', value: { kind: 'absent' } },
      { key: 'push_to_talk.key', value: { kind: 'text', value: '<' } },
    ])
  })

  it('a save while a capture runs is refused without a call', async () => {
    const { store, state, mic, fake } = await readyMic()
    store.set('microphone.device', 'Headset (Arctis 7)')
    const capture = store.captureBinding()
    await store.save('microphone')
    expect(fake.calls.save).toHaveLength(0)
    expect(state().saveError).toBe(SAVE_DURING_CAPTURE)
    mic.end(samples.binding.key)
    await capture
    await store.save('microphone')
    expect(fake.calls.save).toHaveLength(1)
  })

  it('a captured joystick button sets joystick and removes key', async () => {
    const { store, state, mic, fake } = await readyMic()
    const capture = store.captureBinding()
    mic.end(samples.binding.joystick)
    await capture
    expect(state().draft?.push_to_talk.key).toBeNull()
    expect(state().draft?.push_to_talk.joystick).toEqual({ device: 'MOZA', button: 5 })
    expect(bindingLabel(state().draft!.push_to_talk)).toBe('MOZA button 5')
    const edits = [
      { key: 'push_to_talk.key', value: { kind: 'absent' } },
      { key: 'push_to_talk.joystick.device', value: { kind: 'text', value: 'MOZA' } },
      { key: 'push_to_talk.joystick.button', value: { kind: 'integer', value: 5 } },
    ]
    expect(store.edits()).toEqual(edits)
    // The save sends them in one call, and the binding survives the reload.
    fake.setFile(config((c) => {
      c.push_to_talk.key = null
      c.push_to_talk.joystick = { device: 'MOZA', button: 5 }
    }))
    await store.save('microphone')
    expect(fake.calls.save).toEqual([edits])
    expect(state().dirty).toBe(false)
    expect(state().saved?.push_to_talk.joystick).toEqual({ device: 'MOZA', button: 5 })
  })

  it('a cancelled capture changes nothing', async () => {
    const { store, state, mic, advance } = await readyMic(config(withJoystick))
    const before = structuredClone(state().draft)
    // Esc: the Rust side ends the capture with an error text.
    let capture = store.captureBinding()
    mic.end(new Error('the capture was cancelled with Esc'))
    await capture
    expect(state().draft).toEqual(before)
    expect(store.edits()).toEqual([])
    expect(state().dirty).toBe(false)
    expect(state().captureError).toBeNull()
    expect(state().captureMessage).toMatch(/did not change/)
    // The page cancels (the user leaves the screen).
    advance(1_000)
    capture = store.captureBinding()
    await store.cancelCapture()
    await capture
    expect(mic.calls.cancel).toBe(1)
    expect(state().capturing).toBe(false)
    expect(state().draft).toEqual(before)
    expect(state().captureError).toBeNull()
    // A binding that comes after the cancel of the page is ignored.
    advance(1_000)
    mic.setCancelEnds(false)
    capture = store.captureBinding()
    const cancelling = store.cancelCapture()
    mic.end(samples.binding.key)
    await Promise.all([capture, cancelling])
    expect(state().draft).toEqual(before)
    expect(store.edits()).toEqual([])
  })

  it('an empty wake word list with always-listening shows the warning', async () => {
    const { store, state } = await readyMic()
    expect(wakeWordWarning(state().draft)).toBeNull()
    store.set('microphone.mode', 'always_listening')
    expect(wakeWordWarning(state().draft)).toBeNull()
    store.set('always_listening.wake_words', [])
    expect(wakeWordWarning(state().draft)).toMatch(/sends each match/)
    expect(wakeWordWarning(state().draft)).toMatch(/A wrong match is worse than a miss/)
    // Push-to-talk does not use the wake words: no warning.
    store.set('microphone.mode', 'push_to_talk')
    expect(wakeWordWarning(state().draft)).toBeNull()
    // The warning comes from the draft, before the save.
    store.set('microphone.mode', 'always_listening')
    expect(state().saved?.always_listening.wake_words).toEqual(['copilot', 'co pilot'])
    expect(wakeWordWarning(state().draft)).not.toBeNull()
    expect(store.edits()).toEqual([
      { key: 'microphone.mode', value: { kind: 'text', value: 'always_listening' } },
      { key: 'always_listening.wake_words', value: { kind: 'text_list', value: [] } },
    ])
  })

  it('a capture while a session runs shows the refusal as text', async () => {
    const { store, state, mic } = await readyMic()
    const capture = store.captureBinding()
    mic.end(
      new Error('stop the session first: the session reads the talk button, and two readers do not work'),
    )
    await capture
    expect(state().captureError).toMatch(/stop the session first/)
    expect(state().dirty).toBe(false)
  })

  it('only one capture runs, and the click that ends a capture does not start the next', async () => {
    const { store, state, mic, advance } = await readyMic()
    const first = store.captureBinding()
    // A second click (or a repeated Enter) during the capture is ignored.
    void store.captureBinding()
    expect(mic.calls.capture).toBe(1)
    mic.end(samples.binding.key)
    await first
    // The release of Space clicks the focused button again: ignored.
    await store.captureBinding()
    expect(mic.calls.capture).toBe(1)
    expect(state().capturing).toBe(false)
    advance(1_000)
    void store.captureBinding()
    expect(mic.calls.capture).toBe(2)
    await store.cancelCapture()
  })

  it('a cancel that comes before the capture starts is sent again', async () => {
    vi.useFakeTimers()
    try {
      const { store, state, mic } = await readyMic()
      mic.setCancelEnds(false)
      const capture = store.captureBinding()
      const cancelling = store.cancelCapture()
      await vi.advanceTimersByTimeAsync(250)
      expect(mic.calls.cancel).toBeGreaterThanOrEqual(2)
      mic.setCancelEnds(true)
      await vi.advanceTimersByTimeAsync(250)
      await Promise.all([capture, cancelling])
      expect(state().capturing).toBe(false)
      const count = mic.calls.cancel
      await vi.advanceTimersByTimeAsync(1_000)
      expect(mic.calls.cancel).toBe(count)
    } finally {
      vi.useRealTimers()
    }
  })

  it('a capture of the saved binding is no change, and a discard restores the binding', async () => {
    const { store, state, mic, advance } = await readyMic()
    let capture = store.captureBinding()
    mic.end(samples.binding.key)
    await capture
    expect(state().dirty).toBe(false)
    expect(store.edits()).toEqual([])
    advance(1_000)
    capture = store.captureBinding()
    mic.end(samples.binding.joystick)
    await capture
    expect(state().dirty).toBe(true)
    store.discard()
    expect(state().draft?.push_to_talk).toEqual(state().saved?.push_to_talk)
    expect(store.edits()).toEqual([])
  })

  it('the key < keeps its text from the file to the label and back', async () => {
    const { store, state } = await readyMic(config((c) => (c.push_to_talk.key = '<')))
    expect(bindingLabel(state().draft!.push_to_talk)).toBe('<')
    expect(store.edits()).toEqual([])
    // No key and no joystick: the program uses F13.
    const none = await readyMic(config((c) => (c.push_to_talk.key = null)))
    expect(bindingLabel(none.state().draft!.push_to_talk)).toBe('F13 (default)')
  })

  it('the device list loads, and a saved device that is absent stays visible', async () => {
    const { store, state, mic } = await readyMic(
      config((c) => (c.microphone.device = 'USB Audio CODEC')),
    )
    await store.loadDevices()
    expect(state().devices).toEqual(['Microphone (RODE NT-USB)', 'Headset (Arctis 7)'])
    const devices = state().devices!
    expect(deviceStatus('', devices)).toEqual({ kind: 'default' })
    expect(deviceStatus('Headset (Arctis 7)', devices)).toEqual({ kind: 'listed' })
    // The rule of the audio part: the same name without regard to case, then a part of a name.
    expect(deviceStatus('headset (arctis 7)', devices)).toEqual({ kind: 'matched', device: 'Headset (Arctis 7)' })
    expect(deviceStatus('rode', devices)).toEqual({ kind: 'matched', device: 'Microphone (RODE NT-USB)' })
    expect(deviceStatus('USB Audio CODEC', devices)).toEqual({ kind: 'absent' })
    expect(deviceLabel('USB Audio CODEC', devices)).toBe('USB Audio CODEC (not connected)')
    // The saved name stays in the draft: no edit.
    expect(store.edits()).toEqual([])
    store.set('microphone.device', '')
    expect(store.edits()).toEqual([{ key: 'microphone.device', value: { kind: 'text', value: '' } }])
    store.discard()
    expect(absentDeviceNote(state())).toMatch(/"USB Audio CODEC" is not connected/)
    // A failed list keeps the form and says why.
    mic.setListError(new Error('no audio host'))
    await store.loadDevices()
    expect(state().devicesError).toBe('no audio host')
  })

  it('a failed device list does not say that the saved device is absent', async () => {
    const { store, state, mic } = await readyMic(config((c) => (c.microphone.device = 'Headset (Arctis 7)')))
    mic.setListError(new Error('no audio host'))
    await store.loadDevices()
    // The state of the devices is not known: no list, and no note about the saved device.
    expect(state().devices).toBeNull()
    expect(state().devicesError).toBe('no audio host')
    expect(absentDeviceNote(state())).toBeNull()
    mic.setListError(null)
    await store.loadDevices()
    expect(state().devicesError).toBeNull()
    expect(absentDeviceNote(state())).toBeNull()
  })

  it('the device that the page names is the device that the runtime selects', () => {
    // The runtime compares the whole list without regard to case first.
    const devices = ['Mic (USB)', 'MIC (USB)', 'Headset Mic (USB) 2']
    expect(deviceStatus('Mic (USB)', devices)).toEqual({ kind: 'listed' })
    expect(deviceStatus('MIC (USB)', devices)).toEqual({ kind: 'matched', device: 'Mic (USB)' })
    expect(deviceStatus('mic (usb)', devices)).toEqual({ kind: 'matched', device: 'Mic (USB)' })
    // A part of a name counts only when no name is the same.
    expect(deviceStatus('usb) 2', devices)).toEqual({ kind: 'matched', device: 'Headset Mic (USB) 2' })
    expect(deviceStatus('  Mic (USB)  ', devices)).toEqual({ kind: 'matched', device: 'Mic (USB)' })
    expect(deviceLabel('MIC (USB)', devices)).toBe('MIC (USB) (uses Mic (USB))')
  })

  it('the microphone fields have the ranges of the program', async () => {
    const { store, state } = await readyMic()
    const errors = () => fieldErrors(state(), CORES)
    for (const key of ['push_to_talk.pre_roll_ms', 'push_to_talk.release_tail_ms'] as const) {
      store.set(key, 0)
      expect(errors()[key], key).toBeUndefined()
      store.set(key, 2000)
      expect(errors()[key], key).toBeUndefined()
      store.set(key, 2001)
      expect(errors()[key], key).toMatch(/from 0 to 2000/)
      store.set(key, Number.NaN)
      expect(errors()[key], key).toMatch(/whole number/)
      store.set(key, 250)
    }
    store.set('microphone.vad_threshold_dbfs', parseDecimal('-40,5'))
    expect(state().draft?.microphone.vad_threshold_dbfs).toBe(-40.5)
    expect(errors()['microphone.vad_threshold_dbfs']).toBeUndefined()
    for (const bad of [Number.NaN, 3, -101]) {
      store.set('microphone.vad_threshold_dbfs', bad)
      expect(errors()['microphone.vad_threshold_dbfs'], String(bad)).toBeDefined()
    }
    store.set('microphone.vad_threshold_dbfs', -40)
    store.set('microphone.mode', 'shout')
    expect(errors()['microphone.mode']).toBeDefined()
    store.set('microphone.mode', 'push_to_talk')
    for (const bad of [['...'], ['x'.repeat(257)], ['co\tpilot'], Array.from({ length: 65 }, (_, i) => `w${i}`)]) {
      store.set('always_listening.wake_words', bad)
      expect(errors()['always_listening.wake_words'], bad[0]).toBeDefined()
    }
    store.set('always_listening.wake_words', ['copilot', 'Hey Jarvis'])
    expect(errors()['always_listening.wake_words']).toBeUndefined()
    store.set('microphone.device', 'Mic\nTwo')
    expect(errors()['microphone.device']).toBeDefined()
  })

  it('a wake word to add is checked first', () => {
    expect(wakeWordProblem('  ', ['copilot'])).toMatch(/Enter/)
    expect(wakeWordProblem('...', ['copilot'])).toMatch(/letter or a digit/)
    expect(wakeWordProblem('CoPilot', ['copilot'])).toMatch(/already/)
    expect(wakeWordProblem('x'.repeat(257), [])).toMatch(/256/)
    expect(wakeWordProblem('hey ship', ['copilot'])).toBeNull()
    const full = Array.from({ length: 64 }, (_, i) => `w${i}`)
    expect(wakeWordProblem('more', full)).toMatch(/64/)
  })

  it('wake words are trimmed, and spaces at the ends are no change', async () => {
    const { store, state } = await readyMic()
    store.set('always_listening.wake_words', [' copilot ', 'co pilot'])
    expect(store.edits()).toEqual([])
    expect(state().dirty).toBe(false)
    store.set('always_listening.wake_words', ['copilot', ' hey ship '])
    expect(store.edits()).toEqual([
      { key: 'always_listening.wake_words', value: { kind: 'text_list', value: ['copilot', 'hey ship'] } },
    ])
  })

  it('a bad field of another screen or mode stops the save and is named', async () => {
    const { store, state, fake } = await readyMic()
    // A pre-roll error that the always-listening mode hides.
    store.set('push_to_talk.pre_roll_ms', 5000)
    store.set('microphone.mode', 'always_listening')
    await store.save('microphone')
    expect(fake.calls.save).toEqual([])
    expect(state().saveError).toContain('Push-to-talk: Pre-roll')
    expect(state().saveError).toMatch(/switch the mode/i)
    // From the Setup screen the same field names its screen.
    await store.save('setup')
    expect(fake.calls.save).toEqual([])
    expect(state().saveError).toContain('Microphone: Push-to-talk: Pre-roll')
    expect(state().saveError).toMatch(/open the Microphone screen/i)
    // A Setup field from the Microphone screen.
    store.discard()
    store.set('stt.parakeet.threads', 0)
    await store.save('microphone')
    expect(fake.calls.save).toEqual([])
    expect(state().saveError).toContain('Setup: Threads')
    expect(state().saveError).toMatch(/open the Setup screen/i)
  })

  it('the unsaved mark names the screens with changes', async () => {
    const { store, state, mic } = await readyMic()
    expect(changedScreens(state())).toEqual([])
    store.set('matching.threshold', 0.9)
    expect(changedScreens(state())).toEqual(['setup'])
    const capture = store.captureBinding()
    mic.end(samples.binding.joystick)
    await capture
    expect(changedScreens(state())).toEqual(['setup', 'microphone'])
    store.set('matching.threshold', 0.8)
    expect(changedScreens(state())).toEqual(['microphone'])
  })
})

describe('the Microphone screen: device list, capture races, and ranges', () => {
  it('a mode switch during a capture cancels it, and a late binding does not change the draft', async () => {
    const { store, state, mic } = await readyMic()
    mic.setCancelEnds(false)
    const capture = store.captureBinding()
    store.set('microphone.mode', 'always_listening')
    await settle()
    expect(mic.calls.cancel).toBeGreaterThanOrEqual(1)
    // The first key of a typed wake word comes back as the binding.
    mic.end({ kind: 'key', label: 'key "C"', key: 'C', joystick: null })
    await capture
    expect(state().capturing).toBe(false)
    expect(state().draft?.push_to_talk).toEqual(state().saved?.push_to_talk)
    expect(store.edits()).toEqual([
      { key: 'microphone.mode', value: { kind: 'text', value: 'always_listening' } },
    ])
  })

  it('a binding that comes while the draft mode is not push-to-talk is dropped', async () => {
    const { store, state, mic } = await readyMic(config((c) => (c.microphone.mode = 'always_listening')))
    const capture = store.captureBinding()
    mic.end(samples.binding.joystick)
    await capture
    expect(state().draft?.push_to_talk).toEqual(state().saved?.push_to_talk)
    expect(store.edits()).toEqual([])
  })

  it('discard during a capture cancels it, and a late binding is dropped', async () => {
    const { store, state, mic } = await readyMic()
    store.set('push_to_talk.pre_roll_ms', 100)
    mic.setCancelEnds(false)
    const capture = store.captureBinding()
    store.discard()
    await settle()
    expect(mic.calls.cancel).toBeGreaterThanOrEqual(1)
    mic.end(samples.binding.joystick)
    await capture
    expect(state().dirty).toBe(false)
    expect(store.edits()).toEqual([])
    expect(state().captureMessage).toMatch(/did not change/)
  })

  it('a cancel that loses the race to a real binding drops it and says so', async () => {
    const { store, state, mic } = await readyMic()
    mic.setCancelEnds(false)
    const capture = store.captureBinding()
    const cancelling = store.cancelCapture()
    mic.end(samples.binding.joystick)
    await Promise.all([capture, cancelling])
    expect(store.edits()).toEqual([])
    expect(state().captureError).toBeNull()
    expect(state().captureMessage).toBe('Cancelled. The talk button did not change.')
  })

  it('the cancel of the page names the token of its capture', async () => {
    const { store, mic, advance } = await readyMic()
    let capture = store.captureBinding()
    await store.cancelCapture()
    await capture
    advance(1_000)
    capture = store.captureBinding()
    await store.cancelCapture()
    await capture
    const [first, second] = mic.tokens.capture
    expect(Number.isInteger(first)).toBe(true)
    expect(first).toBeGreaterThan(0)
    expect(second).not.toBe(first)
    expect(mic.tokens.cancel).toEqual([first, second])
  })

  it('a successful capture says the new label', async () => {
    const { store, state, mic } = await readyMic()
    const capture = store.captureBinding()
    mic.end(samples.binding.joystick)
    await capture
    expect(state().captureMessage).toBe('The talk button is now MOZA button 5. Save to keep it.')
  })

  it('an unchanged value that only the page refuses is a note, not an error', async () => {
    const odd = config((c) => {
      c.microphone.mode = 'always_listening'
      c.microphone.vad_threshold_dbfs = 5.0
      c.always_listening.wake_words = ['...', 'copilot']
    })
    const { store, state, fake } = await readyMic(odd)
    expect(fieldErrors(state(), CORES)['microphone.vad_threshold_dbfs']).toBeUndefined()
    expect(fieldErrors(state(), CORES)['always_listening.wake_words']).toBeUndefined()
    expect(fieldNotes(state())['microphone.vad_threshold_dbfs']).toMatch(/-100 to 0/)
    expect(fieldNotes(state())['always_listening.wake_words']).toMatch(/letter or a digit/)
    // An unrelated edit saves and does not touch these values.
    store.set('push_to_talk.release_tail_ms', 400)
    await store.save('microphone')
    expect(state().saveError).toBeNull()
    expect(fake.calls.save).toEqual([
      [{ key: 'push_to_talk.release_tail_ms', value: { kind: 'integer', value: 400 } }],
    ])
    // A changed value keeps the error, and the note goes.
    store.set('microphone.vad_threshold_dbfs', 6)
    expect(fieldErrors(state(), CORES)['microphone.vad_threshold_dbfs']).toMatch(/-100 to 0/)
    expect(fieldNotes(state())['microphone.vad_threshold_dbfs']).toBeUndefined()
    store.set('always_listening.wake_words', ['...'])
    expect(fieldErrors(state(), CORES)['always_listening.wake_words']).toMatch(/letter or a digit/)
    // A rule of the program stays an error also without a change.
    const nan = await readyMic(config((c) => (c.microphone.vad_threshold_dbfs = Number.NaN)))
    expect(fieldErrors(nan.state(), CORES)['microphone.vad_threshold_dbfs']).toBeDefined()
  })

  it('the Status screen warns while the saved configuration listens always without a wake word', async () => {
    const { store, state, fake } = await readyMic()
    expect(savedWakeWordWarning(state())).toBeNull()
    // A draft change alone does not warn on the Status screen: the session uses the file.
    store.set('microphone.mode', 'always_listening')
    store.set('always_listening.wake_words', [])
    expect(savedWakeWordWarning(state())).toBeNull()
    fake.setFile(config((c) => {
      c.microphone.mode = 'always_listening'
      c.always_listening.wake_words = []
    }))
    await store.save('microphone')
    expect(savedWakeWordWarning(state())).toBe(NO_WAKE_WORD_WARNING)
    expect(wakeWordWarning(state().draft)).toBe(NO_WAKE_WORD_WARNING)
  })
})
