//! One retry loop shared by the native library and binary, with caller-selected backoff.

use std::io;
use std::time::{Duration, Instant};

/// Retry only contention, preserving the last I/O error for caller-specific context.
pub(crate) fn open_with_retry<T>(
    timeout: Duration,
    initial_backoff: Duration,
    max_backoff: Duration,
    open: impl FnMut() -> io::Result<T>,
) -> io::Result<T> {
    let started = Instant::now();
    retry(
        timeout,
        initial_backoff,
        max_backoff,
        open,
        || started.elapsed(),
        std::thread::sleep,
    )
}

fn retry<T>(
    timeout: Duration,
    mut backoff: Duration,
    max_backoff: Duration,
    mut open: impl FnMut() -> io::Result<T>,
    mut elapsed: impl FnMut() -> Duration,
    mut pause: impl FnMut(Duration),
) -> io::Result<T> {
    loop {
        match open() {
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                let remaining = timeout.saturating_sub(elapsed());
                if remaining.is_zero() {
                    return Err(error);
                }
                pause(backoff.min(remaining));
                backoff = (backoff * 2).min(max_backoff);
            }
            result => return result,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    #[test]
    fn backoff_preserves_the_runtime_schedule_and_deadline() {
        let elapsed = Cell::new(Duration::ZERO);
        let mut waits = Vec::new();
        let mut attempts = 0;
        let error = retry(
            Duration::from_secs(1),
            Duration::from_millis(10),
            Duration::from_millis(250),
            || {
                attempts += 1;
                Err::<(), _>(io::Error::new(
                    io::ErrorKind::WouldBlock,
                    attempts.to_string(),
                ))
            },
            || elapsed.get(),
            |wait| {
                waits.push(wait.as_millis());
                elapsed.set(elapsed.get() + wait);
            },
        )
        .unwrap_err();
        assert_eq!(waits, [10, 20, 40, 80, 160, 250, 250, 190]);
        assert_eq!(attempts, 9);
        assert_eq!(error.kind(), io::ErrorKind::WouldBlock);
        assert_eq!(error.to_string(), "9", "keep the last contention error");
    }

    #[test]
    fn fixed_backoff_retries_transient_contention() {
        let elapsed = Cell::new(Duration::ZERO);
        let mut waits = Vec::new();
        let mut attempts = 0;
        let value = retry(
            Duration::from_millis(250),
            Duration::from_millis(1),
            Duration::from_millis(1),
            || {
                attempts += 1;
                if attempts < 4 {
                    Err(io::Error::from(io::ErrorKind::WouldBlock))
                } else {
                    Ok("opened")
                }
            },
            || elapsed.get(),
            |wait| {
                waits.push(wait.as_millis());
                elapsed.set(elapsed.get() + wait);
            },
        )
        .unwrap();
        assert_eq!(value, "opened");
        assert_eq!(waits, [1, 1, 1]);
        assert_eq!(attempts, 4);
    }

    #[test]
    fn zero_budget_still_attempts_open_once() {
        let mut attempts = 0;
        let error = open_with_retry(
            Duration::ZERO,
            Duration::from_millis(10),
            Duration::from_millis(250),
            || {
                attempts += 1;
                Err::<(), _>(io::Error::new(io::ErrorKind::WouldBlock, "original error"))
            },
        )
        .unwrap_err();
        assert_eq!(attempts, 1);
        assert_eq!(error.kind(), io::ErrorKind::WouldBlock);
        assert_eq!(error.to_string(), "original error");
    }

    #[test]
    fn other_io_errors_are_returned_without_retry() {
        for kind in [
            io::ErrorKind::NotFound,
            io::ErrorKind::PermissionDenied,
            io::ErrorKind::InvalidData,
        ] {
            let mut attempts = 0;
            let error = retry(
                Duration::from_millis(250),
                Duration::from_millis(1),
                Duration::from_millis(1),
                || {
                    attempts += 1;
                    Err::<(), _>(io::Error::new(kind, "original error"))
                },
                || Duration::ZERO,
                |_| panic!("non-contention errors must not wait"),
            )
            .unwrap_err();
            assert_eq!(error.kind(), kind);
            assert_eq!(error.to_string(), "original error");
            assert_eq!(attempts, 1);
        }
    }
}
