use std::time::Duration;

/// Calculate an exponential retry delay without overflowing or panicking.
///
/// The delay is `base * 2^attempt`, capped at `maximum`. Attempts whose
/// multiplier cannot be represented by `u32` are already beyond the useful
/// exponential range and therefore return `maximum` directly.
pub fn capped_exponential_delay(base: Duration, maximum: Duration, attempt: usize) -> Duration {
    if base >= maximum {
        return maximum;
    }

    let Ok(shift) = u32::try_from(attempt) else {
        return maximum;
    };
    let Some(multiplier) = 1_u32.checked_shl(shift) else {
        return maximum;
    };

    base.checked_mul(multiplier).unwrap_or(maximum).min(maximum)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn caps_large_attempts_without_overflow() {
        let base = Duration::from_secs(1);
        let maximum = Duration::from_secs(60);

        assert_eq!(
            capped_exponential_delay(base, maximum, 0),
            Duration::from_secs(1)
        );
        assert_eq!(
            capped_exponential_delay(base, maximum, 1),
            Duration::from_secs(2)
        );
        assert_eq!(capped_exponential_delay(base, maximum, 31), maximum);
        assert_eq!(capped_exponential_delay(base, maximum, 32), maximum);
        assert_eq!(capped_exponential_delay(base, maximum, usize::MAX), maximum);
    }

    #[test]
    fn caps_when_base_is_not_smaller_than_maximum() {
        assert_eq!(
            capped_exponential_delay(Duration::from_secs(90), Duration::from_secs(60), 0,),
            Duration::from_secs(60)
        );
    }
}
