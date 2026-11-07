//! In-memory backoff for failed logins. Each username and each client IP has its
//! own counter. The first failures are free. After that, each failure locks the
//! key for twice as long as the one before, up to a cap. Nothing is saved to
//! disk, so a restart clears every lock.

use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use ipnet::IpNet;

use crate::rules::{canonical_ip, parse_cidr};

/// Failures that are free before a key starts to be locked.
const FREE_FAILURES: u32 = 5;
/// Lock after the first locked failure. Each later failure doubles it.
const BASE_LOCK: Duration = Duration::from_secs(30);
/// Longest lock. It is reached on the eleventh failure.
const MAX_LOCK: Duration = Duration::from_secs(15 * 60);
/// A key that stays quiet this long after its lock ends is forgotten.
const FORGET_AFTER: Duration = MAX_LOCK;
/// Most keys kept in each map. Expired keys are dropped first, then the oldest.
const MAX_KEYS: usize = 4096;
/// Longest username kept as a key, so a long request body cannot fill memory.
const MAX_KEY_CHARS: usize = 128;

#[derive(Debug)]
struct Entry {
    failures: u32,
    last_failure: Instant,
    /// Set once the failures are past the free ones.
    locked_until: Option<Instant>,
}

impl Entry {
    /// Forgotten once it has been quiet for `FORGET_AFTER`, counted from the end of any lock.
    fn expired(&self, now: Instant) -> bool {
        let quiet_since = self.locked_until.unwrap_or(self.last_failure);
        now >= quiet_since + FORGET_AFTER
    }
}

/// Lock length after the `failures`-th failure: 30s, doubling, capped at 15 minutes.
fn lock_for(failures: u32) -> Duration {
    let doublings = failures.saturating_sub(FREE_FAILURES + 1).min(16);
    BASE_LOCK.saturating_mul(1u32 << doublings).min(MAX_LOCK)
}

/// Whole seconds to wait, rounded up, for a `Retry-After` header.
pub fn retry_after_secs(wait: Duration) -> u64 {
    wait.as_secs() + u64::from(wait.subsec_nanos() > 0)
}

/// Failure counters for one kind of key (a username or an IP).
#[derive(Default)]
struct Backoff {
    entries: Mutex<HashMap<String, Entry>>,
}

impl Backoff {
    fn lock(&self) -> MutexGuard<'_, HashMap<String, Entry>> {
        self.entries.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// How long `key` must still wait, or `None` when it is not locked now.
    fn locked_for(&self, key: &str, now: Instant) -> Option<Duration> {
        let until = self.lock().get(key)?.locked_until?;
        until.checked_duration_since(now).filter(|wait| !wait.is_zero())
    }

    fn record_failure(&self, key: &str, now: Instant) {
        let mut map = self.lock();
        map.retain(|_, entry| !entry.expired(now));
        if !map.contains_key(key) && map.len() >= MAX_KEYS {
            evict_oldest(&mut map);
        }
        let entry = map.entry(key.to_string()).or_insert(Entry {
            failures: 0,
            last_failure: now,
            locked_until: None,
        });
        entry.failures = entry.failures.saturating_add(1);
        entry.last_failure = now;
        let failures = entry.failures;
        entry.locked_until = (failures > FREE_FAILURES).then(|| now + lock_for(failures));
    }

    fn forget(&self, key: &str) {
        self.lock().remove(key);
    }
}

/// Drops the entry whose most recent failure is oldest.
fn evict_oldest(map: &mut HashMap<String, Entry>) {
    let oldest = map
        .iter()
        .min_by_key(|(_, entry)| entry.last_failure)
        .map(|(key, _)| key.clone());
    if let Some(key) = oldest {
        map.remove(&key);
    }
}

/// Login limits: one counter per username and one per client IP. The counts do not
/// depend on whether the account exists, so a lock reveals nothing about usernames.
#[derive(Default)]
pub struct LoginLimiter {
    by_user: Backoff,
    by_ip: Backoff,
    /// Held from the limit check until the outcome is recorded, so attempts run one at a time.
    turn: tokio::sync::Mutex<()>,
}

