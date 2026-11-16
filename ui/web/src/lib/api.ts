// The typed layer over the Tauri commands and the session event.
//
// This is the only file that imports from `@tauri-apps/api` and from the
// Tauri plugins. Tests replace
// the functions of this file with fakes.
//
// The types copy the types of `ui/src/dto.rs`. Result fields are snake_case
// as the Rust side sends them. Argument names are camelCase, because Tauri
// converts them to the snake_case names of the Rust arguments.

import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import { open } from '@tauri-apps/plugin-dialog'

/** The result of the Tauri command `app_info`. */
export interface AppInfo {
  version: string
  engine_local: boolean
}

/** `ValueDto`: the new value of one configuration key. */
export type ConfigValue =
  | { kind: 'text'; value: string }
  | { kind: 'number'; value: number }
  | { kind: 'integer'; value: number }
  | { kind: 'flag'; value: boolean }
  | { kind: 'text_list'; value: string[] }
  | { kind: 'absent' }

/** `EditDto`: one edit of the configuration. */
export interface ConfigEdit {
  key: string
  value: ConfigValue
}

/** `SaveResultDto`. */
export interface SaveResult {
  problems: string[]
  written: boolean
  restart_needed: boolean
}

/** `KeyResultDto`: the result of `set_api_key` and `clear_api_key`. Never the key. */
export interface KeyResult {
  /** A session that uses the cloud engine runs or starts. It keeps the key of its start. */
  restart_needed: boolean
}

export interface EdcopilotConfig {
  dir: string
  /** Empty: the default from `dir`. */
  request_file: string
  voice_commands_ini: string
  dynamic_vars_dir: string
  /** `edcopilot.profile` as the file has it. Empty: the default from `dir`. */
  profile: string
  /** The profile file that the program reads. Empty without a folder and without a value. */
  profile_in_use: string
}

export interface DataConfig {
  aliases_dir: string
  sound_alikes: string
  synonyms: string
  fillers: string
}

export interface MatchingConfig {
  threshold: number
  exact_single_word: boolean
}

export interface MicrophoneConfig {
  device: string
  mode: string
  vad_threshold_dbfs: number
}

export interface Joystick {
  device: string
  button: number
}

export interface PushToTalkConfig {
  /** Null and no joystick: the program uses the key F13. */
  key: string | null
  joystick: Joystick | null
  pre_roll_ms: number
  release_tail_ms: number
}

export interface AlwaysListeningConfig {
  wake_words: string[]
}

export interface ParakeetConfig {
  model_dir: string
  threads: number
  hotwords_file: string
  hotwords_score: number
}

export interface OpenAiCompatibleConfig {
  /** As the file has it, or without its secret parts when `base_url_problem` is set. */
  base_url: string
  /** The text when the address of the file holds a user name, a password, a query or a fragment. */
  base_url_problem: string | null
  /** The name of the variable. Never the key. */
  api_key_env: string
  model: string
  language: string
  timeout_seconds: number
}

export interface SttConfig {
  engine: string
  parakeet: ParakeetConfig
  openai_compatible: OpenAiCompatibleConfig
}

export interface DiagnosticsConfig {
  log_file: string
  save_audio_dir: string
}

/** `[updates]`: the switch of the automatic update check. */
export interface UpdatesConfig {
  check: boolean
}

/** `ConfigDto`: all settings. */
export interface Config {
  path: string
  api_key_set: boolean
  api_key_host: string | null
  /**
   * The hotword list that the hotword switch writes, as an absolute path.
   * The hotword table edits the same file while the hotwords are off.
   */
  default_hotwords_file: string
  /** The stamp of the file content. `saveConfig` sends it back. */
  version: string
  edcopilot: EdcopilotConfig
  data: DataConfig
  matching: MatchingConfig
  microphone: MicrophoneConfig
  push_to_talk: PushToTalkConfig
  always_listening: AlwaysListeningConfig
  stt: SttConfig
  diagnostics: DiagnosticsConfig
  updates: UpdatesConfig
}

/** `ReportDto`: the counts of the loaded data. */
export interface LoadReport {
  line: string
  commands: number
  matchable: number
  unresolved_grammar: number
  composite_parts: number
  vocabulary_files: number
  vocabulary_names: number
  alias_phrases: number
  sound_alikes: number
  synonyms: number
  synonym_warnings: string[]
  fillers: number
  fillers_active: boolean
  filler_warnings: string[]
  problems: string[]
}

