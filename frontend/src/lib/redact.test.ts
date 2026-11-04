import { describe, expect, it } from 'vitest';
import { SECRET_MASK, redactAddress, redactAddressList, redactDisplay, redactYaml } from './redact';

describe('redactAddress', () => {
  it('masks the last two IPv4 octets', () => {
    expect(redactAddress('192.168.1.25')).toBe('192.168.x.x');
  });

  it('keeps the CIDR prefix length', () => {
    expect(redactAddress('10.0.20.0/24')).toBe('10.0.x.x/24');
  });

  it('masks IPv6 addresses after the first two groups', () => {
    expect(redactAddress('2001:db8:abcd::1')).toBe('2001:db8:x:x');
    expect(redactAddress('fd00:1:2:3::/64')).toBe('fd00:1:x:x/64');
    expect(redactAddress('::1')).toBe('x:x');
  });

  it('leaves hostnames and other text unchanged', () => {
    expect(redactAddress('laptop.lan')).toBe('laptop.lan');
    expect(redactAddress('Living room TV')).toBe('Living room TV');
    expect(redactAddress('')).toBe('');
  });
});

describe('redactDisplay and redactAddressList', () => {
  it('passes values through when redaction is off', () => {
    expect(redactDisplay('192.168.1.25', false)).toBe('192.168.1.25');
    expect(redactAddressList(['10.1.2.3/32'], false)).toEqual(['10.1.2.3/32']);
  });

  it('masks every entry when redaction is on', () => {
    expect(redactDisplay('192.168.1.25', true)).toBe('192.168.x.x');
    expect(redactAddressList(['192.168.1.0/24', 'fd00::/8'], true)).toEqual(['192.168.x.x/24', 'fd00:x:x/8']);
  });
});

describe('redactYaml', () => {
  it('masks scalar values of secret-looking keys', () => {
    const input = [
      'upstreams:',
      '  - name: cloudflare',
      '    token: "abc123"',
      'admin:',
      '  password: hunter2',
      '  api_key: xyz',
    ].join('\n');
    const out = redactYaml(input).split('\n');
    expect(out[2]).toBe(`    token: ${SECRET_MASK}`);
    expect(out[4]).toBe(`  password: ${SECRET_MASK}`);
    expect(out[5]).toBe(`  api_key: ${SECRET_MASK}`);
  });

  it('keeps ordinary keys, comments and empty secret values', () => {
    const input = ['listen: 0.0.0.0:53', '# password: commented', 'secret:', '  - item'].join('\n');
    expect(redactYaml(input)).toBe(input);
  });

  it('masks list items that use a secret key', () => {
    expect(redactYaml('  - password: abc')).toBe(`  - password: ${SECRET_MASK}`);
  });
});
