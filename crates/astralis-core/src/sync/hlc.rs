//! Hybrid logical clock: wall-clock milliseconds plus a counter, so timestamps from different
//! devices order sensibly and never go backwards even if a device's clock does.
//! Encoded as `"{ms:013}.{counter:05}.{device}"`, which sorts correctly as plain text.

use crate::util::now_ms;

#[derive(Debug, Default)]
pub struct Clock {
    last_ms: i64,
    counter: u32,
}

impl Clock {
    pub fn now(&mut self, device: &str) -> String {
        let ms = now_ms();
        if ms > self.last_ms {
            self.last_ms = ms;
            self.counter = 0;
        } else {
            self.counter += 1;
        }
        format!("{:013}.{:05}.{}", self.last_ms, self.counter, device)
    }

    /// Moves the clock past a timestamp received from another device.
    pub fn observe(&mut self, remote: &str) {
        let mut parts = remote.splitn(3, '.');
        let (Some(ms), Some(c)) = (
            parts.next().and_then(|s| s.parse::<i64>().ok()),
            parts.next().and_then(|s| s.parse::<u32>().ok()),
        ) else {
            return;
        };
        if (ms, c) > (self.last_ms, self.counter) {
            self.last_ms = ms;
            self.counter = c;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn monotonic_and_sortable() {
        let mut c = Clock::default();
        let a = c.now("dev");
        let b = c.now("dev");
        assert!(b > a);
        c.observe("9999999999999.00007.other");
        let d = c.now("dev");
        assert!(d.starts_with("9999999999999.00008"), "{d}");
    }
}
