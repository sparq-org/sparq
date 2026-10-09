//! [FABLE-5] sq-oy1f.40 — shared machinery for the W3C JSON-LD 1.1 conformance
//! lane submodules (`to_rdf`, `from_rdf`, `expand`, `compact`, `flatten`,
//! `frame`). Split out of the former monolithic `tests/jsonld_suite.rs` — the
//! manifest walker, the `Score` scoreboard, and the not-implemented bucket
//! reporting live here; each per-lane runner lives in its own sibling submodule.
//! Pure refactor: the CI invocation, the compiled test binary, and the pass
//! counts are byte-identical to before the split.
//!
//! [FABLE-5] sq-hmd7l.15 — the canonical-dataset helpers, the JSON-LD
//! document-level comparator, and the AST bridge moved VERBATIM to the lib-side
//! `sparq_conformance::jsonld_bench` module (re-exported below) so the
//! `bench/jsonld` harness's output-equality gate shares this ONE comparator.
//!
//! The six ratchet FLOORS now live LIB-SIDE in `sparq_conformance::floors::<lane>`
//! (sq-oy1f.40) and are re-exported here so the per-lane runners keep spelling
//! `TORDF_FLOOR` etc. unchanged while reading the ONE compile-time source the
//! scoreboard registry also imports (no textual drift).

use oxrdf::Dataset;
use serde_json::Value;
use sparq_core::Graph;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

// [FABLE-5] sq-hmd7l.15 — the document-level comparator, the AST bridge, and the
// canonical-dataset helpers moved VERBATIM to the LIB-SIDE `jsonld_bench` module
// (behind the same `jsonld-suite` feature) so the `bench/jsonld` comparative
// harness's output-equality gate shares the ONE comparator this conformance
// ratchet trusts. Re-exported here so every lane keeps spelling
// `json_ld_equal` / `jsonld_to_canonical_dataset` / … unchanged.
// (`read_context_member` is NOT re-exported: since sq-oy1f.27 the compact lane
// drives the native compact() oracle, so no test module uses it — the
// `bench_jsonld` example imports it from the lib directly.)
// [OPUS-4.8] sq-hmd7l.15 merge-reconcile: `dataset_to_nquads` is likewise NOT
// re-exported. origin/main (sq-oy1f.29) retired the RDF-oracle frame lane — the
// only test-binary consumer — for the native document-level framer, so no test
// module references it; the `bench_jsonld` example still imports it from the lib
// directly. It stays defined LIB-SIDE for that bench consumer.
pub use sparq_conformance::jsonld_bench::{
    json_ld_equal, jsonld_to_canonical_dataset, nquads_to_canonical_dataset, sparq_json_to_serde,
};

// [FABLE-5] sq-oy1f.40 — the six ratchet floors, re-exported from the LIB-SIDE
// single source (`sparq_conformance::floors::<lane>::FLOOR`). The runner's
// `assert!(pass >= *_FLOOR)` and `scoreboard::SUITES`' `ratchet_floor` now read the
// SAME const at compile time, so they cannot drift (kills the #1463 floor-drift
// class). The `*_FLOOR` aliases keep the runner call sites unchanged.
pub use sparq_conformance::floors::compact::FLOOR as COMPACT_FLOOR;
pub use sparq_conformance::floors::expand::FLOOR as EXPAND_FLOOR;
pub use sparq_conformance::floors::flatten::FLOOR as FLATTEN_FLOOR;
pub use sparq_conformance::floors::frame::FLOOR as FRAME_FLOOR;
pub use sparq_conformance::floors::from_rdf::FLOOR as FROMRDF_FLOOR;
pub use sparq_conformance::floors::to_rdf::FLOOR as TORDF_FLOOR;

/// [OPUS-4.8] sq-oy1f.19 — the framing suite's declared base (its
/// `baseIri`), used to resolve each frame test's input path into the document
/// IRI. Distinct from `SUITE_BASE` (the json-ld-api base) — framing is a
/// separate W3C repo.
pub const FRAME_SUITE_BASE: &str = "https://w3c.github.io/json-ld-framing/tests/";

/// The suite's declared base for resolving each test's input path into the
/// document IRI (the toRdf base when `option.base` is absent).
pub const SUITE_BASE: &str = "https://w3c.github.io/json-ld-api/tests/";

pub fn suite_root() -> PathBuf {
    // CARGO_MANIFEST_DIR = crates/sparq-conformance; the workspace root is two up.
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/w3c/json-ld-api/tests")
}

/// [OPUS-4.8] sq-oy1f.19 — the SEPARATE `w3c/json-ld-framing` suite root
/// (`scripts/fetch-jsonld-framing-tests.sh`). Framing lives in its own W3C
/// repo, not under `json-ld-api`.
pub fn frame_suite_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/w3c/json-ld-framing/tests")
}

#[derive(Default)]
pub struct Score {
    pub pass: usize,
    pub fail: usize,
    pub skip: usize,
    pub failures: Vec<(String, String)>,
}

