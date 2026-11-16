import { beforeEach, describe, expect, it, vi } from 'vitest'

const invoke = vi.fn()
const listen = vi.fn()
vi.mock('@tauri-apps/api/core', () => ({ invoke: (...args: unknown[]) => invoke(...args) }))
vi.mock('@tauri-apps/api/event', () => ({ listen: (...args: unknown[]) => listen(...args) }))
const open = vi.fn()
vi.mock('@tauri-apps/plugin-dialog', () => ({ open: (...args: unknown[]) => open(...args) }))

import * as api from './api'

beforeEach(() => {
  invoke.mockReset()
  listen.mockReset()
  open.mockReset()
})

describe('pickFolder', () => {
  it('opens a folder dialog and gives the folder or null', async () => {
    open.mockResolvedValueOnce('D:\\Models\\My Model')
    expect(await api.pickFolder('Model folder', 'D:\\Models')).toBe('D:\\Models\\My Model')
    expect(open.mock.calls[0][0]).toEqual({
      title: 'Model folder',
      directory: true,
      multiple: false,
      defaultPath: 'D:\\Models',
    })
    open.mockResolvedValueOnce(null)
    expect(await api.pickFolder('Model folder', '')).toBeNull()
    expect(open.mock.calls[1][0].defaultPath).toBeUndefined()
  })

  it('a dialog error becomes an Error', async () => {
    open.mockRejectedValueOnce('dialog.open not allowed')
    const error = await api.pickFolder('x', '').catch((e: unknown) => e)
    expect((error as Error).message).toBe('dialog.open not allowed')
  })
})

describe('yesNo', () => {
  it('gives yes for true and no for false', () => {
    expect(api.yesNo(true)).toBe('yes')
    expect(api.yesNo(false)).toBe('no')
  })
})

describe('commands', () => {
  it('sends the command names and the camelCase arguments', async () => {
    invoke.mockResolvedValue(undefined)
    await api.appInfo()
    await api.getConfig()
    await api.saveConfig([{ key: 'stt.parakeet.threads', value: { kind: 'integer', value: 6 } }], 'v1')
    await api.check()
    await api.startSession(true)
    await api.stopSession()
    await api.sessionStatus()
    await api.listDevices()
    await api.captureBinding(7)
    await api.cancelCapture(7)
    await api.cancelCapture()
    await api.setApiKey('k', 'https://api.openai.com', 'OPENAI_API_KEY')
    await api.clearApiKey()
    expect(invoke.mock.calls).toEqual([
      ['app_info', undefined],
      ['get_config', undefined],
      ['save_config', { edits: [{ key: 'stt.parakeet.threads', value: { kind: 'integer', value: 6 } }], version: 'v1' }],
      ['check', undefined],
      ['start_session', { dryRun: true }],
      ['stop_session', undefined],
      ['session_status', undefined],
      ['list_devices', undefined],
      ['capture_binding', { token: 7 }],
      ['cancel_capture', { token: 7 }],
      ['cancel_capture', undefined],
      ['set_api_key', { key: 'k', expectedOrigin: 'https://api.openai.com', expectedName: 'OPENAI_API_KEY' }],
      ['clear_api_key', undefined],
    ])
  })

  it('sends the model download commands without arguments and listens to model://event', async () => {
    invoke.mockResolvedValue(undefined)
    await api.modelInfo()
    await api.downloadModel()
    await api.cancelModelDownload()
    expect(invoke.mock.calls).toEqual([
      ['model_info', undefined],
      ['download_model', undefined],
      ['cancel_model_download', undefined],
    ])
    listen.mockImplementation(async (_name: string, cb: (e: { payload: unknown }) => void) => {
      cb({ payload: { type: 'verifying' } })
      return () => {}
    })
    const seen: api.ModelEvent[] = []
    await api.onModelEvent((e) => seen.push(e))
    expect(listen.mock.calls[0][0]).toBe('model://event')
    expect(seen).toEqual([{ type: 'verifying' }])
  })

  it('sends the commands of the Commands screen with their arguments', async () => {
    invoke.mockResolvedValue(undefined)
    await api.listCommands('sail out', true, 200, 200)
    await api.testPhrase('help chart log')
    expect(invoke.mock.calls).toEqual([
      ['list_commands', { filter: 'sail out', all: true, offset: 200, limit: 200 }],
      ['test_phrase', { phrase: 'help chart log' }],
    ])
  })

  it('sends the diagnostics commands with a part name and listens to tray://start', async () => {
    invoke.mockResolvedValue(undefined)
    await api.setDiagnostics('log', true)
    await api.setDiagnostics('audio', false)
    await api.openFolder('audio')
    await api.openFolder('data')
    await api.startupNotes()
    await api.takeTrayStart()
    expect(invoke.mock.calls).toEqual([
      ['set_diagnostics', { part: 'log', on: true }],
      ['set_diagnostics', { part: 'audio', on: false }],
      ['open_folder', { part: 'audio' }],
      ['open_folder', { part: 'data' }],
      ['startup_notes', undefined],
      ['take_tray_start', undefined],
    ])
    listen.mockImplementation(async (_name: string, cb: (e: { payload: unknown }) => void) => {
      cb({ payload: null })
      return () => {}
    })
    let starts = 0
    await api.onTrayStart(() => (starts += 1))
    expect(listen.mock.calls[0][0]).toBe('tray://start')
    expect(starts).toBe(1)
  })

  it('a rejection with a text becomes an Error with that text', async () => {
    invoke.mockRejectedValue('a session is already running')
    const error = await api.startSession(false).catch((e: unknown) => e)
    expect(error).toBeInstanceOf(Error)
    expect((error as Error).message).toBe('a session is already running')
  })

  it('onSessionEvent gives the payload to the handler', async () => {
    const unlisten = vi.fn()
    listen.mockImplementation(async (_name: string, cb: (e: { payload: unknown }) => void) => {
      cb({ payload: { type: 'talk', down: true } })
      return unlisten
    })
    const seen: api.SessionEvent[] = []
    const remove = await api.onSessionEvent((e) => seen.push(e))
    expect(listen.mock.calls[0][0]).toBe('session://event')
    expect(seen).toEqual([{ type: 'talk', down: true }])
    remove()
    expect(unlisten).toHaveBeenCalled()
  })
})
