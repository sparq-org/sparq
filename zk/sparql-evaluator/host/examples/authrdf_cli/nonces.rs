use sparq_proved_evaluator::{Error, Nonces};
use std::{
    fs::{self, File, OpenOptions},
    io::{ErrorKind, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

// The directory and this persistent lock file must be verifier-owned. Locking
// the data file itself would race across rename, which replaces its inode.
pub(crate) struct FileNonces(pub(crate) PathBuf);
static TEMP_ID: AtomicU64 = AtomicU64::new(0);
const STORE_ERROR: Error = Error("nonce storage failed");

fn regular_or_missing(path: &Path) -> Result<(), Error> {
    match fs::symlink_metadata(path) {
        Ok(meta) if meta.file_type().is_file() => Ok(()),
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(()),
        _ => Err(STORE_ERROR),
    }
}

impl Nonces for FileNonces {
    fn consume(&mut self, nonce: [u8; 32]) -> Result<bool, Error> {
        let mut lock_name = self.0.as_os_str().to_os_string();
        lock_name.push(".lock");
        let lock_path = PathBuf::from(lock_name);
        regular_or_missing(&lock_path)?;
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(lock_path)
            .map_err(|_| STORE_ERROR)?;
        lock.lock().map_err(|_| STORE_ERROR)?;
        regular_or_missing(&self.0)?;
        let mut seen: Vec<[u8; 32]> = match fs::read(&self.0) {
            Ok(bytes) => serde_json::from_slice(&bytes).map_err(|_| STORE_ERROR)?,
            Err(e) if e.kind() == ErrorKind::NotFound => Vec::new(),
            Err(_) => return Err(STORE_ERROR),
        };
        if seen.contains(&nonce) {
            return Ok(false);
        }
        seen.push(nonce);
        let bytes = serde_json::to_vec(&seen).map_err(|_| STORE_ERROR)?;
        let mut temporary = self.0.as_os_str().to_os_string();
        temporary.push(format!(
            ".{}.{}.tmp",
            std::process::id(),
            TEMP_ID.fetch_add(1, Ordering::Relaxed)
        ));
        let temporary = PathBuf::from(temporary);
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&temporary).map_err(|_| STORE_ERROR)?;
        let result = (|| {
            file.write_all(&bytes)
                .and_then(|()| file.sync_all())
                .map_err(|_| STORE_ERROR)?;
            fs::rename(&temporary, &self.0).map_err(|_| STORE_ERROR)?;
            let parent = self
                .0
                .parent()
                .filter(|p| !p.as_os_str().is_empty())
                .unwrap_or(Path::new("."));
            File::open(parent)
                .and_then(|dir| dir.sync_all())
                .map_err(|_| STORE_ERROR)?;
            Ok(true)
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        result
    }
}
