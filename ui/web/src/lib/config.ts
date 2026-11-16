// The configuration store: the saved settings, the draft of the Setup and
// Microphone screens, and the result of `check`. The Words and Diagnostics
// screens use the same store. The draft is one draft for all screens: a
// save writes the changes of each screen.
//
// The store also holds the device list and the capture of the talk button.
// A capture changes only the draft. The save writes the binding with the
// edits of `voicebridge bind`.
//
// The draft stays in the store when the user goes to another screen. A save
// sends only the keys that changed, in one call of `save_config`. The Rust
// side is the authority: the store always shows what `save_config` returns.
//
// The API key is never part of the state. `saveApiKey` gives the key to
// `set_api_key` and keeps no copy.

import { writable, type Readable } from 'svelte/store'
import * as api from './api'
import type {
  Binding,
  CheckResult,
  Config,
  ConfigEdit,
  ConfigValue,
  KeyResult,
  PushToTalkConfig,
  SaveResult,
} from './api'
import { edit, integerValue, numberValue } from './config-values'
import { sessionStore } from './session'

/** The largest value of `stt.openai_compatible.timeout_seconds`. */
export const MAX_TIMEOUT_SECONDS = 600
/** The number of logical processors when the browser does not give it. */
export const FALLBACK_CORES = 64
/** The longest variable name that the Rust side accepts. */
export const MAX_VARIABLE_NAME = 64
/** The fixed text for a variable name that is refused. It never repeats the value. */
export const NAME_PROBLEM =
  'Enter only the name of an environment variable (letters, digits and _), not the key itself.'

/** Why the key field is off when the address of the file holds a secret part. */
export const BASE_URL_BLOCK = 'The server address in the file holds a secret or a query. Correct the file first'

/**
 * The note for the server address when the file holds a user name, a
 * password, a query or a fragment. The field shows the address without them.
 */
export const BASE_URL_NOTE =
  'The server address in the file holds a secret or a query (a user name, a password, a part after "?" or "#"). ' +
  'This window does not show these parts and does not save a key for this address. Correct the file.'

/** The note after a key change while a cloud session runs or starts. */
export const KEY_RESTART = 'The running session keeps the old key until you stop and start it.'

/** The text of the program when another program changed a key of the save. */
export const CONFIG_CHANGED =
  'the configuration file changed, read it again: another program changed a setting that this save changes too'

/** The functions of `api.ts` that the store uses. Tests give a fake. */
export interface ConfigApi {
  getConfig(): Promise<Config>
  saveConfig(edits: ConfigEdit[], version: string): Promise<SaveResult>
  check(): Promise<CheckResult>
  setApiKey(key: string, expectedOrigin: string, expectedName: string): Promise<KeyResult>
  clearApiKey(): Promise<KeyResult>
}

/** The functions of `api.ts` for the Microphone screen. Tests give a fake. */
export interface MicrophoneApi {
  listDevices(): Promise<string[]>
  captureBinding(token: number): Promise<Binding>
  cancelCapture(token?: number): Promise<void>
}

type Engine = 'parakeet' | 'openai_compatible'
type Mode = 'push_to_talk' | 'always_listening'
/** The screens that edit the configuration. */
export type ConfigScreen = 'setup' | 'microphone'

const ENGINE_LABEL: Record<Engine, string> = {
  parakeet: 'Local engine',
  openai_compatible: 'Cloud engine',
}

const MODE_LABEL: Record<Mode, string> = {
  push_to_talk: 'Push-to-talk',
  always_listening: 'Always listening',
}

const SCREEN_LABEL: Record<ConfigScreen, string> = {
  setup: 'Setup',
  microphone: 'Microphone',
}

/** The largest value of `pre_roll_ms` and `release_tail_ms` (`MAX_PTT_EXTRA_MS`). */
export const MAX_PTT_EXTRA_MS = 2000
/** The range of the voice threshold that the page accepts, in dBFS. */
export const MIN_VAD_DBFS = -100
export const MAX_VAD_DBFS = 0
/** The limits of a list and of a text in a configuration edit. */
export const MAX_WAKE_WORDS = 64
export const MAX_WAKE_WORD_CHARS = 256
export const MAX_TEXT_CHARS = 4096
/** After the end of a capture, a click on the capture button is ignored for this time. */
export const CAPTURE_REST_MS = 500
/** The time between two cancel calls while a capture does not end. */
export const CANCEL_RETRY_MS = 100
/** The largest number of cancel calls for one capture. */
export const MAX_CANCEL_CALLS = 50

/**
 * The value kind, the label, the screen, and the engine or the mode of each
 * key that a screen can edit. A field with an engine or a mode shows only
 * when that engine or mode is selected.
 */
const FIELDS = {
  'edcopilot.dir': { kind: 'text', label: 'EDCoPilot folder', screen: 'setup' },
  'stt.engine': { kind: 'text', label: 'Engine', screen: 'setup' },
  'stt.parakeet.model_dir': { kind: 'text', label: 'Model folder', screen: 'setup', engine: 'parakeet' },
  'stt.parakeet.threads': { kind: 'integer', label: 'Threads', screen: 'setup', engine: 'parakeet' },
  'stt.parakeet.hotwords_file': { kind: 'text', label: 'Hotwords', screen: 'setup', engine: 'parakeet' },
  'stt.openai_compatible.base_url': {
    kind: 'text',
    label: 'Server address',
    screen: 'setup',
    engine: 'openai_compatible',
  },
  'stt.openai_compatible.api_key_env': {
    kind: 'text',
    label: 'Variable name',
    screen: 'setup',
    engine: 'openai_compatible',
  },
  'stt.openai_compatible.model': { kind: 'text', label: 'Model', screen: 'setup', engine: 'openai_compatible' },
  'stt.openai_compatible.language': {
    kind: 'text',
    label: 'Language',
    screen: 'setup',
    engine: 'openai_compatible',
  },
  'stt.openai_compatible.timeout_seconds': {
    kind: 'integer',
    label: 'Time limit',
    screen: 'setup',
    engine: 'openai_compatible',
  },
  'matching.threshold': { kind: 'number', label: 'Threshold', screen: 'setup' },
  'matching.exact_single_word': { kind: 'flag', label: 'One-word phrases must be exact', screen: 'setup' },
  'updates.check': { kind: 'flag', label: 'Check for updates at the start and one time each day', screen: 'setup' },
  'microphone.device': { kind: 'text', label: 'Input device', screen: 'microphone' },
  'microphone.mode': { kind: 'text', label: 'Mode', screen: 'microphone' },
  'push_to_talk.pre_roll_ms': { kind: 'integer', label: 'Pre-roll', screen: 'microphone', mode: 'push_to_talk' },
  'push_to_talk.release_tail_ms': {
    kind: 'integer',
    label: 'Release tail',
    screen: 'microphone',
    mode: 'push_to_talk',
  },
  'always_listening.wake_words': {
    kind: 'text_list',
    label: 'Wake words',
    screen: 'microphone',
    mode: 'always_listening',
  },
  'microphone.vad_threshold_dbfs': {
    kind: 'number',
    label: 'Voice threshold',
    screen: 'microphone',
    mode: 'always_listening',
  },
} as const satisfies Record<
  string,
  { kind: string; label: string; screen: ConfigScreen; engine?: Engine; mode?: Mode }
