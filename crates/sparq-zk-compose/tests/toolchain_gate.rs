//! The LIVE-toolchain gate's own behaviour (`tests/support/mod.rs`).
//!
//! The gate is what every live suite in this crate relies on to NOT report `ok` for
//! a body that never executed, so its three outcomes are pinned here directly:
//! present → run, missing + declared → named visible skip, missing + undeclared →
//! fail. Deleting the panic (restoring the old early `return`) turns
//! `a_missing_tool_fails_the_test_when_the_lane_declares_nothing` and
//! `a_zero_valued_opt_out_does_not_declare_the_lane_toolchain_free` red.
//!
//! The two env-dependent outcomes need a controlled environment, and
//! `SPARQ_ZK_ALLOW_MISSING_TOOLCHAIN` is set at workflow level in the toolchain-free
//! CI lanes, so they cannot be observed in-process (and mutating the process
//! environment from a test is unsound). Each re-execs THIS test binary for the
//! single `gate_child_probe` case with the environment it wants and reads the
//! child's exit status and output.

mod support;

/// Marks the re-exec'd child run. Absent ⇒ this is the parent sweep.
const CHILD_MARKER: &str = "SPARQ_ZK_GATE_SELFTEST_CHILD";

/// A tool name no box has on `PATH`, so "missing" is not environment-dependent.
const ABSENT_TOOL: &str = "sparq-zk-compose-no-such-tool";

/// Re-exec this test binary for `gate_child_probe` alone, with
/// `SPARQ_ZK_ALLOW_MISSING_TOOLCHAIN` set to `allow` (or removed when `None`).
fn run_child(allow: Option<&str>) -> std::process::Output {
    let exe = std::env::current_exe().expect("the running test binary has a path");
    // zk-toolchain-gate: allow re-exec of this test binary, not a tool acquisition
    let mut cmd = std::process::Command::new(exe);
    // NOT `--nocapture`: libtest DISCARDS a passing test's captured output, so the
    // skip line is only observable here because the gate writes it to the process's
    // real stderr. Passing `--nocapture` would hide a regression in exactly that.
    cmd.args(["--exact", "gate_child_probe", "--test-threads=1"])
        .env(CHILD_MARKER, "1")
        .env_remove(support::ALLOW_MISSING_ENV);
    if let Some(value) = allow {
        cmd.env(support::ALLOW_MISSING_ENV, value);
    }
    cmd.output().expect("re-exec of the test binary")
}

/// Everything the child wrote, whichever stream libtest chose for it.
fn output_text(out: &std::process::Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

/// The probe the cases below re-exec. In the PARENT sweep it is a deliberate no-op
/// (the environment it needs is the child's, not this process's); only the re-exec
/// with `CHILD_MARKER` set reaches the gate.
#[test]
fn gate_child_probe() {
    if std::env::var_os(CHILD_MARKER).is_none() {
        return;
    }
    // Reached only in the child. Panics unless the child's env declares the lane
    // toolchain-free, in which case this returns false.
    assert!(
        !support::require_toolchain("gate_child_probe", &[ABSENT_TOOL]),
        "the gate cannot report a tool that does not exist as present"
    );
}

/// A missing tool with no declaration FAILS, and the failure names the tool, the
/// PATH fix and the opt-out. This is the whole point: the old gate returned early
/// and the test was reported `ok`.
#[test]
fn a_missing_tool_fails_the_test_when_the_lane_declares_nothing() {
    let out = run_child(None);
    let text = output_text(&out);
    assert!(
        !out.status.success(),
        "an undeclared missing tool must fail the test, not pass it: {}",
        text
    );
    for needle in [
        ABSENT_TOOL,
        "gate_child_probe",
        "/root/.nargo/bin",
        support::ALLOW_MISSING_ENV,
    ] {
        assert!(
            text.contains(needle),
            "the failure must name {}: {}",
            needle,
            text
        );
    }
}

/// With the opt-out declared, the same case passes — but the skip is PRINTED and
/// names the test, the missing tool and the variable that allowed it — and stays
/// visible although libtest discards a PASSING test's captured output.
#[test]
fn the_declared_opt_out_turns_the_failure_into_a_named_visible_skip() {
    let out = run_child(Some("1"));
    let text = output_text(&out);
    assert!(
        out.status.success(),
        "a declared toolchain-free lane must skip, not fail: {}",
        text
    );
    for needle in [
        "SKIP gate_child_probe",
        ABSENT_TOOL,
        support::ALLOW_MISSING_ENV,
    ] {
        assert!(
            text.contains(needle),
            "the skip line must name {}: {}",
            needle,
            text
        );
    }
}

/// `…=0` is not a declaration, so it must NOT buy back the silent pass.
#[test]
fn a_zero_valued_opt_out_does_not_declare_the_lane_toolchain_free() {
    let out = run_child(Some("0"));
    assert!(
        !out.status.success(),
        "{}=0 must leave the gate failing: {}",
        support::ALLOW_MISSING_ENV,
        output_text(&out)
    );
}

/// An empty value is not a declaration either.
#[test]
fn an_empty_opt_out_does_not_declare_the_lane_toolchain_free() {
    let out = run_child(Some(""));
    assert!(
        !out.status.success(),
        "an empty {} must leave the gate failing: {}",
        support::ALLOW_MISSING_ENV,
        output_text(&out)
    );
}

/// A tool that IS on PATH passes the gate, so the gate is not simply always-false.
/// `cargo --version` exits 0 wherever this suite can be built or run at all.
#[test]
fn a_tool_on_path_passes_the_gate() {
    assert!(support::require_toolchain(
        "a_tool_on_path_passes_the_gate",
        &["cargo"]
    ));
}
