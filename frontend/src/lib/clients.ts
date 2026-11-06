import type { Client, ClientUpdateBody } from '../api/types';

/** Splits a text box into one entry per line. Trims each, drops blanks and repeats. */
export function linesToList(text: string): string[] {
  const out: string[] = [];
  for (const raw of text.split('\n')) {
    const line = raw.trim();
    if (line && !out.includes(line)) out.push(line);
  }
  return out;
}

/** The client's rule set as it stands, ready to change one field and save. */
export function clientBody(client: Client): ClientUpdateBody {
  return {
    cidrs: client.cidrs ?? [],
    upstream_group: client.upstream_group,
    block_lists: client.block_lists,
    allow_domains: client.allow_domains ?? [],
    block_domains: client.block_domains ?? [],
    blocked_services: client.blocked_services,
  };
}

const CIDR_PATTERN = /^[0-9a-fA-F:.]+\/\d{1,3}$/;

/** Returns an error message, or null when the entry looks like a CIDR network. */
export function cidrError(entry: string): string | null {
  return CIDR_PATTERN.test(entry) ? null : `"${entry}" is not a network in CIDR form, e.g. 192.168.1.0/24`;
}