>

export type FieldKey = keyof typeof FIELDS
type FieldKind = (typeof FIELDS)[FieldKey]['kind']
type FieldValue = string | number | boolean | string[]

export const FIELD_KEYS = Object.keys(FIELDS) as FieldKey[]

function engineOfField(key: FieldKey): Engine | undefined {
  const field = FIELDS[key]
  return 'engine' in field ? field.engine : undefined
}

function modeOfField(key: FieldKey): Mode | undefined {
  const field = FIELDS[key]
  return 'mode' in field ? field.mode : undefined
}

/** What the last save did. Null before the first save and after a discard. */
export type SaveOutcome = 'saved' | 'refused' | 'unchanged' | null

export interface ConfigState {
  /** True while `get_config` runs. */
  loading: boolean
  /** The error text of `get_config`. The screen then shows no form. */
  loadError: string | null
  /**
   * The error text when "Check again" could not read the file again. The
   * saved settings and the draft stay, and the form stays.
   */
  readError: string | null
  /** The settings as the file has them. Null before the load or after a failed load. */
  saved: Config | null
  /** The settings of the screen. Null when `saved` is null. */
  draft: Config | null
  /** True when the draft differs from the saved settings. */
  dirty: boolean
  /** True from the click on Save until the file is read again and checked. */
  saving: boolean
  saveOutcome: SaveOutcome
  /**
   * The problems of the last save. With `saveOutcome` "refused" they refused
   * the save. With "saved" they are older problems that are still to fix.
   */
  problems: string[]
  /** The text of a save that could not run, or of a page error. */
  saveError: string | null
  /**
   * True after a save that the program refused because another program
   * changed a key of the save. The screen offers `readAgainKeepingChanges`.
   */
  fileChanged: boolean
  /** The file was written while a session runs, and that session still runs. */
  restartNeeded: boolean
  /** True after the page dropped a refused variable name from the draft. */
  nameDropped: boolean
  /** True while `check` runs. */
  checking: boolean
  /** The last result of `check`. Null before the first result. */
  check: CheckResult | null
  /** The error text when `check` could not run. */
  checkError: string | null
  /** True while a key call runs. */
  keyBusy: boolean
  /** The result of the last key call. Never holds the key. */
  keyMessage: string | null
  keyError: string | null
  /** The input devices. Null before the first list. */
  devices: string[] | null
  /** The error text when the list of the devices failed. */
  devicesError: string | null
  /** True while a capture of the talk button runs. */
  capturing: boolean
  /** The result of the last capture that is not an error, for example a cancel. */
  captureMessage: string | null
  /** The error text of the last capture. */
  captureError: string | null
}

export interface ConfigStore extends Readable<ConfigState> {
  /** Reads the file, then runs the check. The window calls it at its start. */
  init(): Promise<void>
  /** Reads the file. The draft keeps the keys that the user changed. */
  load(): Promise<void>
  /**
   * After a save that was refused because the file changed: reads the file
   * again (the saved values and the version) and keeps the changes of the
   * draft. Then a save sends the new version.
   */
  readAgainKeepingChanges(): Promise<void>
  /** Runs `check`. */
  runCheck(): Promise<void>
  /**
   * Reads the file again, then runs `check`: the values that the screens
   * show (for example the profile file in use) are the values of the
   * check. A draft without changes takes the new file. A draft with
   * changes stays when the file has another version: `fileChanged` lets
   * the user choose (`readAgainKeepingChanges` or a discard). A read that
   * fails keeps the saved settings and the draft, sets `readError`, and
   * the check still runs. Does nothing while a save runs.
   */
  checkAgain(): Promise<void>
  /** Changes one value of the draft. */
  set(key: FieldKey, value: FieldValue): void
  /** On: the list that the file names, or the shipped list. Off: empty. */
  setHotwords(on: boolean): void
  /** Puts the saved variable name back when the page refuses the draft name. */
  dropInvalidName(): void
  /** One edit for each changed key. Throws when a value cannot be built. */
  edits(): ConfigEdit[]
  /**
   * Sends the edits in one call. Never rejects. `from` is the screen of the
   * Save button: a refusal names the fields that this screen does not show.
   */
  save(from?: ConfigScreen): Promise<void>
  /** Reads the input devices. */
  loadDevices(): Promise<void>
  /**
   * Waits for one press of a key or a joystick button. The draft gets the
   * binding. A cancel or an error changes nothing in the draft.
   */
  captureBinding(): Promise<void>
  /** Ends the capture in work. The draft does not change. */
  cancelCapture(): Promise<void>
  /** The draft gets the saved values again. */
  discard(): void
  /** Gives the key to `set_api_key`. True when the key store has it. */
  saveApiKey(key: string): Promise<boolean>
  /** Removes the key with `clear_api_key`. */
  removeApiKey(): Promise<void>
  /**
   * After a model download: reads the file again, puts the model folder in
   * the draft, and runs the check. When the Rust side could not save the
   * folder, the draft has it as an unsaved change. `restartNeeded`: a
   * running session keeps its old model, and the screen says so.
   */
  useModelFolder(dir: string, restartNeeded?: boolean): Promise<void>
  /**
   * Another file of the program (a word table) was written while a session
   * runs. That session keeps its old data, and the screens say so.
   */
  markRestartNeeded(): void
}

export function initialConfigState(): ConfigState {
  return {
    loading: false,
    loadError: null,
    readError: null,
    saved: null,
    draft: null,
    dirty: false,
    saving: false,
    saveOutcome: null,
    problems: [],
    saveError: null,
    fileChanged: false,
    restartNeeded: false,
    nameDropped: false,
    checking: false,
    check: null,
    checkError: null,
    keyBusy: false,
    keyMessage: null,
    keyError: null,
    devices: null,
    devicesError: null,
    capturing: false,
    captureMessage: null,
    captureError: null,
  }
}

