//! Bounded retries for directory opens that encounter a short-lived lock holder.

use std::io;
use std::time::{Duration, Instant};

pub(super) const OPEN_RETRY_TIMEOUT: Duration = Duration::from_millis(250);

/// Only lock contention is retried. Keep the I/O error intact until the caller adds context.
pub(super) fn open_with_retry<T>(
    timeout: Duration,
    mut open: impl FnMut() -> io::Result<T>,
) -> io::Result<T> {
    let started = Instant::now();
    loop {
        match open() {
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                let remaining = timeout.saturating_sub(started.elapsed());
                if remaining.is_zero() {
                    return Err(error);
                }
                std::thread::sleep(remaining.min(Duration::from_millis(1)));
            }
            result => return result,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn other_io_errors_are_returned_without_retry() {
        for kind in [
            io::ErrorKind::NotFound,
            io::ErrorKind::PermissionDenied,
            io::ErrorKind::InvalidData,
        ] {
            let mut attempts = 0;
            let error = open_with_retry(OPEN_RETRY_TIMEOUT, || {
                attempts += 1;
                Err::<(), _>(io::Error::new(kind, "original error"))
            })
            .unwrap_err();
            assert_eq!(error.kind(), kind);
            assert_eq!(error.to_string(), "original error");
            assert_eq!(attempts, 1);
        }
    }
}
