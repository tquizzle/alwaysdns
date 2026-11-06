import { describe, expect, it } from 'vitest';
import type { Settings } from '../api/types';
import { exportFilename, looksLikeIp, nonBlankRows, parseDays, rewriteError, settingsBody } from './settings';

describe('exportFilename', () => {
  it('carries the date with zero padding, so files from different days sort and do not collide', () => {
    expect(exportFilename(new Date(2026, 0, 5))).toBe('alwaysdns-settings-2026-01-05.yaml');
    expect(exportFilename(new Date(2026, 10, 6))).toBe('alwaysdns-settings-2026-11-06.yaml');
  });
});

const current: Settings = {
  rewrites: [{ domain: 'nas.example.test', answer: '192.0.2.5' }],
  allow_domains: ['ok.example.org'],
  block_domains: ['bad.example'],
  bootstrap: ['9.9.9.10'],
  default_upstream_group: 'default',
  retention: { query_days: 30, stats_days: 365 },
  listen: { dns: '0.0.0.0:53', http: '0.0.0.0:8080' },
  groups: ['default'],
};

describe('settingsBody', () => {
  it('changes one section and sends the rest as the server has it, so a save cannot clear another section', () => {
    const body = settingsBody(current, { block_domains: ['bad.example', 'worse.example'] });
    expect(body.block_domains).toEqual(['bad.example', 'worse.example']);
    expect(body.rewrites).toEqual(current.rewrites);
    expect(body.allow_domains).toEqual(current.allow_domains);
    expect(body.retention).toEqual(current.retention);
  });

  it('never sends the read-only listeners or the group names back', () => {
    const body = settingsBody(current, {});
    expect(Object.keys(body)).not.toContain('listen');
    expect(Object.keys(body)).not.toContain('groups');
  });
});

describe('parseDays', () => {
  it('accepts whole days of at least one and rejects the rest, because zero days would delete all history', () => {
    expect(parseDays(' 30 ')).toBe(30);
    expect(parseDays('0')).toBeNull();
    expect(parseDays('-5')).toBeNull();
    expect(parseDays('1.5')).toBeNull();
    expect(parseDays('')).toBeNull();
  });
});

describe('local answer rows', () => {
  it('recognises IPv4 and IPv6 addresses and rejects other text', () => {
    expect(looksLikeIp('192.0.2.5')).toBe(true);
    expect(looksLikeIp('2001:db8::5')).toBe(true);
    expect(looksLikeIp('999.1.1.1')).toBe(false);
    expect(looksLikeIp('nas.example.test')).toBe(false);
  });

  it('ignores an untouched row but reports a half-filled one', () => {
    expect(nonBlankRows([{ domain: '', answer: ' ' }, { domain: 'a.test', answer: '' }])).toHaveLength(1);
    expect(rewriteError({ domain: 'a.test', answer: '' })).toMatch(/not an IP/);
    expect(rewriteError({ domain: '', answer: '192.0.2.5' })).toMatch(/name/);
    expect(rewriteError({ domain: '*.example.test', answer: '192.0.2.5' })).toBeNull();
  });
});
