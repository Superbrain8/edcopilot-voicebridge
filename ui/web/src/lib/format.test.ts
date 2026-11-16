import { describe, expect, it } from 'vitest'
import { outcomeMeaning, percent, phaseLabel, phaseOf, sentence, stopText } from './format'
import { samples } from './fixtures/dto-samples'
import { initialState } from './session'

describe('format', () => {
  it('sentence ends a text with a full stop when it has no end mark', () => {
    expect(sentence('the file is locked')).toBe('the file is locked.')
    expect(sentence('the file is locked.')).toBe('the file is locked.')
    expect(sentence('really?')).toBe('really?')
    expect(sentence('stop!')).toBe('stop!')
    expect(sentence('  the end  ')).toBe('the end.')
    expect(sentence('')).toBe('')
  })

  it('percent rounds the confidence', () => {
    expect(percent(0.95)).toBe('95%')
    expect(percent(0.805)).toBe('81%')
    expect(percent(null)).toBe('')
  })

  it('stopText names the error and gives nothing for a requested stop', () => {
    expect(stopText({ kind: 'requested', message: null })).toBeNull()
    expect(stopText(null)).toBeNull()
    expect(stopText({ kind: 'device_error', message: 'device removed' })).toBe(
      'Microphone error: device removed',
    )
  })

  it('stopText calls an input error an audio input error', () => {
    expect(stopText({ kind: 'input_error', message: 'x' })).toBe('Audio input error: x')
  })

  it('phaseLabel names the mode of a start in work', () => {
    const s = { ...initialState(), loaded: true, starting: true }
    expect(phaseLabel({ ...s, startingDryRun: true })).toBe('Starting, dry run')
    expect(phaseLabel({ ...s, startingDryRun: false })).toBe('Starting')
    expect(phaseLabel({ ...s, startingDryRun: null })).toBe('Starting')
    expect(phaseLabel({ ...s, starting: false })).toBe('Stopped')
  })

  it('phaseOf gives the phase in the order of priority', () => {
    const s = { ...initialState(), loaded: true }
    expect(phaseOf({ ...s, loaded: false })).toBe('loading')
    expect(phaseOf(s)).toBe('stopped')
    expect(phaseOf({ ...s, starting: true })).toBe('starting')
    expect(phaseOf({ ...s, running: true })).toBe('running')
    expect(phaseOf({ ...s, running: true, stopping: true })).toBe('stopping')
    expect(phaseOf({ ...s, stopReason: { kind: 'engine_error', message: 'x' } })).toBe('failed')
  })
})

describe('outcomeMeaning', () => {
  it('says why a phrase of a command that the program cannot send gives nothing', () => {
    const blocked = samples.phrase_test.no_match_blocked.outcome
    expect(blocked.blocked).toBe('no_token')
    expect(blocked.reason).toBe('blocked: the text is a phrase of a command that the program cannot send')
    expect(outcomeMeaning(blocked)).toBe(
      'The phrase is also a phrase of a command that the program cannot send. The program would send nothing.',
    )
  })

  it('keeps the no-match sentence for the other causes and for no candidate', () => {
    const plain = 'No command matches this phrase. The program would send nothing.'
    expect(outcomeMeaning(samples.phrase_test.no_match_tie.outcome)).toBe(plain)
    expect(outcomeMeaning(samples.phrase_test.no_match.outcome)).toBe(plain)
    for (const blocked of ['tie', 'required_words', 'forms', 'dropped_word'] as const) {
      expect(outcomeMeaning({ kind: 'no_match', blocked })).toBe(plain)
    }
  })

  it('gives the sentence of each other kind', () => {
    expect(outcomeMeaning(samples.phrase_test.dry_run.outcome)).toBe(
      'The program would send this command. The test sends nothing.',
    )
    expect(outcomeMeaning(samples.phrase_test.rejected.outcome)).toBe('The program would not send this command.')
    expect(outcomeMeaning(samples.phrase_test.no_wake_word.outcome)).toBe(
      'The phrase has no wake word. The program would send nothing.',
    )
  })
})
