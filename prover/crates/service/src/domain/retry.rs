use std::time::Duration;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RetryPolicy {
    pub base: Duration,
    pub max_attempts: u32,
    pub max_rebases: u32,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            base: Duration::from_secs(60),
            max_attempts: 5,
            max_rebases: 3,
        }
    }
}

impl RetryPolicy {
    pub fn not_before(&self, attempts: u32, at_ms: u128) -> u128 {
        let doublings = attempts.saturating_sub(1).min(20);
        at_ms + self.base.as_millis() * (1u128 << doublings)
    }

    pub fn parked(&self, attempts: u32) -> bool {
        attempts >= self.max_attempts
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn policy() -> RetryPolicy {
        RetryPolicy {
            base: Duration::from_secs(60),
            max_attempts: 3,
            max_rebases: 1,
        }
    }

    #[test]
    fn backoff_doubles_from_base() {
        let policy = policy();
        assert_eq!(policy.not_before(1, 1_000), 61_000);
        assert_eq!(policy.not_before(2, 1_000), 121_000);
        assert_eq!(policy.not_before(3, 1_000), 241_000);
    }

    #[test]
    fn parks_at_max_attempts() {
        let policy = policy();
        assert!(!policy.parked(2));
        assert!(policy.parked(3));
        assert!(policy.parked(4));
    }
}
