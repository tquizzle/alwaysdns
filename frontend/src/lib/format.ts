// Pure formatting helpers. No DOM or React imports so they can be unit tested.

const FALLBACK = '-';

export function isFiniteNumber(value: unknown): value is number {
  return typeof value === 'number' && Number.isFinite(value);
}

/** Integer with thousands separators, e.g. 12345 -> "12,345". */
export function formatInt(value: number | null | undefined): string {
  if (!isFiniteNumber(value)) return FALLBACK;
  return Math.round(value).toLocaleString('en-US');
}

/** Short form for large counts, e.g. 1234 -> "1.2K", 2_500_000 -> "2.5M". */
export function formatCompact(value: number | null | undefined): string {
  if (!isFiniteNumber(value)) return FALLBACK;
  const abs = Math.abs(value);
  if (abs < 1000) return String(Math.round(value));
  const units: Array<[number, string]> = [
    [1e9, 'B'],
    [1e6, 'M'],
    [1e3, 'K'],
  ];
  for (const [size, suffix] of units) {
    if (abs >= size) {
      const scaled = value / size;
      const digits = Math.abs(scaled) >= 100 ? 0 : 1;
      return `${scaled.toFixed(digits).replace(/\.0$/, '')}${suffix}`;
    }
  }
  return String(Math.round(value));
}

/** Milliseconds: "12 ms" below one second, "1.2 s" from 1000 ms up. */
export function formatMs(value: number | null | undefined): string {
  if (!isFiniteNumber(value)) return FALLBACK;
  if (value >= 1000) return `${(value / 1000).toFixed(value >= 10_000 ? 0 : 1)} s`;
  if (value < 10) return `${value.toFixed(1)} ms`;
  return `${Math.round(value)} ms`;
}

/**
 * Converts a ratio in the range 0 to 1 into a percentage string.
 * Returns the fallback when the ratio is missing or not finite.
 */
export function formatPercent(ratio: number | null | undefined, digits = 1): string {
  if (!isFiniteNumber(ratio)) return FALLBACK;
  return `${(ratio * 100).toFixed(digits)}%`;
}

/** part / whole as a ratio, or null when the whole is zero or missing. */
export function ratioOf(part: number | null | undefined, whole: number | null | undefined): number | null {
  if (!isFiniteNumber(part) || !isFiniteNumber(whole) || whole <= 0) return null;
  return part / whole;
}

/** Clamp to the range 0 to 1, treating non-finite input as 0. */
export function clamp01(value: number): number {
  if (!Number.isFinite(value)) return 0;
  return Math.min(1, Math.max(0, value));
}

/**
 * Parses an API timestamp. Accepts ISO strings, epoch seconds and epoch
 * milliseconds (values below 1e12 are treated as seconds).
 */
export function parseTimestamp(value: string | number | null | undefined): Date | null {
  if (value === null || value === undefined || value === '') return null;
  let date: Date;
  if (typeof value === 'number') {
    if (!Number.isFinite(value)) return null;
    date = new Date(Math.abs(value) < 1e12 ? value * 1000 : value);
  } else {
    date = new Date(value);
  }
  return Number.isNaN(date.getTime()) ? null : date;
}

/** Local time as HH:MM:SS, or the fallback for unparseable input. */
export function formatTime(value: string | number | null | undefined): string {
  const date = parseTimestamp(value);
  if (!date) return FALLBACK;
  return date.toLocaleTimeString('en-GB', { hour: '2-digit', minute: '2-digit', second: '2-digit' });
}

/** Local date and time for tooltips, e.g. "2026-11-01 14:05:09". */
export function formatDateTime(value: string | number | null | undefined): string {
  const date = parseTimestamp(value);
  if (!date) return FALLBACK;
  const pad = (n: number) => String(n).padStart(2, '0');
  return `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())} ${pad(date.getHours())}:${pad(date.getMinutes())}:${pad(date.getSeconds())}`;
}

/** Shows the value, or the fallback for empty strings. */
export function orDash(value: string | null | undefined): string {
  return value && value.trim() !== '' ? value : FALLBACK;
}

/** Splits a latency into a number and unit for KPI cards: 850 -> {value: "850.0", unit: "ms"}. */
export function splitMs(value: number | null | undefined): { value: string; unit: string } {
  if (!isFiniteNumber(value)) return { value: FALLBACK, unit: '' };
  if (value >= 1000) return { value: (value / 1000).toFixed(2), unit: 's' };
  return { value: value.toFixed(1), unit: 'ms' };
}
