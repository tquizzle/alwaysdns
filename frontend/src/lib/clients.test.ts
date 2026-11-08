import { describe, expect, it } from 'vitest';
import type { Client } from '../api/types';
import { clientBody, fromSafeSearchChoice, toSafeSearchChoice } from './clients';

const client: Client = {
  name: 'Kid',
  cidrs: ['192.0.2.20/32'],
  allow_domains: [],
  block_domains: [],
  upstream_group: 'default',
  block_lists: ['ads'],
  blocked_services: [],
  safe_search: false,
  queries: 0,
  blocked: 0,
};

describe('safe search menu choices', () => {
  it('maps the three states both ways, because null (follow global) and false (off) must stay different', () => {
    expect(toSafeSearchChoice(null)).toBe('inherit');
    expect(toSafeSearchChoice(true)).toBe('on');
    expect(toSafeSearchChoice(false)).toBe('off');
    expect(fromSafeSearchChoice('inherit')).toBeNull();
    expect(fromSafeSearchChoice('on')).toBe(true);
    expect(fromSafeSearchChoice('off')).toBe(false);
  });

  it('round-trips every state', () => {
    for (const value of [null, true, false]) {
      expect(fromSafeSearchChoice(toSafeSearchChoice(value))).toBe(value);
    }
  });
});

describe('clientBody', () => {
  it('carries the safe search override, so a quick edit like ticking a block list cannot reset it to inherit', () => {
    expect(clientBody(client).safe_search).toBe(false);
    expect(clientBody({ ...client, safe_search: true }).safe_search).toBe(true);
    expect(clientBody({ ...client, safe_search: null }).safe_search).toBeNull();
  });
});
