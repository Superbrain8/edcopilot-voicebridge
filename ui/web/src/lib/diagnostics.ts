// The Diagnostics screen store: the switches of the utterance log and the
// audio capture, and "Open folder".
//
// The page sends only the name of a part ("log" or "audio"). The Rust side
// picks the path, saves with the rules of `save_config`, and opens only the
// folder of the saved file. The switches save at once. They do not use the
// draft of the Setup and Microphone screens.

import { writable, type Readable } from 'svelte/store'
import * as api from './api'
import type { CheckResult, Config, DiagnosticsPart, SaveResult } from './api'

/** The text beside the switch of the audio capture. */
export const AUDIO_NOTE = 'The files hold your voice and stay on this computer.'
/**
 * The text beside a switch that is on. The configuration has only the path:
 * "off" is the empty path, so an own path does not come back.
 */
export const OFF_NOTE =
  'Off removes this path from the configuration. On again uses the default folder below the configuration file.'
/** The text beside the data folder. The uninstaller never deletes it. */
export const UNINSTALL_NOTE =
  'An uninstall keeps this folder. Delete it by hand to remove your settings, the model and the recordings.'

/** A part with a switch. The data folder has none. */
export type SwitchPart = Exclude<DiagnosticsPart, 'data'>

/** The functions of `api.ts` that the store uses. Tests give a fake. */
export interface DiagnosticsApi {
  setDiagnostics(part: DiagnosticsPart, on: boolean): Promise<SaveResult>
  openFolder(part: DiagnosticsPart): Promise<void>
}

/** The functions of the configuration store that the store uses. */
export interface DiagnosticsConfig {
  load(): Promise<void>
  runCheck(): Promise<void>
  markRestartNeeded(): void
}

export interface DiagnosticsState {
  /** The part of a call in work, or null. */
  busy: DiagnosticsPart | null
  /** The problems of a refused switch. */
  problems: string[]
  /** The error of the last call, or null. */
  error: string | null
  /**
   * The number of switch calls that ended. The screen shows the saved state
   * again after each call, also after a refusal.
   */
  calls: number
}

export interface DiagnosticsStore extends Readable<DiagnosticsState> {
  /** Switches the part on or off. Never rejects. */
  setPart(part: SwitchPart, on: boolean): Promise<void>
  /** Opens the folder of the part. Never rejects. */
  open(part: DiagnosticsPart): Promise<void>
}

/** True when the saved file has a path for the part. */
export function partOn(config: Config, part: SwitchPart): boolean {
  const value = part === 'log' ? config.diagnostics.log_file : config.diagnostics.save_audio_dir
  return value.trim() !== ''
}

/**
 * The lines of `voicebridge check` in the order of the console: the report
 * line, the lines of the profile report, the warnings, the information
 * lines, the problems, and the last line.
 */
export function checkOutput(check: CheckResult): string[] {
  const lines: string[] = []
  if (check.report) lines.push(check.report.line)
  if (check.report && check.profile) lines.push(...check.profile.lines)
  for (const warning of check.warnings) lines.push(`warning: ${warning}`)
  lines.push(...check.info)
  for (const problem of check.problems) lines.push(`problem: ${problem}`)
  lines.push(
    check.problems.length === 0 ? 'configuration is valid' : `${check.problems.length} problem(s) found`,
  )
  return lines
}

export function createDiagnosticsStore(source: DiagnosticsApi, config: DiagnosticsConfig): DiagnosticsStore {
  const store = writable<DiagnosticsState>({ busy: null, problems: [], error: null, calls: 0 })
  let busy: DiagnosticsPart | null = null

  async function setPart(part: SwitchPart, on: boolean): Promise<void> {
    if (busy !== null) return
    busy = part
    store.update((s) => ({ ...s, busy: part, problems: [], error: null }))
    try {
      const result = await source.setDiagnostics(part, on)
      if (result.restart_needed) config.markRestartNeeded()
      store.update((s) => ({ ...s, problems: result.written ? [] : result.problems }))
      await config.load()
      await config.runCheck()
    } catch (error) {
      store.update((s) => ({ ...s, error: api.toError(error).message }))
    } finally {
      busy = null
      store.update((s) => ({ ...s, busy: null, calls: s.calls + 1 }))
    }
  }

  async function open(part: DiagnosticsPart): Promise<void> {
    store.update((s) => ({ ...s, error: null }))
    try {
      await source.openFolder(part)
    } catch (error) {
      store.update((s) => ({ ...s, error: api.toError(error).message }))
    }
  }

  return { subscribe: store.subscribe, setPart, open }
}