/** The value of `key` in `config`. */
export function valueOf(config: Config, key: FieldKey): FieldValue {
  let node: unknown = config
  for (const part of key.split('.')) node = (node as Record<string, unknown>)[part]
  return node as FieldValue
}

function withValue(config: Config, key: FieldKey, value: FieldValue): Config {
  const copy = structuredClone(config)
  const parts = key.split('.')
  let node = copy as unknown as Record<string, unknown>
  for (const part of parts.slice(0, -1)) node = node[part] as Record<string, unknown>
  node[parts[parts.length - 1]] = value
  return copy
}

/** The entries of a list as they go to the file: without spaces at their ends. */
function listOf(value: FieldValue): string[] {
  return Array.isArray(value) ? value.map((entry) => entry.trim()) : []
}

/** The value as it goes to the file: a text without spaces at its ends. */
function normal(key: FieldKey, value: FieldValue): FieldValue {
  const kind = FIELDS[key].kind
  if (kind === 'text' && typeof value === 'string') return value.trim()
  if (kind === 'text_list') return listOf(value)
  return value
}

function same(key: FieldKey, a: FieldValue, b: FieldValue): boolean {
  const x = normal(key, a)
  const y = normal(key, b)
  if (Array.isArray(x) && Array.isArray(y)) {
    return x.length === y.length && x.every((entry, i) => entry === y[i])
  }
  return Object.is(x, y) || x === y
}

/**
 * True when two talk buttons are the same. The texts are compared as they
 * are: `voicebridge bind` writes them without a change too.
 */
function sameBinding(a: PushToTalkConfig, b: PushToTalkConfig): boolean {
  return (
    a.key === b.key &&
    a.joystick?.device === b.joystick?.device &&
    a.joystick?.button === b.joystick?.button
  )
}

/** True when the draft has another talk button than the file. */
export function bindingChanged(s: Pick<ConfigState, 'saved' | 'draft'>): boolean {
  const { saved, draft } = s
  if (!saved || !draft) return false
  return !sameBinding(saved.push_to_talk, draft.push_to_talk)
}

/** The keys where the draft differs from the saved settings. */
export function changedKeys(s: Pick<ConfigState, 'saved' | 'draft'>): FieldKey[] {
  const { saved, draft } = s
  if (!saved || !draft) return []
  return FIELD_KEYS.filter((key) => !same(key, valueOf(saved, key), valueOf(draft, key)))
}

/** The screens that have unsaved changes, in the order of the screens. */
export function changedScreens(s: Pick<ConfigState, 'saved' | 'draft'>): ConfigScreen[] {
  const screens = new Set<ConfigScreen>(changedKeys(s).map((key) => FIELDS[key].screen))
  if (bindingChanged(s)) screens.add('microphone')
  return (['setup', 'microphone'] as const).filter((screen) => screens.has(screen))
}

/**
 * `fresh` with the values of `current` where `current` differs from `base`.
 * Thus a value that the user changed after `base` survives a reload.
 */
function merge(base: Config, current: Config, fresh: Config): Config {
  let out = structuredClone(fresh)
  for (const key of FIELD_KEYS) {
    if (!same(key, valueOf(base, key), valueOf(current, key))) {
      out = withValue(out, key, valueOf(current, key))
    }
  }
  if (!sameBinding(base.push_to_talk, current.push_to_talk)) {
    out.push_to_talk.key = current.push_to_talk.key
    out.push_to_talk.joystick = structuredClone(current.push_to_talk.joystick)
  }
  return out
}

function valueFor(kind: FieldKind, value: FieldValue): ConfigValue {
  switch (kind) {
    case 'text':
      return { kind: 'text', value: String(value).trim() }
    case 'integer':
      return integerValue(Number(value))
    case 'number':
      return numberValue(Number(value))
    case 'flag':
      return { kind: 'flag', value: Boolean(value) }
    case 'text_list':
      return { kind: 'text_list', value: listOf(value) }
  }
}

/**
 * The edits of a talk button, as `voicebridge bind` makes them: a key
 * removes the joystick, a joystick removes the key.
 */
function bindingEdits(ptt: PushToTalkConfig): ConfigEdit[] {
  if (ptt.joystick) {
    return [
      edit('push_to_talk.key', { kind: 'absent' }),
      edit('push_to_talk.joystick.device', { kind: 'text', value: ptt.joystick.device }),
      edit('push_to_talk.joystick.button', integerValue(ptt.joystick.button)),
    ]
  }
  if (ptt.key !== null) {
    return [
      edit('push_to_talk.joystick', { kind: 'absent' }),
      edit('push_to_talk.key', { kind: 'text', value: ptt.key }),
    ]
  }
  // Only a file without a binding has this state, and a capture never makes it.
  return []
}

/** The talk button in words: "F13", "<", "Mouse4", or "MOZA button 5". */
export function bindingLabel(ptt: PushToTalkConfig): string {
  if (ptt.joystick) return `${ptt.joystick.device} button ${ptt.joystick.button}`
  if (ptt.key !== null) return ptt.key
  return 'F13 (default)'
}

/** The selected mode, or null for a value that the program does not know. */
export function modeOf(draft: Config | null): Mode | null {
  const mode = draft?.microphone.mode
  return mode === 'push_to_talk' || mode === 'always_listening' ? mode : null
}

/** The warning text of always-listening without a wake word. */
export const NO_WAKE_WORD_WARNING =
  'Always-listening without a wake word sends each match: each phrase that the microphone hears ' +
  'and that matches a command fires that command, also a phrase that you say to another person. ' +
  'A wrong match is worse than a miss. Add a wake word, or use push-to-talk.'

/** The warning when the draft listens always and has no wake word, else null. */
export function wakeWordWarning(draft: Config | null): string | null {
  if (!draft || modeOf(draft) !== 'always_listening') return null
  return listOf(draft.always_listening.wake_words).length === 0 ? NO_WAKE_WORD_WARNING : null
}

/**
 * The same warning for the saved file. The Status screen shows it, because
 * a session uses the file and not the draft.
 */
export function savedWakeWordWarning(s: Pick<ConfigState, 'saved'>): string | null {
  return wakeWordWarning(s.saved)
}

/**
 * The problem of one wake word, or null. The program compares whole words
 * and ignores case and punctuation, so a word without a letter or a digit
 * never matches.
 */
function wakeWordEntryProblem(word: string): string | null {
  const problem = wakeWordFileProblem(word)
  if (problem) return problem
  if (!hasWordCharacter(word)) return 'A wake word needs a letter or a digit.'
  return null
}

