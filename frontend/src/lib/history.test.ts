import { describe, expect, it } from 'vitest';
import {
  MAX_HISTORY_POINTS,
  bucketLabel,
  bucketSeconds,
  formatBucketTick,
  historyParams,
  validateHistoryRange,
} from './history';

describe('bucketLabel', () => {
  it('names each bucket size', () => {
    expect(bucketLabel('minute')).toBe('Per minute');
    expect(bucketLabel('hour')).toBe('Per hour');
    expect(bucketLabel('day')).toBe('Per day');
  });
});

describe('bucketSeconds', () => {
  it('gives the length of each bucket in seconds', () => {
    expect(bucketSeconds('minute')).toBe(60);
    expect(bucketSeconds('hour')).toBe(3_600);
    expect(bucketSeconds('day')).toBe(86_400);
  });
});

describe('validateHistoryRange', () => {
  const hour = 3_600_000;

  it('accepts a window that is drawable at the chosen bucket', () => {
    expect(validateHistoryRange(0, 7 * 24 * hour, 'hour')).toBeNull();
  });

  it('rejects a window that ends before it starts or has no length', () => {
    expect(validateHistoryRange(2 * hour, hour, 'hour')).toBe('The start must be before the end.');
    expect(validateHistoryRange(hour, hour, 'hour')).toBe('The start must be before the end.');
  });

  it('rejects windows that would draw too many points', () => {
    // One year at minute resolution is far more points than the chart can draw.
    const year = 365 * 24 * hour;
    const message = validateHistoryRange(0, year, 'minute');
    expect(message).toContain('limit 5,000');
    expect(MAX_HISTORY_POINTS).toBe(5_000);
  });
});

describe('historyParams', () => {
  it('turns valid inputs into unix ms request params', () => {
    const result = historyParams('2026-11-01T00:00', '2026-11-02T00:00', 'day');
    expect(result.error).toBeNull();
    expect(result.params).toEqual({
      from: new Date(2026, 10, 1).getTime(),
      to: new Date(2026, 10, 2).getTime(),
      bucket: 'day',
    });
  });

  it('reports the field that is not a valid date', () => {
    expect(historyParams('', '2026-11-02T00:00', 'hour')).toEqual({
      params: null,
      error: 'Enter a valid start date and time.',
    });
    expect(historyParams('2026-11-01T00:00', 'nope', 'hour')).toEqual({
      params: null,
      error: 'Enter a valid end date and time.',
    });
  });

  it('passes through the range check', () => {
    expect(historyParams('2026-11-02T00:00', '2026-11-01T00:00', 'hour').error).toBe(
      'The start must be before the end.',
    );
  });
});

describe('formatBucketTick', () => {
  it('shows month, day and time for minute and hour buckets', () => {
    const seconds = new Date(2026, 10, 1, 14, 5).getTime() / 1000;
    expect(formatBucketTick(seconds, 'hour')).toBe('11-01 14:05');
    expect(formatBucketTick(seconds, 'minute')).toBe('11-01 14:05');
  });

  it('shows the full date for day buckets', () => {
    const seconds = new Date(2026, 10, 1, 14, 5).getTime() / 1000;
    expect(formatBucketTick(seconds, 'day')).toBe('2026-11-01');
  });

  it('falls back for a value that is not a time', () => {
    expect(formatBucketTick(Number.NaN, 'hour')).toBe('-');
  });
});
