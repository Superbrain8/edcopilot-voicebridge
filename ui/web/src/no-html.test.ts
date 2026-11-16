import { describe, expect, it } from 'vitest'

// The texts of the program (heard text, token, reason, problems, warnings)
// must always show as text. This test fails when a file inserts HTML.
const sources = import.meta.glob<string>(['./**/*.svelte', './**/*.ts', '!./**/*.test.ts'], {
  query: '?raw',
  import: 'default',
  eager: true,
})

const FORBIDDEN = ['{@html', 'innerHTML', 'outerHTML', 'insertAdjacentHTML']

describe('sources', () => {
  it('no component or module inserts HTML', () => {
    const names = Object.keys(sources)
    expect(names.filter((n) => n.endsWith('.svelte')).length).toBeGreaterThanOrEqual(5)
    expect(names.filter((n) => n.endsWith('.ts')).length).toBeGreaterThanOrEqual(5)
    for (const name of names) {
      for (const pattern of FORBIDDEN) {
        expect(sources[name], `${name} has ${pattern}`).not.toContain(pattern)
      }
    }
  })
})
