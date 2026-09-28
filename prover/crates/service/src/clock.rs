use std::time::{Duration, SystemTime, UNIX_EPOCH};

use tokio::time::Instant;

#[derive(Clone, Debug)]
pub struct Clock {
    epoch_ms: u128,
    started: Instant,
}

impl Default for Clock {
    fn default() -> Self {
        Self {
            epoch_ms: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system time before unix epoch")
                .as_millis(),
            started: Instant::now(),
        }
    }
}

impl Clock {
    pub fn now_ms(&self) -> u128 {
        self.epoch_ms + self.started.elapsed().as_millis()
    }

    pub async fn sleep_until_ms(&self, at_ms: u128) {
        let now = self.now_ms();
        if at_ms > now {
            tokio::time::sleep(Duration::from_millis((at_ms - now) as u64)).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(start_paused = true)]
    async fn clock_follows_tokio_time() {
        let clock = Clock::default();
        let start = clock.now_ms();
        tokio::time::advance(Duration::from_secs(5)).await;
        assert_eq!(clock.now_ms(), start + 5_000);
    }

    #[tokio::test(start_paused = true)]
    async fn sleep_until_returns_at_the_deadline() {
        let clock = Clock::default();
        let deadline = clock.now_ms() + 30_000;
        clock.sleep_until_ms(deadline).await;
        assert_eq!(clock.now_ms(), deadline);
        clock.sleep_until_ms(deadline - 1).await;
        assert_eq!(clock.now_ms(), deadline);
    }
}
