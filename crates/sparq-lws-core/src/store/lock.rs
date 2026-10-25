//! Advisory exclusive ownership of a data directory through its persistent `LOCK` file.
//!
//! Keep the guard alive until every user of the directory has stopped. The lock file
//! is retained after drop: unlinking it would let a new opener lock a different inode.
//! Place it outside any graph directory that compaction can replace.

use std::fs::{File, OpenOptions, TryLockError};
use std::io::{self, Read, Write};
use std::path::Path;
use std::time::{Duration, Instant};

/// Exclusive ownership of `<dir>/LOCK`, released when this guard drops.
#[derive(Debug)]
pub struct DataDirLock {
    _file: File,
}

impl DataDirLock {
    /// Acquire a directory's lock and record this process's PID after acquisition.
    ///
    /// Creates `dir` if needed. By default a held lock fails immediately; setting
    /// `SOLID_SERVER_WAIT_FOR_LOCK_SECS` to a nonnegative integer enables bounded
    /// polling for that many seconds. This call blocks; async callers should run it
    /// off the reactor. Keep the guard for the service lifetime, outside any index
    /// directory that `Graph::compact()` renames.
    ///
    /// # Errors
    /// Returns [`io::ErrorKind::WouldBlock`] naming the recorded holder PID when
    /// the lock remains held. Invalid environment values return `InvalidInput`;
    /// filesystem or platform locking errors are propagated.
    pub fn acquire(dir: &Path) -> io::Result<Self> {
        let wait = match std::env::var("SOLID_SERVER_WAIT_FOR_LOCK_SECS") {
            Ok(value) => Duration::from_secs(value.trim().parse().map_err(|_| {
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "SOLID_SERVER_WAIT_FOR_LOCK_SECS must be a nonnegative integer",
                )
            })?),
            Err(std::env::VarError::NotPresent) => Duration::ZERO,
            Err(e) => return Err(io::Error::new(io::ErrorKind::InvalidInput, e)),
        };
        std::fs::create_dir_all(dir)?;
        let path = dir.join("LOCK");
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)?;
        let started = Instant::now();
        loop {
            match file.try_lock() {
                Ok(()) => break,
                Err(TryLockError::WouldBlock) => {
                    let remaining = wait.saturating_sub(started.elapsed());
                    if remaining.is_zero() {
                        // Bound reads even if a foreign or damaged LOCK file is large.
                        let mut holder = String::new();
                        let _ =
                            File::open(&path).and_then(|f| f.take(64).read_to_string(&mut holder));
                        let pid = holder.trim().parse::<u32>().ok();
                        let holder = pid.map_or_else(|| "unknown".into(), |pid| pid.to_string());
                        return Err(io::Error::new(
                            io::ErrorKind::WouldBlock,
                            format!("data directory {} is locked by PID {holder}", dir.display()),
                        ));
                    }
                    // Poll for handover without busy-waiting or exceeding the configured delay.
                    std::thread::sleep(remaining.min(Duration::from_millis(100)));
                }
                Err(TryLockError::Error(e)) => return Err(e),
            }
        }
        // Never truncate another process's PID before owning the OS lock.
        file.set_len(0)?;
        writeln!(file, "{}", std::process::id())?;
        file.sync_all()?;
        Ok(Self { _file: file })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::BufRead;
    use std::process::{Command, Stdio};

    struct TestDir(std::path::PathBuf);

    impl TestDir {
        fn new() -> Self {
            let dir = std::env::temp_dir().join(format!(
                "sparq-lws-lock-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            Self(dir)
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn lock_excludes_other_openers_preserves_pid_and_releases_on_drop() {
        let dir = TestDir::new();
        let first = DataDirLock::acquire(&dir.0).unwrap();
        let pid = format!("{}\n", std::process::id());
        assert_eq!(std::fs::read_to_string(dir.0.join("LOCK")).unwrap(), pid);
        let err = DataDirLock::acquire(&dir.0).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::WouldBlock);
        assert!(err
            .to_string()
            .contains(&format!("PID {}", std::process::id())));
        assert_eq!(std::fs::read_to_string(dir.0.join("LOCK")).unwrap(), pid);
        drop(first);
        let second = DataDirLock::acquire(&dir.0).unwrap();
        assert!(dir.0.join("LOCK").is_file());
        drop(second);
    }

    #[test]
    fn wait_for_lock_env_acquires_after_holder_drops() {
        let dir = TestDir::new();
        let holder = DataDirLock::acquire(&dir.0).unwrap();
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "store::lock::tests::wait_for_lock_env_child",
                "--nocapture",
            ])
            .env("SPARQ_LWS_LOCK_TEST_DIR", &dir.0)
            .env("SOLID_SERVER_WAIT_FOR_LOCK_SECS", "3")
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let mut output = io::BufReader::new(child.stdout.take().unwrap());
        let mut line = String::new();
        loop {
            assert_ne!(
                output.read_line(&mut line).unwrap(),
                0,
                "child did not start"
            );
            if line.contains("LOCK_WAIT_READY") {
                break;
            }
            line.clear();
        }
        // Keep the lock across multiple polls; the child must honor the wait env var.
        std::thread::sleep(Duration::from_millis(250));
        assert_eq!(
            std::fs::read_to_string(dir.0.join("LOCK")).unwrap().trim(),
            std::process::id().to_string(),
            "a waiter must not overwrite the holder PID"
        );
        drop(holder);
        let mut rest = String::new();
        output.read_to_string(&mut rest).unwrap();
        assert!(child.wait().unwrap().success(), "{line}{rest}");
        assert!(rest.contains("LOCK_WAIT_ACQUIRED"), "{rest}");
        assert_eq!(
            std::fs::read_to_string(dir.0.join("LOCK")).unwrap().trim(),
            child.id().to_string()
        );
    }

    #[test]
    fn lock_wait_environment_rejects_invalid_values_and_bounds_contention() {
        let dir = TestDir::new();
        let _holder = DataDirLock::acquire(&dir.0).unwrap();
        for (value, expected) in [("invalid", "InvalidInput"), ("1", "WouldBlock")] {
            let output = Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "store::lock::tests::wait_for_lock_env_child",
                    "--nocapture",
                ])
                .env("SPARQ_LWS_LOCK_TEST_DIR", &dir.0)
                .env("SPARQ_LWS_LOCK_TEST_ERROR", expected)
                .env("SOLID_SERVER_WAIT_FOR_LOCK_SECS", value)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
    }

    #[test]
    fn wait_for_lock_env_child() {
        let Some(dir) = std::env::var_os("SPARQ_LWS_LOCK_TEST_DIR") else {
            return;
        };
        println!("LOCK_WAIT_READY");
        io::stdout().flush().unwrap();
        let started = Instant::now();
        let result = DataDirLock::acquire(Path::new(&dir));
        if let Ok(expected) = std::env::var("SPARQ_LWS_LOCK_TEST_ERROR") {
            let err = result.unwrap_err();
            assert_eq!(format!("{:?}", err.kind()), expected);
            if err.kind() == io::ErrorKind::WouldBlock {
                assert!(started.elapsed() >= Duration::from_secs(1));
                assert!(err.to_string().contains("PID "));
            }
            return;
        }
        let _guard = result.unwrap();
        println!("LOCK_WAIT_ACQUIRED");
    }
}
