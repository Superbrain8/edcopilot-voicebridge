// The store of the Words screen: one draft for each word table.
//
// The Rust side does all the checks. The rules of the tables are in the
// loaders of the program, and `check_table` runs the check of
// `voicebridge check` on the rows. This store only decides when the Save
// button is on:
//
// - The check of the CURRENT rows has come back, and it has no problem.
//   Each change of the rows gets a new number. An answer for older rows
//   does not count.
// - Each warning of that check is confirmed. A confirmation belongs to the
//   number of the rows: a change after the confirmation needs a new one.
//
// The save sends the confirmed warnings and the version of the read. The
// Rust side checks the rows again and refuses a problem, a warning that is
// not confirmed, and a file that changed after the read.

import { writable, type Readable } from 'svelte/store'
import * as api from './api'
import type { TableCheck, TableKind, TableRow, TableSave, WordTable } from './api'
import { configStore } from './config'

/** The functions of `api.ts` that the store uses. Tests give a fake. */
export interface TablesApi {
  readTable(kind: TableKind, language: string | null): Promise<WordTable>
  checkTable(kind: TableKind, language: string | null, rows: TableRow[]): Promise<TableCheck>
  saveTable(
    kind: TableKind,
    language: string | null,
    rows: TableRow[],
    confirmed: string[],
    version: string,
  ): Promise<TableSave>
}

/** The time without a change before the rows go to the check. */
export const CHECK_DELAY_MS = 300
/** The two lists of the filler file. */
export const NOT_LAST = 'not_last'
export const MAY_BE_LAST = 'may_be_last'

/** What the screen shows for one table. */
export interface TableInfo {
  kind: TableKind
  label: string
  /** The heading of the first column. */
  spoken: string
  /** The heading of the second column, or null when the table has one column. */
  meant: string | null
  /** `list`: one entry on each line. `word`: one entry. `filler`: the list of the word. */
  meantForm: 'list' | 'word' | 'filler' | null
  help: string
}

export const TABLES: readonly TableInfo[] = [
  {
    kind: 'aliases',
    label: 'Aliases',
    spoken: 'Token template',
    meant: 'Extra phrases (one on each line)',
    meantForm: 'list',
    help: 'Extra spoken phrases for a command in one language. The key is the token template of the command, exactly as in the command map.',
  },
  {
    kind: 'synonyms',
    label: 'Synonyms',
    spoken: 'Your words',
    meant: 'EDCoPilot phrases (one on each line)',
    meantForm: 'list',
    help: 'Your own words for a phrase of EDCoPilot. A synonym applies only when the text as heard matches no command.',
  },
  {
    kind: 'sound_alikes',
    label: 'Sound-alikes',
    spoken: 'Heard word',
    meant: 'Meant word',
    meantForm: 'word',
    help: 'A real word that the speech engine writes for a command word. The program tries it only when the text as heard matches no command.',
  },
  {
    kind: 'fillers',
    label: 'Filler words',
    spoken: 'Word',
    meant: 'Last word',
    meantForm: 'filler',
    help: 'Small words that the program can drop when nothing else matches. At most 3 filler words in one text.',
  },
  {
    kind: 'hotwords',
    label: 'Hotwords',
    spoken: 'Phrase',
    meant: null,
    meantForm: null,
    help: 'Names that the local engine pulls toward in a second pass. A hotword must not be near another hotword, a name, or a command word.',
  },
]

/** The id of a tab: the kind, and the language for an alias file. */
export function tabId(kind: TableKind, language: string | null): string {
  return kind === 'aliases' ? `aliases:${language ?? ''}` : kind
}

export interface TabState {
  kind: TableKind
  language: string | null
  loading: boolean
  /** The error text of the read. The screen then shows no rows. */
  loadError: string | null
  /** The table as the file has it. Null before the read and after a failed read. */
  table: WordTable | null
  /** The rows of the screen. */
  rows: TableRow[]
  /** The number of the rows. Each change adds one. */
  edit: number
  /** True while a check of this tab runs. */
  checking: boolean
  /** The newest check that came back, and the number of its rows. */
  check: TableCheck | null
  checkedEdit: number
  /** The error of the check of `checkedEdit`. Then `check` is null. */
  checkError: string | null
  /** The number of the rows whose warnings the user confirmed. */
  confirmedEdit: number
  saving: boolean
  saveMessage: string | null
  saveError: string | null
  /** True after a save that the program refused because the file changed. */
  fileChanged: boolean
}

