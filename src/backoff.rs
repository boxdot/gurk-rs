use std::time::Duration;

const FIBONACCI_TIMEOUTS: [Duration; 9] = [
    Duration::from_secs(1),
    Duration::from_secs(2),
    Duration::from_secs(3),
    Duration::from_secs(5),
    Duration::from_secs(8),
    Duration::from_secs(13),
    Duration::from_secs(21),
    Duration::from_secs(34),
    Duration::from_secs(55),
];

#[derive(Debug, Default)]
pub struct Backoff {
    count: usize,
}

impl Backoff {
    pub fn new() -> Self {
        Default::default()
    }

    pub fn get(&mut self) -> Duration {
        let timeout = FIBONACCI_TIMEOUTS[self.count];
        if self.count + 1 < FIBONACCI_TIMEOUTS.len() {
            self.count += 1;
        }
        timeout
    }

    pub fn reset(&mut self) {
        self.count = 0;
    }
}
