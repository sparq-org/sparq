//! A **cross-process** [`UsageCounterStore`] backed by a single on-disk file guarded by
//! an OS-level lockfile. sq-5z1q.
//!
//! [`crate::count::InMemoryCounterStore`] closes the *in-process* TOCTOU race with a
//! `Mutex`, but its counts live only in one process's heap: two `sparq` processes (two
//! workers behind a load balancer, a CLI invocation alongside a server) each hold their
//! **own** map, so an "at most N" budget can be exercised up to N times *per process* —
//! the limit is not shared. The [`UsageCounterStore`] trait was written with exactly this
//! follow-up in mind (its [`try_consume`](UsageCounterStore::try_consume) atomicity
//! contract says a distributed store "MUST provide the same atomicity … across the
//! processes that share the limit"). This module supplies the first such store.
//!
//! # Why a file (and not Redis/SQL) here
//!
//! The bead names Redis `INCR` and SQL `UPDATE … WHERE consumed < limit RETURNING` as the
//! canonical *distributed* backends. Both are correct and both pull in a client crate +
//! a running service. `sparq-policy`'s design rule is a **lean, dependency-free** count
//! layer (the `count-enforcement` feature "adds ONLY the `count` module … no new deps,
//! std-only"). A file on a shared filesystem is the std-only point in that design space
//! that still gives **genuine cross-process** atomicity: every process that opens the
//! same path shares one budget, with no new dependency and no `unsafe`. A Redis/SQL
//! [`UsageCounterStore`] is a drop-in the operator can add in their own crate against the
//! unchanged trait — this module proves the seam works across process boundaries and is
//! the reference an operator can deploy as-is on a single host or a shared mount (NFS
//! caveat below).
//!
//! # How the cross-process atomicity works (load-bearing — read this)
//!
//! [`InMemoryCounterStore`](crate::count::InMemoryCounterStore) does the whole
//! compare-and-increment under one `Mutex`. [`FileCounterStore`] does the whole
//! compare-and-increment under one **OS-level mutual-exclusion lock**, so it is the exact
//! same shape one level down:
//!
//! 1. Acquire an exclusive lock by atomically *creating* a sibling lockfile
//!    (`OpenOptions::create_new(true)` ⇒ the `O_EXCL` open, which the OS guarantees
//!    succeeds for **exactly one** caller even across processes). If it already exists,
//!    another holder is mid-update; retry with a short back-off up to a bounded timeout.
//! 2. Under the lock: read the counter file, parse the `(key → consumed)` map, do the
//!    `consumed < limit` test, and — only if there is room — write the incremented map
//!    back via a temp-file + atomic `rename` (so a crash mid-write never leaves a
//!    half-written count). This is the indivisible step the trait requires.
//! 3. Release the lock by removing the lockfile (always — even on the error paths).
//!
//! No reader between steps 1 and 3 in *any* process can observe or mutate the map, so two
//! concurrent exercises of an "at most N" budget can never both read `N-1` and both
//! grant. The single shared file is the cross-process analogue of the single shared
//! `HashMap`; the lockfile is the cross-process analogue of the `Mutex`.
//!
//! # Fail-closed
//!
//! Every failure mode maps to [`ConsumeResult::Unavailable`] (a DENY upstream, never a
//! silent grant): the lock could not be acquired within the timeout (a crashed holder
//! left the lockfile behind — we do **not** steal a held lock, since stealing it would
//! reintroduce the race), an I/O error reading or writing the counter file, or a
//! corrupt/unparseable counter file. We never grant beyond a limit we cannot account for.
//!
//! # Deployment boundary (honest)
//!
//! - **Same host:** correct and atomic — `O_EXCL` create is atomic on every mainstream
//!   local filesystem, so all processes on one machine sharing one directory share one
//!   budget. This is the deploy target.
//! - **Shared network mount (NFS):** `O_EXCL` create is atomic on **modern NFS (NFSv3+
//!   with a sane server)**; on old/misconfigured NFS it historically was not. For a
//!   multi-*host* deployment over a questionable mount, prefer a Redis/SQL store (the
//!   trait seam). This is documented, not silently assumed safe.
//! - This store is for **persisted/shared** counting; it does more I/O than the in-memory
//!   store and is not meant for a hot single-process path (use [`InMemoryCounterStore`]
//!   there). It is wired through the **same** [`evaluate_and_exercise`] path and obeys the
//!   identical `try_consume` contract, so swapping stores changes only where the counts
//!   live, never the enforcement semantics.
//!
//! [`InMemoryCounterStore`]: crate::count::InMemoryCounterStore
//! [`evaluate_and_exercise`]: crate::count::evaluate_and_exercise

