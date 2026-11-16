import { get, writable } from 'svelte/store'
import { describe, expect, it, vi } from 'vitest'
import type { CheckResult, Config, LeftOutList } from './api'
import { checkOutput } from './diagnostics'
import { samples } from './fixtures/dto-samples'
import {
  LEFT_OUT_PAGE_ROWS,
  createLeftOutStore,
  followChecks,
  leftOutListShown,
  profileField,
  profileView,
  type LeftOutApi,
} from './profile'

const NO_CHECK = 'The check did not run. The program shows no report of the EDCoPilot profile.'

describe('profile report', () => {
  it('the Diagnostics screen shows the profile report', () => {
    const view = profileView(samples.check.loaded, null)
    expect(view.kind).toBe('report')
    expect(view.lines).toEqual(samples.check.loaded.profile!.lines)
    expect(view.lines[0]).toBe('commands from the EDCoPilot profile: 2 commands, 3 left out')
    expect(view.lines.at(-1)).toMatch(/^the EDCoPilot profile changed: /)
    expect(view.problems).toEqual([])
  })

  it('a profile that does not load shows the problem text and no report', () => {
    const view = profileView(samples.check.no_profile, null)
    expect(view.kind).toBe('problem')
    expect(view.lines).toEqual([])
    expect(view.problems).toEqual(samples.check.no_profile.edcopilot_problems)
  })

  it('a check that did not run never shows a clean state', () => {
    expect(profileView(null, 'the command failed')).toEqual({ kind: 'not_checked', lines: [], problems: [NO_CHECK] })
    expect(profileView(null, null)).toEqual({ kind: 'waiting', lines: [], problems: [] })
  })

  it('a failed latest check never shows the report of an older check', () => {
    expect(profileView(samples.check.loaded, 'the command failed')).toEqual({
      kind: 'not_checked',
      lines: [],
      problems: [NO_CHECK],
    })
  })

  it('a check without a profile report shows the problems of the check, never a clean state', () => {
    const check: CheckResult = { ...samples.check.loaded, profile: null, problems: ['the commands did not load'] }
    expect(profileView(check, null)).toEqual({ kind: 'problem', lines: [], problems: ['the commands did not load'] })
  })

  it('the check output has the profile lines after the report line, as the console', () => {
    const lines = checkOutput(samples.check.loaded)
    expect(lines[0]).toBe(samples.check.loaded.report!.line)
    expect(lines.slice(1, 4)).toEqual(samples.check.loaded.profile!.lines)
    expect(lines[4]).toBe('warning: w')
  })
})

describe('setup field of the EDCoPilot folder', () => {
  const saved: Config = samples.config

  it('the Setup screen shows the profile problem at the EDCoPilot folder', () => {
    const field = profileField(saved, samples.check.no_profile)
    expect(field.problems).toEqual([samples.check.no_profile.edcopilot_problems[0]])
    expect(field.problems[0]).toContain('cannot read the EDCoPilot profile')
    expect(field.file).toBe('C:\\EDCoPilot\\EDCoPilot-Profile.vap')
  })

  it('shows the profile file that the program reads and no problem after a clean check', () => {
    const field = profileField(saved, samples.check.loaded)
    expect(field.problems).toEqual([])
    expect(field.file).toBe(saved.edcopilot.profile_in_use)
    expect(profileField(null, null)).toEqual({ file: null, problems: [] })
    const empty = { ...saved, edcopilot: { ...saved.edcopilot, profile_in_use: '' } }
    expect(profileField(empty, null).file).toBeNull()
  })

  it('a check that did not run shows no problem of an older check', () => {
    const field = profileField(saved, samples.check.no_profile, 'the check failed')
    expect(field.problems).toEqual([])
    expect(field.file).toBe('C:\\EDCoPilot\\EDCoPilot-Profile.vap')
    expect(profileField(saved, samples.check.no_profile, null).problems).toHaveLength(1)
  })
})

describe('the list of the left-out commands on the Diagnostics screen', () => {
  it('the list shows with a report and after a check that did not run, not with a problem', () => {
    expect(leftOutListShown('report')).toBe(true)
    expect(leftOutListShown('not_checked')).toBe(true)
    expect(leftOutListShown('problem')).toBe(false)
    expect(leftOutListShown('waiting')).toBe(false)
  })
})

/** A fake list of `n` left-out commands. */
function fakeLeftOut(n: number) {
  const calls: { filter: string; offset: number; limit: number }[] = []
  let catalogId = 1
  let fail: string | null = null
  const rows = Array.from({ length: n }, (_, i) => ({
    class: 'not_understood',
    line: `left out: "lantern ${i}" (not_understood): an invented reason`,
  }))
  const source: LeftOutApi = {
    async listLeftOut(filter, offset, limit): Promise<LeftOutList> {
      calls.push({ filter, offset, limit })
      if (fail !== null) throw new Error(fail)
      const shown = rows.filter((r) => r.line.includes(filter))
      return { rows: shown.slice(offset, offset + limit), offset, total: shown.length, catalog_id: catalogId }
    },
  }
  return {
    source,
    calls,
    setCatalog: (id: number) => (catalogId = id),
    setFail: (text: string | null) => (fail = text),
  }
}

