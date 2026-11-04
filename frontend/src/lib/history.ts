import type { Bucket, HistoryParams } from '../api/types';
import { parseTimestamp } from './format';
import { parseLocalDateTime } from './timeInput';

export const BUCKET_OPTIONS: { value: Bucket; label: string }[] = [
  { value: 'minute', label: 'Per minute' },
  { value: 'hour', label: 'Per hour' },
  { value: 'day', label: 'Per day' },
];

const BUCKET_SECONDS: Record<Bucket, number> = {
  minute: 60,
  hour: 3_600,
  day: 86_400,
};

/** Upper bound on points per history chart, so one request cannot draw an unreadable line. */
export const MAX_HISTORY_POINTS = 5_000;

/** Human label for a bucket size, e.g. "Per hour". */
export function bucketLabel(bucket: Bucket): string {
  return BUCKET_OPTIONS.find((option) => option.value === bucket)?.label ?? 'Per hour';
}

/** Length of one bucket in seconds. */
export function bucketSeconds(bucket: Bucket): number {
  return BUCKET_SECONDS[bucket];
}

/** Returns a message when the window cannot be drawn at this bucket size, or null when it can. */
export function validateHistoryRange(fromMs: number, toMs: number, bucket: Bucket): string | null {
  if (!(toMs > fromMs)) return 'The start must be before the end.';
  const points = Math.ceil((toMs - fromMs) / 1000 / bucketSeconds(bucket));
  if (points > MAX_HISTORY_POINTS) {
    return `That range at this bucket size would draw ${points.toLocaleString('en-US')} points (limit ${MAX_HISTORY_POINTS.toLocaleString('en-US')}). Narrow the range or pick a larger bucket.`;
  }
  return null;
}

/** Parses the two datetime-local inputs and checks them, giving request params or a message. */
export function historyParams(
  fromText: string,
  toText: string,
  bucket: Bucket,
): { params: HistoryParams | null; error: string | null } {
  const from = parseLocalDateTime(fromText);
  if (from === null) return { params: null, error: 'Enter a valid start date and time.' };
  const to = parseLocalDateTime(toText);
  if (to === null) return { params: null, error: 'Enter a valid end date and time.' };

  const error = validateHistoryRange(from, to, bucket);
  if (error) return { params: null, error };
  return { params: { from, to, bucket }, error: null };
}

const pad = (n: number) => String(n).padStart(2, '0');

/** Axis label for a history point: "MM-DD HH:mm" for minute and hour buckets, "YYYY-MM-DD" for day buckets. */
export function formatBucketTick(t: number, bucket: Bucket): string {
  const date = parseTimestamp(t);
  if (!date) return '-';
  const monthDay = `${pad(date.getMonth() + 1)}-${pad(date.getDate())}`;
  if (bucket === 'day') return `${date.getFullYear()}-${monthDay}`;
  return `${monthDay} ${pad(date.getHours())}:${pad(date.getMinutes())}`;
}