use crate::count::{ConsumeResult, CountKey, UsageCounterStore};
use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// File name of the counter data file inside the store directory.
const DATA_FILE: &str = "counters.tsv";
/// File name of the exclusive lockfile inside the store directory.
const LOCK_FILE: &str = "counters.lock";
/// File name prefix of the temp file used for the atomic rewrite.
const TEMP_PREFIX: &str = "counters.tmp.";

/// Default upper bound on how long [`FileCounterStore::try_consume`] waits to acquire the
/// cross-process lock before failing closed. Generous enough to ride out another
/// process's brief read-modify-write, short enough not to wedge a request indefinitely on
/// a crashed lock holder.
const DEFAULT_LOCK_TIMEOUT: Duration = Duration::from_secs(5);

/// How long to sleep between lock-acquire attempts.
const LOCK_RETRY_BACKOFF: Duration = Duration::from_millis(2);

/// A cross-process [`UsageCounterStore`] persisting all usage counters in one file under
/// `dir`, with the whole compare-and-increment serialized by an OS-level lockfile so it
/// is atomic **across processes** that share `dir`. sq-5z1q.
///
/// This is the cross-process answer to the single-process
/// [`InMemoryCounterStore`](crate::count::InMemoryCounterStore): see the module docs for
/// the atomicity mechanism and the deployment boundary (same host / NFS / Redis-or-SQL).
///
/// ```rust,ignore
/// // cargo: sparq-policy with --features count-enforcement
/// use sparq_policy::{evaluate_and_exercise, FileCounterStore, Request};
/// // Two processes pointed at the same dir share ONE "at most N" budget.
/// let store = FileCounterStore::new("/var/lib/sparq/odrl-counts").unwrap();
/// let req = Request::new("http://www.w3.org/ns/odrl/2/read")
///     .on("urn:asset/x").by("https://alice.ex/me"); // policy: count lteq 3
/// let _ = evaluate_and_exercise(&policy, &req, &store);
/// ```
#[derive(Debug, Clone)]
pub struct FileCounterStore {
    dir: PathBuf,
    lock_timeout: Duration,
}

impl FileCounterStore {
    /// Open (creating it if absent) a file-backed store rooted at `dir`. Every process
    /// that opens the **same** `dir` shares one set of budgets.
    ///
    /// Returns an error only if `dir` cannot be created / is not a usable directory; a
    /// successfully-opened store never surfaces I/O errors as panics — per-operation
    /// failures are reported as [`ConsumeResult::Unavailable`] (fail-closed).
    pub fn new(dir: impl AsRef<Path>) -> std::io::Result<FileCounterStore> {
        Self::with_lock_timeout(dir, DEFAULT_LOCK_TIMEOUT)
    }

    /// As [`new`](FileCounterStore::new) but with an explicit lock-acquire timeout (mainly
    /// for tests / tuning). On timeout, [`try_consume`](FileCounterStore::try_consume)
    /// fails closed ([`ConsumeResult::Unavailable`]) rather than steal a held lock.
    pub fn with_lock_timeout(
        dir: impl AsRef<Path>,
        lock_timeout: Duration,
    ) -> std::io::Result<FileCounterStore> {
        let dir = dir.as_ref().to_path_buf();
        fs::create_dir_all(&dir)?;
        Ok(FileCounterStore { dir, lock_timeout })
    }

