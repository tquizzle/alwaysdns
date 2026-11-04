import type { ListKind } from '../api/types';
import { redactAddress } from './redact';

export const LIST_KIND_LABEL: Record<ListKind, string> = {
  block: 'Block lists',
  allow: 'Allow lists',
};

export const MAX_LIST_URL_LENGTH = 2048;

/** Returns a message for an unusable list URL, or null when it can be saved. */
export function validateListUrl(input: string): string | null {
  const value = input.trim();
  if (value === '') return 'Enter the list URL.';
  if (/\s/.test(value)) return 'The URL cannot contain spaces.';
  if (value.length > MAX_LIST_URL_LENGTH) {
    return `The URL must be ${MAX_LIST_URL_LENGTH} characters or fewer.`;
  }
  let url: URL;
  try {
    url = new URL(value);
  } catch {
    return 'Enter a full URL, including http:// or https://.';
  }
  if (url.protocol !== 'http:' && url.protocol !== 'https:') {
    return 'The URL must start with http:// or https://.';
  }
  if (url.hostname === '') return 'The URL needs a host name.';
  return null;
}

/** Returns a message for an empty list name, or null when it is usable. */
export function validateListName(input: string): string | null {
  return input.trim() === '' ? 'Enter a name for the list.' : null;
}

/**
 * Display form of a list URL for viewers. Drops any user name and password,
 * replaces query and fragment values (they often carry tokens) with a
 * placeholder, and masks an IPv4 host the same way client addresses are masked.
 * Unparseable input keeps its path but loses everything after the first ? or #.
 */
export function redactListUrl(input: string): string {
  let url: URL;
  try {
    url = new URL(input.trim());
  } catch {
    return input.replace(/[?#].*$/, '?...');
  }
  // URL hostnames keep the brackets of an IPv6 literal, which the masker does not expect.
  const bare = url.hostname.replace(/^\[(.*)\]$/, '$1');
  const masked = redactAddress(bare);
  const host = bare === url.hostname ? masked : `[${masked}]`;
  const port = url.port ? `:${url.port}` : '';
  const query = url.search ? '?...' : '';
  const fragment = url.hash ? '#...' : '';
  return `${url.protocol}//${host}${port}${url.pathname}${query}${fragment}`;
}
