//! Login rate limiting and account lockout manager.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

const MAX_TRACKED_ATTEMPTS: usize = 4096;

#[derive(Debug, Clone)]
struct Attempt {
    failed_count: u32,
    last_failed_at: u64,
    locked_until: Option<u64>,
}

#[derive(Debug)]
pub struct LockoutManager {
    attempts: Mutex<HashMap<String, Attempt>>,
    max_failed_attempts: u32,
    lockout_duration_secs: u64,
}

impl LockoutManager {
    pub fn new(max_failed_attempts: u32, lockout_duration_secs: u64) -> Self {
        Self {
            attempts: Mutex::new(HashMap::new()),
            max_failed_attempts,
            lockout_duration_secs,
        }
    }

    fn now() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
    }

    /// Checks if a key (e.g. username) is currently locked out.
    /// Returns `Ok(())` if allowed, or `Err(remaining_secs)` if locked.
    pub fn check_lockout(&self, key: &str) -> Result<(), u64> {
        let now = Self::now();
        let mut map = self.attempts.lock().unwrap_or_else(|e| e.into_inner());
        if !map.contains_key(key) && map.len() >= MAX_TRACKED_ATTEMPTS {
            map.retain(|_, attempt| {
                now.saturating_sub(attempt.last_failed_at) < self.lockout_duration_secs
            });
            if map.len() >= MAX_TRACKED_ATTEMPTS {
                return Err(self.lockout_duration_secs);
            }
        }
        if let Some(record) = map.get_mut(key) {
            match record.locked_until {
                Some(locked_until) if now < locked_until => return Err(locked_until - now),
                Some(_) => {
                    // Lockout period has elapsed; clear the lock
                    record.failed_count = 0;
                    record.locked_until = None;
                }
                None => {}
            }
        }
        Ok(())
    }

    /// Records a failed login attempt for a key.
    /// Returns `Some(lockout_secs)` if this attempt triggered a lockout, `None` otherwise.
    pub fn record_failure(&self, key: &str) -> Option<u64> {
        let now = Self::now();
        let mut map = self.attempts.lock().unwrap_or_else(|e| e.into_inner());
        if !map.contains_key(key) && map.len() >= MAX_TRACKED_ATTEMPTS {
            map.retain(|_, attempt| {
                now.saturating_sub(attempt.last_failed_at) < self.lockout_duration_secs
            });
            if map.len() >= MAX_TRACKED_ATTEMPTS {
                return Some(self.lockout_duration_secs);
            }
        }
        let record = map.entry(key.to_string()).or_insert(Attempt {
            failed_count: 0,
            last_failed_at: now,
            locked_until: None,
        });

        record.failed_count = record.failed_count.saturating_add(1);
        record.last_failed_at = now;

        if record.failed_count >= self.max_failed_attempts {
            let locked_until = now + self.lockout_duration_secs;
            record.locked_until = Some(locked_until);
            Some(self.lockout_duration_secs)
        } else {
            None
        }
    }

    /// Records a successful login for a key, resetting any failure counter.
    pub fn record_success(&self, key: &str) {
        let mut map = self.attempts.lock().unwrap_or_else(|e| e.into_inner());
        map.remove(key);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn attacker_selected_usernames_cannot_grow_memory_or_bypass_full_cache() {
        let manager = LockoutManager::new(5, 900);
        for index in 0..MAX_TRACKED_ATTEMPTS + 100 {
            manager.record_failure(&format!("unknown{index}"));
        }
        assert_eq!(manager.attempts.lock().unwrap().len(), MAX_TRACKED_ATTEMPTS);
        assert!(manager.check_lockout("new-user").is_err());
        {
            let mut map = manager.attempts.lock().unwrap();
            for record in map.values_mut() {
                record.last_failed_at = 0;
            }
        }
        assert!(manager.check_lockout("new-user").is_ok());
        assert!(manager.record_failure("new-user").is_none());
        assert_eq!(manager.attempts.lock().unwrap().len(), 1);
    }
}
