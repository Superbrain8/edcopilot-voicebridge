import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { get } from 'svelte/store'
import type { CommandList, CommandRow, PhraseTest } from './api'
import {
  FILTER_DELAY_MS,
  DATA_CHANGED,
  MAX_FILTER_CHARS,
  MAX_PHRASE_CHARS,
  PAGE_ROWS,
  categoryLabel,
  createCommandsStore,
  groupRows,
  listState,
  type CommandsApi,
} from './commands'
import { samples } from './fixtures/dto-samples'

function deferred<T>() {
  let resolve!: (value: T) => void
  let reject!: (error: unknown) => void
  const promise = new Promise<T>((res, rej) => {
    resolve = res
    reject = rej
  })
  return { promise, resolve, reject }
}

function row(category: string, phrase: string, works = true): CommandRow {
  return {
    category,
    phrase,
    token: phrase.toUpperCase(),
    works,
    reason: works ? null : 'does not work: no phrase',
    alias: false,
  }
}

function list(rows: CommandRow[], extra: Partial<CommandList> = {}): CommandList {
  return {
    rows,
    offset: 0,
    total: rows.length,
    catalog_id: 1,
    composite: [],
    summary: `${rows.length} shown.`,
    problems: [],
    warnings: [],
    ...extra,
  }
}

/** `count` rows from `start`. */
function rowsFrom(start: number, count: number): CommandRow[] {
  return Array.from({ length: count }, (_, i) => row('C', `p${start + i}`))
}

type ListCall = {
  filter: string
  all: boolean
  offset: number
  limit: number
  answer: ReturnType<typeof deferred<CommandList>>
}

/**
 * A fake API. Each call waits until the test answers it. Any other property
 * of the API fails the test: the store may call only these two functions.
 */
function fakeApi() {
  const lists: ListCall[] = []
  const tests: { phrase: string; answer: ReturnType<typeof deferred<PhraseTest>> }[] = []
  const known: CommandsApi = {
    listCommands: (filter, all, offset, limit) => {
      const answer = deferred<CommandList>()
      lists.push({ filter, all, offset, limit, answer })
      return answer.promise
    },
    testPhrase: (phrase) => {
      const answer = deferred<PhraseTest>()
      tests.push({ phrase, answer })
      return answer.promise
    },
  }
  const api = new Proxy(known, {
    get(target, name) {
      if (!(name in target)) throw new Error(`the store used ${String(name)}`)
      return target[name as keyof CommandsApi]
    },
  })
  return { api, lists, tests }
}

/** Lets the promise callbacks run. */
async function flush() {
  for (let i = 0; i < 5; i++) await Promise.resolve()
}

