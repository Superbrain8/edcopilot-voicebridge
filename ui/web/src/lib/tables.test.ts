import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { get, writable } from 'svelte/store'
import type { TableCheck, TableKind, TableRow, TableSave, WordTable } from './api'
import { createConfigStore, type ConfigApi, type SessionPhase } from './config'
import { samples } from './fixtures/dto-samples'
import {
  CHECK_DELAY_MS,
  MAY_BE_LAST,
  NOT_LAST,
  FILE_CHANGED,
  canSave,
  checkIsCurrent,
  cleanRows,
  createTablesStore,
  isDirty,
  meantFromText,
  meantText,
  needsConfirmation,
  tabId,
  type TablesApi,
} from './tables'

function deferred<T>() {
  let resolve!: (value: T) => void
  let reject!: (error: unknown) => void
  const promise = new Promise<T>((res, rej) => {
    resolve = res
    reject = rej
  })
  return { promise, resolve, reject }
}

type Answer<T> = ReturnType<typeof deferred<T>>

interface CheckCall {
  kind: TableKind
  language: string | null
  rows: TableRow[]
  answer: Answer<TableCheck>
}

interface SaveCall {
  kind: TableKind
  language: string | null
  rows: TableRow[]
  confirmed: string[]
  version: string
  answer: Answer<TableSave>
}

/** A fake API: each call waits until the test answers it. */
function fakeApi() {
  const reads: { kind: TableKind; language: string | null; answer: Answer<WordTable> }[] = []
  const checks: CheckCall[] = []
  const saves: SaveCall[] = []
  const api: TablesApi = {
    readTable: (kind, language) => {
      const answer = deferred<WordTable>()
      reads.push({ kind, language, answer })
      return answer.promise
    },
    checkTable: (kind, language, rows) => {
      const answer = deferred<TableCheck>()
      checks.push({ kind, language, rows: structuredClone(rows), answer })
      return answer.promise
    },
    saveTable: (kind, language, rows, confirmed, version) => {
      const answer = deferred<TableSave>()
      saves.push({ kind, language, rows: structuredClone(rows), confirmed: [...confirmed], version, answer })
      return answer.promise
    },
  }
  return { api, reads, checks, saves }
}

const synonyms = samples.table.synonyms
const CLEAN: TableCheck = { problems: [], warnings: [] }
const WARNING = 'synonym "sail out" is also in the phrase "sail out to <System>" -> X'
const PROBLEM = 'D:\\vb\\core\\data\\synonyms.toml: synonym "a." has punctuation in a word'

function saved(extra: Partial<TableSave> = {}): TableSave {
  return {
    written: true,
    problems: [],
    warnings: [],
    unconfirmed: [],
    version: 'v2',
    restart_needed: false,
    ...extra,
  }
}

/** A store with the synonym table loaded and its first check answered. */
async function loaded(options: { onSaved?: (result: TableSave) => void } = {}) {
  const fake = fakeApi()
  const store = createTablesStore(fake.api, options)
  const opened = store.open('synonyms', null)
  fake.reads[0].answer.resolve(structuredClone(synonyms))
  await opened
  await vi.advanceTimersByTimeAsync(CHECK_DELAY_MS)
  fake.checks[0].answer.resolve(CLEAN)
  await vi.advanceTimersByTimeAsync(0)
  const id = tabId('synonyms', null)
  const tab = () => get(store).tabs[id]
  return { fake, store, id, tab }
}

beforeEach(() => {
  vi.useFakeTimers()
})

afterEach(() => {
  vi.useRealTimers()
})