/** True when the text has a letter or a digit. */
function hasWordCharacter(word: string): boolean {
  return /[\p{L}\p{N}]/u.test(word)
}

/** The problem of a wake word that the configuration edit refuses, or null. */
function wakeWordFileProblem(word: string): string | null {
  if ([...word].length > MAX_WAKE_WORD_CHARS) {
    return `A wake word can have at most ${MAX_WAKE_WORD_CHARS} characters.`
  }
  if (/\p{Cc}/u.test(word)) return 'A wake word cannot have a tab or a line break.'
  return null
}

/** The problem of a wake word that the user wants to add to `list`, or null. */
export function wakeWordProblem(word: string, list: string[]): string | null {
  const text = word.trim()
  if (text === '') return 'Enter a wake word first.'
  const entry = wakeWordEntryProblem(text)
  if (entry) return entry
  const lower = text.toLowerCase()
  if (list.some((w) => w.trim().toLowerCase() === lower)) return 'This wake word is already in the list.'
  if (list.length >= MAX_WAKE_WORDS) return `The list can have at most ${MAX_WAKE_WORDS} wake words.`
  return null
}

/**
 * How a device name of the file relates to the connected devices, with the
 * rule of the audio part: the same name without regard to case, then the
 * first name that contains the text. An empty name is the default device.
 */
export type DeviceStatus =
  | { kind: 'default' }
  | { kind: 'listed' }
  | { kind: 'matched'; device: string }
  | { kind: 'absent' }

export function deviceStatus(name: string, devices: string[]): DeviceStatus {
  const wanted = name.trim().toLowerCase()
  if (wanted === '') return { kind: 'default' }
  // The order of `select_device_name`: the same name over the whole list
  // first, then the first name that contains the text.
  const lower = devices.map((d) => d.trim().toLowerCase())
  let i = lower.indexOf(wanted)
  if (i < 0) i = lower.findIndex((d) => d.includes(wanted))
  if (i < 0) return { kind: 'absent' }
  return devices[i] === name ? { kind: 'listed' } : { kind: 'matched', device: devices[i] }
}

/**
 * The note when the device of the draft is not connected. Null when the
 * device list is not known, for example after a failed list.
 */
export function absentDeviceNote(s: Pick<ConfigState, 'draft' | 'devices' | 'devicesError'>): string | null {
  if (!s.draft || s.devices === null || s.devicesError !== null) return null
  const name = s.draft.microphone.device
  if (deviceStatus(name, s.devices).kind !== 'absent') return null
  return `The device "${name}" is not connected. The start fails until you connect it or choose another device.`
}

/** The text of a device name of the file in the device list. */
export function deviceLabel(name: string, devices: string[]): string {
  const status = deviceStatus(name, devices)
  switch (status.kind) {
    case 'default':
      return 'Default device'
    case 'listed':
      return name
    case 'matched':
      return `${name} (uses ${status.device})`
    case 'absent':
      return `${name} (not connected)`
  }
}

/** A decimal text such as "0.8" or "0,8". Other texts give NaN. */
export function parseDecimal(text: string): number {
  if (!/^\s*-?\d+([.,]\d+)?\s*$/.test(text)) return Number.NaN
  return Number(text.trim().replace(',', '.'))
}

/** A text of digits only, such as "4". Other texts give NaN. */
export function parseWhole(text: string): number {
  if (!/^\s*\d+\s*$/.test(text)) return Number.NaN
  return Number(text.trim())
}

const KEY_PREFIXES = ['sk_', 'gsk_', 'pk_', 'rk_', 'xai_']

/**
 * True for the name of an environment variable, with the rule of the Rust
 * side: a letter or `_` first, then letters, digits and `_`, at most 64
 * characters. A text that starts with a known key prefix, or that has a run
 * of 20 or more characters without `_` that mixes letters and digits, is a
 * key and not a name.
 */
export function isVariableName(name: string): boolean {
  if (!/^[A-Za-z_][A-Za-z0-9_]*$/.test(name) || name.length > MAX_VARIABLE_NAME) return false
  const lower = name.toLowerCase()
  if (KEY_PREFIXES.some((p) => lower.startsWith(p))) return false
  return !name
    .split('_')
    .some((run) => run.length >= 20 && /[A-Za-z]/.test(run) && /[0-9]/.test(run))
}

/** The origin of an http or https URL, for example "https://api.openai.com". */
export function originOf(url: string): string | null {
  try {
    const parsed = new URL(url.trim())
    if (parsed.protocol !== 'http:' && parsed.protocol !== 'https:') return null
    return parsed.origin
  } catch {
    return null
  }
}

/** The selected engine, or null for a value that the program does not know. */
export function engineOf(draft: Config | null): Engine | null {
  const engine = draft?.stt.engine
  return engine === 'parakeet' || engine === 'openai_compatible' ? engine : null
}

function wholeInRange(value: FieldValue, low: number, high: number, example = 4): string | null {
  const n = Number(value)
  if (typeof value !== 'number' || !Number.isInteger(n)) {
    return `Enter a whole number such as ${example}, from ${low} to ${high}.`
  }
  if (n < low || n > high) return `The value must be from ${low} to ${high}.`
  return null
}

/**
 * The page checks of all fields, for quick feedback. A changed field with an
 * error stops the save, also when its engine is not selected. The Rust side
 * checks again at the save.
 */
export function fieldErrors(
  s: Pick<ConfigState, 'draft'> & Partial<Pick<ConfigState, 'saved'>>,
  cores: number,
): Partial<Record<FieldKey, string>> {
  const errors: Partial<Record<FieldKey, string>> = {}
  const d = s.draft
  if (!d) return errors
  if (d.edcopilot.dir.trim() === '') errors['edcopilot.dir'] = 'Enter the EDCoPilot folder.'
  const threshold = d.matching.threshold
  if (typeof threshold !== 'number' || Number.isNaN(threshold)) {
    errors['matching.threshold'] = 'Enter a number such as 0.8 or 0,8, from 0.0 to 1.0.'
  } else if (threshold < 0 || threshold > 1) {
    errors['matching.threshold'] = 'The value must be from 0.0 to 1.0.'
  }
  const p = d.stt.parakeet
  if (d.stt.engine === 'parakeet' && p.model_dir.trim() === '') {
    errors['stt.parakeet.model_dir'] = 'Enter the model folder.'
  }
  const threads = wholeInRange(p.threads, 1, cores)
  if (threads) errors['stt.parakeet.threads'] = threads
  const c = d.stt.openai_compatible
  if (originOf(c.base_url) === null) {
    errors['stt.openai_compatible.base_url'] = 'Enter an address that starts with http:// or https://.'
  }
  const timeout = wholeInRange(c.timeout_seconds, 1, MAX_TIMEOUT_SECONDS)
  if (timeout) errors['stt.openai_compatible.timeout_seconds'] = timeout
  if (c.api_key_env.trim() !== '' && !isVariableName(c.api_key_env.trim())) {
    errors['stt.openai_compatible.api_key_env'] = NAME_PROBLEM
  }
  microphoneErrors(d, errors)
  for (const [key, text] of Object.entries(pageOnlyProblems(d)) as [FieldKey, string][]) {
    if (errors[key] === undefined && changedKey(s, key)) errors[key] = text
  }
  return errors
}