/** `ProfileReportDto`: the build from the EDCoPilot profile, with the texts of `voicebridge check`. */
export interface ProfileReport {
  records: number
  left_out: number
  /** All lines in the order of `voicebridge check`. */
  lines: string[]
}

/** `CheckDto`: the content of `voicebridge check`. */
export interface CheckResult {
  problems: string[]
  warnings: string[]
  report: LoadReport | null
  /** Null when the commands did not load. */
  profile: ProfileReport | null
  /** The problems at the field of the EDCoPilot folder. Each one is also in `problems`. */
  edcopilot_problems: string[]
  info: string[]
}

/** `LeftOutRowDto`: one command that the program left out of the profile. */
export interface LeftOutRow {
  /** `no_write`, `multi_write`, `maintenance`, `orphan`, or `not_understood`. */
  class: string
  /** The line of `voicebridge check --left-out`. */
  line: string
}

/** `LeftOutListDto`: one page of the left-out commands. */
export interface LeftOutList {
  rows: LeftOutRow[]
  offset: number
  total: number
  catalog_id: number
}

/** `StartInfoDto`. */
export interface StartInfo {
  report: LoadReport
  engine: string
  mode: 'push_to_talk' | 'always_listening'
  mode_line: string
  input_device: string | null
  threads: number | null
  dry_run: boolean
  hotwords_line: string
  diagnostics_lines: string[]
  /** The start lines as the console shows them. */
  lines: string[]
}

export type OutcomeKind =
  | 'sent'
  | 'dry_run'
  | 'no_match'
  | 'no_wake_word'
  | 'rejected'
  | 'not_consumed'
  | 'engine_error'
  | 'send_error'

/** Why a candidate gave no command: `OutcomeDto.blocked`. */
export type BlockReason = 'tie' | 'required_words' | 'no_token' | 'forms' | 'dropped_word'

export type Mark =
  | 'sound_alike'
  | 'synonym'
  | 'filler'
  | 'hotwords'
  | 'glitch'
  | 'clipping'
  | 'very_quiet'

/** `OutcomeDto`. */
export interface Outcome {
  kind: OutcomeKind
  text: string | null
  token: string | null
  confidence: number | null
  reason: string | null
  /** For a `no_match` that the program blocked: the cause. */
  blocked: BlockReason | null
  marks: Mark[]
}

/** `UtteranceDto`. */
export interface Utterance {
  outcome: Outcome
  transcribe_ms: number
  total_ms: number
  clip_ms: number
  peak_dbfs: number
  rms_dbfs: number
  audio_file: string | null
}

/** `StopReasonDto`. `message` is null for `requested`. */
export interface StopReason {
  kind: 'requested' | 'device_error' | 'engine_error' | 'input_error'
  message: string | null
}

/** `EventDto`: the payload of `session://event`. */
export type SessionEvent =
  | { type: 'warning'; text: string }
  | { type: 'started'; info: StartInfo }
  | { type: 'talk'; down: boolean }
  | ({ type: 'utterance' } & Utterance)
  | { type: 'dropped'; reason: string }
  | { type: 'vocabulary_reloaded'; files: number; names: number }
  | { type: 'stopped'; reason: StopReason; glitches: number }

/** `StatusDto`. */
export interface SessionStatus {
  running: boolean
  /** True while a start is in work. `running` is false then. */
  starting: boolean
  dry_run: boolean
  start_info: StartInfo | null
  stop_reason: StopReason | null
}

/** `BindingDto`: a captured talk button. */
export interface Binding {
  kind: 'key' | 'joystick'
  label: string
  key: string | null
  joystick: Joystick | null
}

/** `ModelEventDto`: the payload of `model://event`, one step of the model download. */
export type ModelEvent =
  | { type: 'progress'; done: number; total: number }
  | { type: 'verifying' }
  | { type: 'unpacking' }
  /**
   * `config_error` is null when the configuration has the new model folder.
   * `restart_needed`: the folder changed while a session runs, and that
   * session keeps the model of its start.
   */
  | { type: 'done'; model_dir: string; config_error: string | null; restart_needed: boolean }
  | { type: 'failed'; message: string }
  | { type: 'cancelled' }

/** `ModelInfoDto`: the model that the download gets. */
export interface ModelInfo {
  name: string
  archive_bytes: number
  unpacked_bytes: number
  /** Null when the program cannot find the per-user data folder. */
  models_dir: string | null
  /** True while a download runs. */
  running: boolean
  /** The last step of the running download, or null. */
  step: Extract<ModelEvent, { type: 'progress' | 'verifying' | 'unpacking' }> | null
}