export interface TablesState {
  /** The id of the tab that shows. */
  active: string
  tabs: Record<string, TabState>
  /** The languages of the alias folder and the new languages of this window. */
  languages: string[]
}

/** The start of the text of the program when the file changed after the read. */
export const FILE_CHANGED = 'the file changed after the screen read it: read the table again, then make the change again'

function newTab(kind: TableKind, language: string | null): TabState {
  return {
    kind,
    language,
    loading: false,
    loadError: null,
    table: null,
    rows: [],
    edit: 0,
    checking: false,
    check: null,
    checkedEdit: -1,
    checkError: null,
    confirmedEdit: -1,
    saving: false,
    saveMessage: null,
    saveError: null,
    fileChanged: false,
  }
}

function sameRows(a: TableRow[], b: TableRow[]): boolean {
  return (
    a.length === b.length &&
    a.every(
      (row, i) =>
        row.spoken === b[i].spoken &&
        row.meant.length === b[i].meant.length &&
        row.meant.every((m, j) => m === b[i].meant[j]),
    )
  )
}

/**
 * The rows as they go to the program: without the spaces at the ends of
 * each entry, as the program trims them. A list field keeps each typed line
 * in the store, also an empty line, so that the field does not jump while
 * the user types. The empty lines of a list do not go to the program.
 */
export function cleanRows(kind: TableKind, rows: TableRow[]): TableRow[] {
  const list = kind === 'aliases' || kind === 'synonyms'
  return rows.map((row) => {
    const meant = row.meant.map((m) => m.trim())
    return {
      spoken: row.spoken.trim(),
      meant: list ? meant.filter((m) => m !== '') : meant,
    }
  })
}

/** True when the rows differ from the file. */
export function isDirty(tab: TabState): boolean {
  // Both sides as the program reads them: an entry with spaces at its ends
  // in the file is not a change.
  return tab.table !== null && !sameRows(cleanRows(tab.kind, tab.rows), cleanRows(tab.kind, tab.table.rows))
}

/** True when the newest answer (a check or an error) is for the current rows. */
export function checkIsCurrent(tab: TabState): boolean {
  return tab.checkedEdit === tab.edit && !tab.checking
}

/** True when the check of the current rows has warnings that are not confirmed. */
export function needsConfirmation(tab: TabState): boolean {
  return (
    checkIsCurrent(tab) &&
    tab.check !== null &&
    tab.check.problems.length === 0 &&
    tab.check.warnings.length > 0 &&
    tab.confirmedEdit !== tab.edit
  )
}

/** True when the Save button is on. */
export function canSave(tab: TabState): boolean {
  return (
    tab.table !== null &&
    !tab.loading &&
    !tab.saving &&
    isDirty(tab) &&
    checkIsCurrent(tab) &&
    tab.checkError === null &&
    tab.check !== null &&
    tab.check.problems.length === 0 &&
    (tab.check.warnings.length === 0 || tab.confirmedEdit === tab.edit)
  )
}

/** The entries of a list field: one on each line. `cleanRows` drops the empty lines. */
export function meantFromText(text: string): string[] {
  return text.split('\n').map((line) => line.replace(/\r$/, ''))
}

/** The text of a list field. */
export function meantText(row: TableRow): string {
  return row.meant.join('\n')
}

/** The first value of a row for a new row of the table. */
function emptyRow(kind: TableKind): TableRow {
  if (kind === 'fillers') return { spoken: '', meant: [MAY_BE_LAST] }
  if (kind === 'sound_alikes') return { spoken: '', meant: [''] }
  return { spoken: '', meant: [] }
}

export interface TablesOptions {
  /** Called after each save that wrote the file. */
  onSaved?: (result: TableSave) => void
}

