// Display-only redaction. These helpers never change data sent to the API;
// they mask client addresses on screen when enabled.

const IPV4 = /^(\d{1,3})\.(\d{1,3})\.\d{1,3}\.\d{1,3}$/;
const IPV6_HEX_GROUP = /^[0-9a-fA-F]{0,4}$/;

/**
 * Masks the host part of an address, keeping the first two groups:
 *   192.168.1.25 -> 192.168.x.x
 *   192.168.1.0/24 -> 192.168.x.x/24
 *   2001:db8:abcd::1 -> 2001:db8:x:x
 * Anything that is not an IP address (for example a hostname) is returned unchanged.
 */
export function redactAddress(input: string): string {
  const trimmed = input.trim();
  if (trimmed === '') return input;

  const slash = trimmed.indexOf('/');
  const addr = slash === -1 ? trimmed : trimmed.slice(0, slash);
  const suffix = slash === -1 ? '' : trimmed.slice(slash);

  const v4 = IPV4.exec(addr);
  if (v4) {
    return `${v4[1]}.${v4[2]}.x.x${suffix}`;
  }

  if (addr.includes(':') && isLikelyIPv6(addr)) {
    const groups = addr.split(':');
    const kept = groups.slice(0, 2).filter((g) => g !== '');
    const head = kept.length > 0 ? `${kept.join(':')}:` : '';
    return `${head}x:x${suffix}`;
  }

  return input;
}

function isLikelyIPv6(addr: string): boolean {
  const groups = addr.split(':');
  if (groups.length < 2 || groups.length > 8) return false;
  return groups.every((g) => IPV6_HEX_GROUP.test(g));
}

/** Applies redactAddress to each entry of a list when redaction is enabled. */
export function redactAddressList(values: string[], enabled: boolean): string[] {
  return enabled ? values.map(redactAddress) : values;
}

/** Returns the value unchanged when redaction is off, otherwise a masked version. */
export function redactDisplay(value: string, enabled: boolean): string {
  return enabled ? redactAddress(value) : value;
}
