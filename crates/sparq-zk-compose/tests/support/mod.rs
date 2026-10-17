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

/// Gate a LIVE test body on `tools` all being on `PATH`.
///
/// * all present → `true`; the caller runs its live body.
/// * absent, [`ALLOW_MISSING_ENV`] set → prints a skip line naming `test`, the
///   missing tool(s) and the variable, then `false`; the caller returns.
/// * absent otherwise → PANICS, naming the missing tool, how to put the toolchain
///   on `PATH`, and the opt-out variable.
///
/// `test` is the calling test's own name. It is passed explicitly rather than read
/// from the thread name because libtest runs tests on the main thread (named
/// `main`) under `--test-threads=1`, which is exactly how the toolchain lane runs
/// them.
pub fn require_toolchain(test: &str, tools: &[&str]) -> bool {
    let missing: Vec<&str> = tools.iter().copied().filter(|t| !on_path(t)).collect();
    if missing.is_empty() {
        return true;
    }
    let missing = missing.join(", ");
    if allow_missing() {
        skip_line(&format!(
            "SKIP {}: the live Noir toolchain is missing ({} not on PATH) and {}=1 declares \
             this lane toolchain-free — the live body did NOT run.",
            test, missing, ALLOW_MISSING_ENV
        ));
        return false;
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
