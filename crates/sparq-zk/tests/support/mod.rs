//! Shared LIVE-toolchain gate for this crate's integration suites.
//!
//! The suites in `tests/` split into two halves. The STRUCTURAL half needs no
//! external tooling: it builds manifests by hand and asserts the verifier's
//! structural gates reject them before the per-sub-proof crypto loop. The LIVE
//! half shells out to a real `nargo execute` and/or `bb prove`/`bb verify` — and
//! that half is where the repo's strongest claim (a forged proof is REJECTED)
//! actually gets exercised.
//!
//! Every live test used to carry its own private `toolchain_available()` copy and
//! `return`ed early when `nargo`/`bb` were absent, so a run WITHOUT the toolchain
//! reported `ok` for a test whose body never executed — a false green. This module
//! is the single gate they now share, and it FAILS LOUD by default: a missing tool
//! panics unless the lane explicitly declares itself toolchain-free via
//! [`ALLOW_MISSING_ENV`], in which case the skip is printed to stderr and names the
//! test, the missing tool and the variable that allowed it.
//!
//! It lives in `tests/` (not behind a `#[doc(hidden)]` module in `src/`) so the
//! harness contract stays out of the crate's published API surface: it is compiled
//! only into the test binaries that declare `mod support;`.

/// Set to a non-empty value other than `0` to turn a missing `nargo`/`bb` from a
/// hard failure into a printed skip. A CI lane that deliberately runs without the
/// Noir toolchain (`ci.yml`, `feature-matrix.yml`) sets it at the workflow level, so
/// the skip is DECLARED where the toolchain is absent on purpose rather than
/// inferred silently inside the test.
pub const ALLOW_MISSING_ENV: &str = "SPARQ_ZK_ALLOW_MISSING_TOOLCHAIN";

