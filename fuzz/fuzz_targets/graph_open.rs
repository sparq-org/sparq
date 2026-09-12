// Coverage-guided fuzz target for the on-disk (mmap) store loader.
// Bead sq-ovnf; threat-model T-MMAP-FUZZ. [OPUS-4.8]
//
// SURFACE: `sparq_core::Graph::open(dir)` over a CORRUPT / HOSTILE on-disk store
// directory. `open` mmaps and parses a fixed set of files written by `Graph::save`:
//   - perm0.bin .. perm5.bin   (TripleStore::open — raw [u32;3] rows OR the
//                               block-compressed format auto-detected by FILE_MAGIC)
//   - dict-meta.bin + dict-terms/offs/hash/hid.bin  (Dict::open_mmap; attacker-
//                               controlled counts/lengths — the sq-znld hardening)
//   - dict.bin                 (legacy single-file dict fallback)
//   - numerics-v2.bin / temporals-v2.bin (current sidecars, length-validated)
//   - numerics.bin / temporals.bin   (legacy sidecars, ignored and regenerated)
//   - predstats.bin            (persisted per-predicate stats)
//   - named.bin                (named-graph manifest — the sq-3ui0 / open_named path)
//
// STRATEGY: write a VALID baseline store (so every required file exists and is
// mmap-able — a missing/zero-length file just short-circuits to a clean Err, which
// reaches little parsing code), then OVERWRITE a fuzz-selected subset of those files
// with hostile bytes drawn from the input. This drives libFuzzer's bytes straight
// into each on-disk byte-parser (count/length fields, magic detection, offset
// tables, manifest entries) — the deepest hostile-bytes reach for the mmap loader.
//
// INVARIANT: a corrupt store directory must yield a clean `Err` — NEVER a panic, an
// out-of-bounds mmap read, an unchecked-count OOM/overflow abort, or UB. libFuzzer
// flags any crash and saves the reproducing input.
#![no_main]

use libfuzzer_sys::fuzz_target;
use std::io::Write;

// The on-disk files `Graph::open` reads. We may overwrite any subset with hostile
// bytes; a file left untouched keeps its valid baseline content.
const STORE_FILES: &[&str] = &[
    "perm0.bin",
    "perm1.bin",
    "perm2.bin",
    "perm3.bin",
    "perm4.bin",
    "perm5.bin",
    "dict-meta.bin",
    "dict-terms.bin",
    "dict-offs.bin",
    "dict-hash.bin",
    "dict-hid.bin",
    "dict.bin",
    "numerics-v2.bin",
    "temporals-v2.bin",
    "numerics.bin", // Legacy hostile bytes must remain ignored.
    "temporals.bin",
    "predstats.bin",
    "named.bin",
];

fuzz_target!(|data: &[u8]| {
    // First byte is a bitmask selecting WHICH files to corrupt; second byte seeds a
    // tiny LCG that slices the remaining bytes across the selected files. With <2
    // bytes there is nothing to corrupt — just exercise opening a clean store.
    let dir = match tempfile::tempdir() {
        Ok(d) => d,
        Err(_) => return, // a transient temp-dir failure is environmental, not a finding
    };

    // Baseline: a tiny but VALID store on disk. If save somehow fails, bail (env issue).
    let g = match sparq_core::Graph::load_str(
        "<http://a/s> <http://a/p> <http://a/o> .\n<http://a/s> <http://a/p2> \"42\"^^<http://www.w3.org/2001/XMLSchema#integer> .\n",
        "ntriples",
    ) {
        Ok(g) => g,
        Err(_) => return,
    };
    if g.save(dir.path()).is_err() {
        return;
    }
    // NOTE: a default-graph-only `save` deliberately does NOT write `named.bin`; the
    // corruption loop below `File::create`s it when selected, which itself drives the
    // `open_named` byte-parser against a hostile manifest (the sq-3ui0 surface).

    if data.len() >= 2 {
        let mask = data[0];
        let mut lcg = data[1] as u32 | 1;
        let payload = &data[2..];
        let mut off = 0usize;
        for (i, name) in STORE_FILES.iter().enumerate() {
            if i >= 8 {
                // The mask is 8 bits; files past 8 cycle through the high files using
                // the LCG so every file is reachable across the corpus.
                lcg = lcg.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                if (lcg >> 16) & 1 == 0 {
                    continue;
                }
            } else if mask & (1 << i) == 0 {
                continue;
            }
            // Carve a hostile slice for this file from the payload (length chosen by
            // the LCG so corrupt files vary in size, incl. truncated/empty).
            lcg = lcg.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let want = if payload.is_empty() {
                0
            } else {
                (lcg as usize >> 8) % (payload.len() + 1)
            };
            let end = (off + want).min(payload.len());
            let slice = &payload[off..end];
            off = end;
            let path = dir.path().join(name);
            if let Ok(mut f) = std::fs::File::create(&path) {
                let _ = f.write_all(slice);
            }
        }
    }

    // The contract: a corrupt directory yields Err (or, if the corruption happened to
    // be benign, a valid Graph). A panic/abort/UB here is the bug.
    let _ = sparq_core::Graph::open(dir.path());
});
