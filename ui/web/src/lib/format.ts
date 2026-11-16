// Texts of the window for the values of the session.

import type { Mark, Outcome, OutcomeKind, StopReason } from './api'
import type { SessionState } from './session'

/**
 * The text as a sentence: a full stop at the end when the text has no end
 * mark. An error text of the program often has no full stop, and the screen
 * puts another sentence after it.
 */
export function sentence(text: string): string {
  const trimmed = text.trim()
  if (trimmed === '' || /[.!?]$/.test(trimmed)) return trimmed
  return `${trimmed}.`
}

/** The look of a result row. Each tone has a color, a shape, and a word. */
export type Tone = 'sent' | 'dry' | 'miss' | 'rejected' | 'unread' | 'error' | 'dropped'

export const OUTCOME_LABEL: Record<OutcomeKind, string> = {
  sent: 'Sent',
  dry_run: 'Dry run',
  no_match: 'No match',
  no_wake_word: 'No wake word',
  rejected: 'Rejected',
  not_consumed: 'Not read',
  engine_error: 'Engine error',
  send_error: 'Send error',
}

export const OUTCOME_TONE: Record<OutcomeKind, Tone> = {
  sent: 'sent',
  dry_run: 'dry',
  no_match: 'miss',
  no_wake_word: 'miss',
  rejected: 'rejected',
  not_consumed: 'unread',
  engine_error: 'error',
  send_error: 'error',
}

export const MARK_LABEL: Record<Mark, string> = {
  sound_alike: 'sound-alike',
  synonym: 'synonym',
  filler: 'filler',
  hotwords: 'hotwords',
  glitch: 'glitch',
  clipping: 'clipping',
  very_quiet: 'very quiet',
}

/** Marks that tell about a problem of the audio. */
export const AUDIO_MARKS: ReadonlySet<Mark> = new Set<Mark>(['glitch', 'clipping', 'very_quiet'])

/** The sentence under the result of a phrase test. It says what the program would do. */
export function outcomeMeaning(outcome: Pick<Outcome, 'kind' | 'blocked'>): string {
  switch (outcome.kind) {
    case 'dry_run':
      return 'The program would send this command. The test sends nothing.'
    case 'rejected':
      return 'The program would not send this command.'
    case 'no_match':
      if (outcome.blocked === 'no_token') {
        return 'The phrase is also a phrase of a command that the program cannot send. The program would send nothing.'
      }
      return 'No command matches this phrase. The program would send nothing.'
    case 'no_wake_word':
      return 'The phrase has no wake word. The program would send nothing.'
    default:
      return 'The program would send nothing.'
  }
}

/** A confidence of 0.95 gives "95%". */
export function percent(confidence: number | null): string {
  if (confidence === null || !Number.isFinite(confidence)) return ''
  return `${Math.round(confidence * 100)}%`
}

/** The time of day as 14:02:09. */
export function clock(at: number): string {
  const d = new Date(at)
  const two = (n: number) => String(n).padStart(2, '0')
  return `${two(d.getHours())}:${two(d.getMinutes())}:${two(d.getSeconds())}`
}

const STOP_LABEL: Record<StopReason['kind'], string> = {
  requested: 'Stopped',
  device_error: 'Microphone error',
  engine_error: 'Engine error',
  // The audio input or the control of the session failed.
  input_error: 'Audio input error',
}

/** The text of a stop reason. Null for a stop that the user asked for. */
export function stopText(reason: StopReason | null): string | null {
  if (!reason || reason.kind === 'requested') return null
  const label = STOP_LABEL[reason.kind] ?? 'Error'
  return reason.message ? `${label}: ${reason.message}` : label
}

/** The phase of the session for the status bar. */
export type Phase = 'loading' | 'starting' | 'stopping' | 'running' | 'stopped' | 'failed'

export function phaseOf(s: SessionState): Phase {
  if (!s.loaded) return 'loading'
  if (s.stopping) return 'stopping'
  if (s.starting) return 'starting'
  if (s.running) return 'running'
  if (stopText(s.stopReason) !== null) return 'failed'
  return 'stopped'
}

export const PHASE_LABEL: Record<Phase, string> = {
  loading: 'Reading status',
  starting: 'Starting',
  stopping: 'Stopping',
  running: 'Running',
  stopped: 'Stopped',
  failed: 'Stopped by error',
}

/** The word of the phase. A start in work names the dry run when it is known. */
export function phaseLabel(s: SessionState): string {
  const phase = phaseOf(s)
  if (phase === 'starting' && s.startingDryRun === true) return 'Starting, dry run'
  return PHASE_LABEL[phase]
}
