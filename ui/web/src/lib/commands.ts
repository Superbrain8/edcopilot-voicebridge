// The store of the Commands screen: the command list and the phrase test.
//
// The Rust side does all the work. It filters the list with the rules of
// `voicebridge commands`, gives one page of rows at a time, and tests a
// phrase with the options of the live session, always as a dry run. This
// store only keeps the newest answer and the pages that the screen shows.
//
// The store has no function that sends a command. A rejected match stays a
// rejected match.

import { writable, type Readable } from 'svelte/store'
import * as api from './api'
import type { CommandList, CommandRow, PhraseTest } from './api'

/** The functions of `api.ts` that the store uses. Tests give a fake. */
export interface CommandsApi {
  listCommands(filter: string, all: boolean, offset: number, limit: number): Promise<CommandList>
  testPhrase(phrase: string): Promise<PhraseTest>
}

/** The time without a key press before the filter goes to the program. */
export const FILTER_DELAY_MS = 150
/** The number of rows of one page. */
export const PAGE_ROWS = 200
/** The longest filter. The Rust side refuses a longer one. */
export const MAX_FILTER_CHARS = 200
/** The longest phrase. The Rust side refuses a longer one. */
export const MAX_PHRASE_CHARS = 500
/** The note when the command data changed between two pages. */
export const DATA_CHANGED = 'The command data changed. The list starts again at the first page.'

export interface CommandsState {
  /** The filter as typed. */
  filter: string
  /** True: also the commands that do not work, with the reason. */
  all: boolean
  /** True while the first page of a list is in work. */
  loading: boolean
  /** True while the next page is in work. */
  loadingMore: boolean
  /** The newest list with the pages so far. Null before the first answer and after an error. */
  list: CommandList | null
  /** The error text of the newest list call. */
  error: string | null
  /** A note about the list, for example `DATA_CHANGED`. */
  notice: string | null
  /** The phrase as typed. */
  phrase: string
  /** True while a phrase test is in work. */
  testing: boolean
  /** The result of the newest phrase test of this phrase. */
  result: PhraseTest | null
  /** The error text of the newest phrase test of this phrase. */
  testError: string | null
}

export function initialCommandsState(): CommandsState {
  return {
    filter: '',
    all: false,
    loading: false,
    loadingMore: false,
    list: null,
    error: null,
    notice: null,
    phrase: '',
    testing: false,
    result: null,
    testError: null,
  }
}

/**
 * The state of the list for the screen: `failed` (the list did not load),
 * `empty` (no command at all without a filter), `no_match` (no row for the
 * filter), `loaded`, or `waiting` (before the first answer).
 */
export type ListState = 'waiting' | 'failed' | 'empty' | 'no_match' | 'loaded'

export function listState(s: Pick<CommandsState, 'list' | 'error' | 'filter'>): ListState {
  if (s.error !== null) return 'failed'
  if (s.list === null) return 'waiting'
  if (s.list.total > 0) return 'loaded'
  return s.filter.trim() === '' ? 'empty' : 'no_match'
}

/** The rows of one category. */
export interface CategoryGroup {
  category: string
  rows: CommandRow[]
}

/** The name of a category, as the console shows it. */
export function categoryLabel(category: string): string {
  return category === '' ? '(no category)' : category
}

/**
 * The loaded rows, grouped by category in the order of the list (the Rust
 * side sorts them). `hidden` is the number of rows that are not loaded.
 */
export function groupRows(list: Pick<CommandList, 'rows' | 'total'>): { groups: CategoryGroup[]; hidden: number } {
  const groups: CategoryGroup[] = []
  for (const row of list.rows) {
    const last = groups[groups.length - 1]
    if (last && last.category === row.category) last.rows.push(row)
    else groups.push({ category: row.category, rows: [row] })
  }
  return { groups, hidden: Math.max(0, list.total - list.rows.length) }
}

