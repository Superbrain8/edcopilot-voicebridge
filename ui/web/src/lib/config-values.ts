// Builders for the edits of `save_config`. The Rust side reads an
// `integer` value as i64, so a value of that kind must be a whole number.

import type { ConfigEdit, ConfigValue } from './api'

/** An `integer` value. A value that is not a whole safe integer is refused. */
export function integerValue(value: number): ConfigValue {
  if (!Number.isSafeInteger(value)) {
    throw new Error(`${value} is not a whole number`)
  }
  return { kind: 'integer', value }
}

/** A `number` value. NaN and infinite values are refused. */
export function numberValue(value: number): ConfigValue {
  if (!Number.isFinite(value)) {
    throw new Error(`${value} is not a finite number`)
  }
  return { kind: 'number', value }
}

/** One edit, for example `edit('stt.parakeet.threads', integerValue(6))`. */
export function edit(key: string, value: ConfigValue): ConfigEdit {
  return { key, value }
}
