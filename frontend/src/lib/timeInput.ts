// Conversions between epoch milliseconds and the local "YYYY-MM-DDTHH:mm"
// strings used by datetime-local inputs. Pure, so they can be unit tested.

const LOCAL_RE = /^(\d{4})-(\d{2})-(\d{2})T(\d{2}):(\d{2})(?::(\d{2}))?$/;

const pad = (n: number) => String(n).padStart(2, '0');

/** Epoch ms to a local datetime-local value, truncated to the minute. */
export function toLocalInputValue(ms: number): string {
  const d = new Date(ms);
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}T${pad(d.getHours())}:${pad(d.getMinutes())}`;
}

/** Local datetime-local text to epoch ms, or null when it is not a real date and time. */
export function parseLocalDateTime(value: string): number | null {
  const m = LOCAL_RE.exec(value.trim());
  if (!m) return null;
  const year = Number(m[1]);
  const month = Number(m[2]);
  const day = Number(m[3]);
  const hour = Number(m[4]);
  const minute = Number(m[5]);
  const second = m[6] === undefined ? 0 : Number(m[6]);

  const d = new Date(year, month - 1, day, hour, minute, second);
  // Reject overflowing input such as 2026-02-31 or a local time skipped by DST.
  if (
    d.getFullYear() !== year ||
    d.getMonth() !== month - 1 ||
    d.getDate() !== day ||
    d.getHours() !== hour ||
    d.getMinutes() !== minute
  ) {
    return null;
  }
  const ms = d.getTime();
  return Number.isNaN(ms) ? null : ms;
}
