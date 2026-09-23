//! Время как зависимость: ретраи и троттлинг проверяются в тестах без реального сна.

use std::sync::Mutex;
use std::time::{Duration, Instant};

pub trait Clock {
    /// Монотонное время от создания часов — для троттлинга.
    fn elapsed(&self) -> Duration;
    fn sleep(&self, duration: Duration);
    /// Unix-время в мс — запасной вариант, если в ответе нет заголовка `Date`.
    fn unix_millis(&self) -> i64;
}

pub struct SystemClock {
    start: Instant,
}

impl SystemClock {
    pub fn new() -> Self {
        SystemClock { start: Instant::now() }
    }
}

impl Default for SystemClock {
    fn default() -> Self {
        Self::new()
    }
}

impl Clock for SystemClock {
    fn elapsed(&self) -> Duration {
        self.start.elapsed()
    }

    fn sleep(&self, duration: Duration) {
        std::thread::sleep(duration);
    }

    fn unix_millis(&self) -> i64 {
        crate::time::system_now_millis()
    }
}

/// Часы для тестов: `sleep` мгновенно сдвигает время и запоминает паузу.
pub struct FakeClock {
    start_unix_millis: i64,
    now: Mutex<Duration>,
    sleeps: Mutex<Vec<Duration>>,
}

impl FakeClock {
    pub fn new(start_unix_millis: i64) -> Self {
        FakeClock {
            start_unix_millis,
            now: Mutex::new(Duration::ZERO),
            sleeps: Mutex::new(Vec::new()),
        }
    }

    pub fn sleeps(&self) -> Vec<Duration> {
        self.sleeps.lock().unwrap().clone()
    }
}

impl Clock for FakeClock {
    fn elapsed(&self) -> Duration {
        *self.now.lock().unwrap()
    }

    fn sleep(&self, duration: Duration) {
        self.sleeps.lock().unwrap().push(duration);
        *self.now.lock().unwrap() += duration;
    }

    fn unix_millis(&self) -> i64 {
        self.start_unix_millis + self.elapsed().as_millis() as i64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fake_clock_advances_on_sleep() {
        let clock = FakeClock::new(1_000);
        clock.sleep(Duration::from_millis(250));
        assert_eq!(clock.elapsed(), Duration::from_millis(250));
        assert_eq!(clock.unix_millis(), 1_250);
        assert_eq!(clock.sleeps(), vec![Duration::from_millis(250)]);
    }
}
