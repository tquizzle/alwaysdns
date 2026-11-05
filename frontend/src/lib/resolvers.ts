import type { Protocol } from '../api/types';

export const RESOLVER_NAME_MAX = 64;

export const PROTOCOL_LABEL: Record<Protocol, string> = { doh: 'DoH', doq: 'DoQ' };

/** Returns an error message, or null when the upstream URL is usable. */
export function upstreamUrlError(raw: string): string | null {
  const value = raw.trim();
  if (!value.startsWith('https://') && !value.startsWith('quic://')) {
    return `"${value}" must start with https:// (DoH) or quic:// (DoQ)`;
  }
  try {
    new URL(value);
  } catch {
    return `"${value}" is not a valid URL`;
  }
  return null;
}

/** Returns an error message, or null when the resolver name is usable. */
export function resolverNameError(raw: string): string | null {
  const name = raw.trim();
  if (name.length === 0) return 'Give the resolver a name.';
  if (name.length > RESOLVER_NAME_MAX) return `The name must be ${RESOLVER_NAME_MAX} characters or fewer.`;
  if (name.includes('://')) return 'The name must not look like a URL.';
  return null;
}

/** A copy of `items` with the entry at `index` moved one place. Moves past either end change nothing. */
export function moveItem<T>(items: readonly T[], index: number, delta: -1 | 1): T[] {
  const next = [...items];
  const target = index + delta;
  if (index < 0 || index >= next.length || target < 0 || target >= next.length) return next;
  const [item] = next.splice(index, 1);
  if (item === undefined) return next;
  next.splice(target, 0, item);
  return next;
}
