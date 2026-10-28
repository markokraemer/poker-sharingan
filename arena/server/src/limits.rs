//! Abuse limits: a per-key token-bucket rate limiter (key = bearer token, else client IP).
//! Per-player caps (tables, matches, websockets) and size limits live next to the code they guard
//! and read their values from `ServerConfig`.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Instant;

struct Bucket {
    tokens: f64,
    last: Instant,
}

pub struct RateLimiter {
    rps: f64,
    burst: f64,
    buckets: Mutex<HashMap<String, Bucket>>,
}

const MAX_KEYS: usize = 50_000;

impl RateLimiter {
    /// `rps <= 0` disables limiting. `burst` is the bucket size (at least 1).
    pub fn new(rps: f64, burst: f64) -> Self {
        RateLimiter { rps, burst: burst.max(1.0), buckets: Mutex::new(HashMap::new()) }
    }

    pub fn enabled(&self) -> bool {
        self.rps > 0.0
    }

    /// `Ok(())` if a request may proceed, else the number of milliseconds until one may.
    pub fn check(&self, key: &str) -> Result<(), u64> {
        if !self.enabled() {
            return Ok(());
        }
        let now = Instant::now();
        let mut g = self.buckets.lock().unwrap_or_else(|e| e.into_inner());
        if g.len() >= MAX_KEYS && !g.contains_key(key) {
            // Forget buckets that have fully refilled; they carry no information.
            let (rps, burst) = (self.rps, self.burst);
            g.retain(|_, b| b.tokens + now.duration_since(b.last).as_secs_f64() * rps < burst);
            if g.len() >= MAX_KEYS {
                g.clear();
            }
        }
        let b = g.entry(key.to_string()).or_insert(Bucket { tokens: self.burst, last: now });
        b.tokens = (b.tokens + now.duration_since(b.last).as_secs_f64() * self.rps).min(self.burst);
        b.last = now;
        if b.tokens >= 1.0 {
            b.tokens -= 1.0;
            Ok(())
        } else {
            Err((((1.0 - b.tokens) / self.rps) * 1000.0).ceil() as u64)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn burst_then_block_then_independent_keys() {
        let l = RateLimiter::new(1.0, 3.0);
        for _ in 0..3 {
            assert!(l.check("a").is_ok());
        }
        let wait = l.check("a").unwrap_err();
        assert!(wait > 0 && wait <= 1000, "{wait}");
        assert!(l.check("b").is_ok());
    }

    #[test]
    fn refills_over_time() {
        let l = RateLimiter::new(1000.0, 1.0);
        assert!(l.check("a").is_ok());
        assert!(l.check("a").is_err());
        std::thread::sleep(std::time::Duration::from_millis(5));
        assert!(l.check("a").is_ok());
    }

    #[test]
    fn zero_disables() {
        let l = RateLimiter::new(0.0, 1.0);
        for _ in 0..100 {
            assert!(l.check("a").is_ok());
        }
    }
}
