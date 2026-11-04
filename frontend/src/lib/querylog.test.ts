import { describe, expect, it } from 'vitest';
import { buildQueryLogQuery, timeRangeToQuery } from './querylog';
import { parseLocalDateTime, toLocalInputValue } from './timeInput';

const MINUTE = 60_000;
const HOUR = 60 * MINUTE;
const DAY = 24 * HOUR;

describe('timeRangeToQuery', () => {
  // Chosen so that flooring to the minute changes nothing in the presets below.
  const now = Date.UTC(2026, 10, 1, 12, 0, 0);

  const preset = (value: 'hour' | 'day' | 'week' | 'month') =>
    timeRangeToQuery({ preset: value, customFrom: '', customTo: '' }, now);

  it('looks back from now for each preset, with no upper bound', () => {
    expect(preset('hour')).toEqual({ from: now - HOUR });
    expect(preset('day')).toEqual({ from: now - DAY });
    expect(preset('week')).toEqual({ from: now - 7 * DAY });
    expect(preset('month')).toEqual({ from: now - 30 * DAY });
  });

  it('rounds now down to the minute so the query key stays stable between renders', () => {
    const later = now + 42_000;
    expect(timeRangeToQuery({ preset: 'hour', customFrom: '', customTo: '' }, later)).toEqual({
      from: now - HOUR,
    });
  });

  it('uses the custom start and end when both are given', () => {
    const from = parseLocalDateTime('2026-11-01T08:00');
    const to = parseLocalDateTime('2026-11-01T09:30');
    expect(
      timeRangeToQuery({ preset: 'custom', customFrom: '2026-11-01T08:00', customTo: '2026-11-01T09:30' }, now),
    ).toEqual({ from, to });
  });

  it('allows a custom range with only one side', () => {
    const from = parseLocalDateTime('2026-11-01T08:00');
    expect(timeRangeToQuery({ preset: 'custom', customFrom: '2026-11-01T08:00', customTo: '' }, now)).toEqual({
      from,
      to: undefined,
    });
    expect(timeRangeToQuery({ preset: 'custom', customFrom: '', customTo: '2026-11-01T08:00' }, now)).toEqual({
      from: undefined,
      to: from,
    });
  });

  it('reports an error for an invalid or reversed custom range', () => {
    expect(timeRangeToQuery({ preset: 'custom', customFrom: 'yesterday', customTo: '' }, now).error).toBe(
      'Enter a valid start date and time.',
    );
    expect(timeRangeToQuery({ preset: 'custom', customFrom: '', customTo: '2026-02-31T10:00' }, now).error).toBe(
      'Enter a valid end date and time.',
    );
    expect(
      timeRangeToQuery({ preset: 'custom', customFrom: '2026-11-02T08:00', customTo: '2026-11-01T08:00' }, now).error,
    ).toBe('The start must be before the end.');
  });
});

describe('parseLocalDateTime and toLocalInputValue', () => {
  it('round-trips to the minute', () => {
    const ms = new Date(2026, 10, 1, 14, 5, 40).getTime();
    expect(toLocalInputValue(ms)).toBe('2026-11-01T14:05');
    expect(parseLocalDateTime(toLocalInputValue(ms))).toBe(new Date(2026, 10, 1, 14, 5).getTime());
  });

  it('accepts optional seconds and rejects non-dates', () => {
    expect(parseLocalDateTime('2026-11-01T14:05:09')).toBe(new Date(2026, 10, 1, 14, 5, 9).getTime());
    expect(parseLocalDateTime('2026-13-01T10:00')).toBeNull();
    expect(parseLocalDateTime('')).toBeNull();
  });
});

describe('buildQueryLogQuery', () => {
  it('builds paging parameters and omits offset on the first page', () => {
    expect(buildQueryLogQuery({ limit: 50, offset: 0 })).toBe('limit=50');
    expect(buildQueryLogQuery({ limit: 50, offset: 100 })).toBe('limit=50&offset=100');
    expect(buildQueryLogQuery({})).toBe('');
  });

  it('includes each filter that has a value', () => {
    expect(
      buildQueryLogQuery({
        limit: 100,
        offset: 0,
        domain: ' ads.example.com ',
        client: '10.0.0.7',
        result: 'blocked',
        from: 1_700_000_000_000,
        to: 1_700_003_600_000,
      }),
    ).toBe('limit=100&domain=ads.example.com&client=10.0.0.7&result=blocked&from=1700000000000&to=1700003600000');
  });

  it('leaves out blank text filters and the all-results value', () => {
    expect(buildQueryLogQuery({ limit: 50, domain: '   ', client: '', result: '' })).toBe('limit=50');
  });

  it('keeps a zero lower time bound, which is a real value', () => {
    expect(buildQueryLogQuery({ from: 0 })).toBe('from=0');
  });
});