impl Score {
    pub fn pass(&mut self) {
        self.pass += 1;
    }
    pub fn fail(&mut self, id: &str, why: String) {
        self.fail += 1;
        self.failures.push((id.to_string(), why));
    }
    pub fn skip(&mut self) {
        self.skip += 1;
    }
}

// ---- The parsed manifest entry --------------------------------------------

/// One parsed manifest entry's salient fields.
pub struct Entry {
    pub id: String,
    pub is_negative: bool,
    pub input: String,
    pub expect: Option<String>,
    pub base_override: Option<String>,
    /// An optional-feature requirement (`GeneralizedRdf` / `I18nDatatype` /
    /// `CompoundLiteral`); sparq does not opt into these, so such entries are
    /// SKIPPED (not failed) — they are out of the current gated surface.
    pub requires: Option<String>,
    /// [OPUS-4.8] sq-3uos5 — the `compact` manifest's top-level `context`
    /// member: a path (relative to the suite root) to the sibling
    /// `*-context.jsonld` file holding the caller `@context` to compact
    /// against. (The toRdf/fromRdf manifests have no such member; it stays
    /// `None` there.)
    pub context: Option<String>,
    /// [OPUS-4.8] sq-3uos5 — `option.specVersion` (`json-ld-1.0`/`-1.1`) and
    /// `option.processingMode`, recorded so the compact runner can SKIP the
    /// JSON-LD-1.0-only error-raising negatives sparq's 1.1 writer does not
    /// model (it is not a faithful pass for a 1.1 processor to "reject" them).
    pub spec_version: Option<String>,
    pub processing_mode: Option<String>,
    /// [OPUS-4.8] sq-oy1f.19 — the `frame` manifest's top-level `frame`
    /// member: a path (relative to the suite root) to the sibling
    /// `*-frame.jsonld` document (the frame pattern + its `@context`) to frame
    /// the input against. `None` for the toRdf/fromRdf/compact manifests.
    pub frame: Option<String>,
    /// [OPUS-4.8] sq-oy1f.19 — a NegativeEvaluationTest's expected JSON-LD
    /// error code (`invalid frame`, `invalid @embed value`, …).
    /// [FABLE-5] sq-oy1f.29: the native framer now RAISES the spec's
    /// frame-validation errors, so the frame runner RUNS these negatives (pass
    /// iff the raised code equals this expected code) instead of skipping them.
    pub expect_error_code: Option<String>,
    /// [SONNET-4.6] sq-kk1mq — `option.expandContext`: a path (relative to the
    /// suite root) to a context file. Forwarded to the native expand() oracle as
    /// `JsonLdOptions.expand_context`; `None` for the toRdf/fromRdf/compact/frame
    /// manifests (the expand manifest is the only one that has this option).
    pub expand_context_path: Option<String>,
    /// [FABLE-5] sq-oy1f.27 — `option.compactArrays` / `option.compactToRelative`
    /// (compact-manifest options). Forwarded to the native compact() oracle as
    /// `JsonLdOptions.compact_arrays` / `.compact_to_relative`; `None` (spec default
    /// `true`) when the manifest entry does not set them.
    pub compact_arrays: Option<bool>,
    pub compact_to_relative: Option<bool>,
    /// [FABLE-5] sq-oy1f.29 — `option.omitGraph` / `option.ordered` (frame-manifest
    /// options). Forwarded to the native frame() oracle as
    /// `FrameOptions.omit_graph` / `JsonLdOptions.ordered`; `None` when the manifest
    /// entry does not set them (mode-dependent spec default / `false`).
    pub omit_graph: Option<bool>,
    pub ordered: Option<bool>,
}

