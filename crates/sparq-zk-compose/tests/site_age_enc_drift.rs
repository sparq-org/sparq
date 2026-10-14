// sq-1s2.4 (FL1 follow-up) — BUILD-TIME DRIFT GUARD for the browser
// age-gate's precomputed `operand_enc` anchors.
//
// The ZK car-hire flagship proves `age >= 25` LIVE in the browser against the
// committed `filter_int_d2` circuit. The public input `operand_enc` binds the
// hidden age to `"<age>"^^xsd:integer`, and the site ships it PRECOMPUTED (there
// is no Rust in the browser) in two places:
//
//   * site/src/lib/zk-prover.ts          — `AGE_OPERAND_ENC` (the provable ages)
//   * site/scripts/capture-zk-manifest.mjs — `OPERAND_ENC` (the captured age)
//
// Those constants MUST equal `encode_int_literal(age)` from THIS crate — the same
// native encoder the scan proof uses for the operand column. If a circuit / encoder
// bump changes the term encoding but the hard-coded site constants are not
// regenerated, the enc silently goes STALE: the in-browser witness solve then fails
// and the live age-gate becomes UNSATISFIABLE with no compile-time signal.
//
// This test is that missing signal. It reads the ACTUAL committed site constants and
// asserts each is still exactly what the native encoder produces. On drift it fails
// RED and prints the freshly-regenerated value to paste back into the site — i.e. it
// "regenerates AGE_OPERAND_ENC from native encode_int_literal" as a gate, closing the
// FL1 follow-up. It reads the real files (no duplicated hex constant), so the check is
// non-vacuous: it verifies what the site actually ships, not a private copy.

use sparq_zk_compose::build::encode_int_literal;
use std::path::PathBuf;

// Path construction contract for the two out-of-crate reads below: each read joins
// CARGO_MANIFEST_DIR (absolute => cwd-independent) with the FULL site-file path in
// ONE literal, semicolon-terminated statement, so the CI ownership audit
// (scripts/ci_audit_inputs.py) statically resolves the exact file being read. Both
// files are attributed to this crate in ci/path-ownership.toml — a site-side edit
// of either anchor re-runs this drift guard instead of being skipped as inert.

/// The native encoding of `"<age>"^^xsd:integer`, lowercased for a case-insensitive
/// compare against the site's `0x…` hex literals.
fn native_enc(age: u64) -> String {
    encode_int_literal(age).0.to_lowercase()
}

/// The first `"…"`-quoted run in `s` (values may sit on the line after the `=`).
fn first_quoted(s: &str) -> Option<String> {
    let open = s.find('"')?;
    let rest = &s[open + 1..];
    let close = rest.find('"')?;
    Some(rest[..close].to_string())
}

/// Parse the `AGE_OPERAND_ENC` object literal in `zk-prover.ts` into `(age, hex)`
/// pairs. Deliberately tolerant of formatting/comment lines; strict about the
/// `<int>: "<hex>"` shape it does recognise.
fn parse_age_operand_enc(ts: &str) -> Vec<(u64, String)> {
    let decl = ts
        .find("AGE_OPERAND_ENC")
        .expect("site/src/lib/zk-prover.ts no longer declares AGE_OPERAND_ENC — did it move? update this drift guard (sq-1s2.4)");
    let after = &ts[decl..];
    let open = after
        .find('{')
        .expect("AGE_OPERAND_ENC declaration has no `{` — parser/site out of sync");
    let close = after[open..]
        .find("};")
        .map(|i| i + open)
        .expect("AGE_OPERAND_ENC object literal has no closing `};`");
    let body = &after[open + 1..close];

    let mut out = Vec::new();
    for line in body.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with("//") {
            continue;
        }
        let Some(colon) = line.find(':') else { continue };
        let Ok(age) = line[..colon].trim().parse::<u64>() else {
            continue;
        };
        let hex = first_quoted(&line[colon + 1..]).unwrap_or_else(|| {
            panic!("AGE_OPERAND_ENC entry for age {age} has no quoted hex value")
        });
        out.push((age, hex.to_lowercase()));
    }
    out
}

/// Value of `const <name> = <int>;` (the first such decl) in a JS/TS source.
fn parse_const_u64(src: &str, name: &str) -> u64 {
    let needle = format!("const {name}");
    let at = src
        .find(&needle)
        .unwrap_or_else(|| panic!("expected `const {name}` in capture-zk-manifest.mjs"));
    let eq = src[at..]
        .find('=')
        .map(|i| i + at)
        .unwrap_or_else(|| panic!("`const {name}` has no `=`"));
    let semi = src[eq..]
        .find(';')
        .map(|i| i + eq)
        .unwrap_or_else(|| panic!("`const {name}` has no `;`"));
    src[eq + 1..semi]
        .trim()
        .parse::<u64>()
        .unwrap_or_else(|e| panic!("`const {name}` value is not an integer: {e}"))
}