/// `<tool> --version` exits 0 — i.e. the tool is on `PATH` and runnable.
fn on_path(tool: &str) -> bool {
    std::process::Command::new(tool)
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Print a skip notice that survives libtest's output capture.
///
/// `eprintln!` goes through std's thread-local output capture, which libtest
/// installs and then DISCARDS for a test that passes — so a skip announced with
/// `eprintln!` alone is invisible in CI, which is the exact failure mode this gate
/// exists to end. Opening `/dev/stderr` writes to the process's real stderr instead,
/// in one write so parallel tests cannot interleave a line. Where that file does not
/// exist (non-Unix), fall back to the captured path, which is still shown under
/// `--nocapture`. A runner that captures the process's stderr itself (nextest does,
/// for a passing test, unless `success-output` is set) still hides the line; the
/// skip is then declared by the lane's environment rather than printed.
fn skip_line(msg: &str) {
    use std::io::Write;
    let direct = std::fs::OpenOptions::new()
        .append(true)
        .open("/dev/stderr")
        .and_then(|mut f| f.write_all(format!("{msg}\n").as_bytes()));
    if direct.is_err() {
        eprintln!("{}", msg);
    }
}

fn allow_missing() -> bool {
    std::env::var_os(ALLOW_MISSING_ENV).is_some_and(|v| !v.is_empty() && v != "0")
}

/// A [`std::process::Command`] for a live tool, for use AFTER
/// [`require_toolchain`] has confirmed the tool is on `PATH`.
///
/// Call sites spawn through this instead of naming the executable to
/// `Command::new` themselves, so the acquisition of `nargo` / `bb` stays inside this
/// module. `scripts/check-zk-toolchain-gate.py` lints the ZK test trees for any
/// other acquisition path (an executable literal or a non-literal argument to
/// `Command::new`, an `ErrorKind::NotFound` arm, a `which` probe), so a fresh
/// private copy of the gate cannot be added without a declared, reasoned exemption.
/// This helper does NOT gate: it is [`require_toolchain`] that turns a missing tool
/// into a hard failure or a declared, printed skip.

/// Names the file the gate appends to when a live body is allowed to run.
///
/// PRIVATE to this module on purpose: a test must not be able to name, read or
/// redirect the ledger. `zk-toolchain.yml` sets it at JOB level and its final step
/// verifies that every gate call site in the ZK test sources left an entry. A skip
/// appends nothing and a panic appends nothing, so a lane that skipped past the gate
/// cannot produce a complete ledger. The ledger records at GATE ENTRY: it proves the
/// gate was entered with the tools present, NOT that the body ran to completion or
/// asserted anything.
const LEDGER_ENV: &str = "SPARQ_ZK_LIVE_LEDGER";

/// Evidence that [`require_toolchain`]/[`gate`] found every requested tool on
/// `PATH` and let the live body run.
///
/// Only [`gate`] mints one — the field is private to this module — and
/// [`live_tool`] takes one, so a live tool cannot be spawned without having passed
/// the gate first. `#[allow(dead_code)]` because each test binary declaring
/// `mod support;` uses a subset of this module.
#[allow(dead_code)]
#[derive(Clone, Copy)]
pub struct LiveToolchain {
    _seal: (),
}

/// The ledger key for a gate call: the CALL SITE, `<file>:<line>`.
///
/// The identity is the location, never the `test` argument. A caller-supplied name
/// could be any string — including another test's — so keying on the name would let
/// one call site record an entry that satisfies a different one. `Location::file()`
/// is the path rustc was given, which for this workspace is relative to the
/// workspace root; backslashes are normalised so a key does not depend on the host.
fn ledger_key(loc: &std::panic::Location<'static>) -> String {
    format!("{}:{}", loc.file().replace('\\', "/"), loc.line())
}

/// Append the CALL SITE's key to the ledger, if the lane asked for one.
///
/// Opened with `O_APPEND` (`append(true)`) and written as ONE line in ONE
/// `write_all`, so entries from tests running in parallel, or from a re-exec'd child
/// process, cannot interleave. Best-effort by design: a write failure must not turn
/// a passing live test red, and a missing entry is what the verifier reports.
fn record(path: &std::path::Path, loc: &std::panic::Location<'static>) {
    use std::io::Write;
    let line = format!("{}\n", ledger_key(loc));
    let _ = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .and_then(|mut f| f.write_all(line.as_bytes()));
}

/// Where this process records, or `None` when the lane asked for no ledger.
///
/// Under `cfg(test)` the module's own tests can redirect it — that is how the
/// call-site identity below is tested without naming [`LEDGER_ENV`] anywhere a test
/// could reach. Nothing outside this file can set it.
fn ledger_path() -> Option<std::path::PathBuf> {
    #[cfg(test)]
    {
        if let Ok(guard) = TEST_LEDGER.lock() {
            if let Some(path) = guard.clone() {
                return Some(path);
            }
        }
    }
    match std::env::var_os(LEDGER_ENV) {
        Some(p) if !p.is_empty() => Some(std::path::PathBuf::from(p)),
        _ => None,
    }
}

/// A ledger path for this module's own tests only. Private, and `cfg(test)`-only.
#[cfg(test)]
static TEST_LEDGER: std::sync::Mutex<Option<std::path::PathBuf>> =
    std::sync::Mutex::new(None);

/// A [`std::process::Command`] for a live tool.
///
/// Takes a [`LiveToolchain`], which only [`gate`] mints, so the gate cannot be
/// bypassed. It ALSO re-checks `PATH` itself and panics when the tool is absent, so
/// neither a stale token nor an `is_err()` test on the spawn result can be turned
/// into a soft pass. `scripts/check-zk-toolchain-gate.py` lints the ZK test trees
/// for every other acquisition path.
#[allow(dead_code)]
pub fn live_tool(_gate: &LiveToolchain, tool: &str) -> std::process::Command {
    assert!(
        on_path(tool),
        "{} is not on PATH at spawn time; the live toolchain gate must be passed \
         first and the tool must stay available",
        tool
    );
    std::process::Command::new(tool)
}

/// Gate a LIVE test body on `tools` all being on `PATH`, returning the token
/// [`live_tool`] needs.
///
/// * all present -> `Some(token)`, and the CALL SITE's key (`<file>:<line>`) is
///   appended to the ledger.
/// * absent, [`ALLOW_MISSING_ENV`] set -> prints a skip line naming `test`, the
///   missing tool(s) and the variable, then `None`; NOTHING is recorded.
/// * absent otherwise -> PANICS, naming the missing tool, how to put the toolchain
///   on `PATH`, and the opt-out variable; nothing is recorded.
///
/// `test` names the calling test, for the skip and panic messages ONLY — it is not
/// the ledger identity (see [`ledger_key`]). It is passed explicitly rather than
/// read from the thread name because libtest runs tests on the main thread (named
/// `main`) under `--test-threads=1`, which is exactly how the toolchain lane runs
/// them. `scripts/check-zk-toolchain-gate.py` still requires it to be a string
/// literal, so a skip or panic line names something a reader can find in the
/// source, but a wrong name can no longer satisfy another call site's entry.
#[allow(dead_code)]
#[track_caller]
pub fn gate(test: &str, tools: &[&str]) -> Option<LiveToolchain> {
    let missing: Vec<&str> = tools.iter().copied().filter(|t| !on_path(t)).collect();
    if missing.is_empty() {
        if let Some(path) = ledger_path() {
            record(&path, std::panic::Location::caller());
        }
        return Some(LiveToolchain { _seal: () });
    }
    let missing = missing.join(", ");
    if allow_missing() {
        skip_line(&format!(
            "SKIP {}: the live Noir toolchain is missing ({} not on PATH) and {}=1 declares \
             this lane toolchain-free — the live body did NOT run.",
            test, missing, ALLOW_MISSING_ENV
        ));
        return None;
    }
    panic!(
        "{} is a LIVE test, so its body would not have run — not on PATH: {}. \
         Put the pinned toolchain on PATH (in a cloud container the tools are installed \
         outside PATH: `export PATH=$PATH:/root/.nargo/bin:/root/.bb`; otherwise install \
         the pinned nargo/bb with noirup/bbup at the versions in \
         .github/workflows/zk-toolchain.yml), or set {}=1 to declare this lane \
         toolchain-free and turn this failure into a visible, named skip.",
        test, missing, ALLOW_MISSING_ENV
    );
}

/// [`gate`] as a boolean, for the call sites that do not spawn a tool themselves.
/// `#[track_caller]` so the ledger key is the TEST's line, not this forwarding
/// call's line inside the gate module.
#[allow(dead_code)]
#[track_caller]
pub fn require_toolchain(test: &str, tools: &[&str]) -> bool {
    gate(test, tools).is_some()
}

// --------------------------------------------------------------------------- #
// The gate's own identity tests.
//
// Compiled into every binary that declares `mod support;` and into the crate's lib
// target (which includes this same file with `#[path]`), so they run wherever the
// gate does. They are about the LEDGER KEY: that it is the caller's own line, that
// `#[track_caller]` carries that line through the `require_toolchain` wrapper, and
// that nothing is recorded on the skip or panic paths.
// --------------------------------------------------------------------------- #

#[cfg(test)]
mod gate_identity_tests {
    use super::{ALLOW_MISSING_ENV, TEST_LEDGER, gate, require_toolchain};

    /// A name no executable resolves to, so the gate takes a non-recording path.
    const ABSENT: &str = "sparq-zk-gate-identity-no-such-tool-xyzzy";

    fn scratch(label: &str) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!(
            "sparq_gate_identity_{}_{}_{}.txt",
            label,
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        let _ = std::fs::remove_file(&path);
        path
    }

    /// The redirect is ONE global, so these three tests take turns. Without this
    /// lock they overwrite each other's path and each one's entry lands in another
    /// one's file. A poisoned lock is still usable here: the state it guards is
    /// reset on entry.
    static SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn redirected<T>(path: &std::path::Path, body: impl FnOnce() -> T) -> T {
        let _turn = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let set = |value: Option<std::path::PathBuf>| {
            *TEST_LEDGER.lock().unwrap_or_else(|e| e.into_inner()) = value;
        };
        set(Some(path.to_path_buf()));
        let out = body();
        set(None);
        out
    }

    fn ledger(path: &std::path::Path) -> String {
        std::fs::read_to_string(path).unwrap_or_default()
    }

    /// The recorded key is the line of the GATE CALL in the calling test, not a line
    /// inside the gate module. This is what makes the key unforgeable: the `test`
    /// argument is a message, and the inventory recomputes these keys from source.
    #[test]
    fn the_recorded_key_is_the_call_site_of_the_gate_call() {
        let path = scratch("call_site");
        let line = line!() + 2;
        let token = redirected(&path, || {
            gate("a name that is deliberately not this test's name", &["cargo"])
        });
        assert!(token.is_some(), "cargo must be on PATH for this test to mean anything");
        let want = format!("{}:{}\n", file!().replace('\\', "/"), line);
        assert!(
            ledger(&path).contains(&want),
            "the ledger must hold {want:?}, got {:?}",
            ledger(&path)
        );
    }

    /// `require_toolchain` forwards the caller's location, so a test that uses the
    /// boolean wrapper is recorded at ITS line and not at the wrapper's.
    #[test]
    fn require_toolchain_records_its_own_caller_not_the_wrapper() {
        let path = scratch("wrapper");
        let line = line!() + 1;
        let ok = redirected(&path, || require_toolchain("wrapper probe", &["cargo"]));
        assert!(ok, "cargo must be on PATH for this test to mean anything");
        let want = format!("{}:{}\n", file!().replace('\\', "/"), line);
        assert!(
            ledger(&path).contains(&want),
            "the ledger must hold {want:?}, got {:?}",
            ledger(&path)
        );
    }

    /// A missing tool records NOTHING — on the declared-skip path and on the panic
    /// path alike. A ledger entry therefore means the body was allowed to run.
    #[test]
    fn a_missing_tool_records_nothing_on_either_path() {
        let skipped = scratch("skip");
        let declared = std::env::var_os(ALLOW_MISSING_ENV).is_some();
        let out = redirected(&skipped, || {
            std::panic::catch_unwind(|| require_toolchain("absent probe", &[ABSENT]))
        });
        match out {
            Ok(allowed) => assert!(
                !allowed && declared,
                "an absent tool must not pass the gate"
            ),
            Err(_) => assert!(!declared, "the panic path is the undeclared one"),
        }
        let want = format!("{}:", file!().replace('\\', "/"));
        assert!(
            !ledger(&skipped).contains(&want),
            "the skip and panic paths must record nothing from this file, got {:?}",
            ledger(&skipped)
        );
    }
}