/** The page checks of the Microphone screen. */
function microphoneErrors(d: Config, errors: Partial<Record<FieldKey, string>>): void {
  const device = d.microphone.device
  if ([...device].length > MAX_TEXT_CHARS || /\p{Cc}/u.test(device)) {
    errors['microphone.device'] = 'This device name cannot go into the file. Choose another device.'
  }
  if (modeOf(d) === null) errors['microphone.mode'] = 'Choose push-to-talk or always listening.'
  const ptt = d.push_to_talk
  const preRoll = wholeInRange(ptt.pre_roll_ms, 0, MAX_PTT_EXTRA_MS, 250)
  if (preRoll) errors['push_to_talk.pre_roll_ms'] = preRoll
  const tail = wholeInRange(ptt.release_tail_ms, 0, MAX_PTT_EXTRA_MS, 300)
  if (tail) errors['push_to_talk.release_tail_ms'] = tail
  const vad = d.microphone.vad_threshold_dbfs
  if (typeof vad !== 'number' || !Number.isFinite(vad)) {
    errors['microphone.vad_threshold_dbfs'] =
      `Enter a number such as -40 or -40,5, from ${MIN_VAD_DBFS} to ${MAX_VAD_DBFS}.`
  }
  const words = listOf(d.always_listening.wake_words)
  if (words.length > MAX_WAKE_WORDS) {
    errors['always_listening.wake_words'] = `The list can have at most ${MAX_WAKE_WORDS} wake words.`
  } else {
    const problem = words.map(wakeWordFileProblem).find((p) => p !== null)
    if (problem) errors['always_listening.wake_words'] = problem
  }
}

/**
 * The rules that only this page has. The program accepts these values.
 * A changed value with such a problem is an error. An unchanged value of
 * the file gets a note, see `fieldNotes`.
 */
function pageOnlyProblems(d: Config): Partial<Record<FieldKey, string>> {
  const out: Partial<Record<FieldKey, string>> = {}
  const vad = d.microphone.vad_threshold_dbfs
  if (Number.isFinite(vad) && (vad < MIN_VAD_DBFS || vad > MAX_VAD_DBFS)) {
    out['microphone.vad_threshold_dbfs'] = `This screen accepts only values from ${MIN_VAD_DBFS} to ${MAX_VAD_DBFS}.`
  }
  if (listOf(d.always_listening.wake_words).some((w) => !hasWordCharacter(w))) {
    out['always_listening.wake_words'] = 'A wake word needs a letter or a digit.'
  }
  return out
}

function changedKey(s: Pick<ConfigState, 'draft'> & Partial<Pick<ConfigState, 'saved'>>, key: FieldKey): boolean {
  if (!s.saved || !s.draft) return true
  return !same(key, valueOf(s.saved, key), valueOf(s.draft, key))
}

/**
 * Notes for unchanged values of the file that only this page refuses. They
 * do not stop a save, and the program accepts them.
 */
export function fieldNotes(
  s: Pick<ConfigState, 'draft' | 'saved'>,
): Partial<Record<FieldKey, string>> {
  const notes: Partial<Record<FieldKey, string>> = {}
  if (!s.draft) return notes
  const problems = pageOnlyProblems(s.draft)
  if (problems['microphone.vad_threshold_dbfs'] && !changedKey(s, 'microphone.vad_threshold_dbfs')) {
    notes['microphone.vad_threshold_dbfs'] =
      `The file has a value outside ${MIN_VAD_DBFS} to ${MAX_VAD_DBFS}. The program uses it. ` +
      `To change it here, enter a value from ${MIN_VAD_DBFS} to ${MAX_VAD_DBFS}.`
  }
  if (problems['always_listening.wake_words'] && !changedKey(s, 'always_listening.wake_words')) {
    notes['always_listening.wake_words'] =
      'The file has a wake word without a letter or a digit. It never matches. ' +
      'A change of the list here needs a letter or a digit in each wake word.'
  }
  return notes
}

/**
 * Why a field does not show on the screen `from`, as a context for the
 * text and an action for the user. Null when the field shows.
 */
function hiddenBy(
  key: FieldKey,
  draft: Config | null,
  from: ConfigScreen,
): { context: string; action: string } | null {
  const screen = FIELDS[key].screen
  const engine = engineOfField(key)
  const mode = modeOfField(key)
  const offEngine = engine !== undefined && engine !== engineOf(draft)
  const offMode = mode !== undefined && mode !== modeOf(draft)
  const group = offEngine && engine ? ENGINE_LABEL[engine] : offMode && mode ? MODE_LABEL[mode] : null
  if (screen !== from) {
    const context = [SCREEN_LABEL[screen], group, FIELDS[key].label].filter(Boolean).join(': ')
    return { context, action: `open the ${SCREEN_LABEL[screen]} screen` }
  }
  if (group === null) return null
  return { context: `${group}: ${FIELDS[key].label}`, action: offEngine ? 'switch the engine' : 'switch the mode' }
}

/**
 * The text that refuses a save: each changed field with a page error. A
 * field that the screen `from` does not show names its screen, its engine,
 * or its mode.
 */
function refusalText(
  s: Pick<ConfigState, 'saved' | 'draft'>,
  cores: number,
  from: ConfigScreen,
): string | null {
  const errors = fieldErrors(s, cores)
  const bad = changedKeys(s).filter((key) => errors[key] !== undefined)
  if (bad.length === 0) return null
  const hidden = bad.flatMap((key) => {
    const why = hiddenBy(key, s.draft, from)
    return why ? [{ key, ...why }] : []
  })
  if (hidden.length === 0) return 'Correct the marked fields first.'
  const lines = hidden.map((h) => `${h.context}: ${errors[h.key]}`)
  const actions = [...new Set(hidden.map((h) => h.action))].join(' or ')
  const action = actions.charAt(0).toUpperCase() + actions.slice(1)
  return `${lines.join(' ')} ${action} to correct it, or discard the changes.`
}

