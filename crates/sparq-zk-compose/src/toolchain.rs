// [OPUS-5.5] beadzkp-13.5: one exact pinned-version field matcher, shared by the
// `successful-results` library paths and `examples/result_experiment.rs` (via `#[path]`).
//! Internal exact `--version` field matcher for the pinned Noir/Barretenberg toolchain.
//!
//! Version text is a guard against running an unpinned toolchain, not an attestation:
//! callers still require a successful `--version` exit, and the experiment driver also
//! hashes the resolved binary. Output that is not valid UTF-8 is rejected outright.

/// Pinned `(tool, version)` pairs; the verifier byte layout depends on these exact builds.
pub(crate) const PINNED_TOOLS: [(&str, &str); 2] =
    [("nargo", "1.0.0-beta.21"), ("bb", "5.0.0-nightly.20260324")];

/// Returns whether raw `tool --version` stdout reports exactly the pinned version.
///
/// `nargo` must print `nargo version = <pin>` as its whole first line; the following
/// `noirc`/git metadata lines are permitted. `bb` output, trimmed, must equal the pin.
/// Unsupported tools, empty or non-UTF-8 output, prefix/suffix near matches, and a later
/// line mentioning the pin after a wrong first field all return `false`.
pub(crate) fn pinned_version_output(tool: &str, stdout: &[u8]) -> bool {
    let Some(&(_, expected)) = PINNED_TOOLS.iter().find(|(name, _)| *name == tool) else {
        return false;
    };
    let Ok(output) = std::str::from_utf8(stdout) else {
        return false;
    };
    match tool {
        "nargo" => output
            .lines()
            .next()
            .is_some_and(|line| line.strip_prefix("nargo version = ") == Some(expected)),
        "bb" => output.trim() == expected,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::{PINNED_TOOLS, pinned_version_output as ok};

    const N: &str = "1.0.0-beta.21";
    const B: &str = "5.0.0-nightly.20260324";

    #[test]
    fn pins_match_documented_versions() {
        assert_eq!(PINNED_TOOLS, [("nargo", N), ("bb", B)]);
    }

    #[test]
    fn known_outputs_are_accepted() {
        let nargo = format!(
            "nargo version = {N}\nnoirc version = {N}+0123456789abcdef\n\
             (git version hash: 0123456789abcdef, is dirty: false)\n"
        );
        assert!(ok("nargo", nargo.as_bytes()));
        assert!(ok("nargo", format!("nargo version = {N}").as_bytes()));
        assert!(ok(
            "nargo",
            format!("nargo version = {N}\r\nnoirc").as_bytes()
        ));
        assert!(ok("bb", B.as_bytes()));
        assert!(ok("bb", format!("{B}\n").as_bytes()));
        // `bb` output is trimmed by contract, so surrounding whitespace is accepted.
        for pad in [" ", "\t", "\r\n", "\n\n"] {
            assert!(ok("bb", format!("{pad}{B}{pad}").as_bytes()), "{pad:?}");
        }
    }

    #[test]
    fn near_matches_and_diagnostic_rescue_are_rejected() {
        for suffix in ["0", "-other", "+other", ".1"] {
            assert!(!ok("bb", format!("{B}{suffix}").as_bytes()));
            let nargo = format!("nargo version = {N}{suffix}\n{N}");
            assert!(!ok("nargo", nargo.as_bytes()));
        }
        // The Nargo first line is matched untrimmed: trailing blanks are not the pin.
        for suffix in [" ", "\t"] {
            let nargo = format!("nargo version = {N}{suffix}\n{N}");
            assert!(!ok("nargo", nargo.as_bytes()), "{suffix:?}");
        }
        for prefix in ["1", "v", "bb ", "bb version = "] {
            assert!(!ok("bb", format!("{prefix}{B}").as_bytes()));
        }
        for first in [
            format!(" nargo version = {N}"),
            format!("nargo version={N}"),
            format!("Nargo version = {N}"),
            format!("noirc version = {N}\nnargo version = {N}"),
            format!("nargo version = 1{N}"),
            format!("nargo version = 1.0.0-beta.2\nexpected {N}"),
            format!("\nnargo version = {N}"),
        ] {
            assert!(!ok("nargo", first.as_bytes()), "{first:?}");
        }
        assert!(!ok("bb", format!("bad\n{B}").as_bytes()));
        assert!(!ok("bb", format!("{B}\nwarning: {B}").as_bytes()));
        assert!(!ok("bb", b"5.0.0-nightly.20260325"));
    }

    #[test]
    fn unsupported_tools_empty_and_non_utf8_are_rejected() {
        for tool in ["", "Nargo", "nargo ", "bbb", "noirc", "git"] {
            assert!(!ok(tool, format!("nargo version = {N}").as_bytes()));
            assert!(!ok(tool, B.as_bytes()));
        }
        for tool in ["nargo", "bb"] {
            assert!(!ok(tool, b""));
            assert!(!ok(tool, b" \n"));
        }
        let mut nargo = format!("nargo version = {N}\nnoirc ").into_bytes();
        nargo.push(0xff);
        assert!(!ok("nargo", &nargo));
        let mut bb = B.as_bytes().to_vec();
        bb.push(0xff);
        assert!(!ok("bb", &bb));
    }
}