/// Read `<cat>-manifest.jsonld` and return its `sequence` entries.
pub fn read_manifest(root: &Path, cat: &str) -> Result<Vec<Entry>, String> {
    let path = root.join(format!("{}-manifest.jsonld", cat));
    let text =
        std::fs::read_to_string(&path).map_err(|e| format!("read {}: {e}", path.display()))?;
    let v: Value =
        serde_json::from_str(&text).map_err(|e| format!("parse {}: {e}", path.display()))?;
    let seq = v
        .get("sequence")
        .and_then(Value::as_array)
        .ok_or("manifest has no sequence array")?;
    let mut out = Vec::new();
    for e in seq {
        let id = e
            .get("@id")
            .and_then(Value::as_str)
            .unwrap_or("?")
            .to_string();
        let types: Vec<&str> = e
            .get("@type")
            .map(|t| match t {
                Value::Array(a) => a.iter().filter_map(Value::as_str).collect(),
                Value::String(s) => vec![s.as_str()],
                _ => vec![],
            })
            .unwrap_or_default();
        let is_negative = types.iter().any(|t| t.contains("NegativeEvaluationTest"));
        let input = e
            .get("input")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let expect = e.get("expect").and_then(Value::as_str).map(str::to_string);
        let opt = e.get("option");
        let base_override = opt
            .and_then(|o| o.get("base"))
            .and_then(Value::as_str)
            .map(str::to_string);
        let requires = e
            .get("requires")
            .and_then(Value::as_str)
            .map(str::to_string);
        // [OPUS-4.8] sq-3uos5 — compact-only manifest members (None elsewhere).
        let context = e.get("context").and_then(Value::as_str).map(str::to_string);
        let spec_version = opt
            .and_then(|o| o.get("specVersion"))
            .and_then(Value::as_str)
            .map(str::to_string);
        let processing_mode = opt
            .and_then(|o| o.get("processingMode"))
            .and_then(Value::as_str)
            .map(str::to_string);
        // [OPUS-4.8] sq-oy1f.19 — frame-only manifest members (None elsewhere).
        let frame = e.get("frame").and_then(Value::as_str).map(str::to_string);
        let expect_error_code = e
            .get("expectErrorCode")
            .and_then(Value::as_str)
            .map(str::to_string);
        // [SONNET-4.6] sq-kk1mq — expand-only manifest option: `option.expandContext`
        // is a path (relative to the suite root) to a context file.
        let expand_context_path = opt
            .and_then(|o| o.get("expandContext"))
            .and_then(Value::as_str)
            .map(str::to_string);
        // [FABLE-5] sq-oy1f.27 — compact-manifest options (None elsewhere).
        let compact_arrays = opt
            .and_then(|o| o.get("compactArrays"))
            .and_then(Value::as_bool);
        let compact_to_relative = opt
            .and_then(|o| o.get("compactToRelative"))
            .and_then(Value::as_bool);
        // [FABLE-5] sq-oy1f.29 — frame-manifest options (None elsewhere).
        let omit_graph = opt
            .and_then(|o| o.get("omitGraph"))
            .and_then(Value::as_bool);
        let ordered = opt.and_then(|o| o.get("ordered")).and_then(Value::as_bool);
        out.push(Entry {
            id,
            is_negative,
            input,
            expect,
            base_override,
            requires,
            context,
            spec_version,
            processing_mode,
            frame,
            expect_error_code,
            expand_context_path,
            compact_arrays,
            compact_to_relative,
            omit_graph,
            ordered,
        });
    }
    Ok(out)
}

/// The document IRI / base for a toRdf test: `option.base` if given, else the
/// suite base joined with the input path.
pub fn doc_base(e: &Entry) -> String {
    e.base_override
        .clone()
        .unwrap_or_else(|| format!("{}{}", SUITE_BASE, e.input))
}

/// [OPUS-4.8] sq-oy1f.19 — the base/document IRI for a `frame` test: the
/// `option.base` override if given, else the framing suite base joined with the
/// input path. (Distinct from `doc_base`, which uses the json-ld-api base.)
pub fn frame_doc_base(e: &Entry) -> String {
    e.base_override
        .clone()
        .unwrap_or_else(|| format!("{}{}", FRAME_SUITE_BASE, e.input))
}

/// Parse a JSON-LD document through the REAL sparq-core ingest path (oxjsonld
/// behind the `jsonld` feature) into a canonicalized dataset. Uses
/// `oxjsonld` directly here (the same parser sparq-core calls) with the test's
/// base so the comparison is base-faithful.
pub fn parse_jsonld_dataset(text: &str, base: &str) -> Result<Dataset, String> {
    // Round-trip through sparq-core to exercise its wiring, THEN canonicalize
    // via the dataset path. sparq-core's loader is the user-facing entry; we
    // assert it accepts the document, and use oxjsonld for the quad-level
    // dataset (which preserves named graphs) used in the comparison.
    let _accepted = Graph::load_str_with_base(text, "jsonld", base)
        .or_else(|_| Graph::load_dataset(text, "jsonld"))?;
    jsonld_to_canonical_dataset(text, base)
}

/// The known-gap categories: present in the W3C suite but NOT a sparq-shipped
/// gateable surface yet. Reported as not-implemented (never failed). The size
/// is read from each manifest so the scoreboard shows the real backlog and
/// shrinks honestly as categories light up.
///
/// [OPUS-4.8] sq-3uos5 — `compact` GRADUATED out of this bucket: it is now a
/// gated category (`compact::run_compact`, `COMPACT_FLOOR`). [OPUS-4.8] sq-oy1f.19 —
/// `frame` likewise GRADUATED. [OPUS-4.8] sq-oy1f — `expand` + `flatten` now
/// GRADUATED too: each is a gated category (`flatten::run_expand_or_flatten` /
/// `expand::run_expand_native`, `EXPAND_FLOOR` / `FLATTEN_FLOOR`).
pub const NOT_IMPLEMENTED_CATS: &[(&str, &str)] = &[
    ("html", "HTML script extraction not implemented"),
    (
        "remote-doc",
        "remote @context loader not wired (oxjsonld needs a LoadDocumentCallback)",
    ),
];

pub fn not_implemented_counts(root: &Path) -> BTreeMap<&'static str, usize> {
    let mut m = BTreeMap::new();
    for (cat, _why) in NOT_IMPLEMENTED_CATS {
        let n = read_manifest(root, cat).map(|e| e.len()).unwrap_or(0);
        m.insert(*cat, n);
    }
    m
}

// [FABLE-5] sq-hmd7l.15 — the `json_ld_equal` comparator unit tests moved to the
// lib-side `sparq_conformance::jsonld_bench` module together with the comparator.