export interface CommandsStore extends Readable<CommandsState> {
  /** The screen opens: loads the list, and tests the last phrase again. */
  init(): Promise<void>
  /** Sets the filter. The list loads after a pause of `FILTER_DELAY_MS`. */
  setFilter(filter: string): void
  /** Sets the switch and loads the list at once. */
  setAll(all: boolean): void
  /** Gets the next page of rows. */
  showMore(): Promise<void>
  /** Sets the phrase. The result of the old phrase goes away. */
  setPhrase(phrase: string): void
  /** Tests the phrase as a dry run. Never rejects. */
  test(): Promise<void>
  /** Stops a filter that waits. */
  destroy(): void
}

export function createCommandsStore(source: CommandsApi): CommandsStore {
  const store = writable<CommandsState>(initialCommandsState())
  let state = initialCommandsState()
  store.subscribe((s) => (state = s))
  // The number of the newest list and phrase test. An answer of an older
  // call is dropped. A new filter or switch makes a new list.
  let listCall = 0
  let testCall = 0
  let timer: ReturnType<typeof setTimeout> | null = null

  function cancelTimer(): void {
    if (timer !== null) clearTimeout(timer)
    timer = null
  }

  async function load(): Promise<void> {
    cancelTimer()
    const mine = ++listCall
    const { filter, all } = state
    store.update((s) => ({ ...s, loading: true, loadingMore: false, notice: null }))
    try {
      const list = await source.listCommands(filter, all, 0, PAGE_ROWS)
      if (mine !== listCall) return
      store.update((s) => ({ ...s, loading: false, list, error: null }))
    } catch (e) {
      if (mine !== listCall) return
      store.update((s) => ({ ...s, loading: false, list: null, error: api.toError(e).message }))
    }
  }

  async function showMore(): Promise<void> {
    const list = state.list
    if (list === null || state.loading || state.loadingMore || list.rows.length >= list.total) return
    const mine = listCall
    const offset = list.rows.length
    store.update((s) => ({ ...s, loadingMore: true }))
    try {
      const page = await source.listCommands(state.filter, state.all, offset, PAGE_ROWS)
      if (mine !== listCall || state.list?.rows.length !== offset) return
      if (page.catalog_id !== state.list.catalog_id) {
        // The program built the list again from changed files. The rows of
        // this page do not fit the rows so far: start again at the first page.
        const reload = load()
        const reloadCall = listCall
        await reload
        if (reloadCall === listCall && state.error === null) {
          store.update((s) => ({ ...s, notice: DATA_CHANGED }))
        }
        return
      }
      store.update((s) => ({
        ...s,
        loadingMore: false,
        list: s.list && { ...page, offset: 0, rows: [...s.list.rows, ...page.rows] },
      }))
    } catch (e) {
      if (mine !== listCall) return
      store.update((s) => ({ ...s, loadingMore: false, error: api.toError(e).message, list: null }))
    }
  }

  async function test(): Promise<void> {
    const phrase = state.phrase
    if (phrase.trim() === '') return
    const mine = ++testCall
    store.update((s) => ({ ...s, testing: true, testError: null }))
    try {
      const result = await source.testPhrase(phrase)
      if (mine !== testCall) return
      store.update((s) => ({ ...s, testing: false, result, testError: null }))
    } catch (e) {
      if (mine !== testCall) return
      store.update((s) => ({ ...s, testing: false, result: null, testError: api.toError(e).message }))
    }
  }

  return {
    subscribe: store.subscribe,
    async init() {
      // The saved settings can be new: the old result can be wrong now.
      const again = state.phrase.trim() !== '' ? test() : Promise.resolve()
      await Promise.all([load(), again])
    },
    setFilter(filter: string) {
      store.update((s) => ({ ...s, filter }))
      cancelTimer()
      timer = setTimeout(() => {
        timer = null
        void load()
      }, FILTER_DELAY_MS)
    },
    setAll(all: boolean) {
      store.update((s) => ({ ...s, all }))
      void load()
    },
    showMore,
    setPhrase(phrase: string) {
      if (phrase === state.phrase) return
      // An answer in work belongs to the old phrase.
      testCall += 1
      store.update((s) => ({ ...s, phrase, result: null, testError: null, testing: false }))
    },
    test,
    destroy: cancelTimer,
  }
}

/** The store of the window, with the real commands. */
export const commandsStore: CommandsStore = createCommandsStore(api)