    fn data_path(&self) -> PathBuf {
        self.dir.join(DATA_FILE)
    }

    fn lock_path(&self) -> PathBuf {
        self.dir.join(LOCK_FILE)
    }

    /// Acquire the exclusive cross-process lock (an `O_EXCL`-created sibling lockfile),
    /// retrying with a short back-off up to `self.lock_timeout`. Returns the guard (whose
    /// `Drop` removes the lockfile) or `None` on timeout (fail-closed: we never break a
    /// lock another holder still owns).
    fn acquire_lock(&self) -> Option<LockGuard> {
        let lock_path = self.lock_path();
        let deadline = Instant::now() + self.lock_timeout;
        loop {
            match OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&lock_path)
            {
                Ok(_file) => return Some(LockGuard { path: lock_path }),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                    if Instant::now() >= deadline {
                        return None; // someone else holds it past our patience → fail closed
                    }
                    std::thread::sleep(LOCK_RETRY_BACKOFF);
                }
                // A *different* I/O error (e.g. the dir vanished, permissions) → fail
                // closed; the caller maps `None` to `Unavailable`.
                Err(_) => return None,
            }
        }
    }
}

/// RAII guard for the lockfile: dropping it removes the lockfile, releasing the
/// cross-process lock even on an early return / panic. sq-5z1q.
struct LockGuard {
    path: PathBuf,
}

impl Drop for LockGuard {
    fn drop(&mut self) {
        // Best-effort: if removal fails the lockfile becomes stale and the NEXT acquirer
        // will time out and fail closed (never silently grant) — the safe failure mode.
        let _ = fs::remove_file(&self.path);
    }
}

impl UsageCounterStore for FileCounterStore {
    fn try_consume(&self, key: &CountKey, limit: u64) -> ConsumeResult {
        // 1. Acquire the cross-process lock; no other process can read or mutate the map
        //    between here and this guard's drop.
        let Some(_guard) = self.acquire_lock() else {
            return ConsumeResult::Unavailable; // lock timeout / IO error → fail closed
        };
        // 2. Read the whole map under the lock.
        let mut map = match read_map(&self.data_path()) {
            Ok(m) => m,
            Err(_) => return ConsumeResult::Unavailable, // unreadable / corrupt → fail closed
        };
        // 3. The atomic compare-and-increment, in memory under the lock.
        let entry = map.entry(key.clone()).or_insert(0);
        if *entry >= limit {
            return ConsumeResult::Exhausted; // guard drops here → lock released
        }
        let new = *entry + 1;
        *entry = new;
        // 4. Persist the incremented map (temp + atomic rename) BEFORE reporting success —
        //    a write failure must NOT report a consume that did not durably happen.
        if write_map_atomic(&self.dir, &self.data_path(), &map).is_err() {
            return ConsumeResult::Unavailable; // could not persist → fail closed
        }
        ConsumeResult::Consumed(new)
        // `_guard` drops here → lockfile removed → lock released.
    }

    fn consumed(&self, key: &CountKey) -> Option<u64> {
        // Pure read — the trait permits this to be lock-free (it never gates an exercise;
        // `try_consume` is the only gate, and it IS locked). A missing file means zero
        // consumed; an unreadable/corrupt file means we cannot report → `None`.
        let path = self.data_path();
        if !path.exists() {
            return Some(0);
        }
        let map = read_map(&path).ok()?;
        Some(map.get(key).copied().unwrap_or(0))
    }
}

