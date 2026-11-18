use alloc::vec::Vec;
use ip_protocol::wire::Neighbour;
use net_protocol::ethernet::MAC_BYTES;
use net_protocol::ipv4::IPV4_BYTES;

pub const ENTRY_TTL_MS: u64 = 600_000;
pub const RESOLVE_RETRY_MS: u64 = 1_000;
pub const RESOLVE_MAX_ATTEMPTS: u32 = 3;
pub const MAX_CONCURRENT_RESOLVES: usize = 8;

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum Want {
    Resolved([u8; MAC_BYTES]),
    Started,
    Coalesced,
    Full,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Deadlines {
    pub retry: Vec<[u8; IPV4_BYTES]>,
    pub exhausted: Vec<[u8; IPV4_BYTES]>,
}

impl Deadlines {
    fn new() -> Deadlines {
        Deadlines {
            retry: Vec::new(),
            exhausted: Vec::new(),
        }
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
struct Entry {
    ip: [u8; IPV4_BYTES],
    mac: [u8; MAC_BYTES],
    updated_ms: u64,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
struct Flight {
    target: [u8; IPV4_BYTES],
    attempts: u32,
    last_ms: u64,
}

pub struct ArpCache {
    entries: Vec<Entry>,
    flights: Vec<Flight>,
}

impl ArpCache {
    pub fn new() -> ArpCache {
        ArpCache {
            entries: Vec::new(),
            flights: Vec::new(),
        }
    }

    pub fn learn(&mut self, ip: [u8; IPV4_BYTES], mac: [u8; MAC_BYTES], now_ms: u64) {
        match self.entries.iter_mut().find(|entry| entry.ip == ip) {
            Some(entry) => {
                entry.mac = mac;
                entry.updated_ms = now_ms;
            }
            None => self.entries.push(Entry {
                ip,
                mac,
                updated_ms: now_ms,
            }),
        }
        self.flights.retain(|flight| flight.target != ip);
    }

    pub fn lookup(&mut self, ip: [u8; IPV4_BYTES], now_ms: u64) -> Option<[u8; MAC_BYTES]> {
        self.entries
            .retain(|entry| now_ms.saturating_sub(entry.updated_ms) <= ENTRY_TTL_MS);
        self.entries
            .iter()
            .find(|entry| entry.ip == ip)
            .map(|entry| entry.mac)
    }

    pub fn snapshot(&mut self, now_ms: u64) -> Vec<Neighbour> {
        self.entries
            .retain(|entry| now_ms.saturating_sub(entry.updated_ms) <= ENTRY_TTL_MS);
        self.entries
            .iter()
            .map(|entry| Neighbour {
                address: entry.ip,
                mac: entry.mac,
                updated_ms: entry.updated_ms,
            })
            .collect()
    }

    pub fn want(&mut self, ip: [u8; IPV4_BYTES], now_ms: u64) -> Want {
        if let Some(mac) = self.lookup(ip, now_ms) {
            return Want::Resolved(mac);
        }
        if self.flights.iter().any(|flight| flight.target == ip) {
            return Want::Coalesced;
        }
        if self.flights.len() == MAX_CONCURRENT_RESOLVES {
            return Want::Full;
        }
        self.flights.push(Flight {
            target: ip,
            attempts: 1,
            last_ms: now_ms,
        });
        Want::Started
    }

    pub fn deadlines(&mut self, now_ms: u64) -> Deadlines {
        let mut deadlines = Deadlines::new();
        for flight in self.flights.iter_mut() {
            if now_ms.saturating_sub(flight.last_ms) < RESOLVE_RETRY_MS {
                continue;
            }
            if flight.attempts < RESOLVE_MAX_ATTEMPTS {
                flight.attempts += 1;
                flight.last_ms = now_ms;
                deadlines.retry.push(flight.target);
            } else {
                deadlines.exhausted.push(flight.target);
            }
        }
        self.flights
            .retain(|flight| !deadlines.exhausted.contains(&flight.target));
        deadlines
    }
}

impl Default for ArpCache {
    fn default() -> ArpCache {
        ArpCache::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MAC: [u8; MAC_BYTES] = [0x02, 0x52, 0x4e, 0x45, 0x32, 0x07];
    const PEER_MAC: [u8; MAC_BYTES] = [0x52, 0x55, 0x0a, 0x00, 0x02, 0x00];
    const IP: [u8; IPV4_BYTES] = [10, 0, 2, 15];
    const PEER: [u8; IPV4_BYTES] = [10, 0, 2, 2];

    #[test]
    fn learn_then_lookup_hits() {
        let mut cache = ArpCache::new();
        assert_eq!(cache.lookup(IP, 0), None);
        cache.learn(IP, MAC, 1_000);
        assert_eq!(cache.lookup(IP, 1_000), Some(MAC));
        assert_eq!(cache.lookup(PEER, 1_000), None);
    }

    #[test]
    fn learn_from_request_and_reply_both_refresh() {
        let mut cache = ArpCache::new();
        cache.learn(PEER, MAC, 0);
        cache.learn(PEER, PEER_MAC, 5_000);
        assert_eq!(cache.lookup(PEER, 5_000), Some(PEER_MAC));
    }

    #[test]
    fn entries_age_out_after_ttl() {
        let mut cache = ArpCache::new();
        cache.learn(IP, MAC, 0);
        assert_eq!(cache.lookup(IP, ENTRY_TTL_MS), Some(MAC));
        assert_eq!(cache.lookup(IP, ENTRY_TTL_MS + 1), None);
    }

    #[test]
    fn refresh_defers_expiration() {
        let mut cache = ArpCache::new();
        cache.learn(IP, MAC, 0);
        cache.learn(IP, MAC, ENTRY_TTL_MS);
        assert_eq!(cache.lookup(IP, ENTRY_TTL_MS * 2), Some(MAC));
    }

    #[test]
    fn want_resolved_reports_cache_hit_without_flight() {
        let mut cache = ArpCache::new();
        cache.learn(IP, MAC, 0);
        assert_eq!(cache.want(IP, 0), Want::Resolved(MAC));
        let deadlines = cache.deadlines(RESOLVE_RETRY_MS * 10);
        assert!(deadlines.retry.is_empty() && deadlines.exhausted.is_empty());
    }

    #[test]
    fn want_coalesces_concurrent_waiters_on_one_flight() {
        let mut cache = ArpCache::new();
        assert_eq!(cache.want(PEER, 0), Want::Started);
        assert_eq!(cache.want(PEER, 10), Want::Coalesced);
        assert_eq!(cache.want(PEER, 20), Want::Coalesced);
    }

    #[test]
    fn learn_completes_the_flight() {
        let mut cache = ArpCache::new();
        assert_eq!(cache.want(PEER, 0), Want::Started);
        cache.learn(PEER, PEER_MAC, 50);
        assert_eq!(cache.want(PEER, 50), Want::Resolved(PEER_MAC));
        let deadlines = cache.deadlines(RESOLVE_RETRY_MS * 10);
        assert!(deadlines.retry.is_empty() && deadlines.exhausted.is_empty());
    }

    #[test]
    fn retries_until_max_attempts_then_exhausts() {
        let mut cache = ArpCache::new();
        assert_eq!(cache.want(PEER, 0), Want::Started);
        let first = cache.deadlines(RESOLVE_RETRY_MS);
        assert_eq!(first.retry, alloc::vec![PEER]);
        assert!(first.exhausted.is_empty());
        let second = cache.deadlines(RESOLVE_RETRY_MS * 2);
        assert_eq!(second.retry, alloc::vec![PEER]);
        let third = cache.deadlines(RESOLVE_RETRY_MS * 3);
        assert!(third.retry.is_empty());
        assert_eq!(third.exhausted, alloc::vec![PEER]);
        let after = cache.deadlines(RESOLVE_RETRY_MS * 10);
        assert!(after.retry.is_empty() && after.exhausted.is_empty());
        assert_eq!(cache.want(PEER, RESOLVE_RETRY_MS * 10), Want::Started);
    }

    #[test]
    fn flights_are_bounded_and_report_full() {
        let mut cache = ArpCache::new();
        for index in 0..MAX_CONCURRENT_RESOLVES {
            let target = [192, 168, 1, index as u8];
            assert_eq!(cache.want(target, 0), Want::Started);
        }
        let eleventh = [192, 168, 9, 9];
        assert_eq!(cache.want(eleventh, 0), Want::Full);
        assert_eq!(cache.want(PEER, 0), Want::Full);
    }

    #[test]
    fn exhausted_flight_frees_room_for_a_fresh_resolve() {
        let mut cache = ArpCache::new();
        for index in 0..MAX_CONCURRENT_RESOLVES {
            assert_eq!(cache.want([192, 168, 1, index as u8], 0), Want::Started);
        }
        for attempt in 1..RESOLVE_MAX_ATTEMPTS as u64 {
            let deadlines = cache.deadlines(attempt * RESOLVE_RETRY_MS);
            assert_eq!(deadlines.retry.len(), MAX_CONCURRENT_RESOLVES);
            assert!(deadlines.exhausted.is_empty());
        }
        let exhausted = cache.deadlines(RESOLVE_RETRY_MS * RESOLVE_MAX_ATTEMPTS as u64);
        assert_eq!(exhausted.exhausted.len(), MAX_CONCURRENT_RESOLVES);
        assert_eq!(cache.want([192, 168, 9, 9], 0), Want::Started);
    }

    #[test]
    fn deadlines_do_not_fire_before_the_retry_interval() {
        let mut cache = ArpCache::new();
        assert_eq!(cache.want(PEER, 1_000), Want::Started);
        let early = cache.deadlines(1_000 + RESOLVE_RETRY_MS - 1);
        assert!(early.retry.is_empty() && early.exhausted.is_empty());
    }

    #[test]
    fn snapshot_prunes_expired_and_maps_fresh_entries() {
        let mut cache = ArpCache::new();
        cache.learn(IP, MAC, 0);
        cache.learn(PEER, PEER_MAC, 1_000);
        let rows = cache.snapshot(ENTRY_TTL_MS + 1);
        assert_eq!(
            rows,
            alloc::vec![Neighbour {
                address: PEER,
                mac: PEER_MAC,
                updated_ms: 1_000,
            }]
        );
        assert!(cache.snapshot(ENTRY_TTL_MS + 2_000).is_empty());
    }
}