describe('commands store', () => {
  beforeEach(() => {
    vi.useFakeTimers()
  })
  afterEach(() => {
    vi.useRealTimers()
  })

  it('init loads the first page of the commands that work, without a filter', async () => {
    const fake = fakeApi()
    const store = createCommandsStore(fake.api)
    const done = store.init()
    expect(fake.lists.map((l) => [l.filter, l.all, l.offset, l.limit])).toEqual([['', false, 0, PAGE_ROWS]])
    fake.lists[0].answer.resolve(samples.command_list)
    await done
    const s = get(store)
    expect(s.list).toEqual(samples.command_list)
    expect(s.loading).toBe(false)
    expect(s.error).toBeNull()
  })

  it('the filter waits for a pause in the typing', async () => {
    const fake = fakeApi()
    const store = createCommandsStore(fake.api)
    store.setFilter('s')
    store.setFilter('se')
    store.setFilter('sail out')
    expect(get(store).filter).toBe('sail out')
    expect(fake.lists).toHaveLength(0)
    vi.advanceTimersByTime(FILTER_DELAY_MS)
    expect(fake.lists.map((l) => l.filter)).toEqual(['sail out'])
  })

  it('a late answer of an older filter does not replace the newer list', async () => {
    const fake = fakeApi()
    const store = createCommandsStore(fake.api)
    store.setFilter('sail')
    vi.advanceTimersByTime(FILTER_DELAY_MS)
    store.setFilter('sail out')
    vi.advanceTimersByTime(FILTER_DELAY_MS)
    expect(fake.lists.map((l) => l.filter)).toEqual(['sail', 'sail out'])
    const newer = list([row('Harbour traffic', 'sail out for home')])
    const older = list([row('A', 'sail a'), row('B', 'sail b')])
    fake.lists[1].answer.resolve(newer)
    await flush()
    fake.lists[0].answer.resolve(older)
    await flush()
    expect(get(store).list).toEqual(newer)
    // A late error of an older filter is dropped too.
    store.setFilter('x')
    vi.advanceTimersByTime(FILTER_DELAY_MS)
    store.setFilter('xy')
    vi.advanceTimersByTime(FILTER_DELAY_MS)
    fake.lists[3].answer.resolve(newer)
    await flush()
    fake.lists[2].answer.reject('late error')
    await flush()
    expect(get(store).error).toBeNull()
  })

  it('the switch loads again at once with the commands that do not work', async () => {
    const fake = fakeApi()
    const store = createCommandsStore(fake.api)
    store.setFilter('gold')
    store.setAll(true)
    // The switch does not wait, and it uses the filter that is typed.
    expect(fake.lists.map((l) => [l.filter, l.all])).toEqual([['gold', true]])
    vi.advanceTimersByTime(FILTER_DELAY_MS)
    expect(fake.lists).toHaveLength(1)
    expect(get(store).all).toBe(true)
  })

  it('Show more gets the next page and adds its rows', async () => {
    const fake = fakeApi()
    const store = createCommandsStore(fake.api)
    const done = store.init()
    fake.lists[0].answer.resolve(list(rowsFrom(0, PAGE_ROWS), { total: 450, summary: '450 shown.' }))
    await done
    const more = store.showMore()
    // A second click while the page loads does nothing.
    void store.showMore()
    expect(fake.lists.map((l) => [l.offset, l.limit])).toEqual([
      [0, PAGE_ROWS],
      [PAGE_ROWS, PAGE_ROWS],
    ])
    fake.lists[1].answer.resolve(
      list(rowsFrom(PAGE_ROWS, PAGE_ROWS), { offset: PAGE_ROWS, total: 450, summary: '450 shown.' }),
    )
    await more
    const s = get(store)
    expect(s.list?.rows.length).toBe(2 * PAGE_ROWS)
    expect(s.list?.rows[PAGE_ROWS].phrase).toBe(`p${PAGE_ROWS}`)
    expect(groupRows(s.list!).hidden).toBe(450 - 2 * PAGE_ROWS)
  })

  it('a page from a newer catalog reloads the list', async () => {
    const fake = fakeApi()
    const store = createCommandsStore(fake.api)
    const done = store.init()
    fake.lists[0].answer.resolve(list(rowsFrom(0, PAGE_ROWS), { total: 450 }))
    await done
    store.setAll(true)
    fake.lists[1].answer.resolve(list(rowsFrom(0, PAGE_ROWS), { total: 450 }))
    await flush()
    // A data file changed: the program built a new catalog.
    const more = store.showMore()
    fake.lists[2].answer.resolve(
      list(rowsFrom(1000, PAGE_ROWS), { offset: PAGE_ROWS, total: 460, catalog_id: 2 }),
    )
    await flush()
    // The page does not fit: the list loads again from the first row, with the same filter and switch.
    expect(fake.lists.map((l) => [l.filter, l.all, l.offset])).toEqual([
      ['', false, 0],
      ['', true, 0],
      ['', true, PAGE_ROWS],
      ['', true, 0],
    ])
    expect(get(store).list?.rows.length).toBe(PAGE_ROWS)
    fake.lists[3].answer.resolve(list(rowsFrom(0, PAGE_ROWS), { total: 460, catalog_id: 2 }))
    await more
    const s = get(store)
    expect(s.list?.rows.map((r) => r.phrase).slice(0, 2)).toEqual(['p0', 'p1'])
    expect(s.list?.total).toBe(460)
    expect(s.notice).toBe(DATA_CHANGED)
    // A new filter removes the note.
    store.setFilter('gold')
    vi.advanceTimersByTime(FILTER_DELAY_MS)
    expect(get(store).notice).toBeNull()
  })

  it('a page of an old filter is dropped', async () => {
    const fake = fakeApi()
    const store = createCommandsStore(fake.api)
    const done = store.init()
    fake.lists[0].answer.resolve(list(rowsFrom(0, PAGE_ROWS), { total: 450 }))
    await done
    const more = store.showMore()
    store.setFilter('gold')
    vi.advanceTimersByTime(FILTER_DELAY_MS)
    const gold = list([row('Trade', 'gold')])
    fake.lists[2].answer.resolve(gold)
    await flush()
    fake.lists[1].answer.resolve(list(rowsFrom(PAGE_ROWS, PAGE_ROWS), { offset: PAGE_ROWS, total: 450 }))
    await more
    expect(get(store).list).toEqual(gold)
  })

  it('a list error shows the error text and no list', async () => {
    const fake = fakeApi()
    const store = createCommandsStore(fake.api)
    const done = store.init()
    fake.lists[0].answer.reject('cannot read core/data/none.json')
    await done
    const s = get(store)
    expect(s.list).toBeNull()
    expect(s.error).toBe('cannot read core/data/none.json')
    expect(listState(s)).toBe('failed')
  })

  it('the state of the list: loaded, empty without a filter, or no row for the filter', async () => {
    const fake = fakeApi()
    const store = createCommandsStore(fake.api)
    const done = store.init()
    fake.lists[0].answer.resolve(list([], { problems: ['voice commands file not found: x'] }))
    await done
    expect(listState(get(store))).toBe('empty')
    expect(get(store).list?.problems).toEqual(['voice commands file not found: x'])
    store.setFilter('zzz')
    vi.advanceTimersByTime(FILTER_DELAY_MS)
    fake.lists[1].answer.resolve(list([]))
    await flush()
    expect(listState(get(store))).toBe('no_match')
    store.setFilter('gold')
    vi.advanceTimersByTime(FILTER_DELAY_MS)
    fake.lists[2].answer.resolve(list([row('Trade', 'gold')]))
    await flush()
    expect(listState(get(store))).toBe('loaded')
  })

  it('a phrase test gives the result as the Rust side sends it, also a rejected match', async () => {
    const fake = fakeApi()
    const store = createCommandsStore(fake.api)
    store.setPhrase('cancal')
    const done = store.test()
    expect(fake.tests.map((t) => t.phrase)).toEqual(['cancal'])
    expect(get(store).testing).toBe(true)
    fake.tests[0].answer.resolve(samples.phrase_test.rejected)
    await done
    const s = get(store)
    expect(s.testing).toBe(false)
    expect(s.result).toEqual(samples.phrase_test.rejected)
    expect(s.result?.outcome.kind).toBe('rejected')
  })

  it('a late answer of an older phrase does not replace the newer result', async () => {
    const fake = fakeApi()
    const store = createCommandsStore(fake.api)
    store.setPhrase('cancal')
    const first = store.test()
    store.setPhrase('help chart log')
    const second = store.test()
    fake.tests[1].answer.resolve(samples.phrase_test.dry_run)
    await second
    fake.tests[0].answer.resolve(samples.phrase_test.rejected)
    await first
    expect(get(store).result).toEqual(samples.phrase_test.dry_run)
    expect(get(store).testing).toBe(false)
  })

  it('a change of the phrase clears the old result and drops its answer', async () => {
    const fake = fakeApi()
    const store = createCommandsStore(fake.api)
    store.setPhrase('cancal')
    await (async () => {
      const done = store.test()
      fake.tests[0].answer.resolve(samples.phrase_test.rejected)
      await done
    })()
    expect(get(store).result).not.toBeNull()
    store.setPhrase('cancel')
    expect(get(store).result).toBeNull()
    // An answer in work for the old phrase does not come back.
    const late = store.test()
    store.setPhrase('cancel please')
    fake.tests[1].answer.resolve(samples.phrase_test.dry_run)
    await late
    expect(get(store).result).toBeNull()
    expect(get(store).testing).toBe(false)
  })

  it('opening the screen again tests the last phrase again', async () => {
    const fake = fakeApi()
    const store = createCommandsStore(fake.api)
    void store.init()
    expect(fake.tests).toHaveLength(0)
    store.setPhrase('cancal')
    const done = store.test()
    fake.tests[0].answer.resolve(samples.phrase_test.rejected)
    await done
    // The settings changed on another screen. The screen opens again.
    void store.init()
    expect(fake.tests.map((t) => t.phrase)).toEqual(['cancal', 'cancal'])
    fake.tests[1].answer.resolve(samples.phrase_test.dry_run)
    await flush()
    expect(get(store).result).toEqual(samples.phrase_test.dry_run)
  })

  it('a phrase test error shows its text, and an empty phrase is not tested', async () => {
    const fake = fakeApi()
    const store = createCommandsStore(fake.api)
    store.setPhrase('   ')
    await store.test()
    expect(fake.tests).toHaveLength(0)
    store.setPhrase('help')
    const done = store.test()
    fake.tests[0].answer.reject('matching.threshold must be between 0.0 and 1.0')
    await done
    expect(get(store).testError).toBe('matching.threshold must be between 0.0 and 1.0')
    expect(get(store).result).toBeNull()
  })

  it('the length limits are the limits of the program', () => {
    expect(MAX_FILTER_CHARS).toBe(200)
    expect(MAX_PHRASE_CHARS).toBe(500)
  })
})

describe('groupRows', () => {
  it('groups the rows by category in their order and counts the rows that are not loaded', () => {
    const rows = [row('A', 'a1'), row('A', 'a2'), row('', 'n1'), row('B', 'b1')]
    const page = groupRows(list(rows, { total: 5 }))
    expect(page.hidden).toBe(1)
    expect(page.groups.map((g) => [g.category, g.rows.map((r) => r.phrase)])).toEqual([
      ['A', ['a1', 'a2']],
      ['', ['n1']],
      ['B', ['b1']],
    ])
  })

  it('3000 rows: the page has one page of rows', () => {
    const page = groupRows(list(rowsFrom(0, PAGE_ROWS), { total: 3000 }))
    expect(page.groups.reduce((n, g) => n + g.rows.length, 0)).toBe(PAGE_ROWS)
    expect(page.hidden).toBe(3000 - PAGE_ROWS)
  })

  it('a row without category is under "(no category)", as on the console', () => {
    expect(categoryLabel('')).toBe('(no category)')
    expect(categoryLabel('Panels')).toBe('Panels')
  })
})