export interface TablesStore extends Readable<TablesState> {
  /** Shows the tab of the table. Reads the file when the tab has no rows yet. */
  open(kind: TableKind, language: string | null): Promise<void>
  /** Adds a language to the list and shows its alias table. The file comes at the save. */
  newLanguage(name: string): Promise<void>
  setSpoken(id: string, index: number, value: string): void
  setMeant(id: string, index: number, meant: string[]): void
  addRow(id: string): void
  removeRow(id: string, index: number): void
  /** Confirms the warnings of the check of the current rows. */
  confirm(id: string): void
  /** Saves the rows when the Save button is on. Never rejects. */
  save(id: string): Promise<void>
  /** Reads the file again. The changes of the tab go away. */
  discard(id: string): Promise<void>
  /**
   * After a save that the program refused because the file changed: reads
   * the file again and keeps the rows of the screen. The check runs again.
   */
  readAgainKeepRows(id: string): Promise<void>
  /** The ids of the tabs with changes that are not saved. */
  changedTabs(): string[]
  /** Stops the checks that wait. */
  destroy(): void
}

export function createTablesStore(source: TablesApi, options: TablesOptions = {}): TablesStore {
  const store = writable<TablesState>({ active: 'synonyms', tabs: {}, languages: [] })
  let state: TablesState = { active: 'synonyms', tabs: {}, languages: [] }
  store.subscribe((s) => (state = s))
  const timers = new Map<string, ReturnType<typeof setTimeout>>()
  // The tabs with a check in work. One check at a time for each tab: the
  // program runs the full check for each call.
  const inFlight = new Set<string>()
  // The tabs whose rows changed while their check ran.
  const pending = new Set<string>()
  // The number of the newest read of each tab. An older read is dropped.
  const reads = new Map<string, number>()
  // False after `destroy`: a check that ends then starts no other check.
  let active = true

  function updateTab(id: string, change: (tab: TabState) => TabState): void {
    store.update((s) => {
      const tab = s.tabs[id]
      if (!tab) return s
      return { ...s, tabs: { ...s.tabs, [id]: change(tab) } }
    })
  }

  function addLanguages(list: string[]): void {
    store.update((s) => {
      const languages = [...s.languages]
      for (const language of list) if (!languages.includes(language)) languages.push(language)
      languages.sort()
      return { ...s, languages }
    })
  }

  async function runCheck(id: string): Promise<void> {
    timers.delete(id)
    const tab = state.tabs[id]
    if (!tab || tab.table === null) return
    if (inFlight.has(id)) {
      // The check in work ends first. Then the newest rows go.
      pending.add(id)
      return
    }
    inFlight.add(id)
    pending.delete(id)
    const mine = tab.edit
    const rows = cleanRows(tab.kind, tab.rows)
    updateTab(id, (t) => ({ ...t, checking: true }))
    try {
      const check = await source.checkTable(tab.kind, tab.language, rows)
      if (state.tabs[id]?.edit === mine) {
        updateTab(id, (t) => ({ ...t, check, checkedEdit: mine, checkError: null }))
      }
    } catch (error) {
      if (state.tabs[id]?.edit === mine) {
        // The error is the answer for these rows. Save stays off.
        updateTab(id, (t) => ({ ...t, check: null, checkedEdit: mine, checkError: api.toError(error).message }))
      }
    } finally {
      inFlight.delete(id)
      updateTab(id, (t) => ({ ...t, checking: false }))
      const now = state.tabs[id]
      if (!active) {
        pending.delete(id)
      } else if (pending.has(id) || (now && now.table !== null && now.checkedEdit !== now.edit && !timers.has(id))) {
        pending.delete(id)
        void runCheck(id)
      }
    }
  }

  function scheduleCheck(id: string, delay = CHECK_DELAY_MS): void {
    const old = timers.get(id)
    if (old !== undefined) clearTimeout(old)
    timers.set(
      id,
      setTimeout(() => void runCheck(id), delay),
    )
  }

  /** Changes the rows: a new number, and a new check after a pause. */
  function changeRows(id: string, change: (rows: TableRow[]) => TableRow[]): void {
    const tab = state.tabs[id]
    if (!tab || tab.table === null || tab.saving) return
    updateTab(id, (t) => ({
      ...t,
      rows: change(structuredClone(t.rows)),
      edit: t.edit + 1,
      saveMessage: null,
      saveError: null,
    }))
    scheduleCheck(id)
  }

  /**
   * The id of the tab for a table that the program read. An alias file can
   * have another case than the request ("DE" for `de.toml`): the tab of the
   * file is then the tab.
   */
  function resolvedId(id: string, kind: TableKind, table: WordTable): string {
    if (kind !== 'aliases' || table.language === null) return id
    const real = tabId(kind, table.language)
    if (real === id) return id
    store.update((s) => {
      const tabs = { ...s.tabs }
      const moved = tabs[id]
      delete tabs[id]
      if (!tabs[real] && moved) tabs[real] = { ...moved, language: table.language }
      return { ...s, tabs, active: s.active === id ? real : s.active }
    })
    return real
  }

  async function read(id: string, kind: TableKind, language: string | null): Promise<void> {
    const mine = (reads.get(id) ?? 0) + 1
    reads.set(id, mine)
    updateTab(id, (t) => ({ ...t, loading: true }))
    try {
      const table = await source.readTable(kind, language)
      if (reads.get(id) !== mine) return
      const real = resolvedId(id, kind, table)
      // A new language joins the list only after the program accepted its name.
      addLanguages(kind === 'aliases' && table.language !== null ? [...table.languages, table.language] : table.languages)
      if (real !== id && state.tabs[real]?.table) return
      updateTab(real, (t) => ({
        ...t,
        loading: false,
        loadError: null,
        table,
        rows: structuredClone(table.rows),
        edit: t.edit + 1,
        fileChanged: false,
      }))
      scheduleCheck(real, 0)
    } catch (error) {
      if (reads.get(id) !== mine) return
      updateTab(id, (t) => ({
        ...t,
        loading: false,
        loadError: api.toError(error).message,
        table: null,
        rows: [],
        edit: t.edit + 1,
      }))
    }
  }

  async function open(kind: TableKind, language: string | null): Promise<void> {
    active = true
    // A language in another case of a known tab uses that tab.
    const knownId =
      kind === 'aliases' && language !== null
        ? (Object.keys(state.tabs).find(
            (key) => key.toLowerCase() === tabId(kind, language).toLowerCase() && state.tabs[key].table !== null,
          ) ?? tabId(kind, language))
        : tabId(kind, language)
    const known = state.tabs[knownId]
    store.update((s) => ({
      ...s,
      active: knownId,
      tabs: known ? s.tabs : { ...s.tabs, [knownId]: newTab(kind, language) },
    }))
    if (!known || (known.table === null && !known.loading)) {
      await read(knownId, kind, language)
    } else if (known.table !== null && !known.saving && !checkIsCurrent(known)) {
      // The screen opens again after `destroy`: the check of the draft runs again.
      scheduleCheck(knownId, 0)
    }
  }

  async function save(id: string): Promise<void> {
    const tab = state.tabs[id]
    if (!tab || !canSave(tab) || tab.table === null || tab.check === null) return
    const mine = tab.edit
    const rows = cleanRows(tab.kind, tab.rows)
    // The confirmation belongs to the warnings of the check of these rows.
    const confirmed = [...tab.check.warnings]
    updateTab(id, (t) => ({ ...t, saving: true, saveMessage: null, saveError: null }))
    let result: TableSave
    try {
      result = await source.saveTable(tab.kind, tab.language, rows, confirmed, tab.table.version)
    } catch (error) {
      updateTab(id, (t) => ({ ...t, saving: false, saveError: api.toError(error).message }))
      return
    }
    if (!result.written) {
      const changed = result.problems.some((p) => p.startsWith(FILE_CHANGED))
      updateTab(id, (t) => {
        const same = t.edit === mine
        return {
          ...t,
          saving: false,
          fileChanged: changed,
          // The check of the save is the newest check of these rows.
          check: same ? { problems: result.problems, warnings: result.warnings } : t.check,
          checkedEdit: same ? mine : t.checkedEdit,
          checkError: same ? null : t.checkError,
          confirmedEdit: same && result.unconfirmed.length > 0 ? -1 : t.confirmedEdit,
          saveError:
            result.problems.length > 0 || result.unconfirmed.length > 0
              ? 'Not saved. See the problems and the warnings below.'
              : null,
          saveMessage: result.problems.length === 0 && result.unconfirmed.length === 0 ? 'No change to save.' : null,
        }
      })
      return
    }
    options.onSaved?.(result)
    try {
      const table = await source.readTable(tab.kind, tab.language)
      addLanguages(table.languages)
      updateTab(id, (t) => {
        // A change during the save stays as a change.
        const same = t.edit === mine
        return {
          ...t,
          saving: false,
          table,
          rows: same ? structuredClone(table.rows) : t.rows,
          edit: t.edit + 1,
          fileChanged: false,
          saveMessage: result.restart_needed
            ? 'Saved. The running session keeps the old table until you stop and start it again.'
            : 'Saved.',
        }
      })
      scheduleCheck(id, 0)
    } catch (error) {
      updateTab(id, (t) => ({
        ...t,
        saving: false,
        saveMessage: 'Saved.',
        saveError: `The table could not be read again: ${api.toError(error).message}`,
      }))
    }
  }

  async function readAgainKeepRows(id: string): Promise<void> {
    const tab = state.tabs[id]
    if (!tab || tab.saving) return
    const mine = (reads.get(id) ?? 0) + 1
    reads.set(id, mine)
    try {
      const table = await source.readTable(tab.kind, tab.language)
      // A newer read (a discard) wins.
      if (reads.get(id) !== mine) return
      updateTab(id, (t) => ({
        ...t,
        table,
        edit: t.edit + 1,
        fileChanged: false,
        saveError: null,
        saveMessage:
          'The file is read again. Your rows stay. A save now replaces what the other program wrote. Check the result, then save again.',
      }))
      scheduleCheck(id, 0)
    } catch (error) {
      if (reads.get(id) !== mine) return
      updateTab(id, (t) => ({ ...t, saveError: `The table could not be read again: ${api.toError(error).message}` }))
    }
  }

  return {
    subscribe: store.subscribe,
    open,
    async newLanguage(name: string) {
      const language = name.trim()
      if (language === '') return
      await open('aliases', language)
    },
    setSpoken(id, index, value) {
      changeRows(id, (rows) => {
        if (rows[index]) rows[index].spoken = value
        return rows
      })
    },
    setMeant(id, index, meant) {
      changeRows(id, (rows) => {
        if (rows[index]) rows[index].meant = [...meant]
        return rows
      })
    },
    addRow(id) {
      const tab = state.tabs[id]
      if (!tab) return
      changeRows(id, (rows) => [...rows, emptyRow(tab.kind)])
    },
    removeRow(id, index) {
      changeRows(id, (rows) => rows.filter((_, i) => i !== index))
    },
    confirm(id) {
      const tab = state.tabs[id]
      if (!tab || !needsConfirmation(tab)) return
      updateTab(id, (t) => ({ ...t, confirmedEdit: t.edit }))
    },
    save,
    async discard(id) {
      const tab = state.tabs[id]
      if (!tab || tab.saving) return
      updateTab(id, (t) => ({ ...t, saveMessage: null, saveError: null }))
      await read(id, tab.kind, tab.language)
    },
    readAgainKeepRows,
    changedTabs() {
      return Object.keys(state.tabs).filter((id) => isDirty(state.tabs[id]))
    },
    destroy() {
      active = false
      for (const timer of timers.values()) clearTimeout(timer)
      timers.clear()
      pending.clear()
    },
  }
}

/**
 * The store of the window, with the real commands. After a save, the
 * configuration store shows the note about the restart while a session
 * runs, and the check of the Status screen runs again.
 */
export const tablesStore: TablesStore = createTablesStore(api, {
  onSaved: (result) => {
    if (result.restart_needed) configStore.markRestartNeeded()
    void configStore.runCheck()
  },
})
