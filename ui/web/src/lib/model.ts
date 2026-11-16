// The model download store: the state of the download of the local model.
//
// The Rust side does all the work: it gets the archive from a fixed address,
// checks it, unpacks it, and sets the model folder in the configuration. The
// page only starts and cancels the download and shows the steps of
// `model://event`.

import { writable, type Readable } from 'svelte/store'
import * as api from './api'
import type { ModelEvent, ModelInfo } from './api'
import { configStore } from './config'

/** The functions of `api.ts` that the store uses. Tests give a fake. */
export interface ModelApi {
  modelInfo(): Promise<ModelInfo>
  downloadModel(): Promise<void>
  cancelModelDownload(): Promise<void>
  onModelEvent(handler: (event: ModelEvent) => void): Promise<() => void>
}

export type ModelPhase =
  | 'idle'
  | 'starting'
  | 'downloading'
  | 'verifying'
  | 'unpacking'
  | 'done'
  | 'failed'
  | 'cancelled'

export interface ModelState {
  /** The model that the download gets. Null before the first read. */
  info: ModelInfo | null
  phase: ModelPhase
  /** The bytes of the archive on the disk, and the size of the archive. */
  done: number
  total: number
  /** The error text of a failed download. */
  message: string | null
  /** The model folder after `done`. */
  modelDir: string | null
  /** The error of the save of the model folder, after `done`. */
  configError: string | null
  /** True from the click on Cancel until the download ends. */
  cancelling: boolean
}

export function initialModelState(): ModelState {
  return {
    info: null,
    phase: 'idle',
    done: 0,
    total: 0,
    message: null,
    modelDir: null,
    configError: null,
    cancelling: false,
  }
}

/** True while a download runs. The button is off then. */
export function busy(s: Pick<ModelState, 'phase'>): boolean {
  return ['starting', 'downloading', 'verifying', 'unpacking'].includes(s.phase)
}

/** The part of the archive on the disk, from 0 to 100. */
export function percent(s: Pick<ModelState, 'done' | 'total'>): number {
  if (s.total <= 0) return 0
  return Math.min(100, Math.floor((s.done * 100) / s.total))
}

/** A size in whole megabytes (1 MB = 1,000,000 bytes). */
export function megabytes(bytes: number): number {
  return Math.round(bytes / 1_000_000)
}

/** For example "100 of 400 MB (25 %)". */
export function progressText(s: Pick<ModelState, 'done' | 'total'>): string {
  return `${megabytes(s.done)} of ${megabytes(s.total)} MB (${percent(s)} %)`
}

export interface ModelStore extends Readable<ModelState> {
  /** Listens to the events and reads the model info. */
  init(): Promise<void>
  /** Starts the download. Never rejects. */
  start(): Promise<void>
  /** Asks the program to stop the download. */
  cancel(): Promise<void>
  /** Waits until the work after `done` is complete. For the tests. */
  settled(): Promise<void>
  /** Removes the listener. */
  destroy(): void
}

/** The text when the program cannot find the per-user data folder. */
export const NO_DATA_DIR =
  'The program cannot find your data folder (APPDATA), so it cannot download the model. Download the model by hand and choose its folder with Browse.'

/** Why the Download button is off, or null. */
export function downloadBlock(s: Pick<ModelState, 'info'>): string | null {
  if (s.info === null) return 'Reading the model information.'
  if (s.info.models_dir === null) return NO_DATA_DIR
  return null
}

/**
 * `onDone` gets the new model folder, and true when a running session keeps
 * its old model. The window gives the configuration store.
 */
export function createModelStore(
  source: ModelApi,
  onDone: (modelDir: string, restartNeeded: boolean) => Promise<void>,
): ModelStore {
  const store = writable<ModelState>(initialModelState())
  let state = initialModelState()
  store.subscribe((s) => (state = s))
  let unlisten: (() => void) | null = null
  // The listen in work. `start` waits for it, so that no event is lost.
  let listening: Promise<void> | null = null
  // The number of the newest `init` or `destroy`. A `listen` result of an
  // older number is removed at once.
  let epoch = 0
  let after: Promise<void> = Promise.resolve()

  function handle(event: ModelEvent): void {
    switch (event.type) {
      case 'progress':
        store.update((s) => ({ ...s, phase: 'downloading', done: event.done, total: event.total }))
        return
      case 'verifying':
      case 'unpacking':
        store.update((s) => ({ ...s, phase: event.type }))
        return
      case 'done':
        store.update((s) => ({
          ...s,
          phase: 'done',
          modelDir: event.model_dir,
          configError: event.config_error,
          message: null,
          cancelling: false,
        }))
        after = onDone(event.model_dir, event.restart_needed).catch(() => {})
        return
      case 'failed':
        store.update((s) => ({ ...s, phase: 'failed', message: event.message, cancelling: false }))
        return
      case 'cancelled':
        store.update((s) => ({ ...s, phase: 'cancelled', message: null, cancelling: false }))
        return
    }
  }

  function listen(): Promise<void> {
    if (unlisten) return Promise.resolve()
    if (listening) return listening
    const mine = ++epoch
    listening = source
      .onModelEvent(handle)
      .then((remove) => {
        if (mine !== epoch) {
          // A `destroy` came during the wait.
          remove()
          return
        }
        unlisten = remove
      })
      .finally(() => {
        if (mine === epoch) listening = null
      })
    return listening
  }

  async function init(): Promise<void> {
    try {
      await listen()
      const info = await source.modelInfo()
      store.update((s) => {
        const next = { ...s, info }
        if (!info.running || busy(s)) return next
        // A download of an earlier page load: show its last step.
        if (info.step === null) return { ...next, phase: 'starting' }
        if (info.step.type === 'progress') {
          return { ...next, phase: 'downloading', done: info.step.done, total: info.step.total }
        }
        return { ...next, phase: info.step.type }
      })
    } catch (error) {
      store.update((s) => ({ ...s, message: api.toError(error).message }))
    }
  }

  async function start(): Promise<void> {
    if (busy(state)) return
    store.update((s) => ({
      ...s,
      phase: 'starting',
      done: 0,
      total: s.info?.archive_bytes ?? 0,
      message: null,
      modelDir: null,
      configError: null,
      cancelling: false,
    }))
    try {
      // The listener must be in place before the first event.
      await listen()
      await source.downloadModel()
    } catch (error) {
      store.update((s) => ({ ...s, phase: 'failed', message: api.toError(error).message }))
    }
  }

  async function cancel(): Promise<void> {
    if (!busy(state)) return
    store.update((s) => ({ ...s, cancelling: true }))
    try {
      await source.cancelModelDownload()
    } catch (error) {
      store.update((s) => ({ ...s, cancelling: false, message: api.toError(error).message }))
    }
  }

  return {
    subscribe: store.subscribe,
    init,
    start,
    cancel,
    settled: () => after,
    destroy: () => {
      epoch += 1
      listening = null
      unlisten?.()
      unlisten = null
    },
  }
}

/** The store of the window, with the real commands and the configuration store. */
export const modelStore: ModelStore = createModelStore(api, (dir, restartNeeded) =>
  configStore.useModelFolder(dir, restartNeeded),
)
