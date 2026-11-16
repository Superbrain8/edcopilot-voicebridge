// The texts of the start: a problem that stops the use of a configuration,
// the error of a failed first start, and the notes and the problems of the
// update step. The Status and Diagnostics screens show them as plain text.

import { writable } from 'svelte/store'
import * as api from './api'
import type { StartupNotes } from './api'

export interface StartupLine {
  kind: 'problem' | 'note'
  text: string
}

/** The lines to show: the problems first, then the notes. */
export function startupLines(notes: StartupNotes): StartupLine[] {
  const problems = [notes.problem, notes.first_start_error, ...notes.update_problems]
    .filter((text): text is string => text !== null && text !== '')
    .map((text) => ({ kind: 'problem' as const, text }))
  const updates = notes.notes.map((text) => ({ kind: 'note' as const, text }))
  return [...problems, ...updates]
}

/** The notes of this start. Null before the first read. */
export const startupStore = writable<StartupNotes | null>(null)

/** Reads the notes one time. A failed read leaves the store empty. */
export async function loadStartupNotes(source: Pick<typeof api, 'startupNotes'> = api): Promise<void> {
  try {
    startupStore.set(await source.startupNotes())
  } catch {
    startupStore.set(null)
  }
}
