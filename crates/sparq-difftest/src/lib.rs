//! Engine-independent value normalisation for value-level SPARQL differential testing.
//!
//! This is **node A** of the value-level multi-oracle differential-testing DAG designed in
//! `research/differential-testing-value-level.md` (bead `sq-qcnn.4`, epic `sq-qcnn`). It is a
//! self-contained library for comparing SPARQL solution *values* across independent engines
//! ("oracles"), so a query that returns the right *number* of rows with a **wrong bound value**
//! (wrong number, wrong term, wrong datatype, wrong order within a tie) can no longer slip past
//! a cardinality-only cross-check.
//!
//! # The load-bearing independence constraint
//!
//! The comparison here **must not** reuse the engine-under-test's own value code (sparq's numeric
//! tower or term comparator). If both sparq's and an oracle's results were canonicalised through
//! sparq's own comparator, any bug in that comparator would apply identically to both sides and
//! **cancel** — the differential would go blind exactly where it must see. So every rule below is
//! implemented from the XSD spec on top of mature third-party exact arithmetic
//! ([`num_bigint`] / [`bigdecimal`]), with **no dependency on any sparq crate**. This is a hard
//! architectural property of the crate, enforced structurally by its dependency list.
//!
//! Honest caveat: an independent normaliser is itself new code and a bug surface of its own. It is
//! only as good as its own tests (one direct unit test per public fn, per the coverage ratchet);
//! the structural mitigation is the *second* independent oracle (a later DAG node), whose orthogonal
//! value model makes a correlated normaliser bug far less likely to hide a real divergence. This
//! crate raises the cost of a value bug surviving; it does not certify absence of bugs.
//!
//! # Two comparison regimes
//!
//! The correct equality differs by term provenance (see the design record §3), and this crate
//! exposes both so the harness-wiring layer (a later DAG node) can pick per query form:
//!
//! * **Strict RDF term equality** ([`term::term_equal_rdf`]) — a variable bound *directly* to a
//!   term in the input graph must be the **same RDF term** in every engine; RDF does not
//!   canonicalise lexical form on parse, so `"01"^^xsd:integer` stays distinct from
//!   `"1"^^xsd:integer`. The only two RDF-1.1 equalities folded in are simple-literal ≡
//!   `xsd:string` and case-insensitive language tags.
//! * **Value-canonical keying** ([`term::canonical_key`]) — for a *computed* term both engines
//!   should emit a canonical form, but they may differ in canonical *lexical* (`6` vs `6.0E0`,
//!   `1.0` vs `1`); keying by exact **value** (arbitrary-precision int/decimal, canonical double,
//!   UTC instant) makes cross-engine canonical-lexical variance — the dominant spurious-mismatch
//!   source — not a false positive, while still catching a genuinely wrong value. This trades away
//!   detection of a pure lexical-preservation difference, which the strict comparator covers.
//!
//! # What is here (and what is deliberately not)
//!
//! Implemented: the neutral term model + the two regimes ([`term`]); exact numeric value equality
//! for arbitrary-precision integer / decimal and `xsd:double`/`float` incl. `INF`/`-INF`/`NaN`
//! ([`numeric`]); `xsd:dateTime`/`date`/`duration` value comparison with timezone normalisation and
//! the ±14h indeterminate window ([`temporal`]); a SPARQL-Results-JSON reader into the neutral
//! `{var → term}` model ([`json`]); the multiset + `ORDER BY` sort-key-equivalence-class
//! comparators ([`multiset`]); the admission-decision (allow/deny set) equivalence +
//! fail-closed-asymmetry comparator ([`decision`]) shared by the trust/WAC/ODRL differential
//! tests (a structural check, not a soundness proof); and cross-oracle **blank-node isomorphism**
//! ([`iso`]) — RDFC-1.0 canonical labelling for `CONSTRUCT`/`DESCRIBE` graph results and for
//! `SELECT` results projecting blank nodes, whose labels are engine-local and therefore comparable
//! only up to a bijection.
//!
//! # Admission-decision overlap boundary
//!
//! [GPT-5.6] Admission-decision comparisons deliberately do not validate or reconcile a
//! producer's contradictory `allow ∩ deny` outcome. In particular, a reference/baseline tuple
//! present in both sets counts as already allowed for the structural
//! `candidate.allow \ baseline.allow` check, so that overlap can mask a deny-to-allow escalation.
//! Rejecting or normalising contradictory outcomes belongs to the producing policy harness and
//! is a documented non-goal of this engine-independent comparator.
//!
//! # Who calls this (and who does not yet)
//!
//! `crates/sparq-bench`'s Oxigraph query fuzzer (`src/fuzz.rs`) is wired onto these comparators as
//! of `sq-qcnn.5`: it bridges both engines' answers into this crate's neutral model (through
//! `sparq-bench`'s own `neutral` module, a structural carrier→model conversion that decides no
//! equality) and then compares by result form — [`multiset::multiset_equal`] for an un-ordered
//! `SELECT`, [`multiset::order_by_equal`] under `ORDER BY`, [`iso::solutions_isomorphic`] for a
//! blank-node-bearing solution table, and [`iso::graph_isomorphic`] for a `CONSTRUCT`/`DESCRIBE`
//! graph. An [`IsoError`] is routed to a COUNTED triage bucket there, never read as agreement.
//!
//! Deliberately **not** here (separate DAG nodes / beads): the query/data generator extension
//! (`sq-qcnn.6`) — so the fuzzer's `ASK` / `CONSTRUCT` comparators are wired and unit-tested but
//! its generator does not emit those forms yet — and the pluggable second-oracle adapter
//! (`sq-qcnn.8`).

pub mod decision;
pub mod iso;
pub mod json;
pub mod multiset;
pub mod numeric;
pub mod temporal;
pub mod term;

pub use decision::{
    decision_diff, fail_closed_asymmetry, Baseline, Candidate, DecisionDiff, DecisionOutcome,
};
pub use iso::{
    canonical_graph, canonical_solutions, graph_isomorphic, solutions_have_blank_nodes,
    solutions_isomorphic, term_has_blank_node, IsoError,
};
pub use json::{parse_results_json, QueryResults, Solution};
pub use multiset::{multiset_equal, order_by_equal};
pub use numeric::{canonical_double_string, numeric_equal, parse_numeric, NumericValue};
pub use temporal::{
    dt_compare, duration_compare, parse_datetime, parse_duration, Duration, TemporalOrder,
};
pub use term::{canonical_key, term_equal_rdf, Term};