/** The text that turns the Start buttons off when the latest check did not run. */
export const CHECK_FAILED_BLOCK = 'The check did not run. Click "Check again" on the Setup screen.'

/** The text that turns the Start buttons off while a check runs, also before the first result. */
export const CHECKING_BLOCK = 'Checking the setup.'

/**
 * The text that turns the Start buttons off when another program changed
 * the configuration file and the draft has changes. The latest check is of
 * the older file: the user chooses first.
 */
export const FILE_CHANGED_BLOCK =
  'Another program changed the configuration file. On the Setup screen, read the file again or discard your changes.'

/** The text of a clean check on the Setup screen. */
export const CHECK_OK = 'The check found no problem.'

/** The text of a clean check, or `CHECKING_BLOCK` while a new check runs. */
export function checkOkText(s: Pick<ConfigState, 'checking'>): string {
  return s.checking ? CHECKING_BLOCK : CHECK_OK
}

/**
 * The text that turns the Start buttons off, or null when a start is
 * possible. "No problem" counts only after a full check: a check that did
 * not run blocks the start, also when an older result is clean. While a
 * check runs, the older result does not count either.
 */
export function startBlock(
  s: Pick<ConfigState, 'check' | 'checkError' | 'checking' | 'fileChanged'>,
): string | null {
  if (s.checking) return CHECKING_BLOCK
  if (s.fileChanged) return FILE_CHANGED_BLOCK
  if (s.checkError !== null) return CHECK_FAILED_BLOCK
  if (s.check === null) return CHECKING_BLOCK
  const n = s.check.problems.length
  if (n === 0) return null
  return `Fix the setup first: ${n} ${n === 1 ? 'problem' : 'problems'}.`
}

/** What the key store has, in words. Never the key. */
export function keyStatus(s: Pick<ConfigState, 'saved'>): string {
  const saved = s.saved
  if (!saved || !saved.api_key_set) return 'No key is saved'
  if (saved.api_key_host === null) {
    const name = saved.stt.openai_compatible.api_key_env
    return `The key comes from the environment variable ${name}`
  }
  return `A key is saved for ${saved.api_key_host}`
}

/**
 * Why the key field is off, or null. The key is saved for the saved server
 * and the saved variable name, so these two must not have a draft change.
 */
export function keyFieldBlock(s: Pick<ConfigState, 'saved' | 'draft'>): string | null {
  const { saved, draft } = s
  if (!saved || !draft) return 'The settings did not load'
  if (saved.stt.openai_compatible.base_url_problem !== null) return BASE_URL_BLOCK
  if (!same('stt.openai_compatible.base_url', saved.stt.openai_compatible.base_url, draft.stt.openai_compatible.base_url)) {
    return 'Save the server address first'
  }
  if (!same('stt.openai_compatible.api_key_env', saved.stt.openai_compatible.api_key_env, draft.stt.openai_compatible.api_key_env)) {
    return 'Save the variable name first'
  }
  return null
}

/** Empties the key input when the key field is off, so that no typed key waits for another server. */
export function clearWhenBlocked(input: { value: string } | null, block: string | null): void {
  if (input && block !== null) input.value = ''
}

/** The warning when the draft server is not the server of the saved key. */
export function hostWarning(s: Pick<ConfigState, 'saved' | 'draft'>): string | null {
  const host = s.saved?.api_key_host
  const draft = s.draft
  if (!host || !draft) return null
  const next = originOf(draft.stt.openai_compatible.base_url)
  if (next === null || next === host) return null
  return `The saved key belongs to ${host}. After the save, enter the key again for ${next}.`
}

/** The number of logical processors of this computer. */
export function logicalCores(): number {
  const n = typeof navigator === 'undefined' ? undefined : navigator.hardwareConcurrency
  return typeof n === 'number' && n > 0 ? n : FALLBACK_CORES
}

/** The part of the session state that the store follows. */
export interface SessionPhase {
  running: boolean
  starting: boolean
}

export interface ConfigOptions {
  cores?: number
  /** When given, `restartNeeded` is cleared when the session stops. */
  session?: Readable<SessionPhase>
  /** The device list and the capture of the talk button. */
  microphone?: MicrophoneApi
  /** The clock in milliseconds. Tests give a fake. */
  now?: () => number
  /** Gives the token of a new capture. */
  newToken?: () => number
}

const NO_MICROPHONE_API = 'the device commands are not available'
/** The text after a cancelled capture. */
export const CAPTURE_CANCELLED = 'Cancelled. The talk button did not change.'
/** The text of a save while the talk button capture runs. */
export const SAVE_DURING_CAPTURE = 'Finish or cancel the talk button capture first, then save.'

/** A new token for a capture: a random number from 1 to 2^32 - 1, the range of `u32`. */
function newCaptureToken(): number {
  const one = new Uint32Array(1)
  do crypto.getRandomValues(one)
  while (one[0] === 0)
  return one[0]
}

