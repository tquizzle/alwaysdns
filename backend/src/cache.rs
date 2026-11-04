//! Bounded, TTL-respecting answer cache. Entries are keyed by upstream group,
//! so answers obtained through one group never serve a client on another.

use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use hickory_proto::op::ResponseCode;
use hickory_proto::rr::{Record, RecordType};

pub const CACHE_CAPACITY: usize = 10_000;
/// Lifetime for negative answers (NXDOMAIN and NODATA).
const NEGATIVE_TTL: Duration = Duration::from_secs(30);

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CacheKey {
    pub group: String,
    pub name: String,
    pub qtype: RecordType,
}

pub struct CachedAnswer {
    pub answers: Vec<Record>,
    pub rcode: ResponseCode,
}

struct Entry {
    answers: Vec<Record>,
    rcode: ResponseCode,
    inserted: Instant,
    expires: Instant,
    original_ttl: u32,
}

#[derive(Default)]
struct Inner {
    map: HashMap<CacheKey, Entry>,
    /// Insertion order for eviction. May contain keys already removed.
    order: VecDeque<CacheKey>,
}

#[derive(Default)]
pub struct DnsCache {
    inner: Mutex<Inner>,
}

impl DnsCache {
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns the answer with TTLs reduced by the time spent in cache.
    pub fn get(&self, key: &CacheKey) -> Option<CachedAnswer> {
        let mut inner = self.inner.lock().ok()?;
        let now = Instant::now();
        let entry = inner.map.get(key)?;
        if entry.expires <= now {
            inner.map.remove(key);
            return None;
        }
        let elapsed = now.duration_since(entry.inserted).as_secs() as u32;
        let remaining = entry.original_ttl.saturating_sub(elapsed).max(1);
        let answers = entry
            .answers
            .iter()
            .map(|r| Record::from_rdata(r.name.clone(), remaining, r.data.clone()))
            .collect();
        Some(CachedAnswer {
            answers,
            rcode: entry.rcode,
        })
    }

    /// Stores a positive answer using the smallest record TTL, or a negative
    /// answer with a fixed short TTL. Answers with a zero TTL are not stored.
    pub fn put(&self, key: CacheKey, answers: &[Record], rcode: ResponseCode) {
        let ttl = if answers.is_empty() {
            NEGATIVE_TTL.as_secs() as u32
        } else {
            answers.iter().map(|r| r.ttl).min().unwrap_or(0)
        };
        if ttl == 0 {
            return;
        }
        let Ok(mut inner) = self.inner.lock() else {
            return;
        };
        let now = Instant::now();

        if !inner.map.contains_key(&key) {
            if inner.map.len() >= CACHE_CAPACITY {
                evict(&mut inner, now);
            }
            inner.order.push_back(key.clone());
        }
        inner.map.insert(
            key,
            Entry {
                answers: answers.to_vec(),
                rcode,
                inserted: now,
                expires: now + Duration::from_secs(u64::from(ttl)),
                original_ttl: ttl,
            },
        );
    }

    pub fn clear(&self) {
        if let Ok(mut inner) = self.inner.lock() {
            inner.map.clear();
            inner.order.clear();
        }
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.inner.lock().map(|i| i.map.len()).unwrap_or(0)
    }
}

/// Drops expired entries first. If the cache is still full, drops the oldest
/// inserted entry.
fn evict(inner: &mut Inner, now: Instant) {
    let expired: Vec<CacheKey> = inner
        .map
        .iter()
        .filter(|(_, e)| e.expires <= now)
        .map(|(k, _)| k.clone())
        .collect();
    for key in expired {
        inner.map.remove(&key);
    }
    while inner.map.len() >= CACHE_CAPACITY {
        match inner.order.pop_front() {
            Some(old) => {
                inner.map.remove(&old);
            }
            None => break,
        }
    }
    // Keep the order queue from growing without bound with stale keys.
    if inner.order.len() > CACHE_CAPACITY * 2 {
        let live: VecDeque<CacheKey> = inner
            .order
            .drain(..)
            .filter(|k| inner.map.contains_key(k))
            .collect();
        inner.order = live;
    }
}

/// Helper for building keys from a name string and type.
pub fn make_key(group: &str, name: &str, qtype: RecordType) -> CacheKey {
    CacheKey {
        group: group.to_string(),
        name: name.to_ascii_lowercase(),
        qtype,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hickory_proto::rr::{Name, RData, rdata::A};
    use std::net::Ipv4Addr;

    fn a_record(name: &str, ttl: u32) -> Record {
        Record::from_rdata(
            Name::from_ascii(name).unwrap(),
            ttl,
            RData::A(A(Ipv4Addr::new(1, 2, 3, 4))),
        )
    }

    #[test]
    fn stores_and_returns_answers() {
        let cache = DnsCache::new();
        let key = make_key("default", "example.com", RecordType::A);
        cache.put(key.clone(), &[a_record("example.com.", 300)], ResponseCode::NoError);
        let hit = cache.get(&key).expect("should be cached");
        assert_eq!(hit.answers.len(), 1);
        assert!(hit.answers[0].ttl <= 300 && hit.answers[0].ttl >= 299);
    }

    #[test]
    fn keys_are_scoped_by_group_and_type() {
        let cache = DnsCache::new();
        cache.put(
            make_key("nextdns", "example.com", RecordType::A),
            &[a_record("example.com.", 300)],
            ResponseCode::NoError,
        );
        assert!(cache.get(&make_key("default", "example.com", RecordType::A)).is_none());
        assert!(cache.get(&make_key("nextdns", "example.com", RecordType::AAAA)).is_none());
    }

    #[test]
    fn zero_ttl_is_not_stored() {
        let cache = DnsCache::new();
        let key = make_key("default", "zero.example", RecordType::A);
        cache.put(key.clone(), &[a_record("zero.example.", 0)], ResponseCode::NoError);
        assert!(cache.get(&key).is_none());
    }

    #[test]
    fn bounded_capacity_evicts_oldest() {
        let cache = DnsCache::new();
        for i in 0..(CACHE_CAPACITY + 50) {
            let key = make_key("default", &format!("h{i}.example"), RecordType::A);
            cache.put(key, &[a_record(&format!("h{i}.example."), 600)], ResponseCode::NoError);
        }
        assert_eq!(cache.len(), CACHE_CAPACITY);
        assert!(cache.get(&make_key("default", "h0.example", RecordType::A)).is_none());
        assert!(cache
            .get(&make_key("default", &format!("h{}.example", CACHE_CAPACITY + 49), RecordType::A))
            .is_some());
    }
}
