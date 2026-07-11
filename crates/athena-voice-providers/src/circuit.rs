use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Circuit-breaker state machine.
///
/// - `Closed` — normal operation; count failures within a rolling window.
/// - `Open`   — reject calls immediately until `until`; then transition to `HalfOpen`.
/// - `HalfOpen` — allow one probe every `half_open_probe_gap`; on success close, on failure open again.
pub struct CircuitBreaker {
    failure_threshold: u32,
    open_duration: Duration,
    half_open_probe_gap: Duration,
    state: Mutex<State>,
}

#[derive(Debug, Clone, Copy)]
enum State {
    Closed { failures: u32 },
    Open { until: Instant },
    HalfOpen { last_probe: Instant },
}

impl CircuitBreaker {
    pub fn new(
        failure_threshold: u32,
        open_duration: Duration,
        half_open_probe_gap: Duration,
    ) -> Self {
        Self {
            failure_threshold,
            open_duration,
            half_open_probe_gap,
            state: Mutex::new(State::Closed { failures: 0 }),
        }
    }

    /// Returns `Ok(())` if the circuit permits a call now, or `Err(retry_after)`
    /// with the wall-clock duration until the caller should try again.
    pub fn can_call(&self) -> Result<(), Duration> {
        let now = Instant::now();
        let mut st = self.state.lock().unwrap();
        match *st {
            State::Closed { .. } => Ok(()),
            State::Open { until } if now >= until => {
                *st = State::HalfOpen { last_probe: now };
                Ok(())
            }
            State::Open { until } => Err(until - now),
            State::HalfOpen { last_probe } if now - last_probe >= self.half_open_probe_gap => {
                *st = State::HalfOpen { last_probe: now };
                Ok(())
            }
            State::HalfOpen { last_probe } => Err(self.half_open_probe_gap - (now - last_probe)),
        }
    }

    /// Records a successful call. Resets or closes the circuit.
    pub fn record_success(&self) {
        let mut st = self.state.lock().unwrap();
        *st = State::Closed { failures: 0 };
    }

    /// Records a failed call. Opens the circuit at threshold; re-opens if in
    /// half-open state.
    pub fn record_failure(&self) {
        let now = Instant::now();
        let mut st = self.state.lock().unwrap();
        match *st {
            State::Closed { failures } => {
                let new_failures = failures + 1;
                if new_failures >= self.failure_threshold {
                    *st = State::Open { until: now + self.open_duration };
                } else {
                    *st = State::Closed { failures: new_failures };
                }
            }
            State::HalfOpen { .. } => {
                *st = State::Open { until: now + self.open_duration };
            }
            State::Open { .. } => {}
        }
    }

    pub fn is_open(&self) -> bool {
        matches!(*self.state.lock().unwrap(), State::Open { .. })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread::sleep;

    fn cb() -> CircuitBreaker {
        CircuitBreaker::new(3, Duration::from_millis(100), Duration::from_millis(30))
    }

    #[test]
    fn closed_after_construction() {
        let c = cb();
        assert!(c.can_call().is_ok());
        assert!(!c.is_open());
    }

    #[test]
    fn open_after_threshold_failures() {
        let c = cb();
        c.record_failure();
        c.record_failure();
        assert!(c.can_call().is_ok());
        c.record_failure();
        assert!(c.is_open());
        assert!(c.can_call().is_err());
    }

    #[test]
    fn half_open_after_duration_then_success_closes() {
        let c = cb();
        for _ in 0..3 {
            c.record_failure();
        }
        assert!(c.is_open());
        sleep(Duration::from_millis(120));
        assert!(c.can_call().is_ok()); // enters half-open
        c.record_success();
        assert!(!c.is_open());
        // Fresh failures count from zero again.
        c.record_failure();
        c.record_failure();
        assert!(c.can_call().is_ok());
    }

    #[test]
    fn failure_in_half_open_reopens() {
        let c = cb();
        for _ in 0..3 {
            c.record_failure();
        }
        sleep(Duration::from_millis(120));
        assert!(c.can_call().is_ok()); // half-open
        c.record_failure();
        assert!(c.is_open());
    }

    #[test]
    fn success_records_reset_failures() {
        let c = cb();
        c.record_failure();
        c.record_failure();
        c.record_success();
        // Should require 3 more failures to open.
        c.record_failure();
        c.record_failure();
        assert!(c.can_call().is_ok());
        c.record_failure();
        assert!(c.is_open());
    }
}