describe('the words store', () => {
  it('reads the table and checks its rows at once', async () => {
    const { fake, tab } = await loaded()
    expect(fake.reads[0]).toMatchObject({ kind: 'synonyms', language: null })
    expect(fake.checks[0].rows).toEqual(synonyms.rows)
    expect(tab().rows).toEqual(synonyms.rows)
    expect(isDirty(tab())).toBe(false)
    // Nothing changed: nothing to save.
    expect(canSave(tab())).toBe(false)
  })

  it('a problem switches Save off', async () => {
    const { fake, store, id, tab } = await loaded()
    store.addRow(id)
    store.setSpoken(id, 2, 'a.')
    store.setMeant(id, 2, ['sail out to'])
    // Save stays off while the check of the current rows is not back.
    expect(canSave(tab())).toBe(false)
    await vi.advanceTimersByTimeAsync(CHECK_DELAY_MS)
    expect(fake.checks).toHaveLength(2)
    expect(fake.checks[1].rows[2]).toEqual({ spoken: 'a.', meant: ['sail out to'] })
    expect(canSave(tab())).toBe(false)
    fake.checks[1].answer.resolve({ problems: [PROBLEM], warnings: [] })
    await vi.advanceTimersByTimeAsync(0)
    expect(tab().check?.problems).toEqual([PROBLEM])
    expect(canSave(tab())).toBe(false)
    await store.save(id)
    expect(fake.saves).toHaveLength(0)
    // The repair gives a new check. Its clean answer switches Save on.
    store.setSpoken(id, 2, 'a')
    await vi.advanceTimersByTimeAsync(CHECK_DELAY_MS)
    fake.checks[2].answer.resolve(CLEAN)
    await vi.advanceTimersByTimeAsync(0)
    expect(canSave(tab())).toBe(true)
  })

  it('a late check answer for older rows does not switch Save on', async () => {
    const { fake, store, id, tab } = await loaded()
    store.addRow(id)
    store.setSpoken(id, 2, 'fly to')
    store.setMeant(id, 2, ['sail out to'])
    await vi.advanceTimersByTimeAsync(CHECK_DELAY_MS)
    const older = fake.checks[1]
    // A new edit while the older check runs. One check at a time: the
    // newer check starts when the older one ends.
    store.setSpoken(id, 2, 'sail out')
    await vi.advanceTimersByTimeAsync(CHECK_DELAY_MS)
    expect(fake.checks).toHaveLength(2)
    older.answer.resolve(CLEAN)
    await vi.advanceTimersByTimeAsync(0)
    expect(canSave(tab())).toBe(false)
    expect(tab().check).toEqual(CLEAN)
    expect(fake.checks).toHaveLength(3)
    const newer = fake.checks[2]
    expect(newer.rows[2].spoken).toBe('sail out')
    newer.answer.resolve({ problems: [PROBLEM], warnings: [] })
    await vi.advanceTimersByTimeAsync(0)
    expect(canSave(tab())).toBe(false)
    expect(tab().check?.problems).toEqual([PROBLEM])
  })

  it('runs one check at a time and then only the newest rows', async () => {
    const { fake, store, id, tab } = await loaded()
    store.addRow(id)
    store.setSpoken(id, 2, 'a')
    await vi.advanceTimersByTimeAsync(CHECK_DELAY_MS)
    expect(fake.checks).toHaveLength(2)
    // Three more edits while the check runs: no other check starts.
    for (const word of ['ab', 'abc', 'abcd']) {
      store.setSpoken(id, 2, word)
      await vi.advanceTimersByTimeAsync(CHECK_DELAY_MS)
    }
    expect(fake.checks).toHaveLength(2)
    fake.checks[1].answer.resolve(CLEAN)
    await vi.advanceTimersByTimeAsync(0)
    // One check of the newest rows, not one for each edit.
    expect(fake.checks).toHaveLength(3)
    expect(fake.checks[2].rows[2].spoken).toBe('abcd')
    fake.checks[2].answer.resolve(CLEAN)
    await vi.advanceTimersByTimeAsync(CHECK_DELAY_MS)
    expect(fake.checks).toHaveLength(3)
    expect(canSave(tab())).toBe(true)
  })

  it('a failed check shows the error and Save stays off', async () => {
    const { fake, store, id, tab } = await loaded()
    store.removeRow(id, 0)
    await vi.advanceTimersByTimeAsync(CHECK_DELAY_MS)
    fake.checks[1].answer.reject('the command failed')
    await vi.advanceTimersByTimeAsync(0)
    expect(checkIsCurrent(tab())).toBe(true)
    expect(tab().checking).toBe(false)
    expect(tab().checkError).toBe('the command failed')
    expect(canSave(tab())).toBe(false)
    // The next edit checks again, and a good answer clears the error.
    store.setSpoken(id, 0, 'navigate')
    await vi.advanceTimersByTimeAsync(CHECK_DELAY_MS)
    fake.checks[2].answer.resolve(CLEAN)
    await vi.advanceTimersByTimeAsync(0)
    expect(tab().checkError).toBeNull()
    expect(canSave(tab())).toBe(true)
  })

  it('spaces at the ends of an entry are not a change', async () => {
    const { store, id, tab } = await loaded()
    store.setSpoken(id, 0, ` ${synonyms.rows[0].spoken} `)
    store.setMeant(id, 0, synonyms.rows[0].meant.map((m) => `${m}  `))
    expect(isDirty(tab())).toBe(false)
    store.setSpoken(id, 0, 'navigate  to')
    expect(isDirty(tab())).toBe(true)
  })

  it('after a change of the file, the file is read again and the rows stay', async () => {
    const { fake, store, id, tab } = await loaded()
    store.removeRow(id, 0)
    await vi.advanceTimersByTimeAsync(CHECK_DELAY_MS)
    fake.checks[1].answer.resolve(CLEAN)
    await vi.advanceTimersByTimeAsync(0)
    const saving = store.save(id)
    fake.saves[0].answer.resolve(saved({ written: false, problems: [FILE_CHANGED], version: 'other' }))
    await saving
    expect(tab().fileChanged).toBe(true)
    const reading = store.readAgainKeepRows(id)
    fake.reads[1].answer.resolve({ ...structuredClone(synonyms), version: 'other' })
    await reading
    expect(tab().fileChanged).toBe(false)
    expect(tab().table?.version).toBe('other')
    expect(tab().rows).toHaveLength(1)
    await vi.advanceTimersByTimeAsync(0)
    fake.checks[2].answer.resolve(CLEAN)
    await vi.advanceTimersByTimeAsync(0)
    expect(canSave(tab())).toBe(true)
    const again = store.save(id)
    expect(fake.saves[1].version).toBe('other')
    fake.saves[1].answer.resolve(saved({ written: false }))
    await again
  })

  it('a file with spaces at the ends of an entry is not a change when it opens', async () => {
    const fake = fakeApi()
    const store = createTablesStore(fake.api)
    const opened = store.open('synonyms', null)
    fake.reads[0].answer.resolve({ ...structuredClone(synonyms), rows: [{ spoken: ' fly to ', meant: [' sail out to'] }] })
    await opened
    expect(isDirty(get(store).tabs[tabId('synonyms', null)])).toBe(false)
  })

  it('a discard during "read again and keep my rows" wins', async () => {
    const { fake, store, id, tab } = await loaded()
    store.removeRow(id, 0)
    const again = store.readAgainKeepRows(id)
    const discard = store.discard(id)
    fake.reads[2].answer.resolve({ ...structuredClone(synonyms), version: 'from-discard' })
    await discard
    fake.reads[1].answer.resolve({ ...structuredClone(synonyms), version: 'from-read-again' })
    await again
    expect(tab().table?.version).toBe('from-discard')
    expect(tab().rows).toEqual(synonyms.rows)
  })

  it('read again and keep my rows says that a save replaces the other change', async () => {
    const { fake, store, id, tab } = await loaded()
    store.removeRow(id, 0)
    const again = store.readAgainKeepRows(id)
    fake.reads[1].answer.resolve({ ...structuredClone(synonyms), version: 'other' })
    await again
    expect(tab().saveMessage).toMatch(/replaces what the other program wrote/)
  })

  it('destroy stops the check that waits for the check in work', async () => {
    const { fake, store, id } = await loaded()
    store.removeRow(id, 0)
    await vi.advanceTimersByTimeAsync(CHECK_DELAY_MS)
    expect(fake.checks).toHaveLength(2)
    store.removeRow(id, 0)
    await vi.advanceTimersByTimeAsync(CHECK_DELAY_MS)
    store.destroy()
    fake.checks[1].answer.resolve(CLEAN)
    await vi.advanceTimersByTimeAsync(CHECK_DELAY_MS)
    expect(fake.checks).toHaveLength(2)
  })

  it('a language in another case uses the tab of its file', async () => {
    const fake = fakeApi()
    const store = createTablesStore(fake.api)
    const opened = store.open('aliases', 'DE')
    fake.reads[0].answer.resolve({ ...structuredClone(samples.table.new_language), language: 'de', languages: ['de'], exists: true })
    await opened
    expect(get(store).active).toBe(tabId('aliases', 'de'))
    expect(Object.keys(get(store).tabs)).toEqual([tabId('aliases', 'de')])
    expect(get(store).languages).toEqual(['de'])
    // The same file again: no second read and no second tab.
    await store.open('aliases', 'de')
    expect(fake.reads).toHaveLength(1)
  })

  it('a warning needs a confirmation', async () => {
    const { fake, store, id, tab } = await loaded()
    store.addRow(id)
    store.setSpoken(id, 2, 'sail out')
    store.setMeant(id, 2, ['chart a way'])
    await vi.advanceTimersByTimeAsync(CHECK_DELAY_MS)
    fake.checks[1].answer.resolve({ problems: [], warnings: [WARNING] })
    await vi.advanceTimersByTimeAsync(0)
    expect(needsConfirmation(tab())).toBe(true)
    expect(canSave(tab())).toBe(false)
    store.confirm(id)
    expect(needsConfirmation(tab())).toBe(false)
    expect(canSave(tab())).toBe(true)
    // An edit after the confirmation needs a new check and a new confirmation.
    store.setMeant(id, 2, ['plot a route'])
    expect(canSave(tab())).toBe(false)
    await vi.advanceTimersByTimeAsync(CHECK_DELAY_MS)
    fake.checks[2].answer.resolve({ problems: [], warnings: [WARNING] })
    await vi.advanceTimersByTimeAsync(0)
    expect(needsConfirmation(tab())).toBe(true)
    expect(canSave(tab())).toBe(false)
    store.confirm(id)
    // The save sends the confirmed warnings, the rows of the check, and the version of the read.
    const saving = store.save(id)
    expect(fake.saves).toHaveLength(1)
    expect(fake.saves[0]).toMatchObject({
      kind: 'synonyms',
      language: null,
      confirmed: [WARNING],
      version: synonyms.version,
    })
    expect(fake.saves[0].rows[2]).toEqual({ spoken: 'sail out', meant: ['plot a route'] })
    fake.saves[0].answer.resolve(saved({ warnings: [WARNING] }))
    await vi.advanceTimersByTimeAsync(0)
    const after = { ...structuredClone(synonyms), rows: fake.saves[0].rows, version: 'v2' }
    fake.reads[1].answer.resolve(after)
    await saving
    await vi.advanceTimersByTimeAsync(0)
    expect(tab().table?.version).toBe('v2')
    expect(isDirty(tab())).toBe(false)
    expect(tab().saveMessage).toMatch(/saved/i)
  })

  it('a warning that the page did not show refuses the save and needs a new confirmation', async () => {
    const { fake, store, id, tab } = await loaded()
    store.addRow(id)
    store.setSpoken(id, 2, 'fly to')
    store.setMeant(id, 2, ['sail out to'])
    await vi.advanceTimersByTimeAsync(CHECK_DELAY_MS)
    fake.checks[1].answer.resolve(CLEAN)
    await vi.advanceTimersByTimeAsync(0)
    const saving = store.save(id)
    expect(fake.saves[0].confirmed).toEqual([])
    fake.saves[0].answer.resolve(
      saved({ written: false, warnings: [WARNING], unconfirmed: [WARNING], version: synonyms.version }),
    )
    await saving
    expect(fake.reads).toHaveLength(1)
    expect(tab().check?.warnings).toEqual([WARNING])
    expect(needsConfirmation(tab())).toBe(true)
    expect(canSave(tab())).toBe(false)
    expect(isDirty(tab())).toBe(true)
  })

  it('an edit while a session runs sets restartNeeded', async () => {
    const configApi: ConfigApi = {
      getConfig: () => Promise.resolve(structuredClone(samples.config)),
      saveConfig: () => Promise.reject(new Error('not used')),
      check: () => Promise.resolve(structuredClone(samples.check.loaded)),
      setApiKey: () => Promise.reject(new Error('not used')),
      clearApiKey: () => Promise.reject(new Error('not used')),
    }
    const phase = writable<SessionPhase>({ running: true, starting: false })
    const config = createConfigStore(configApi, { cores: 8, session: phase })
    const { fake, store, id } = await loaded({
      onSaved: (result) => {
        if (result.restart_needed) config.markRestartNeeded()
      },
    })
    store.addRow(id)
    store.setSpoken(id, 2, 'fly to')
    store.setMeant(id, 2, ['sail out to'])
    await vi.advanceTimersByTimeAsync(CHECK_DELAY_MS)
    fake.checks[1].answer.resolve(CLEAN)
    await vi.advanceTimersByTimeAsync(0)
    expect(get(config).restartNeeded).toBe(false)
    const saving = store.save(id)
    fake.saves[0].answer.resolve(saved({ restart_needed: true }))
    await vi.advanceTimersByTimeAsync(0)
    fake.reads[1].answer.resolve({ ...structuredClone(synonyms), rows: fake.saves[0].rows, version: 'v2' })
    await saving
    expect(get(config).restartNeeded).toBe(true)
    // The note goes away when the session stops.
    phase.set({ running: false, starting: false })
    expect(get(config).restartNeeded).toBe(false)
  })

  it('a refused save keeps the rows and shows the problems', async () => {
    const { fake, store, id, tab } = await loaded()
    store.removeRow(id, 0)
    await vi.advanceTimersByTimeAsync(CHECK_DELAY_MS)
    fake.checks[1].answer.resolve(CLEAN)
    await vi.advanceTimersByTimeAsync(0)
    const saving = store.save(id)
    const changed = 'the file changed after the screen read it: read the table again, then make the change again'
    fake.saves[0].answer.resolve(saved({ written: false, problems: [changed], version: 'other' }))
    await saving
    expect(tab().rows).toHaveLength(1)
    expect(tab().check?.problems).toEqual([changed])
    expect(canSave(tab())).toBe(false)
    // Discard reads the file again.
    const reading = store.discard(id)
    fake.reads[1].answer.resolve({ ...structuredClone(synonyms), version: 'other' })
    await reading
    expect(tab().rows).toEqual(synonyms.rows)
    expect(tab().table?.version).toBe('other')
  })

  it('keeps one draft for each tab and marks the changed tabs', async () => {
    const { fake, store, id } = await loaded()
    store.removeRow(id, 1)
    const opened = store.open('aliases', 'fr')
    fake.reads[1].answer.resolve(structuredClone(samples.table.new_language))
    await opened
    expect(get(store).active).toBe(tabId('aliases', 'fr'))
    expect(get(store).languages).toEqual(['de', 'fr'])
    expect(get(store).tabs[id].rows).toHaveLength(1)
    expect(store.changedTabs()).toEqual([id])
  })

  it('a draft gets its check again when the screen opens again', async () => {
    const { fake, store, id, tab } = await loaded()
    store.removeRow(id, 1)
    // The screen closes before the check runs.
    store.destroy()
    await vi.advanceTimersByTimeAsync(CHECK_DELAY_MS)
    expect(fake.checks).toHaveLength(1)
    await store.open('synonyms', null)
    await vi.advanceTimersByTimeAsync(0)
    expect(fake.checks).toHaveLength(2)
    expect(fake.checks[1].rows).toEqual(tab().rows)
    fake.checks[1].answer.resolve(CLEAN)
    await vi.advanceTimersByTimeAsync(0)
    expect(canSave(tab())).toBe(true)
  })

  it('a new language is added to the list and read as an empty table', async () => {
    const fake = fakeApi()
    const store = createTablesStore(fake.api)
    const opened = store.open('aliases', 'de')
    fake.reads[0].answer.resolve({ ...structuredClone(samples.table.new_language), language: 'de', languages: ['de'] })
    await opened
    const added = store.newLanguage(' pt-BR ')
    expect(fake.reads[1]).toMatchObject({ kind: 'aliases', language: 'pt-BR' })
    fake.reads[1].answer.reject('the language name must have 1 to 32 letters')
    await added
    const tab = get(store).tabs[tabId('aliases', 'pt-BR')]
    expect(tab.loadError).toMatch(/language name/)
    // A name that the program refused does not join the list.
    expect(get(store).languages).toEqual(['de'])
  })

  it('a read error shows the text and no rows', async () => {
    const fake = fakeApi()
    const store = createTablesStore(fake.api)
    const opened = store.open('fillers', null)
    fake.reads[0].answer.reject('the key "extra" is not part of this table: edit the file by hand')
    await opened
    const tab = get(store).tabs[tabId('fillers', null)]
    expect(tab.loadError).toMatch(/edit the file by hand/)
    expect(tab.table).toBeNull()
    expect(canSave(tab)).toBe(false)
  })
})

