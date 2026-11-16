// The report of the build from the EDCoPilot profile, for the Diagnostics
// and Setup screens, and the store of the list of the left-out commands.
//
// The Rust side makes each text of the report: the lines of
// `voicebridge check` and of `voicebridge check --left-out`. This module
// only selects what a screen shows. A check that did not run never shows a
// clean state.

import { writable, type Readable } from 'svelte/store'
import * as api from './api'
import type { CheckResult, Config, LeftOutList } from './api'

/** The number of rows of one page of the left-out list. */
export const LEFT_OUT_PAGE_ROWS = 200
/** The text when the check did not run. */
export const NOT_CHECKED = 'The check did not run. The program shows no report of the EDCoPilot profile.'
/** The note when the data changed between two pages. */
export const LEFT_OUT_CHANGED = 'The EDCoPilot profile changed. The list starts again at the first page.'

/**
 * What the Diagnostics screen shows for the profile: `report` (the lines of
 * `check`), `problem` (the profile did not load), `not_checked` (the check
 * failed), or `waiting`.
 */
export interface ProfileView {
  kind: 'report' | 'problem' | 'not_checked' | 'waiting'
  lines: string[]
  problems: string[]
}

export function profileView(check: CheckResult | null, checkError: string | null): ProfileView {
  // A failed latest check never shows the report of an older check.
  if (checkError !== null) return { kind: 'not_checked', lines: [], problems: [NOT_CHECKED] }
  if (check === null) return { kind: 'waiting', lines: [], problems: [] }
  if (check.profile !== null) return { kind: 'report', lines: check.profile.lines, problems: [] }
  if (check.edcopilot_problems.length > 0) {
    return { kind: 'problem', lines: [], problems: check.edcopilot_problems }
  }
  // The configuration or the commands did not load: the problems of the check say why.
  return { kind: 'problem', lines: [], problems: check.problems }
}

/** What the Setup screen shows below the field of the EDCoPilot folder. */
export interface ProfileField {
  /** The profile file that the program reads, or null. */
  file: string | null
  /** The problems of the folder and of the profile, as the program gives them. */
  problems: string[]
}

/**
 * `checkError`: the error of the latest check. Then the problems of an
 * older check do not show: the screen says that the check did not run.
 */
export function profileField(
  saved: Config | null,
  check: CheckResult | null,
  checkError: string | null = null,
): ProfileField {
  const file = saved && saved.edcopilot.profile_in_use !== '' ? saved.edcopilot.profile_in_use : null
  const problems = checkError === null ? (check?.edcopilot_problems ?? []) : []
  return { file, problems }
}

/**
 * True when the Diagnostics screen shows the list of the left-out commands
 * for this kind of profile view. When the list goes away, the screen
 * closes it.
 */
export function leftOutListShown(kind: ProfileView['kind']): boolean {
  return kind === 'report' || kind === 'not_checked'
}

/** The functions of `api.ts` that the store uses. Tests give a fake. */
export interface LeftOutApi {
  listLeftOut(filter: string, offset: number, limit: number): Promise<LeftOutList>
}

export interface LeftOutState {
  filter: string
  loading: boolean
  /** The newest list with the pages so far. Null before the first answer and after an error. */
  list: LeftOutList | null
  error: string | null
  notice: string | null
  /**
   * True after a check that did not run. The list then has no rows: rows of
   * an older catalog must not stand next to a profile panel that says "not
   * checked". The screen shows `NOT_CHECKED`. The next check that runs
   * clears it.
   */
  notChecked: boolean
}

export interface LeftOutStore extends Readable<LeftOutState> {
  /** Loads the first page. */
  load(): Promise<void>
  /** The screen opened the list: loads the first page. */
  open(): Promise<void>
  /** The screen closed the list. `refresh` then loads nothing. */
  close(): void
  /** Loads the first page again when the list is open. */
  refresh(): Promise<void>
  /** Sets the filter and loads the first page. */
  setFilter(filter: string): Promise<void>
  /** Gets the next page. */
  showMore(): Promise<void>
  /**
   * `false`: the latest check did not run. Clears the list, drops the
   * answers of calls in work, and loads nothing until a check runs again.
   * `true`: a check ran.
   */
  setChecked(ok: boolean): void
}

export function createLeftOutStore(source: LeftOutApi): LeftOutStore {
  const empty: LeftOutState = { filter: '', loading: false, list: null, error: null, notice: null, notChecked: false }
  const store = writable<LeftOutState>(empty)
  let state: LeftOutState = empty
  store.subscribe((s) => (state = s))
  // The number of the newest list. An answer of an older call is dropped.
  let call = 0
  // The copy of `notChecked` for the calls. A store update inside the
  // subscriber of another store reaches `state` only later.
  let notChecked = false

  async function load(): Promise<void> {
    if (notChecked) return
    const mine = ++call
    store.update((s) => ({ ...s, loading: true, notice: null }))
    try {
      const list = await source.listLeftOut(state.filter, 0, LEFT_OUT_PAGE_ROWS)
      if (mine !== call) return
      store.update((s) => ({ ...s, loading: false, list, error: null }))
    } catch (e) {
      if (mine !== call) return
      store.update((s) => ({ ...s, loading: false, list: null, error: api.toError(e).message }))
    }
  }

  async function showMore(): Promise<void> {
    const list = state.list
    if (list === null || state.loading || list.rows.length >= list.total) return
    const mine = call
    const offset = list.rows.length
    store.update((s) => ({ ...s, loading: true }))
    try {
      const page = await source.listLeftOut(state.filter, offset, LEFT_OUT_PAGE_ROWS)
      if (mine !== call) return
      if (page.catalog_id !== list.catalog_id) {
        await load()
        store.update((s) => ({ ...s, notice: s.error === null ? LEFT_OUT_CHANGED : null }))
        return
      }
      store.update((s) => ({
        ...s,
        loading: false,
        list: s.list && { ...page, offset: 0, rows: [...s.list.rows, ...page.rows] },
      }))
    } catch (e) {
      if (mine !== call) return
      store.update((s) => ({ ...s, loading: false, list: null, error: api.toError(e).message }))
    }
  }

  let opened = false

  return {
    subscribe: store.subscribe,
    load,
    async open() {
      opened = true
      await load()
    },
    close() {
      opened = false
    },
    async refresh() {
      if (opened) await load()
    },
    async setFilter(filter: string) {
      store.update((s) => ({ ...s, filter }))
      await load()
    },
    showMore,
    setChecked(ok: boolean) {
      notChecked = !ok
      if (ok) {
        store.update((s) => ({ ...s, notChecked: false }))
        return
      }
      call += 1
      store.update((s) => ({ ...s, loading: false, list: null, error: null, notice: null, notChecked: true }))
    },
  }
}

/**
 * Reloads the open left-out list after each new result of the check: after
 * "Check again", and after a check that follows a change of the files (a
 * new catalog). A check that did not run clears the list: the list then
 * shows the same "not checked" state as the profile panel. Gives the
 * function that ends the follow.
 */
export function followChecks(
  checks: Readable<{ check: CheckResult | null; checkError: string | null }>,
  list: Pick<LeftOutStore, 'refresh' | 'setChecked'>,
): () => void {
  let last: CheckResult | null | undefined = undefined
  let failed = false
  return checks.subscribe(({ check, checkError }) => {
    const first = last === undefined
    const changed = check !== last
    last = check
    if (checkError !== null) {
      if (!failed) list.setChecked(false)
      failed = true
      return
    }
    const recovered = failed
    failed = false
    if (recovered) list.setChecked(true)
    if ((recovered || (!first && changed)) && check !== null) void list.refresh()
  })
}
