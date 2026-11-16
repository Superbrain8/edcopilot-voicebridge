// The screens of the window and the screen that shows now. Six screens do
// not need a router: a store holds the id of the screen.

import { writable } from 'svelte/store'
import type { CheckResult } from './api'

export type ScreenId = 'status' | 'setup' | 'microphone' | 'commands' | 'words' | 'diagnostics'

export interface Screen {
  id: ScreenId
  label: string
}

export const SCREENS: readonly Screen[] = [
  { id: 'status', label: 'Status' },
  { id: 'setup', label: 'Setup' },
  { id: 'microphone', label: 'Microphone' },
  { id: 'commands', label: 'Commands' },
  { id: 'words', label: 'Words' },
  { id: 'diagnostics', label: 'Diagnostics' },
]

export const currentScreen = writable<ScreenId>('status')

/**
 * The screen for "Start..." of the tray menu: Setup when the check has a
 * problem or did not run (`checkError`), else Status. Setup has the
 * "Check again" button. The tray never starts a session itself.
 */
export function screenForStart(check: CheckResult | null, checkError: string | null = null): ScreenId {
  if (checkError !== null) return 'setup'
  return check !== null && check.problems.length > 0 ? 'setup' : 'status'
}

/** The functions of `api.ts` for "Start..." of the tray menu. */
export interface TrayStartApi {
  takeTrayStart(): Promise<boolean>
  onTrayStart(handler: () => void): Promise<() => void>
}

/**
 * Listens to "Start..." of the tray menu. The program keeps a mark until
 * the page takes it, so a click before the page listened is not lost. The
 * page takes the mark when it loads and at each event: `act` runs one time
 * for each click. The result removes the listener.
 */
export async function listenTrayStart(source: TrayStartApi, act: () => void): Promise<() => void> {
  const take = () =>
    source
      .takeTrayStart()
      .then((pending) => {
        if (pending) act()
      })
      .catch(() => {})
  const remove = await source.onTrayStart(() => void take())
  await take()
  return remove
}