describe('the list field', () => {
  it('has one entry on each line, and the empty lines do not go to the program', () => {
    const typed = meantFromText('sail out to\r\nsail out for\n\n')
    expect(typed).toEqual(['sail out to', 'sail out for', '', ''])
    // The field keeps the typed text.
    expect(meantText({ spoken: 'x', meant: typed })).toBe('sail out to\nsail out for\n\n')
    const row = { spoken: 'navigate to', meant: typed }
    expect(cleanRows('synonyms', [row])).toEqual([
      { spoken: 'navigate to', meant: ['sail out to', 'sail out for'] },
    ])
    // A sound-alike keeps its one value, also when it is empty: the program reports it.
    expect(cleanRows('sound_alikes', [{ spoken: 'cell', meant: [''] }])).toEqual([
      { spoken: 'cell', meant: [''] },
    ])
  })

  it('a typed empty line is not a change and is not checked', async () => {
    const { fake, store, id, tab } = await loaded()
    store.setMeant(id, 0, [...synonyms.rows[0].meant, ''])
    expect(isDirty(tab())).toBe(false)
    await vi.advanceTimersByTimeAsync(CHECK_DELAY_MS)
    expect(fake.checks[1].rows[0]).toEqual(synonyms.rows[0])
  })

  it('names the two filler lists', () => {
    expect([NOT_LAST, MAY_BE_LAST]).toEqual(['not_last', 'may_be_last'])
  })
})
