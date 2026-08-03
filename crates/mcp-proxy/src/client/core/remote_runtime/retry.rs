use std::time::Duration;

#[derive(Debug)]
pub(super) enum ReconnectAttemptError {
    Connect(anyhow::Error),
    Install(anyhow::Error),
}

impl ReconnectAttemptError {
    pub(super) fn into_anyhow(self) -> anyhow::Error {
        match self {
            Self::Connect(error) => error.context("backend reconnect failed"),
            Self::Install(error) => error.context("reconnected backend installation failed"),
        }
    }
}

#[derive(Debug, Clone)]
pub(super) struct RetryState {
    pub(super) failures: u32,
    pub(super) max_failures: u32,
    delay: Duration,
    initial_delay: Duration,
    max_delay: Duration,
}

impl RetryState {
    pub(super) fn new(max_failures: u32) -> Self {
        Self {
            failures: 0,
            max_failures,
            delay: Duration::from_secs(1),
            initial_delay: Duration::from_secs(1),
            max_delay: Duration::from_secs(30),
        }
    }

    #[cfg(test)]
    pub(super) fn with_delays(
        max_failures: u32,
        initial_delay: Duration,
        max_delay: Duration,
    ) -> Self {
        Self {
            failures: 0,
            max_failures,
            delay: initial_delay,
            initial_delay,
            max_delay,
        }
    }

    pub(super) fn record_failure(&mut self) -> bool {
        self.failures = self.failures.saturating_add(1);
        self.max_failures > 0 && self.failures >= self.max_failures
    }

    pub(super) fn reset(&mut self) {
        self.failures = 0;
        self.delay = self.initial_delay;
    }

    pub(super) fn current_delay(&self) -> Duration {
        self.delay
    }

    pub(super) fn advance_delay(&mut self) {
        self.delay = self.delay.saturating_mul(2).min(self.max_delay);
    }
}
