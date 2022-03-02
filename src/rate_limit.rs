use std::time::Instant;

pub struct RateLimit {
    min_bytes_per_sec: u128,
    last_check: Instant
}

impl RateLimit {
    pub fn new(min_bytes_per_sec: u128) -> Self {
        Self {
            min_bytes_per_sec: min_bytes_per_sec,
            last_check: Instant::now()
        }
    }

    pub fn exceeds_min_rate(&mut self, bytes_read: usize) -> bool {
        let elapsed = self.last_check.elapsed().as_millis();
        self.last_check = Instant::now();

        if elapsed > 0 {
            (bytes_read as u128 * 1000) / elapsed > self.min_bytes_per_sec
        } else {
            true
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::rate_limit::*;
    use std::time::Duration;
    use std::thread::sleep;

    #[test]
    fn test_rate_limit() {
        let mut l = RateLimit::new(10);

        sleep(Duration::from_millis(500));
        assert_eq!(l.exceeds_min_rate(6), true);
        sleep(Duration::from_secs(1));
        assert_eq!(l.exceeds_min_rate(7), false);
    }
}
