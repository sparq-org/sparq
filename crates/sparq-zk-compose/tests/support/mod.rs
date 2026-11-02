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
/// exists to end. Writing through `std::io::stderr` bypasses that capture without
/// opening `/dev/stderr`, which is unavailable on some runners. Lock the stream
/// while writing the notice so parallel tests cannot interleave a line.
/// A runner that captures the process's stderr itself (nextest does,
/// for a passing test, unless `success-output` is set) still hides the line; the
/// skip is then declared by the lane's environment rather than printed.
fn skip_line(msg: &str) {
    use std::io::Write;
    let _ = std::io::stderr().lock().write_all(format!("{msg}\n").as_bytes());
}

fn allow_missing() -> bool {
    std::env::var_os(ALLOW_MISSING_ENV).is_some_and(|v| !v.is_empty() && v != "0")
}

/// Names the file the gate appends to when a live body is allowed to run.
///
/// Private to this module so no test NAMES it by accident, but it is not a security
/// boundary: the variable is in the job environment and the path is in the workflow,
/// so test code that set out to write its own lines could. What the ledger is
/// evidence for is the accident — a live test that no step runs, that returns before
/// the gate, or that returns after the gate without reaching its assertions.
/// `zk-toolchain.yml` sets it at JOB level and its final step verifies that every
/// gate call site in the ZK test sources left BOTH lines:
///
/// * `enter <file>:<line>` — written by the gate when it lets the body run;
/// * `complete <file>:<line>` — written by [`complete`],
///   which each live test calls as its LAST statement, after its assertions. The key
///   is the GATE's call site, carried from the gate to the completion call, so the
///   two lines of one test are matched by construction.
///
/// A skip appends nothing and a panic appends nothing. A test that panics in an
/// assertion, or returns early, after the gate leaves an `enter` line with no
/// `complete` line, which the verify step reports.
const LEDGER_ENV: &str = "SPARQ_ZK_LIVE_LEDGER";

/// Evidence that [`gate`] found every requested tool on `PATH` and let the live
/// body run, and the obligation to say so once the body has asserted what it is for.
///
/// Only [`gate`] mints one — the fields are private to this module — and
/// [`live_tool`] takes one, so a live tool cannot be spawned without having passed
/// the gate first. The token carries the gate's call site, which [`complete`]
/// records; it is neither `Clone` nor `Copy`, so it is completed exactly once.
///
/// Dropping a token that was never passed to [`complete`] PANICS (unless the thread
/// is already panicking, so a failed assertion still reports itself rather than
/// aborting the process). A test that returns early after the gate therefore fails
/// where it runs, and leaves no completion line in the ledger either.
/// `#[allow(dead_code)]` because each test binary declaring `mod support;` uses a
/// subset of this module.
#[allow(dead_code)]
#[must_use = "a live test must pass its gate token to `support::complete` after its assertions"]
pub struct LiveToolchain {
    site: &'static std::panic::Location<'static>,
    completed: bool,
}

impl Drop for LiveToolchain {
    fn drop(&mut self) {
        if !self.completed && !std::thread::panicking() {
            panic!(
                "the live test gated at {} ended without calling `support::complete` on \
                 its token: either it returned before its assertions, or the completion \
                 call is missing. Call `support::complete(token)` as the test's last \
                 statement.",
                ledger_key(self.site)
            );
        }
    }
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

/// Append `<kind> <key>` for the CALL SITE `loc` to the ledger, if the lane asked
/// for one.
///
/// Opened with `O_APPEND` (`append(true)`) and written as ONE line in ONE
/// `write_all`, so entries from tests running in parallel, or from a re-exec'd child
/// process, cannot interleave. Best-effort by design: a write failure must not turn
/// a passing live test red, and a missing line is what the verifier reports.
fn record(kind: &str, loc: &std::panic::Location<'static>) {
    use std::io::Write;
    let Some(path) = ledger_path() else {
        return;
    };
    let line = format!("{} {}\n", kind, ledger_key(loc));
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
        if let Some(path) = TEST_LEDGER.with(|cell| cell.borrow().clone()) {
            return Some(path);
        }
    }
    match std::env::var_os(LEDGER_ENV) {
        Some(p) if !p.is_empty() => Some(std::path::PathBuf::from(p)),
        _ => None,
    }
}

// A ledger path for this module's own tests only. Private, `cfg(test)`-only, and
// THREAD-LOCAL.
//
// libtest runs tests on worker threads (or, under `--test-threads=1`, one after
// another on the main thread), so a thread-local redirect reaches only the test
// that set it: a live test running in parallel keeps writing to the real ledger.
// A process-wide override could divert that live test's entry into a temporary
// file and turn the lane's ledger check red for the wrong reason — the compose lib
// step `cargo test --lib driver::` runs without `--test-threads=1`, so this is a
// real ordering, not a hypothetical one.
#[cfg(test)]
thread_local! {
    static TEST_LEDGER: std::cell::RefCell<Option<std::path::PathBuf>> =
        const { std::cell::RefCell::new(None) };
}