impl LoginLimiter {
    /// Waits for this attempt's turn. Hold the guard until the outcome is recorded.
    pub async fn take_turn(&self) -> tokio::sync::MutexGuard<'_, ()> {
        self.turn.lock().await
    }

    /// How long a login for `username` from `ip` must wait, or `None` if it may be tried now.
    pub fn wait_before(&self, username: &str, ip: IpAddr, now: Instant) -> Option<Duration> {
        let user = self.by_user.locked_for(&user_key(username), now);
        let addr = self.by_ip.locked_for(&ip.to_string(), now);
        user.max(addr)
    }

    pub fn record_failure(&self, username: &str, ip: IpAddr, now: Instant) {
        self.by_user.record_failure(&user_key(username), now);
        self.by_ip.record_failure(&ip.to_string(), now);
    }

    /// A good login clears the username's counter. The IP counter is left to decay.
    pub fn record_success(&self, username: &str) {
        self.by_user.forget(&user_key(username));
    }
}

/// Lowercased and cut to `MAX_KEY_CHARS`, so that `Admin` and `admin` share a counter.
fn user_key(username: &str) -> String {
    username.to_lowercase().chars().take(MAX_KEY_CHARS).collect()
}

/// Parses `DNSUI_TRUSTED_PROXIES`: comma-separated IPs or CIDRs. Empty means no proxy is trusted.
pub fn parse_trusted_proxies(raw: &str) -> Result<Vec<IpNet>> {
    if raw.trim().is_empty() {
        return Ok(Vec::new());
    }
    raw.split(',').map(parse_trusted_entry).collect()
}

fn parse_trusted_entry(raw: &str) -> Result<IpNet> {
    let entry = raw.trim();
    if entry.is_empty() {
        bail!("DNSUI_TRUSTED_PROXIES has an empty entry (check for a stray comma)");
    }
    let net = parse_cidr(entry)
        .with_context(|| format!("DNSUI_TRUSTED_PROXIES: invalid entry '{entry}'"))?;
    Ok(unmap_net(net))
}

/// Writes an IPv4-mapped IPv6 network (`::ffff:a.b.c.d/n`) as the IPv4 network it covers,
/// so that it matches the canonical IPv4 peer address.
fn unmap_net(net: IpNet) -> IpNet {
    let IpNet::V6(v6) = net else {
        return net;
    };
    match v6.addr().to_ipv4_mapped() {
        Some(v4) if v6.prefix_len() >= 96 => {
            IpNet::new(IpAddr::V4(v4), v6.prefix_len() - 96)
                .expect("a prefix of at most 32 is valid for IPv4")
        }
        _ => net,
    }
}

fn is_trusted(ip: IpAddr, trusted: &[IpNet]) -> bool {
    trusted.iter().any(|net| net.contains(&ip))
}

/// The address for the per-IP login limit. The `X-Forwarded-For` header is believed only
/// when the TCP peer is a trusted proxy. The proxy appends the address it saw, so the
/// rightmost entry that is not a trusted proxy is the client. Anything further left was
/// written by the client and is ignored.
pub fn client_ip(peer: IpAddr, forwarded_for: Option<&str>, trusted: &[IpNet]) -> IpAddr {
    let peer = canonical_ip(peer);
    if !is_trusted(peer, trusted) {
        return peer;
    }
    forwarded_for
        .and_then(|chain| rightmost_untrusted(chain, trusted))
        .unwrap_or(peer)
}

