<script lang="ts">
  import Glyph, { type Shape } from './Glyph.svelte'
  import type { ResultRow } from '../session'
  import {
    AUDIO_MARKS,
    MARK_LABEL,
    OUTCOME_LABEL,
    OUTCOME_TONE,
    clock,
    percent,
    type Tone,
  } from '../format'

  let { row }: { row: ResultRow } = $props()

  const SHAPE: Record<Tone, Shape> = {
    sent: 'dot',
    dry: 'ring',
    miss: 'dash',
    rejected: 'cross',
    unread: 'triangle',
    error: 'diamond',
    dropped: 'square',
  }

  let outcome = $derived(row.kind === 'utterance' ? row.utterance.outcome : null)
  let tone: Tone = $derived(outcome ? OUTCOME_TONE[outcome.kind] : 'dropped')
  let label = $derived(outcome ? OUTCOME_LABEL[outcome.kind] : 'Dropped')
</script>

<!-- All values from the program are text. Svelte escapes them. -->
<li class="row tone-{tone}" data-kind={outcome?.kind ?? 'dropped'}>
  <span class="kind">
    <Glyph shape={SHAPE[tone]} />
    <span class="kind-word">{label}</span>
  </span>

  {#if outcome}
    <span class="heard" class:none={!outcome.text}>{outcome.text || 'nothing heard'}</span>
    <span class="conf">{percent(outcome.confidence)}</span>
    <span class="detail">
      {#if outcome.token}<span class="token">{outcome.token}</span>{/if}
      {#each outcome.marks as mark (mark)}
        <span class="mark" class:audio={AUDIO_MARKS.has(mark)}>{MARK_LABEL[mark] ?? mark}</span>
      {/each}
    </span>
    {#if outcome.reason}
      <span class="reason">{outcome.reason}</span>
    {/if}
  {:else if row.kind === 'dropped'}
    <span class="heard dropped-text">{row.reason}</span>
  {/if}
  <time class="time">{clock(row.at)}</time>
</li>

<style>
  .row {
    display: grid;
    grid-template-columns: 7.5rem minmax(0, 1fr) auto;
    grid-template-areas:
      'kind heard conf'
      'kind detail time'
      'kind reason reason';
    column-gap: var(--gap);
    row-gap: 1px;
    padding: 7px 12px 7px 10px;
    border-left: 4px solid var(--tone);
    background: var(--panel);
    color: var(--text);
  }

  .row + :global(.row) {
    margin-top: 2px;
  }

  .tone-sent { --tone: var(--tone-sent); }
  .tone-dry { --tone: var(--tone-dry); }
  .tone-miss { --tone: var(--line); }
  .tone-rejected { --tone: var(--tone-rejected); }
  .tone-unread { --tone: var(--tone-unread); }
  .tone-error { --tone: var(--tone-error); }
  .tone-dropped { --tone: transparent; }

  .kind {
    grid-area: kind;
    display: flex;
    align-items: center;
    align-self: start;
    gap: 7px;
    padding-top: 3px;
    font-size: var(--step--1);
    font-weight: 600;
    color: var(--tone);
  }

  .tone-miss .kind { color: var(--tone-miss); }
  .tone-dropped .kind { color: var(--tone-dropped); }

  .heard {
    grid-area: heard;
    font-family: var(--font-display);
    font-size: var(--step-2);
    font-weight: 500;
    line-height: 1.25;
    overflow-wrap: anywhere;
  }

  .heard.none {
    color: var(--muted);
    font-style: italic;
  }

  .tone-miss .heard {
    color: var(--muted);
  }

  .conf {
    grid-area: conf;
    align-self: baseline;
    font-size: var(--step-1);
    font-weight: 600;
    font-variant-numeric: tabular-nums;
    color: var(--text);
  }

  .detail {
    grid-area: detail;
    display: flex;
    flex-wrap: wrap;
    align-items: baseline;
    gap: 4px 10px;
    min-width: 0;
    font-size: var(--step-0);
    color: var(--muted);
  }

  .token {
    color: var(--text);
    font-weight: 600;
    overflow-wrap: anywhere;
  }

  .mark {
    padding: 0 6px;
    border: 1px solid var(--line);
    border-radius: var(--radius-s);
    font-size: var(--step--1);
    color: var(--muted);
  }

  .mark.audio {
    border-style: dashed;
    color: var(--tone-dry);
  }

  .reason {
    grid-area: reason;
    font-size: var(--step-0);
    color: var(--tone);
    overflow-wrap: anywhere;
  }

  .tone-miss .reason,
  .tone-dropped .reason {
    color: var(--muted);
  }

  .time {
    grid-area: time;
    align-self: baseline;
    font-size: var(--step--1);
    font-variant-numeric: tabular-nums;
    color: var(--faint);
  }

  /* A dropped utterance is one quiet line. */
  .tone-dropped {
    grid-template-areas: 'kind heard time';
    background: transparent;
    padding-top: 3px;
    padding-bottom: 3px;
  }

  .dropped-text {
    font-family: var(--font);
    font-size: var(--step-0);
    font-weight: 400;
    color: var(--muted);
  }

  .tone-dropped .time {
    align-self: center;
  }

  /* A narrow window: the outcome word goes above the text. */
  @media (max-width: 560px) {
    .row {
      grid-template-columns: minmax(0, 1fr) auto;
      grid-template-areas:
        'kind time'
        'heard conf'
        'detail detail'
        'reason reason';
    }

    .kind {
      padding-top: 0;
    }

    .tone-dropped {
      grid-template-areas:
        'kind time'
        'heard heard';
    }
  }
</style>
