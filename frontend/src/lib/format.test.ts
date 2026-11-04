import { describe, expect, it } from 'vitest';
import {
  clamp01,
  formatCompact,
  formatDateTime,
  formatInt,
  formatMs,
  formatPercent,
  formatTime,
  parseTimestamp,
  ratioOf,
  splitMs,
} from './format';

describe('formatInt', () => {
  it('adds thousands separators and rounds', () => {
    expect(formatInt(12345)).toBe('12,345');
    expect(formatInt(1234.6)).toBe('1,235');
    expect(formatInt(0)).toBe('0');
  });

  it('falls back for missing or non-finite values', () => {
    expect(formatInt(null)).toBe('-');
    expect(formatInt(undefined)).toBe('-');
    expect(formatInt(Number.NaN)).toBe('-');
    expect(formatInt(Number.POSITIVE_INFINITY)).toBe('-');
  });
});

describe('formatCompact', () => {
  it('leaves small numbers alone', () => {
    expect(formatCompact(0)).toBe('0');
    expect(formatCompact(999)).toBe('999');
  });

  it('scales thousands, millions and billions', () => {
    expect(formatCompact(1000)).toBe('1K');
    expect(formatCompact(1234)).toBe('1.2K');
    expect(formatCompact(25_000)).toBe('25K');
    expect(formatCompact(2_500_000)).toBe('2.5M');
    expect(formatCompact(3_000_000_000)).toBe('3B');
  });

  it('handles negatives and missing input', () => {
    expect(formatCompact(-2500)).toBe('-2.5K');
    expect(formatCompact(null)).toBe('-');
  });
});

describe('formatMs', () => {
  it('shows one decimal place under 10 ms and whole milliseconds above', () => {
    expect(formatMs(3.46)).toBe('3.5 ms');
    expect(formatMs(42.4)).toBe('42 ms');
  });

  it('switches to seconds at 1000 ms', () => {
    expect(formatMs(999)).toBe('999 ms');
    expect(formatMs(1000)).toBe('1.0 s');
    expect(formatMs(12_500)).toBe('13 s');
  });

  it('falls back for missing input', () => {
    expect(formatMs(undefined)).toBe('-');
  });
});

describe('splitMs', () => {
  it('splits into value and unit for KPI cards', () => {
    expect(splitMs(850)).toEqual({ value: '850.0', unit: 'ms' });
    expect(splitMs(2500)).toEqual({ value: '2.50', unit: 's' });
    expect(splitMs(null)).toEqual({ value: '-', unit: '' });
  });
});

describe('formatPercent', () => {
  it('treats the input as a 0 to 1 ratio', () => {
    expect(formatPercent(0.1234)).toBe('12.3%');
    expect(formatPercent(1)).toBe('100.0%');
    expect(formatPercent(0.5, 0)).toBe('50%');
  });

  it('falls back when the ratio is missing', () => {
    expect(formatPercent(null)).toBe('-');
    expect(formatPercent(Number.NaN)).toBe('-');
  });
});

describe('ratioOf and clamp01', () => {
  it('computes part over whole, or null when the whole is zero', () => {
    expect(ratioOf(25, 100)).toBe(0.25);
    expect(ratioOf(0, 0)).toBeNull();
    expect(ratioOf(5, null)).toBeNull();
  });

  it('clamps to the unit interval', () => {
    expect(clamp01(-0.5)).toBe(0);
    expect(clamp01(1.7)).toBe(1);
    expect(clamp01(0.4)).toBe(0.4);
    expect(clamp01(Number.NaN)).toBe(0);
  });
});

describe('timestamps', () => {
  it('parses ISO strings, epoch seconds and epoch milliseconds', () => {
    const iso = parseTimestamp('2026-11-01T12:00:00Z');
    const seconds = parseTimestamp(1_793_559_600);
    const millis = parseTimestamp(1_793_559_600_000);
    expect(iso?.toISOString()).toBe('2026-11-01T12:00:00.000Z');
    expect(seconds?.getTime()).toBe(1_793_559_600_000);
    expect(millis?.getTime()).toBe(1_793_559_600_000);
  });

  it('returns null for unusable input', () => {
    expect(parseTimestamp('not a date')).toBeNull();
    expect(parseTimestamp('')).toBeNull();
    expect(parseTimestamp(null)).toBeNull();
  });

  it('formats time and date-time in local form, with fallback', () => {
    const d = new Date(2026, 10, 1, 9, 5, 7);
    expect(formatDateTime(d.toISOString())).toBe('2026-11-01 09:05:07');
    expect(formatTime(d.toISOString())).toBe('09:05:07');
    expect(formatTime('garbage')).toBe('-');
  });
});