describe('left-out list', () => {
  it('loads the first page, then more pages, until all rows show', async () => {
    const fake = fakeLeftOut(570)
    const store = createLeftOutStore(fake.source)
    await store.load()
    expect(get(store).list?.rows.length).toBe(LEFT_OUT_PAGE_ROWS)
    expect(get(store).list?.total).toBe(570)
    await store.showMore()
    await store.showMore()
    expect(get(store).list?.rows.length).toBe(570)
    expect(get(store).list?.rows[569].line).toBe('left out: "lantern 569" (not_understood): an invented reason')
    expect(fake.calls.map((c) => c.offset)).toEqual([0, 200, 400])
    // All rows show: no more call.
    await store.showMore()
    expect(fake.calls.length).toBe(3)
  })

  it('starts again at the first page when the data changed between two pages', async () => {
    const fake = fakeLeftOut(300)
    const store = createLeftOutStore(fake.source)
    await store.load()
    fake.setCatalog(2)
    await store.showMore()
    expect(get(store).list?.rows.length).toBe(LEFT_OUT_PAGE_ROWS)
    expect(get(store).list?.catalog_id).toBe(2)
    expect(get(store).notice).not.toBeNull()
  })

  it('a new check reloads the open list, and only the open list', async () => {
    const fake = fakeLeftOut(30)
    const store = createLeftOutStore(fake.source)
    const checks = writable<{ check: CheckResult | null; checkError: string | null }>({ check: null, checkError: null })
    const stop = followChecks(checks, store)
    // A closed list loads nothing.
    checks.set({ check: samples.check.loaded, checkError: null })
    await Promise.resolve()
    expect(fake.calls.length).toBe(0)
    await store.open()
    expect(fake.calls.length).toBe(1)
    // "Check again", or a check after a change of the catalog: a new result.
    fake.setCatalog(2)
    checks.set({ check: { ...samples.check.loaded }, checkError: null })
    await vi.waitFor(() => expect(get(store).list?.catalog_id).toBe(2))
    expect(fake.calls.length).toBe(2)
    // The same result object again: no load.
    checks.update((s) => ({ ...s }))
    await Promise.resolve()
    expect(fake.calls.length).toBe(2)
    store.close()
    checks.set({ check: { ...samples.check.loaded }, checkError: null })
    await Promise.resolve()
    expect(fake.calls.length).toBe(2)
    stop()
  })

  it('a failed check clears an open list and shows the not-checked state until a check runs', async () => {
    const fake = fakeLeftOut(30)
    const store = createLeftOutStore(fake.source)
    const checks = writable<{ check: CheckResult | null; checkError: string | null }>({
      check: samples.check.loaded,
      checkError: null,
    })
    const stop = followChecks(checks, store)
    await store.open()
    expect(get(store).list?.rows.length).toBeGreaterThan(0)
    const calls = fake.calls.length
    // The latest check fails: the older result stays in the config state,
    // but the list keeps no row of it.
    checks.update((s) => ({ ...s, checkError: 'the command failed' }))
    expect(get(store).list).toBeNull()
    expect(get(store).notChecked).toBe(true)
    expect(get(store).error).toBeNull()
    // No load while the check did not run: not a refresh, not a filter.
    await store.refresh()
    await store.setFilter('lantern')
    expect(fake.calls.length).toBe(calls)
    // A check that runs again loads the list of the new catalog.
    fake.setCatalog(2)
    checks.set({ check: { ...samples.check.loaded }, checkError: null })
    await vi.waitFor(() => expect(get(store).list?.catalog_id).toBe(2))
    expect(get(store).notChecked).toBe(false)
    stop()
  })

  it('an answer that was in work when the check failed is dropped', async () => {
    const fake = fakeLeftOut(30)
    const store = createLeftOutStore(fake.source)
    const checks = writable<{ check: CheckResult | null; checkError: string | null }>({
      check: samples.check.loaded,
      checkError: null,
    })
    const stop = followChecks(checks, store)
    const opening = store.open()
    checks.update((s) => ({ ...s, checkError: 'the command failed' }))
    await opening
    expect(get(store).list).toBeNull()
    expect(get(store).notChecked).toBe(true)
    stop()
  })

  it('a filter loads a new list, and an error shows the text of the program', async () => {
    const fake = fakeLeftOut(30)
    const store = createLeftOutStore(fake.source)
    await store.setFilter('lantern 2')
    expect(fake.calls.at(-1)?.filter).toBe('lantern 2')
    expect(get(store).list?.total).toBe(11)
    fake.setFail('cannot read the EDCoPilot profile D:\\ED\\EDCoPilot-Profile.vap: the file does not exist')
    await store.load()
    expect(get(store).list).toBeNull()
    expect(get(store).error).toContain('cannot read the EDCoPilot profile')
  })
})
