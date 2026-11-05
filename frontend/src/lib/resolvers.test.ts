import { describe, expect, it } from 'vitest';
import { RESOLVER_NAME_MAX, moveItem, resolverNameError, upstreamUrlError } from './resolvers';

describe('upstreamUrlError', () => {
  it('accepts DoH and DoQ URLs', () => {
    expect(upstreamUrlError('https://dns10.quad9.net/dns-query')).toBeNull();
    expect(upstreamUrlError(' quic://abc123.dns.nextdns.io ')).toBeNull();
  });

  it('rejects other schemes and bare hosts, because the server only speaks DoH and DoQ', () => {
    expect(upstreamUrlError('http://dns.example/dns-query')).toMatch(/https:\/\/ \(DoH\) or quic:\/\//);
    expect(upstreamUrlError('tls://dns.example')).not.toBeNull();
    expect(upstreamUrlError('9.9.9.9')).not.toBeNull();
  });

  it('rejects text that has the right prefix but is not a URL', () => {
    expect(upstreamUrlError('https://')).toMatch(/not a valid URL/);
  });
});

describe('resolverNameError', () => {
  it('accepts a friendly name', () => {
    expect(resolverNameError('NextDNS (DoQ)')).toBeNull();
  });

  it('rejects empty, overlong and URL-shaped names, which the server would also refuse', () => {
    expect(resolverNameError('   ')).not.toBeNull();
    expect(resolverNameError('x'.repeat(RESOLVER_NAME_MAX + 1))).not.toBeNull();
    expect(resolverNameError('x'.repeat(RESOLVER_NAME_MAX))).toBeNull();
    expect(resolverNameError('https://dns.example')).toMatch(/look like a URL/);
  });
});

describe('moveItem', () => {
  it('moves an entry up or down without touching the original list', () => {
    const order = ['a', 'b', 'c'];
    expect(moveItem(order, 1, -1)).toEqual(['b', 'a', 'c']);
    expect(moveItem(order, 1, 1)).toEqual(['a', 'c', 'b']);
    expect(order).toEqual(['a', 'b', 'c']);
  });

  it('leaves the order alone when a move would pass either end, so failover order never changes by accident', () => {
    expect(moveItem(['a', 'b'], 0, -1)).toEqual(['a', 'b']);
    expect(moveItem(['a', 'b'], 1, 1)).toEqual(['a', 'b']);
    expect(moveItem(['a', 'b'], 5, -1)).toEqual(['a', 'b']);
  });
});