/** `CommandRowDto`: one line of `voicebridge commands`. */
export interface CommandRow {
  /** Empty: the command has no category. */
  category: string
  phrase: string
  token: string
  works: boolean
  /** Why the command cannot fire. Null when it works. */
  reason: string | null
  /** True for a phrase from an alias file. */
  alias: boolean
}

/** `CompositeGroupDto`: one composite group as the console shows it. */
export interface CompositeGroup {
  heading: string
  lines: string[]
}

/** `CommandListDto`: the result of `list_commands`. */
export interface CommandList {
  /** One page of rows. */
  rows: CommandRow[]
  /** The place of the first row of the page in the whole list. */
  offset: number
  /** The number of all rows of the filter. */
  total: number
  /** The id of the loaded data. Pages with different ids do not fit together. */
  catalog_id: number
  composite: CompositeGroup[]
  /** The last line of the console list. */
  summary: string
  /** The problems of the command data (matching setting, EDCoPilot files, data files). */
  problems: string[]
  /** The warnings of the synonyms, the filler words, the aliases, the vocabulary files, and the load of the commands. */
  warnings: string[]
}

/** `PhraseTestDto`: the result of `test_phrase`. Always a dry run. */
export interface PhraseTest {
  outcome: Outcome
  /** The result line, as the console writes it. */
  line: string
  warnings: string[]
  /** The wake words that the phrase needs. Empty in push-to-talk mode. */
  wake_words: string[]
}

/** The word tables. An alias table also has a language. */
export type TableKind = 'aliases' | 'synonyms' | 'sound_alikes' | 'fillers' | 'hotwords'

/**
 * `RowDto`: one entry of a word table. Aliases: `spoken` is the token
 * template, `meant` has the phrases. Synonyms: the spoken phrase and the
 * EDCoPilot phrases. Sound-alikes: the heard word and the one meant word.
 * Fillers: the word and its list (`not_last` or `may_be_last`). Hotwords:
 * the phrase, and `meant` is empty.
 */
export interface TableRow {
  spoken: string
  meant: string[]
}

/** `TableDto`: a word table as the file has it. */
export interface WordTable {
  kind: TableKind
  /** The language of an alias file. Null for the other tables. */
  language: string | null
  path: string
  /** False when the file does not exist. A save makes it. */
  exists: boolean
  rows: TableRow[]
  /** The stamp of the file content. `saveTable` needs it. */
  version: string
  /** True when the configuration has no hotword file. */
  hotwords_off: boolean
  /** The languages of the alias folder, for each table. */
  languages: string[]
}

/** `TableCheckDto`: the lines of `voicebridge check` for the rows. */
export interface TableCheck {
  problems: string[]
  warnings: string[]
}

/** `TableSaveDto`. */
export interface TableSave {
  written: boolean
  /** The problems that refused the save. */
  problems: string[]
  warnings: string[]
  /** The warnings that the page did not confirm. They refused the save. */
  unconfirmed: string[]
  version: string
  /** The file was written while a session runs. That session keeps its old tables. */
  restart_needed: boolean
}

/** The step of the update. */
export type UpdatePhase =
  | 'idle'
  | 'checking'
  | 'up_to_date'
  | 'available'
  | 'downloading'
  | 'ready'
  | 'check_failed'

/** `UpdateStatusDto`: the state of the update check and install. */
export interface UpdateStatus {
  /** The version of this program. */
  current_version: string
  /** A build with debug assertions: it never checks. */
  developer_build: boolean
  phase: UpdatePhase
  /** The offered version, with `available`, `downloading`, and `ready`. */
  version: string | null
  /** The release notes of the offered version, as plain text. */
  notes: string | null
  /** The publish date of the offered version (RFC 3339). */
  date: string | null
  /** The release page of the offered version. */
  release_page: string | null
  /** The bytes of the download so far. */
  received: number
  /** The size of the download, when the server sent it. */
  total: number | null
  /** Why the last check failed, with `check_failed`. */
  cause: string | null
  /** The text of the last failed download or install of the offer. */
  error: string | null
  /** The end of the last check, in milliseconds since 1970. */
  checked_at: number | null
  /** True after "Later", until the next manual check. */
  hidden: boolean
}