/// Read and parse the on-disk counter map. A **missing** file is an empty map (the
/// initial state); a present-but-unparseable file is an error (caller fails closed).
fn read_map(path: &Path) -> std::io::Result<HashMap<CountKey, u64>> {
    let mut file = match File::open(path) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(HashMap::new()),
        Err(e) => return Err(e),
    };
    let mut buf = String::new();
    file.read_to_string(&mut buf)?;
    parse_map(&buf)
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidData, "corrupt counter file"))
}

/// Parse the line-based counter format: one budget per line,
/// `rule_id<TAB>party<TAB>target<TAB>count`, each of the three key fields percent-escaped
/// (so a TAB / newline / `%` inside an IRI cannot break the framing). Returns `None` on
/// any malformed line (the caller fails closed rather than guess).
fn parse_map(s: &str) -> Option<HashMap<CountKey, u64>> {
    let mut map = HashMap::new();
    for line in s.lines() {
        if line.is_empty() {
            continue;
        }
        let mut parts = line.split('\t');
        let rule_id = unescape(parts.next()?)?;
        let party = unescape(parts.next()?)?;
        let target = unescape(parts.next()?)?;
        let count: u64 = parts.next()?.parse().ok()?;
        if parts.next().is_some() {
            return None; // extra field → malformed
        }
        map.insert(
            CountKey {
                rule_id,
                party,
                target,
            },
            count,
        );
    }
    Some(map)
}

/// Serialize the counter map to the line-based format (the inverse of [`parse_map`]).
/// Keys are emitted in a deterministic (sorted) order so the file is reproducible.
fn serialize_map(map: &HashMap<CountKey, u64>) -> String {
    let mut entries: Vec<(&CountKey, &u64)> = map.iter().collect();
    entries.sort_by(|a, b| {
        (&a.0.rule_id, &a.0.party, &a.0.target).cmp(&(&b.0.rule_id, &b.0.party, &b.0.target))
    });
    let mut out = String::new();
    for (k, v) in entries {
        out.push_str(&escape(&k.rule_id));
        out.push('\t');
        out.push_str(&escape(&k.party));
        out.push('\t');
        out.push_str(&escape(&k.target));
        out.push('\t');
        out.push_str(&v.to_string());
        out.push('\n');
    }
    out
}

/// Write the map to `data_path` atomically: serialize to a uniquely-named temp file in the
/// same directory, flush + fsync, then `rename` over the target (an atomic replace on the
/// same filesystem). A crash mid-write leaves the temp file (cleaned up best-effort), never
/// a half-written counter file. Must be called while holding the lock.
fn write_map_atomic(
    dir: &Path,
    data_path: &Path,
    map: &HashMap<CountKey, u64>,
) -> std::io::Result<()> {
    let body = serialize_map(map);
    // The temp name is unique to this process so two stores in DIFFERENT dirs never
    // collide; within one dir the lock already serializes writers, but a pid suffix keeps
    // a stale temp from a crashed writer from being clobbered mid-rename by another.
    let temp_path = dir.join(format!("{TEMP_PREFIX}{}", std::process::id()));
    {
        let mut f = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&temp_path)?;
        f.write_all(body.as_bytes())?;
        f.flush()?;
        // Durability before the rename: ensure bytes hit disk so a crash can't expose an
        // empty/short file under the real name after the rename is observed.
        f.sync_all()?;
    }
    match fs::rename(&temp_path, data_path) {
        Ok(()) => Ok(()),
        Err(e) => {
            let _ = fs::remove_file(&temp_path); // don't leak the temp on failure
            Err(e)
        }
    }
}

/// Percent-escape the framing-significant bytes (`%`, TAB, CR, LF) so a key field is a
/// single safe token on one line. `%` first so the escape is unambiguous. This is a
/// minimal, reversible escaping — NOT general percent-encoding (we only touch the bytes
/// that would break the TSV framing), which keeps the file human-readable for audit.
fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '%' => out.push_str("%25"),
            '\t' => out.push_str("%09"),
            '\n' => out.push_str("%0A"),
            '\r' => out.push_str("%0D"),
            c => out.push(c),
        }
    }
    out
}

