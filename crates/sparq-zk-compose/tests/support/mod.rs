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
/// `zk-toolchain.yml` sets it at JOB level and its final step verifies that every
/// `require_toolchain` call site in the ZK test trees left an entry. A skip appends
/// nothing and a panic appends nothing, so a lane that skipped past the gate cannot
/// produce a complete ledger. The ledger proves the gate was REACHED; it says
/// nothing about what the body went on to assert.
pub const LEDGER_ENV: &str = "SPARQ_ZK_LIVE_LEDGER";

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

/// This test binary's cargo target name, the first half of a ledger key.
fn binary_name() -> &'static str {
    env!("CARGO_CRATE_NAME")
}

/// This package's name, so two crates' identically named test binaries (the
/// per-crate copies of `toolchain_gate`) get distinct ledger keys.
fn package_name() -> &'static str {
    env!("CARGO_PKG_NAME")
}

/// The ledger key for `test`: `<package>/<binary>::<test>`.
fn ledger_key(test: &str) -> String {
    format!("{}/{}::{}", package_name(), binary_name(), test)
}

/// Append `test`'s key to the ledger, if the lane asked for one.
///
/// Opened with `O_APPEND` (`append(true)`) and written as ONE line in ONE
/// `write_all`, so entries from tests running in parallel, or from a re-exec'd child
/// process, cannot interleave. Best-effort by design: a write failure must not turn
/// a passing live test red, and a missing entry is what the verifier reports.
fn record(test: &str) {
    use std::io::Write;
    let path = match std::env::var_os(LEDGER_ENV) {
        Some(p) if !p.is_empty() => p,
        _ => return,
    };
    let line = format!("{}\n", ledger_key(test));
    let _ = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .and_then(|mut f| f.write_all(line.as_bytes()));
}

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
/// * all present -> `Some(token)`, and `test`'s key is appended to the ledger.
/// * absent, [`ALLOW_MISSING_ENV`] set -> prints a skip line naming `test`, the
///   missing tool(s) and the variable, then `None`; NOTHING is recorded.
/// * absent otherwise -> PANICS, naming the missing tool, how to put the toolchain
///   on `PATH`, and the opt-out variable; nothing is recorded.
///
/// `test` is the calling test's own name. It is passed explicitly rather than read
/// from the thread name because libtest runs tests on the main thread (named
/// `main`) under `--test-threads=1`, which is exactly how the toolchain lane runs
/// them. It must be a STRING LITERAL: the checker builds the expected ledger from
/// those literals and rejects a call site whose name it cannot read.
#[allow(dead_code)]
pub fn gate(test: &str, tools: &[&str]) -> Option<LiveToolchain> {
    let missing: Vec<&str> = tools.iter().copied().filter(|t| !on_path(t)).collect();
    if missing.is_empty() {
        record(test);
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
#[allow(dead_code)]
pub fn require_toolchain(test: &str, tools: &[&str]) -> bool {
    gate(test, tools).is_some()
}
