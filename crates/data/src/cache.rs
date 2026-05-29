//! Time-to-live cache for generator results (`TECH.md §3.4`, `SCHEMA.md §1.5`).
//!
//! Generator output is cached by `(run argv, cwd)` for the spec's `cache.ttlMs`.
//! A warm hit returns the stored suggestion strings without re-executing the
//! process, which is what keeps generator-backed completion inside the NFR1
//! `< 15 ms` warm budget. Expiry is wall-clock based ([`Instant`]); an entry
//! older than its TTL is treated as a miss and re-run.
//!
//! The cache is deliberately small and dependency-free: a [`Mutex`]-guarded
//! [`HashMap`]. It is internally synchronised so a [`crate::SandboxedRunner`] is
//! `Sync` and can be shared behind `&dyn GeneratorRunner`.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Cache key: the exact argv plus the working directory. Two generators with the
/// same argv but different `cwd` (e.g. branches in different repos) never alias.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct CacheKey {
    run: Vec<String>,
    cwd: String,
}

/// A stored result and the instant after which it is stale.
#[derive(Debug, Clone)]
struct CacheEntry {
    values: Vec<String>,
    expires_at: Instant,
}

/// A TTL-keyed cache of generator results.
///
/// Construct with [`TtlCache::new`]. Use [`TtlCache::get`] to read a still-fresh
/// entry and [`TtlCache::put`] to store one with a TTL. A `ttl_ms` of `0` means
/// "do not cache" (the schema default), so [`TtlCache::put`] is a no-op then and
/// [`TtlCache::get`] always misses.
#[derive(Debug, Default)]
pub struct TtlCache {
    entries: Mutex<HashMap<CacheKey, CacheEntry>>,
}

impl TtlCache {
    /// Create an empty cache.
    pub fn new() -> Self {
        TtlCache {
            entries: Mutex::new(HashMap::new()),
        }
    }

    /// Return the cached values for `(run, cwd)` if present and not yet expired
    /// at `now`, else `None`. A poisoned lock degrades to a miss rather than a
    /// panic (the runner will simply re-execute).
    pub fn get(&self, run: &[String], cwd: &str, now: Instant) -> Option<Vec<String>> {
        let key = CacheKey {
            run: run.to_vec(),
            cwd: cwd.to_string(),
        };
        let guard = self.entries.lock().ok()?;
        let entry = guard.get(&key)?;
        if entry.expires_at > now {
            Some(entry.values.clone())
        } else {
            None
        }
    }

    /// Store `values` for `(run, cwd)` with a `ttl` measured from `now`. A
    /// zero-length `ttl` is ignored (no-cache per the schema default). A
    /// poisoned lock is ignored — caching is best-effort.
    pub fn put(&self, run: &[String], cwd: &str, values: Vec<String>, ttl: Duration, now: Instant) {
        if ttl.is_zero() {
            return;
        }
        let key = CacheKey {
            run: run.to_vec(),
            cwd: cwd.to_string(),
        };
        if let Ok(mut guard) = self.entries.lock() {
            guard.insert(
                key,
                CacheEntry {
                    values,
                    expires_at: now + ttl,
                },
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run() -> Vec<String> {
        vec!["git".to_string(), "branch".to_string()]
    }

    #[test]
    fn miss_when_empty() {
        let cache = TtlCache::new();
        assert_eq!(cache.get(&run(), "/repo", Instant::now()), None);
    }

    #[test]
    fn hit_within_ttl() {
        let cache = TtlCache::new();
        let now = Instant::now();
        cache.put(
            &run(),
            "/repo",
            vec!["main".to_string()],
            Duration::from_millis(1000),
            now,
        );
        let hit = cache.get(&run(), "/repo", now + Duration::from_millis(500));
        assert_eq!(hit, Some(vec!["main".to_string()]));
    }

    #[test]
    fn expired_is_a_miss() {
        let cache = TtlCache::new();
        let now = Instant::now();
        cache.put(
            &run(),
            "/repo",
            vec!["main".to_string()],
            Duration::from_millis(100),
            now,
        );
        // One ms past expiry => miss.
        let miss = cache.get(&run(), "/repo", now + Duration::from_millis(101));
        assert_eq!(miss, None);
    }

    #[test]
    fn zero_ttl_does_not_cache() {
        let cache = TtlCache::new();
        let now = Instant::now();
        cache.put(
            &run(),
            "/repo",
            vec!["main".to_string()],
            Duration::ZERO,
            now,
        );
        assert_eq!(cache.get(&run(), "/repo", now), None);
    }

    #[test]
    fn cwd_is_part_of_key() {
        let cache = TtlCache::new();
        let now = Instant::now();
        cache.put(
            &run(),
            "/repo-a",
            vec!["a".to_string()],
            Duration::from_secs(10),
            now,
        );
        assert_eq!(cache.get(&run(), "/repo-b", now), None);
        assert_eq!(
            cache.get(&run(), "/repo-a", now),
            Some(vec!["a".to_string()])
        );
    }
}