/// Inverse of [`escape`]. Returns `None` on a malformed escape (truncated / non-hex /
/// not one of the escapes we emit) so a corrupt field fails the whole parse closed.
///
/// Scanned by `char` so multi-byte UTF-8 is preserved exactly: the only escapes we emit
/// are `%` followed by two ASCII hex digits, so a `char`-wise walk reconstructs the
/// original string losslessly.
fn unescape(s: &str) -> Option<String> {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '%' {
            let h = chars.next()?;
            let l = chars.next()?;
            let code = (hex_nibble(h)? << 4) | hex_nibble(l)?;
            match code {
                0x25 => out.push('%'),
                0x09 => out.push('\t'),
                0x0A => out.push('\n'),
                0x0D => out.push('\r'),
                _ => return None, // we never emit any other escape
            }
        } else {
            out.push(c);
        }
    }
    Some(out)
}

/// The value 0..=15 of a single hex-digit `char`, or `None` if it is not `[0-9A-Fa-f]`.
fn hex_nibble(c: char) -> Option<u8> {
    c.to_digit(16).map(|d| d as u8)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(rule: &str, party: &str, target: &str) -> CountKey {
        CountKey {
            rule_id: rule.into(),
            party: party.into(),
            target: target.into(),
        }
    }

    /// A fresh temp dir unique to this test (no external tempdir dep).
    fn temp_dir(tag: &str) -> PathBuf {
        let mut d = std::env::temp_dir();
        d.push(format!(
            "sparq-policy-filecount-{}-{}-{:?}",
            tag,
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn escape_round_trips_framing_bytes() {
        for s in [
            "plain",
            "a\tb",
            "x\ny",
            "100%",
            "%09literal",
            "urn:asset/x",
            "",
            "héllo€",
        ] {
            assert_eq!(unescape(&escape(s)).as_deref(), Some(s), "round-trip {s:?}");
        }
    }

    #[test]
    fn malformed_escape_rejected() {
        assert_eq!(unescape("%2"), None); // truncated
        assert_eq!(unescape("%ZZ"), None); // non-hex
        assert_eq!(unescape("%FF"), None); // an escape we never emit
    }

    #[test]
    fn map_round_trips_through_file_format() {
        let mut m = HashMap::new();
        m.insert(key("r1", "https://alice.ex/me", "urn:asset/x"), 3);
        m.insert(key("r2", "", ""), 1);
        m.insert(key("tabbed\trule", "p\nq", "t%t"), 7);
        m.insert(key("unicode", "πα", "ω"), 9);
        let parsed = parse_map(&serialize_map(&m)).unwrap();
        assert_eq!(parsed, m);
    }

    #[test]
    fn corrupt_file_parses_to_none() {
        assert!(parse_map("r1\tp\tt").is_none()); // missing count
        assert!(parse_map("r1\tp\tt\tnotanumber").is_none()); // non-numeric count
        assert!(parse_map("r1\tp\tt\t5\textra").is_none()); // extra field
    }

    #[test]
    fn missing_file_is_zero_not_error() {
        let dir = temp_dir("missing");
        let store = FileCounterStore::new(&dir).unwrap();
        // No exercise yet → the data file does not exist → 0 consumed, not an error.
        assert_eq!(store.consumed(&key("r", "p", "t")), Some(0));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn try_consume_grants_then_exhausts_and_persists() {
        let dir = temp_dir("grants");
        let store = FileCounterStore::new(&dir).unwrap();
        let k = key("r", "alice", "x");
        assert_eq!(store.try_consume(&k, 2), ConsumeResult::Consumed(1));
        assert_eq!(store.try_consume(&k, 2), ConsumeResult::Consumed(2));
        assert_eq!(store.try_consume(&k, 2), ConsumeResult::Exhausted);
        assert_eq!(store.consumed(&k), Some(2));
        // A SECOND store over the SAME dir sees the persisted count (cross-instance
        // sharing — the in-process surrogate for cross-process sharing).
        let store2 = FileCounterStore::new(&dir).unwrap();
        assert_eq!(store2.consumed(&k), Some(2));
        assert_eq!(store2.try_consume(&k, 2), ConsumeResult::Exhausted);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn distinct_keys_have_independent_budgets() {
        let dir = temp_dir("isolation");
        let store = FileCounterStore::new(&dir).unwrap();
        let alice = key("r", "alice", "x");
        let bob = key("r", "bob", "x");
        assert_eq!(store.try_consume(&alice, 1), ConsumeResult::Consumed(1));
        assert_eq!(store.try_consume(&alice, 1), ConsumeResult::Exhausted);
        // Bob's budget is untouched.
        assert_eq!(store.try_consume(&bob, 1), ConsumeResult::Consumed(1));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn limit_zero_never_grants() {
        let dir = temp_dir("zero");
        let store = FileCounterStore::new(&dir).unwrap();
        assert_eq!(
            store.try_consume(&key("r", "p", "t"), 0),
            ConsumeResult::Exhausted
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn corrupt_data_file_fails_closed() {
        let dir = temp_dir("corrupt");
        let store = FileCounterStore::new(&dir).unwrap();
        // Write garbage into the data file directly.
        fs::write(store.data_path(), b"this is not a valid counter line\n").unwrap();
        assert_eq!(
            store.try_consume(&key("r", "p", "t"), 5),
            ConsumeResult::Unavailable
        );
        assert_eq!(store.consumed(&key("r", "p", "t")), None);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn held_lock_times_out_fail_closed() {
        let dir = temp_dir("lock");
        // A tiny timeout so the test is fast.
        let store = FileCounterStore::with_lock_timeout(&dir, Duration::from_millis(20)).unwrap();
        // Simulate another (crashed) holder by creating the lockfile and NOT removing it.
        fs::write(store.lock_path(), b"").unwrap();
        // try_consume cannot acquire the lock → fail closed (never steal a held lock).
        assert_eq!(
            store.try_consume(&key("r", "p", "t"), 5),
            ConsumeResult::Unavailable
        );
        // Once the lock is released, it works again.
        fs::remove_file(store.lock_path()).unwrap();
        assert_eq!(
            store.try_consume(&key("r", "p", "t"), 5),
            ConsumeResult::Consumed(1)
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn concurrent_threads_never_overgrant() {
        use std::sync::atomic::{AtomicU64, Ordering};
        use std::sync::Arc;
        const LIMIT: u64 = 50;
        const THREADS: usize = 8;
        const PER_THREAD: usize = 40; // 320 attempts >> 50 budget

        let dir = temp_dir("concurrent");
        let store = Arc::new(FileCounterStore::new(&dir).unwrap());
        let granted = Arc::new(AtomicU64::new(0));
        let k = key("r", "alice", "x");

        let mut handles = Vec::new();
        for _ in 0..THREADS {
            let store = Arc::clone(&store);
            let granted = Arc::clone(&granted);
            let k = k.clone();
            handles.push(std::thread::spawn(move || {
                for _ in 0..PER_THREAD {
                    if matches!(store.try_consume(&k, LIMIT), ConsumeResult::Consumed(_)) {
                        granted.fetch_add(1, Ordering::Relaxed);
                    }
                }
            }));
        }
        for h in handles {
            h.join().unwrap();
        }
        // EXACTLY the limit was granted — the file lock closed the (now cross-thread,
        // and by construction cross-process) TOCTOU window.
        assert_eq!(
            granted.load(Ordering::Relaxed),
            LIMIT,
            "must grant exactly the limit"
        );
        assert_eq!(store.consumed(&k), Some(LIMIT));
        let _ = fs::remove_dir_all(&dir);
    }
}