/// Value of `const <name> = "<hex>";` (the first such decl) — the RHS string may be on
/// the following line, which `first_quoted` handles.
fn parse_const_str(src: &str, name: &str) -> String {
    let needle = format!("const {name}");
    let at = src
        .find(&needle)
        .unwrap_or_else(|| panic!("expected `const {name}` in capture-zk-manifest.mjs"));
    let eq = src[at..]
        .find('=')
        .map(|i| i + at)
        .unwrap_or_else(|| panic!("`const {name}` has no `=`"));
    first_quoted(&src[eq + 1..])
        .unwrap_or_else(|| panic!("`const {name}` has no quoted value"))
        .to_lowercase()
}

/// The site's `AGE_OPERAND_ENC` map (zk-prover.ts) must match native `encode_int_literal`.
#[test]
fn site_age_operand_enc_matches_native_encoder() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../site/src/lib/zk-prover.ts");
    let Ok(ts) = std::fs::read_to_string(&path) else {
        // The crate is `publish = false` and only ever tested inside the full
        // workspace, where site/ is present. A genuinely absent file (an unusual
        // partial checkout) is skipped rather than failed — same offline-tolerant
        // posture as site/scripts/sync-zk-catalog.mjs. In CI the file exists, so the
        // gate below always bites.
        eprintln!("[sq-1s2.4] {} absent — skipping drift guard", path.display());
        return;
    };

    let entries = parse_age_operand_enc(&ts);
    assert!(
        !entries.is_empty(),
        "parsed ZERO ages from AGE_OPERAND_ENC — the parser or the site format drifted; \
         this drift guard must not silently pass vacuously (sq-1s2.4)"
    );

    for (age, site_hex) in &entries {
        let native = native_enc(*age);
        assert_eq!(
            *site_hex, native,
            "\nAGE_OPERAND_ENC enc-drift for age {age}:\n  \
             site ships (site/src/lib/zk-prover.ts): {site_hex}\n  \
             native encode_int_literal({age}):        {native}\n\
             The circuit/encoder changed; regenerate the site constant — set \
             AGE_OPERAND_ENC[{age}] = \"{native}\" (and re-run \
             `node scripts/capture-zk-manifest.mjs` if {age} is the captured age), \
             else the live in-browser age-gate is UNSATISFIABLE. (sq-1s2.4)"
        );
    }
}

/// The captured-manifest helper (capture-zk-manifest.mjs) pins `OPERAND_ENC` for its
/// `CAPTURED_AGE`; it must equal native `encode_int_literal` AND stay consistent with
/// the `AGE_OPERAND_ENC` entry for the same age (the two site files must not disagree).
#[test]
fn capture_manifest_operand_enc_matches_native_encoder() {
    let path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../site/scripts/capture-zk-manifest.mjs");
    let Ok(mjs) = std::fs::read_to_string(&path) else {
        eprintln!("[sq-1s2.4] {} absent — skipping drift guard", path.display());
        return;
    };

    let captured_age = parse_const_u64(&mjs, "CAPTURED_AGE");
    let operand_enc = parse_const_str(&mjs, "OPERAND_ENC");
    let native = native_enc(captured_age);
    assert_eq!(
        operand_enc, native,
        "\nOPERAND_ENC enc-drift in capture-zk-manifest.mjs for CAPTURED_AGE {captured_age}:\n  \
         script pins: {operand_enc}\n  \
         native encode_int_literal({captured_age}): {native}\n\
         Regenerate: set OPERAND_ENC = \"{native}\" and re-run \
         `node scripts/capture-zk-manifest.mjs`, else the captured proof will not verify. (sq-1s2.4)"
    );

    // Cross-file consistency: the captured age's enc must match the AGE_OPERAND_ENC map,
    // so the fallback (captured) and live-proving paths anchor to the SAME term.
    let prover_path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../site/src/lib/zk-prover.ts");
    let Ok(ts) = std::fs::read_to_string(&prover_path) else {
        return;
    };
    let map_hex = parse_age_operand_enc(&ts)
        .into_iter()
        .find(|(age, _)| *age == captured_age)
        .map(|(_, hex)| hex)
        .unwrap_or_else(|| {
            panic!(
                "AGE_OPERAND_ENC has no entry for CAPTURED_AGE {captured_age} — the captured \
                 manifest proves an age the live prover cannot; add {captured_age} to \
                 AGE_OPERAND_ENC in site/src/lib/zk-prover.ts (sq-1s2.4)"
            )
        });
    assert_eq!(
        operand_enc, map_hex,
        "capture-zk-manifest.mjs OPERAND_ENC and AGE_OPERAND_ENC[{captured_age}] disagree — \
         the captured and live-proving anchors must be identical (sq-1s2.4)"
    );
}
