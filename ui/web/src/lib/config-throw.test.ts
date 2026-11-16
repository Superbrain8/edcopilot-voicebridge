import { describe, expect, it, vi } from 'vitest'
import { get } from 'svelte/store'

// The value builders throw, as for a value that passed no page check.
vi.mock('./config-values', async (original) => {
  const real = await original<typeof import('./config-values')>()
  return {
    ...real,
    numberValue: () => {
      throw new Error('0.85 is not a finite number')
    },
  }
})

import { createConfigStore } from './config'
import { samples } from './fixtures/dto-samples'

describe('config store', () => {
  it('a thrown error in the edit build is shown and sends nothing', async () => {
    const saves: unknown[] = []
    const store = createConfigStore(
      {
        getConfig: async () => structuredClone(samples.config),
        saveConfig: async (edits) => {
          saves.push(edits)
          return { problems: [], written: true, restart_needed: false }
        },
        check: async () => ({ problems: [], warnings: [], report: null, profile: null, edcopilot_problems: [], info: [] }),
        setApiKey: async () => ({ restart_needed: false }),
        clearApiKey: async () => ({ restart_needed: false }),
      },
      { cores: 8 },
    )
    await store.init()
    store.set('matching.threshold', 0.85)
    await expect(store.save()).resolves.toBeUndefined()
    expect(saves).toEqual([])
    expect(get(store).saveError).toBe('0.85 is not a finite number')
    expect(get(store).saving).toBe(false)
  })
})