export function createConfigStore(source: ConfigApi, options: ConfigOptions = {}): ConfigStore {
  const cores = options.cores ?? logicalCores()
  const now = options.now ?? (() => Date.now())
  const newToken = options.newToken ?? newCaptureToken
  const microphone = options.microphone
  const store = writable<ConfigState>(initialConfigState())
  const { subscribe } = store
  // A plain copy of the state, so that the functions read it without a subscription.
  let state = initialConfigState()
  store.subscribe((s) => (state = s))
  // Only the newest read of the file and the newest check apply.
  let loadNumber = 0
  let checkNumber = 0
  // The capture in work, and the time when the last capture ended.
  let capture: { promise: Promise<void>; cancelled: boolean; token: number } | null = null
  let captureEnd = Number.NEGATIVE_INFINITY

  function update(change: (s: ConfigState) => ConfigState): void {
    store.update((s) => {
      const next = change(s)
      return { ...next, dirty: changedKeys(next).length > 0 || bindingChanged(next) }
    })
  }

  // The note about the restart belongs to the session that ran at the save.
  options.session?.subscribe((phase) => {
    if (!phase.running && !phase.starting && state.restartNeeded) {
      update((s) => ({ ...s, restartNeeded: false }))
    }
  })

  /**
   * Reads the file. `base` is the draft that the reload replaces: a key where
   * the current draft differs from `base` keeps its draft value. Without
   * `base`, the saved values are the base.
   */
  async function reload(base?: Config): Promise<void> {
    const mine = ++loadNumber
    update((s) => ({ ...s, loading: true }))
    try {
      const config = await source.getConfig()
      if (mine !== loadNumber) return
      update((s) => {
        const from = base ?? s.saved
        const draft = s.draft && from ? merge(from, s.draft, config) : structuredClone(config)
        return { ...s, loading: false, loadError: null, readError: null, saved: config, draft }
      })
    } catch (error) {
      if (mine !== loadNumber) return
      update((s) => ({
        ...s,
        loading: false,
        loadError: api.toError(error).message,
        saved: null,
        draft: null,
      }))
    }
  }

  async function runCheck(): Promise<void> {
    const mine = ++checkNumber
    update((s) => ({ ...s, checking: true }))
    try {
      const result = await source.check()
      if (mine !== checkNumber) return
      update((s) => ({ ...s, checking: false, check: result, checkError: null }))
    } catch (error) {
      if (mine !== checkNumber) return
      update((s) => ({ ...s, checking: false, checkError: api.toError(error).message }))
    }
  }

  async function init(): Promise<void> {
    await reload()
    await runCheck()
  }

  async function checkAgain(): Promise<void> {
    // A save reads the file and checks it at its end.
    if (state.saving) return
    // The button is off from now until the check ends.
    update((s) => ({ ...s, checking: true }))
    if (state.saved === null) {
      // Nothing to keep: the first read failed. Read as at the start.
      await reload()
      if (state.saved !== null) update((s) => ({ ...s, readError: null }))
      await runCheck()
      return
    }
    const mine = ++loadNumber
    // True when the file changed under a draft with changes.
    let conflict = false
    try {
      const config = await source.getConfig()
      if (mine === loadNumber) {
        update((s) => {
          if (s.saved === null || s.draft === null) {
            return { ...s, readError: null, saved: config, draft: structuredClone(config) }
          }
          if (!s.dirty) {
            return { ...s, readError: null, saved: config, draft: structuredClone(config) }
          }
          if (config.version !== s.saved.version) {
            // Another program changed the file, and the draft has changes:
            // the user chooses. The draft and its base stay as they are.
            conflict = true
            return { ...s, readError: null, fileChanged: true }
          }
          return { ...s, readError: null, saved: config, draft: merge(s.saved, s.draft, config) }
        })
      }
    } catch (error) {
      if (mine === loadNumber) {
        const message = api.toError(error).message
        update((s) => ({ ...s, readError: message }))
      }
    }
    if (conflict) {
      // The screen shows the older file. A check of the new file would not
      // fit the screen: the user chooses first (`readAgainKeepingChanges`
      // or `discard`), then the check runs.
      update((s) => ({ ...s, checking: false }))
      return
    }
    await runCheck()
  }

  function set(key: FieldKey, value: FieldValue): void {
    update((s) =>
      s.draft
        ? {
            ...s,
            draft: withValue(s.draft, key, value),
            saveError: null,
            nameDropped: key === 'stt.openai_compatible.api_key_env' ? false : s.nameDropped,
          }
        : s,
    )
    // The talk button belongs to push-to-talk. A capture must not go on
    // unseen while the user types in the always-listening fields.
    if (key === 'microphone.mode' && value !== 'push_to_talk') void cancelCapture()
  }

  /**
   * "On" keeps the list of the file. Else it writes the list that the
   * program gives: the copy in the data folder, or the shipped list of a
   * source tree. The hotword table of the Words screen edits the same file.
   */
  function setHotwords(on: boolean): void {
    const own = state.saved?.stt.parakeet.hotwords_file ?? ''
    const shipped = state.saved?.default_hotwords_file ?? ''
    set('stt.parakeet.hotwords_file', on ? own || shipped : '')
  }

  function dropInvalidName(): void {
    const key = 'stt.openai_compatible.api_key_env'
    if (!state.saved || fieldErrors(state, cores)[key] === undefined) return
    const saved = state.saved.stt.openai_compatible.api_key_env
    update((s) => (s.draft ? { ...s, draft: withValue(s.draft, key, saved), nameDropped: true } : s))
  }

  function edits(): ConfigEdit[] {
    const draft = state.draft
    if (!draft) return []
    const list = changedKeys(state).map((key) => edit(key, valueFor(FIELDS[key].kind, valueOf(draft, key))))
    if (bindingChanged(state)) list.push(...bindingEdits(draft.push_to_talk))
    return list
  }

  async function save(from: ConfigScreen = 'setup'): Promise<void> {
    if (state.saving || !state.saved || !state.draft) return
    // The button is off during a capture. This guard also stops another caller.
    if (state.capturing) {
      update((s) => ({ ...s, saveError: SAVE_DURING_CAPTURE }))
      return
    }
    const sent = structuredClone(state.draft)
    const version = state.saved.version
    // `fileChanged` stays until the program answers a save. A local refusal
    // sends nothing, and the start must stay off on the check of the old file.
    update((s) => ({ ...s, saving: true, saveError: null }))
    try {
      const refusal = refusalText(state, cores, from)
      if (refusal !== null) {
        update((s) => ({ ...s, saveError: refusal }))
        return
      }
      const list = edits()
      if (list.length === 0) {
        update((s) => ({ ...s, saveOutcome: 'unchanged', problems: [] }))
        return
      }
      const result = await source.saveConfig(list, version)
      if (!result.written) {
        const refused = result.problems.length > 0
        update((s) => ({
          ...s,
          saveOutcome: refused ? 'refused' : 'unchanged',
          problems: result.problems,
          fileChanged: result.problems.some((p) => p.startsWith(CONFIG_CHANGED)),
        }))
        if (!refused) await reload(sent)
        return
      }
      update((s) => ({
        ...s,
        saveOutcome: 'saved',
        problems: result.problems,
        restartNeeded: result.restart_needed,
        fileChanged: false,
      }))
      await reload(sent)
      await runCheck()
    } catch (error) {
      // A value that could not be built, or a call that failed. Nothing was written.
      update((s) => ({ ...s, saveError: api.toError(error).message }))
    } finally {
      update((s) => ({ ...s, saving: false }))
    }
  }

  async function readAgainKeepingChanges(): Promise<void> {
    if (state.saving) return
    // The base of the merge is the old read: each key that the user changed
    // keeps its draft value, each other key gets the value of the file.
    await reload()
    if (state.loadError !== null) return
    update((s) => ({ ...s, fileChanged: false, saveOutcome: null, problems: [] }))
    await runCheck()
  }

  function discard(): void {
    // A binding that comes after the discard must not make a new change.
    void cancelCapture()
    // After a change of the file by another program, the screen shows the
    // older file: read the new file and check it.
    const changed = state.fileChanged
    update((s) => ({
      ...s,
      draft: s.saved ? structuredClone(s.saved) : null,
      saveOutcome: null,
      problems: [],
      saveError: null,
      fileChanged: false,
      nameDropped: false,
    }))
    if (changed) void checkAgain()
  }

  /**
   * Sends the key. Only this function holds the key, and it ends when
   * `set_api_key` returns. The reload and the check come after it.
   */
  async function sendKey(key: string): Promise<boolean> {
    if (state.keyBusy) return false
    if (key.trim() === '') {
      update((s) => ({ ...s, keyMessage: null, keyError: 'Enter the key first.' }))
      return false
    }
    const block = keyFieldBlock(state)
    const saved = state.saved
    const origin = saved ? originOf(saved.stt.openai_compatible.base_url) : null
    if (block !== null || !saved || origin === null) {
      const text = block ?? 'The server address is not an http or https address.'
      update((s) => ({ ...s, keyMessage: null, keyError: text }))
      return false
    }
    update((s) => ({ ...s, keyBusy: true, keyMessage: null, keyError: null }))
    try {
      const result = await source.setApiKey(key, origin, saved.stt.openai_compatible.api_key_env)
      // A cloud session keeps the key of its start.
      update((s) => ({
        ...s,
        keyBusy: false,
        keyMessage: result.restart_needed ? `The key is saved. ${KEY_RESTART}` : 'The key is saved.',
        restartNeeded: s.restartNeeded || result.restart_needed,
      }))
      return true
    } catch (error) {
      const text = api.toError(error).message
      // The Rust side never repeats the key. This check is a second guard.
      const shown = text.includes(key.trim()) ? 'The key store did not save the key.' : text
      update((s) => ({ ...s, keyBusy: false, keyError: shown }))
      return false
    }
  }

  async function afterKey(ok: boolean): Promise<boolean> {
    await reload()
    await runCheck()
    return ok
  }

  function saveApiKey(key: string): Promise<boolean> {
    return sendKey(key).then(afterKey)
  }

  async function removeApiKey(): Promise<void> {
    if (state.keyBusy) return
    update((s) => ({ ...s, keyBusy: true, keyMessage: null, keyError: null }))
    let failure: string | null = null
    let restart = false
    try {
      restart = (await source.clearApiKey()).restart_needed
    } catch (error) {
      failure = api.toError(error).message
    }
    update((s) => ({
      ...s,
      restartNeeded: s.restartNeeded || restart,
      keyBusy: false,
      keyMessage: failure !== null ? null : restart ? `The key is removed. ${KEY_RESTART}` : 'The key is removed.',
      keyError: failure,
    }))
    await reload()
    await runCheck()
  }

  async function useModelFolder(dir: string, restartNeeded = false): Promise<void> {
    if (restartNeeded) update((s) => ({ ...s, restartNeeded: true }))
    await reload()
    if (state.draft) set('stt.parakeet.model_dir', dir)
    await runCheck()
  }

  async function loadDevices(): Promise<void> {
    try {
      if (!microphone) throw new Error(NO_MICROPHONE_API)
      const devices = await microphone.listDevices()
      update((s) => ({ ...s, devices, devicesError: null }))
    } catch (error) {
      // The devices are not known now. An old list can name devices that are gone.
      update((s) => ({ ...s, devices: null, devicesError: api.toError(error).message }))
    }
  }

  /**
   * Puts a captured binding in the draft: a key removes the joystick, a
   * joystick removes the key. False when the draft is not push-to-talk:
   * then the binding is dropped.
   */
  function setBinding(binding: Binding): boolean {
    if (modeOf(state.draft) !== 'push_to_talk') return false
    update((s) => {
      if (!s.draft) return s
      const draft = structuredClone(s.draft)
      if (binding.kind === 'joystick' && binding.joystick) {
        draft.push_to_talk.key = null
        draft.push_to_talk.joystick = { ...binding.joystick }
      } else if (binding.key !== null) {
        draft.push_to_talk.key = binding.key
        draft.push_to_talk.joystick = null
      }
      return { ...s, draft, saveError: null }
    })
    return true
  }

  function captureBinding(): Promise<void> {
    // One capture at a time. A click right after the end of a capture is
    // the release of the key that activated the button (Space), not a new
    // request.
    if (capture) return capture.promise
    if (!state.draft || state.saving || now() - captureEnd < CAPTURE_REST_MS) return Promise.resolve()
    const mine = { promise: Promise.resolve(), cancelled: false, token: newToken() }
    update((s) => ({ ...s, capturing: true, captureMessage: null, captureError: null }))
    mine.promise = (async () => {
      let message: string | null = null
      let failure: string | null = null
      try {
        if (!microphone) throw new Error(NO_MICROPHONE_API)
        const binding = await microphone.captureBinding(mine.token)
        // A cancel that comes too late, a discard, or a switch to
        // always-listening drops the binding.
        if (!mine.cancelled && setBinding(binding)) {
          message = `The talk button is now ${bindingLabel(state.draft!.push_to_talk)}. Save to keep it.`
        } else {
          message = CAPTURE_CANCELLED
        }
      } catch (error) {
        const text = api.toError(error).message
        if (mine.cancelled || /cancelled/.test(text)) message = CAPTURE_CANCELLED
        else failure = text
      } finally {
        capture = null
        captureEnd = now()
        update((s) => ({ ...s, capturing: false, captureMessage: message, captureError: failure }))
      }
    })()
    capture = mine
    return mine.promise
  }

  async function cancelCapture(): Promise<void> {
    const current = capture
    if (!current || !microphone) return
    current.cancelled = true
    let ended = false
    void current.promise.then(() => (ended = true))
    // A cancel that reaches the Rust side before the capture starts does
    // nothing. Thus the call repeats until the capture ends.
    for (let i = 0; i < MAX_CANCEL_CALLS && !ended; i++) {
      try {
        await microphone.cancelCapture(current.token)
      } catch {
        // The next call tries again.
      }
      await Promise.race([current.promise, new Promise((r) => setTimeout(r, CANCEL_RETRY_MS))])
    }
    await current.promise
  }

  return {
    subscribe,
    init,
    useModelFolder,
    markRestartNeeded: () => update((s) => ({ ...s, restartNeeded: true })),
    load: () => reload(),
    readAgainKeepingChanges,
    runCheck,
    // The same steps as the start: read the file, then check.
    checkAgain,
    set,
    setHotwords,
    dropInvalidName,
    edits,
    save,
    discard,
    saveApiKey,
    removeApiKey,
    loadDevices,
    captureBinding,
    cancelCapture,
  }
}

/** The store of the window, with the real commands and the session. */
export const configStore: ConfigStore = createConfigStore(api, {
  session: sessionStore,
  microphone: api,
})
