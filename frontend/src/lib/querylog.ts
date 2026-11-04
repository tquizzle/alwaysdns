import type { QueryLogParams, ResultFilter } from '../api/types';
import { parseLocalDateTime } from './timeInput';

export const RESULT_FILTERS: { value: ResultFilter; label: string }[] = [
  { value: '', label: 'All results' },
  { value: 'blocked', label: 'Blocked' },
  { value: 'cached', label: 'Cached' },
  { value: 'forwarded', label: 'Forwarded' },
  { value: 'allowed', label: 'Allowed' },
  { value: 'rewritten', label: 'Rewritten' },
  { value: 'error', label: 'Error' },
];

export type TimePreset = 'hour' | 'day' | 'week' | 'month' | 'custom';

export const TIME_PRESETS: { value: TimePreset; label: string }[] = [
  { value: 'hour', label: 'Last hour' },
  { value: 'day', label: 'Last day' },
  { value: 'week', label: 'Last week' },
  { value: 'month', label: 'Last month' },
  { value: 'custom', label: 'Custom' },
];

const MINUTE_MS = 60_000;
const PRESET_MS: Record<Exclude<TimePreset, 'custom'>, number> = {
  hour: 60 * MINUTE_MS,
  day: 24 * 60 * MINUTE_MS,
  week: 7 * 24 * 60 * MINUTE_MS,
  month: 30 * 24 * 60 * MINUTE_MS,
};

export interface TimeRangeInput {
  preset: TimePreset;
  /** datetime-local text, used only for the custom preset. */
  customFrom: string;
  customTo: string;
}

export interface TimeBounds {
  from?: number;
  to?: number;
  /** Set when the custom range cannot be queried. */
  error?: string;
}

/**
 * Converts the time range control into unix ms bounds. Presets look back from
 * `now`, rounded down to the minute so the query key (and live refresh) stays
 * stable between renders. Custom bounds come from the datetime-local inputs.
 */
export function timeRangeToQuery(input: TimeRangeInput, now: number): TimeBounds {
  if (input.preset !== 'custom') {
    const end = Math.floor(now / MINUTE_MS) * MINUTE_MS;
    return { from: end - PRESET_MS[input.preset] };
  }

  const fromText = input.customFrom.trim();
  const toText = input.customTo.trim();
  const from = fromText === '' ? undefined : parseLocalDateTime(fromText) ?? undefined;
  const to = toText === '' ? undefined : parseLocalDateTime(toText) ?? undefined;

  if (fromText !== '' && from === undefined) return { error: 'Enter a valid start date and time.' };
  if (toText !== '' && to === undefined) return { error: 'Enter a valid end date and time.' };
  if (from !== undefined && to !== undefined && from > to) {
    return { error: 'The start must be before the end.' };
  }
  return { from, to };
}

/**
 * Builds the query string for GET /api/querylog. Empty filters are left out,
 * and offset is left out when it is zero so the first page has no offset.
 */
export function buildQueryLogQuery(params: QueryLogParams): string {
  const qs = new URLSearchParams();
  if (params.limit !== undefined) qs.set('limit', String(params.limit));
  if (params.offset !== undefined && params.offset > 0) qs.set('offset', String(params.offset));

  const domain = params.domain?.trim();
  if (domain) qs.set('domain', domain);

  const client = params.client?.trim();
  if (client) qs.set('client', client);

  if (params.result) qs.set('result', params.result);
  if (params.from !== undefined) qs.set('from', String(params.from));
  if (params.to !== undefined) qs.set('to', String(params.to));

  return qs.toString();
}