/** The name of the Tauri event of the session. */
export const SESSION_EVENT = 'session://event'
/** The name of the Tauri event of the model download. */
export const MODEL_EVENT = 'model://event'
/** The name of the Tauri event of the update. */
export const UPDATE_EVENT = 'update://event'

/**
 * Calls one command. Tauri rejects with the error text of the command. This
 * function changes that text into an `Error` with the same message.
 */
async function call<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  try {
    return await invoke<T>(command, args)
  } catch (error) {
    throw toError(error)
  }
}

/** Changes a rejection value into an `Error`. A text stays as it is. */
export function toError(error: unknown): Error {
  if (error instanceof Error) return error
  if (typeof error === 'string') return new Error(error)
  return new Error(String(error))
}

export function appInfo(): Promise<AppInfo> {
  return call<AppInfo>('app_info')
}

export function getConfig(): Promise<Config> {
  return call<Config>('get_config')
}

/**
 * `version` is the version of the read that the edits belong to. The Rust
 * side refuses the save when another program changed one of the edited keys
 * after that read.
 */
export function saveConfig(edits: ConfigEdit[], version: string): Promise<SaveResult> {
  return call<SaveResult>('save_config', { edits, version })
}

export function check(): Promise<CheckResult> {
  return call<CheckResult>('check')
}

/** Blocks while the model loads. The problems come as the error text, one on each line. */
export function startSession(dryRun: boolean): Promise<void> {
  return call<void>('start_session', { dryRun })
}

export function stopSession(): Promise<void> {
  return call<void>('stop_session')
}

export function sessionStatus(): Promise<SessionStatus> {
  return call<SessionStatus>('session_status')
}

export function listDevices(): Promise<string[]> {
  return call<string[]>('list_devices')
}

/** Waits for one press. `cancelCapture(token)` with the same token ends this capture. */
export function captureBinding(token: number): Promise<Binding> {
  return call<Binding>('capture_binding', { token })
}

/** Ends the capture with this token. Without a token it ends any capture. */
export function cancelCapture(token?: number): Promise<void> {
  return call<void>('cancel_capture', token === undefined ? undefined : { token })
}

/**
 * Saves the key for the server and the variable name that the window shows.
 * The Rust side refuses the key when the file now has other values.
 */
export function setApiKey(key: string, expectedOrigin: string, expectedName: string): Promise<KeyResult> {
  return call<KeyResult>('set_api_key', { key, expectedOrigin, expectedName })
}

export function clearApiKey(): Promise<KeyResult> {
  return call<KeyResult>('clear_api_key')
}

/**
 * Opens the folder dialog of the system. The result is the selected folder
 * as an absolute path, or null when the user cancels. `startDir` is the
 * folder that the dialog shows first. An empty text lets the system select.
 */
export async function pickFolder(title: string, startDir: string): Promise<string | null> {
  try {
    const picked = await open({
      title,
      directory: true,
      multiple: false,
      defaultPath: startDir.trim() === '' ? undefined : startDir,
    })
    return typeof picked === 'string' ? picked : null
  } catch (error) {
    throw toError(error)
  }
}

/** Calls `handler` for each session event. The result removes the listener. */
export async function onSessionEvent(
  handler: (event: SessionEvent) => void,
): Promise<() => void> {
  try {
    return await listen<SessionEvent>(SESSION_EVENT, (event) => handler(event.payload))
  } catch (error) {
    throw toError(error)
  }
}

export function modelInfo(): Promise<ModelInfo> {
  return call<ModelInfo>('model_info')
}

/**
 * Starts the model download and returns at once. The steps come as
 * `model://event`. The call fails when a download already runs.
 */
export function downloadModel(): Promise<void> {
  return call<void>('download_model')
}

export function cancelModelDownload(): Promise<void> {
  return call<void>('cancel_model_download')
}

/** Calls `handler` for each step of the model download. The result removes the listener. */
export async function onModelEvent(handler: (event: ModelEvent) => void): Promise<() => void> {
  try {
    return await listen<ModelEvent>(MODEL_EVENT, (event) => handler(event.payload))
  } catch (error) {
    throw toError(error)
  }
}

/**
 * The commands of `voicebridge commands <filter>`. `all` also gives the
 * commands that do not work, with the reason. The result has the rows from
 * `offset`, at most `limit`, and the number of all rows.
 */
export function listCommands(
  filter: string,
  all: boolean,
  offset: number,
  limit: number,
): Promise<CommandList> {
  return call<CommandList>('list_commands', { filter, all, offset, limit })
}

