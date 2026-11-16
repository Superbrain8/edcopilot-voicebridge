import { describe, expect, it } from 'vitest'
import { edit, integerValue, numberValue } from './config-values'

describe('config values', () => {
  it('an integer value must be a whole number', () => {
    expect(integerValue(6)).toEqual({ kind: 'integer', value: 6 })
    expect(integerValue(-2)).toEqual({ kind: 'integer', value: -2 })
    expect(() => integerValue(6.5)).toThrow('a whole number')
    expect(() => integerValue(Number.NaN)).toThrow('a whole number')
    expect(() => integerValue(2 ** 60)).toThrow('a whole number')
  })

  it('a number value must be finite', () => {
    expect(numberValue(0.85)).toEqual({ kind: 'number', value: 0.85 })
    expect(() => numberValue(Number.POSITIVE_INFINITY)).toThrow('a finite number')
  })

  it('edit builds the JSON of EditDto', () => {
    expect(edit('stt.parakeet.threads', integerValue(6))).toEqual({
      key: 'stt.parakeet.threads',
      value: { kind: 'integer', value: 6 },
    })
    expect(edit('push_to_talk.joystick', { kind: 'absent' })).toEqual({
      key: 'push_to_talk.joystick',
      value: { kind: 'absent' },
    })
  })
})