/// Walks the list from the right and returns the first entry that is not a trusted proxy.
/// `None` when any entry is not an IP, or when every entry is a trusted proxy.
fn rightmost_untrusted(chain: &str, trusted: &[IpNet]) -> Option<IpAddr> {
    let hops = chain
        .split(',')
        .map(|hop| hop.trim().parse::<IpAddr>().map(canonical_ip))
        .collect::<std::result::Result<Vec<IpAddr>, _>>()
        .ok()?;
    hops.into_iter().rev().find(|hop| !is_trusted(*hop, trusted))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    fn ip(last: u8) -> IpAddr {
        IpAddr::V4(Ipv4Addr::new(192, 0, 2, last))
    }

    fn fail_n(limiter: &LoginLimiter, user: &str, addr: IpAddr, n: u32, now: Instant) {
        for _ in 0..n {
            limiter.record_failure(user, addr, now);
        }
    }

    #[test]
    fn five_failures_are_free_and_the_sixth_locks_the_key() {
        // A typo or two must not lock anyone out. The sixth guess is the first one slowed down.
        let limiter = LoginLimiter::default();
        let t0 = Instant::now();
        fail_n(&limiter, "admin", ip(1), 5, t0);
        assert_eq!(limiter.wait_before("admin", ip(1), t0), None);
        limiter.record_failure("admin", ip(1), t0);
        assert_eq!(limiter.wait_before("admin", ip(1), t0), Some(Duration::from_secs(30)));
    }

    #[test]
    fn each_failure_doubles_the_wait_until_it_caps_at_fifteen_minutes() {
        // Doubling makes a guessing run cost more each round. The cap keeps a locked
        // user from waiting unboundedly, and the lock must never exceed it.
        let limiter = LoginLimiter::default();
        let t0 = Instant::now();
        let mut waits: Vec<Option<u64>> = Vec::new();
        for _ in 0..12 {
            limiter.record_failure("admin", ip(1), t0);
            waits.push(limiter.wait_before("admin", ip(1), t0).map(retry_after_secs));
        }
        let mut expected: Vec<Option<u64>> = vec![None; 5];
        expected.extend([30u64, 60, 120, 240, 480, 900, 900].map(Some));
        assert_eq!(waits, expected);
    }

    #[test]
    fn retry_after_rounds_up_so_a_client_never_retries_early() {
        // Rounding down would let a client retry a fraction of a second before the lock ends.
        assert_eq!(retry_after_secs(Duration::from_millis(29_001)), 30);
        assert_eq!(retry_after_secs(Duration::from_secs(30)), 30);
        assert_eq!(retry_after_secs(Duration::from_millis(1)), 1);
    }

    #[test]
    fn a_good_login_clears_the_username_but_not_the_ip_counter() {
        // The right password should wipe the account's history. The IP's history is about the
        // source address, so it keeps counting and still decays on its own.
        let limiter = LoginLimiter::default();
        let t0 = Instant::now();
        fail_n(&limiter, "admin", ip(1), 4, t0);
        limiter.record_success("admin");
        fail_n(&limiter, "admin", ip(2), 2, t0);
        assert_eq!(limiter.wait_before("admin", ip(3), t0), None, "the username count was reset");
        fail_n(&limiter, "bob", ip(1), 2, t0);
        assert!(
            limiter.wait_before("carol", ip(1), t0).is_some(),
            "four failures from this address before the good login still count",
        );
    }

    #[test]
    fn locks_are_per_username_and_per_ip_without_spilling_over() {
        // One user's lock must not block that user's logins from elsewhere, and one
        // address's lock must not block other users from other addresses.
        let limiter = LoginLimiter::default();
        let t0 = Instant::now();
        fail_n(&limiter, "alice", ip(1), 6, t0);
        assert!(limiter.wait_before("alice", ip(2), t0).is_some(), "alice is locked everywhere");
        assert_eq!(limiter.wait_before("bob", ip(2), t0), None, "bob from another address is free");

        for name in ["u1", "u2", "u3", "u4", "u5", "u6"] {
            limiter.record_failure(name, ip(9), t0);
        }
        assert!(limiter.wait_before("fresh", ip(9), t0).is_some(), "six names lock the address");
        assert_eq!(limiter.wait_before("fresh", ip(10), t0), None);
    }

    #[test]
    fn usernames_share_a_counter_regardless_of_case() {
        // Counting case separately would give an attacker six guesses per spelling.
        let limiter = LoginLimiter::default();
        let t0 = Instant::now();
        fail_n(&limiter, "Admin", ip(1), 3, t0);
        fail_n(&limiter, "ADMIN", ip(2), 3, t0);
        assert!(limiter.wait_before("admin", ip(3), t0).is_some());
    }

    #[test]
    fn a_key_quiet_for_fifteen_minutes_after_its_lock_is_dropped() {
        // Keeps memory from growing with every address that ever failed. A key that is
        // still locked, or has been quiet for less than the forget window, must be kept.
        let backoff = Backoff::default();
        let t0 = Instant::now();
        fail_n_backoff(&backoff, "locked", 6, t0);
        backoff.record_failure("quiet", t0);
        let mid = t0 + Duration::from_secs(60);
        backoff.record_failure("other", mid);
        assert_eq!(backoff.lock().len(), 3, "nothing is forgotten before its window ends");

        let later = t0 + Duration::from_secs(1000);
        backoff.record_failure("new", later);
        let map = backoff.lock();
        assert_eq!(map.len(), 1, "only the new key is left after the windows have passed");
        assert!(map.contains_key("new"));
    }

    #[test]
    fn a_full_map_evicts_its_oldest_key_instead_of_growing() {
        // Bounds memory against a flood of addresses or usernames. Without the cap the map grows without limit.
        let backoff = Backoff::default();
        let t0 = Instant::now();
        for i in 0..=MAX_KEYS {
            backoff.record_failure(&format!("key{i}"), t0 + Duration::from_millis(i as u64));
        }
        let map = backoff.lock();
        assert_eq!(map.len(), MAX_KEYS);
        assert!(!map.contains_key("key0"), "the oldest failure is the one evicted");
        assert!(map.contains_key(&format!("key{MAX_KEYS}")));
    }

    #[test]
    fn expired_keys_make_room_before_a_live_key_is_evicted() {
        // Evicting a key that is still locked would hand its owner a free retry. Expired
        // keys are the ones to drop first.
        let backoff = Backoff::default();
        let t0 = Instant::now();
        fail_n_backoff(&backoff, "live", 6, t0);
        for i in 1..MAX_KEYS {
            backoff.record_failure(&format!("old{i}"), t0 + Duration::from_millis(i as u64));
        }
        // Old keys expire by now. The live key is still locked until t0 + 30s, so it expires at t0 + 930s.
        let later = t0 + FORGET_AFTER + Duration::from_secs(10);
        backoff.record_failure("new", later);
        let map = backoff.lock();
        assert!(map.contains_key("live"), "the locked key survived");
        assert_eq!(map.len(), 2);
    }

    fn addr(raw: &str) -> IpAddr {
        raw.parse().unwrap()
    }

    fn proxies(list: &str) -> Vec<IpNet> {
        parse_trusted_proxies(list).unwrap()
    }

    #[test]
    fn untrusted_peer_ignores_a_spoofed_forwarded_for() {
        // Any client can send any X-Forwarded-For. If an untrusted peer's header were believed,
        // an attacker could name a new address on every guess and never reach a lock.
        let nets = proxies("203.0.113.116/32");
        assert_eq!(client_ip(addr("198.51.100.9"), Some("10.0.0.1"), &nets), addr("198.51.100.9"));
    }

    #[test]
    fn trusted_peer_uses_the_client_from_the_header() {
        // Behind Caddy every request arrives from Caddy. The per-IP limit must key on the
        // visitor that Caddy reports, or all visitors share one lock.
        let nets = proxies("203.0.113.116/32");
        assert_eq!(client_ip(addr("203.0.113.116"), Some("198.51.100.10"), &nets), addr("198.51.100.10"));
    }

    #[test]
    fn multi_hop_chain_returns_the_client_behind_every_trusted_proxy() {
        // With a second proxy in front of the first, the header is "client, proxy2". Both proxies
        // must be skipped, or the lock would land on a proxy and be shared by every visitor.
        let nets = proxies("203.0.113.116/32, 203.0.113.117/32");
        let chain = Some("198.51.100.10, 203.0.113.117");
        assert_eq!(client_ip(addr("203.0.113.116"), chain, &nets), addr("198.51.100.10"));
    }

    #[test]
    fn a_spoofed_leftmost_value_is_not_chosen() {
        // A client can send "10.9.9.9" as the first entry. The proxy appends the real client
        // after it, so the rightmost untrusted entry is the one to use, never the leftmost.
        let nets = proxies("203.0.113.116/32");
        let chain = Some("10.9.9.9, 198.51.100.10");
        assert_eq!(client_ip(addr("203.0.113.116"), chain, &nets), addr("198.51.100.10"));
    }

    #[test]
    fn missing_empty_or_malformed_header_falls_back_to_the_peer() {
        // A broken header cannot be trusted to name a client, so the proxy address is used.
        // The limit then stays shared but safe, instead of letting garbage pick the key.
        let nets = proxies("203.0.113.116/32");
        let proxy = addr("203.0.113.116");
        let headers = [None, Some(""), Some("not-an-ip"), Some("198.51.100.10, junk"), Some("198.51.100.10,,")];
        for header in headers {
            assert_eq!(client_ip(proxy, header, &nets), proxy, "header {header:?}");
        }
    }

    #[test]
    fn a_chain_of_only_trusted_proxies_falls_back_to_the_peer() {
        // Every entry is a proxy, so no client can be named and the peer is the only safe key.
        let nets = proxies("203.0.113.116/32, 203.0.113.117/32");
        let proxy = addr("203.0.113.116");
        assert_eq!(client_ip(proxy, Some("203.0.113.117"), &nets), proxy);
    }

    #[test]
    fn empty_trusted_list_ignores_the_header() {
        // The default. With no proxy configured, the key must stay the TCP peer, as before.
        assert_eq!(client_ip(addr("203.0.113.116"), Some("198.51.100.10"), &[]), addr("203.0.113.116"));
    }

    #[test]
    fn ipv6_proxy_and_client_are_matched_by_prefix() {
        // A v6 proxy range must trust its own addresses, and a v6 client in the header must come back whole.
        let nets = proxies("2001:db8::/32");
        assert_eq!(client_ip(addr("2001:db8::1"), Some("2001:4860:4860::8888"), &nets), addr("2001:4860:4860::8888"));
        assert_eq!(client_ip(addr("2001:4860::1"), Some("198.51.100.10"), &nets), addr("2001:4860::1"));
    }

    #[test]
    fn ipv4_mapped_addresses_match_plain_ipv4() {
        // A dual-stack socket reports an IPv4 peer as ::ffff:a.b.c.d. That must match an IPv4 entry,
        // and a mapped address in the header must give the same key as the plain form.
        let mapped_entry = proxies("::ffff:203.0.113.116");
        let mapped = client_ip(addr("::ffff:203.0.113.116"), Some("::ffff:198.51.100.10"), &mapped_entry);
        assert_eq!(mapped, addr("198.51.100.10"));
        let plain_entry = proxies("203.0.113.116/32");
        assert_eq!(client_ip(addr("::ffff:203.0.113.116"), Some("198.51.100.10"), &plain_entry), addr("198.51.100.10"));
    }

    #[test]
    fn invalid_trusted_entry_is_rejected_and_named() {
        // A typo must not quietly trust nothing or the wrong range. Start-up stops and names the entry.
        let err = parse_trusted_proxies("203.0.113.116/32, 10.0.0.0/33").unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("DNSUI_TRUSTED_PROXIES"), "{msg}");
        assert!(msg.contains("10.0.0.0/33"), "{msg}");
        assert!(parse_trusted_proxies("203.0.113.116,").is_err(), "a stray comma is an error");
    }

    #[test]
    fn unset_or_blank_trusted_proxies_trust_nobody() {
        assert!(parse_trusted_proxies("").unwrap().is_empty());
        assert!(parse_trusted_proxies("   ").unwrap().is_empty());
    }

    fn fail_n_backoff(backoff: &Backoff, key: &str, n: u32, now: Instant) {
        for _ in 0..n {
            backoff.record_failure(key, now);
        }
    }
}