/**
 * The commands that the program left out of the EDCoPilot profile, with the
 * reason of each. The result has the rows from `offset`, at most `limit`.
 */
export function listLeftOut(filter: string, offset: number, limit: number): Promise<LeftOutList> {
  return call<LeftOutList>('list_left_out', { filter, offset, limit })
}

/** What the program does with a typed phrase. Always a dry run: nothing is sent. */
export function testPhrase(phrase: string): Promise<PhraseTest> {
  return call<PhraseTest>('test_phrase', { phrase })
}

/** Reads a word table. The program takes the path from the saved configuration. */
export function readTable(kind: TableKind, language: string | null): Promise<WordTable> {
  return call<WordTable>('read_table', { kind, language })
}

/** The problems and the warnings of `voicebridge check` for the rows, before a save. */
export function checkTable(
  kind: TableKind,
  language: string | null,
  rows: TableRow[],
): Promise<TableCheck> {
  return call<TableCheck>('check_table', { kind, language, rows })
}

/**
 * Saves the rows. The program checks them again and refuses a problem, a
 * warning that is not in `confirmed`, and a file that changed after the
 * read with `version`.
 */
export function saveTable(
  kind: TableKind,
  language: string | null,
  rows: TableRow[],
  confirmed: string[],
  version: string,
): Promise<TableSave> {
  return call<TableSave>('save_table', { kind, language, rows, confirmed, version })
}

/** A part of the diagnostics. The page sends this name, never a path. `data` is the data folder: it has no switch. */
export type DiagnosticsPart = 'log' | 'audio' | 'data'

/** `StartupNotesDto`: the texts of the start. */
export interface StartupNotes {
  /** The problem that stops the use of a configuration. */
  problem: string | null
  /** The error of a failed first start, with the data folder. */
  first_start_error: string | null
  /** The notes of the update step, for example the removal of the map keys of an older version. */
  notes: string[]
  /** The errors of the update step. The configuration did not change. */
  update_problems: string[]
  /** The data folder of the user, or null. */
  data_dir: string | null
}

export function startupNotes(): Promise<StartupNotes> {
  return call<StartupNotes>('startup_notes')
}

/** True one time after "Start..." of the tray menu. */
export function takeTrayStart(): Promise<boolean> {
  return call<boolean>('take_tray_start')
}

/**
 * Switches the utterance log or the audio capture on or off. The program
 * picks the path and saves with the rules of `saveConfig`.
 */
export function setDiagnostics(part: DiagnosticsPart, on: boolean): Promise<SaveResult> {
  return call<SaveResult>('set_diagnostics', { part, on })
}

/** Opens the folder of the part. The program takes the folder from the saved file. */
export function openFolder(part: DiagnosticsPart): Promise<void> {
  return call<void>('open_folder', { part })
}

export function updateStatus(): Promise<UpdateStatus> {
  return call<UpdateStatus>('update_status')
}

/**
 * Asks the update server for a newer version. It also works when
 * `updates.check` is false. A failure comes as the error text.
 */
export function checkForUpdates(): Promise<UpdateStatus> {
  return call<UpdateStatus>('check_for_updates')
}

/**
 * Starts the download of the offered update and returns at once. The steps
 * come as `update://event`. The program refuses it while a session runs.
 */
export function installUpdate(): Promise<void> {
  return call<void>('install_update')
}

export function cancelUpdate(): Promise<void> {
  return call<void>('cancel_update')
}

/** "Later": hides the note until the next start or manual check. */
export function hideUpdate(): Promise<void> {
  return call<void>('hide_update')
}

/** Calls `handler` for each change of the update state. The result removes the listener. */
export async function onUpdateEvent(handler: (status: UpdateStatus) => void): Promise<() => void> {
  try {
    return await listen<UpdateStatus>(UPDATE_EVENT, (event) => handler(event.payload))
  } catch (error) {
    throw toError(error)
  }
}

/** The name of the Tauri event of "Start..." in the tray menu. */
export const TRAY_START_EVENT = 'tray://start'

/** Calls `handler` when the user selects "Start..." in the tray menu. The result removes the listener. */
export async function onTrayStart(handler: () => void): Promise<() => void> {
  try {
    return await listen<null>(TRAY_START_EVENT, () => handler())
  } catch (error) {
    throw toError(error)
  }
}

/** The text for the local engine line: "yes" or "no". */
export function yesNo(value: boolean): string {
  return value ? 'yes' : 'no'
}