/// A [`std::process::Command`] for a live tool.
///
/// Takes a [`LiveToolchain`], which only [`gate`] mints, so the gate cannot be
/// skipped. It ALSO re-checks `PATH` itself and panics when the tool is absent, so
/// neither a stale token nor an `is_err()` test on the spawn result can be turned
/// into a soft pass. `scripts/check-zk-toolchain-gate.py` lints the ZK test trees
/// for every other acquisition path, and requires the spawned value's exit status
/// (or its expected failure diagnostic) to be asserted.
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
/// [`live_tool`] needs and [`complete`] consumes.
///
/// * all present -> `Some(token)`, and `enter <file>:<line>` for the CALL SITE is
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
    let site = std::panic::Location::caller();
    let missing: Vec<&str> = tools.iter().copied().filter(|t| !on_path(t)).collect();
    if missing.is_empty() {
        record("enter", site);
        return Some(LiveToolchain {
            site,
            completed: false,
        });
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

/// Close a live test: append `complete <file>:<line>` for the GATE's call site that
/// `token` carries, and discharge the token's drop obligation.
///
/// Call it as the test's LAST statement, after every assertion, so the line exists
/// only for a body that ran its checks to the end. It is deliberately NOT done in
/// `Drop`: a test that returns early must leave no completion line.
#[allow(dead_code)]
pub fn complete(mut token: LiveToolchain) {
    record("complete", token.site);
    token.completed = true;
}

// --------------------------------------------------------------------------- #
// The gate's own identity tests.
//
// Compiled into every binary that declares `mod support;` and into the crate's lib
// target (which includes this same file with `#[path]`), so they run wherever the
// gate does. They are about the LEDGER KEY: that it is the caller's own line, that
// the token carries that line to completion, that
// the completion line carries the GATE's key, that an uncompleted token fails, and
// that nothing is recorded on the skip or panic paths.
// --------------------------------------------------------------------------- #

#[cfg(test)]
mod gate_identity_tests {
    use super::{complete, gate, ALLOW_MISSING_ENV, TEST_LEDGER};

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

    /// Run `body` with this THREAD's ledger pointed at `path`.
    ///
    /// The override is thread-local, so these tests cannot overwrite each other's
    /// path and cannot divert a live test running in parallel. The tests that
    /// provoke a panic catch it INSIDE `body`, so `body()` returns normally and the
    /// redirect is always cleared; a panic escaping `body` would leave it set for
    /// this thread, which reaches nothing but these tests.
    fn redirected<T>(path: &std::path::Path, body: impl FnOnce() -> T) -> T {
        TEST_LEDGER.with(|cell| *cell.borrow_mut() = Some(path.to_path_buf()));
        let out = body();
        TEST_LEDGER.with(|cell| *cell.borrow_mut() = None);
        out
    }

    fn ledger(path: &std::path::Path) -> String {
        std::fs::read_to_string(path).unwrap_or_default()
    }

    /// The recorded key is the line of the GATE CALL in the calling test, not a line
    /// inside the gate module, and the completion line carries that same key. This
    /// is what makes the key unforgeable: the `test` argument is a message, and the
    /// inventory recomputes these keys from source.
    #[test]
    fn the_recorded_key_is_the_call_site_of_the_gate_call() {
        let path = scratch("call_site");
        let line = line!() + 2;
        let token = redirected(&path, || {
            gate("a name that is deliberately not this test's name", &["cargo"])
        });
        let token = token.expect("cargo must be on PATH for this test to mean anything");
        let key = format!("{}:{}", file!().replace('\\', "/"), line);
        assert_eq!(
            ledger(&path),
            format!("enter {key}\n"),
            "the gate records the entry line only"
        );
        redirected(&path, || complete(token));
        assert_eq!(
            ledger(&path),
            format!("enter {key}\ncomplete {key}\n"),
            "the completion line carries the GATE's call site"
        );
    }

    /// A token dropped without `complete` PANICS and records no completion: the
    /// shape of a test that returns early after its gate.
    #[test]
    fn an_uncompleted_token_panics_and_records_no_completion() {
        let path = scratch("early_return");
        let line = line!() + 3;
        let out = redirected(&path, || {
            std::panic::catch_unwind(|| {
                let token = gate("early return probe", &["cargo"]);
                drop(token);
            })
        });
        let key = format!("{}:{}", file!().replace('\\', "/"), line);
        let err = out.expect_err("dropping an uncompleted token must panic");
        let msg = err.downcast_ref::<String>().cloned().unwrap_or_default();
        assert!(
            msg.contains(&key) && msg.contains("support::complete"),
            "the panic must name the gate site and the missing call, got {msg:?}"
        );
        assert_eq!(
            ledger(&path),
            format!("enter {key}\n"),
            "an early return leaves the entry line and NO completion line"
        );
    }

    /// A token dropped while the thread is ALREADY panicking (a failed assertion
    /// after the gate) does not panic a second time — that would abort the process
    /// and hide the assertion's own message — and still records no completion.
    #[test]
    fn a_token_dropped_during_a_failed_assertion_reports_that_assertion() {
        let path = scratch("assert_fail");
        let out = redirected(&path, || {
            std::panic::catch_unwind(|| {
                let _token = gate("failed assertion probe", &["cargo"]);
                panic!("the assertion under test failed");
            })
        });
        let err = out.expect_err("the assertion's panic must propagate");
        assert_eq!(
            err.downcast_ref::<&str>().copied(),
            Some("the assertion under test failed"),
            "the assertion's own message must be the one reported"
        );
        assert!(
            !ledger(&path).contains("complete "),
            "a failed assertion must leave no completion line, got {:?}",
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
            std::panic::catch_unwind(|| gate("absent probe", &[ABSENT]))
        });
        match out {
            Ok(allowed) => assert!(
                allowed.is_none() && declared,
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
