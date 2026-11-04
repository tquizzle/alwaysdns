import { describe, expect, it } from 'vitest';
import { redactListUrl, validateListName, validateListUrl } from './lists';

describe('validateListUrl', () => {
  it('accepts http and https URLs with a host', () => {
    expect(validateListUrl('https://example.com/hosts.txt')).toBeNull();
    expect(validateListUrl('http://10.0.0.5:8080/list')).toBeNull();
    expect(validateListUrl('  https://lists.example.net/ads.txt  ')).toBeNull();
  });

  it('requires a value', () => {
    expect(validateListUrl('')).toBe('Enter the list URL.');
    expect(validateListUrl('   ')).toBe('Enter the list URL.');
  });

  it('rejects text that is not a URL', () => {
    expect(validateListUrl('example.com/hosts.txt')).toBe('Enter a full URL, including http:// or https://.');
    expect(validateListUrl('not a url')).toBe('The URL cannot contain spaces.');
  });

  it('rejects other schemes', () => {
    expect(validateListUrl('ftp://example.com/list')).toBe('The URL must start with http:// or https://.');
    expect(validateListUrl('file:///etc/hosts')).toBe('The URL must start with http:// or https://.');
    expect(validateListUrl('javascript:alert(1)')).toBe('The URL must start with http:// or https://.');
  });

  it('rejects URLs longer than the limit', () => {
    const long = `https://example.com/${'a'.repeat(2100)}`;
    expect(validateListUrl(long)).toBe('The URL must be 2048 characters or fewer.');
  });
});

describe('validateListName', () => {
  it('requires a non-blank name', () => {
    expect(validateListName('')).toBe('Enter a name for the list.');
    expect(validateListName('   ')).toBe('Enter a name for the list.');
    expect(validateListName('Ads')).toBeNull();
  });
});

describe('redactListUrl', () => {
  it('keeps scheme, host and path', () => {
    expect(redactListUrl('https://example.com/lists/hosts.txt')).toBe('https://example.com/lists/hosts.txt');
  });

  it('hides query values and fragments, which often carry tokens', () => {
    expect(redactListUrl('https://example.com/list?key=abc123#frag')).toBe('https://example.com/list?...#...');
  });

  it('drops user name and password', () => {
    expect(redactListUrl('https://user:secret@example.com/list')).toBe('https://example.com/list');
  });

  it('masks an IPv4 host the same way client addresses are masked', () => {
    expect(redactListUrl('http://192.168.1.10:8080/hosts')).toBe('http://192.168.x.x:8080/hosts');
  });

  it('masks an IPv6 literal host, keeping its brackets', () => {
    expect(redactListUrl('http://[2001:db8:abcd::1]:8080/x')).toBe('http://[2001:db8:x:x]:8080/x');
  });

  it('falls back to hiding everything after the first ? or # when parsing fails', () => {
    expect(redactListUrl('lists/hosts?token=abc')).toBe('lists/hosts?...');
  });
});
