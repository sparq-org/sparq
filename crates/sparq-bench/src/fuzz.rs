//! Differential fuzzer: generates random small graphs (with **mixed datatypes** on
//! a numeric predicate — the case that stresses inline-integer range-pruning — and, since
//! sq-qcnn.6, **typed-literal columns**: timezoned and bare dateTimes/dates, the three
//! duration datatypes incl. the incomparable pair, booleans in both lexical forms,
//! integers beyond `i128`, and doubles incl. `INF`/`-INF`/`NaN`) and
//! random queries across every plan shape **and every result form**, then checks that
//!   sparq `query().len()`  ==  Oxigraph solution count            (full differential)
//!   sparq's ANSWER          ==  Oxigraph's ANSWER, by VALUE       (`compare_select` …)
//!   sparq `count()`         ==  sparq `query().len()`             (count-path differential)
//!
//! The VALUE-level check is what makes a same-cardinality WRONG ANSWER visible: a count
//! differential cannot see a property path landing on the wrong endpoints, a `COUNT` /
//! `MIN` / `MAX` returning the wrong number, or a `BIND` / `VALUES` / sub-select binding
//! the wrong term. It runs through **`sparq-difftest`** (bead `sq-qcnn.4`), the
//! engine-INDEPENDENT comparator library — solution-multiset equality, `ORDER BY`
//! sort-key-equivalence-class equality, RDFC-1.0 blank-node isomorphism, `ASK` booleans and
//! `CONSTRUCT` triple sets — so that no value decision is made by the engine under test.
//! See the VALUE-LEVEL CROSS-ORACLE COMPARISON section below for the per-result-form table,
//! the load-bearing independence constraint, and the one deliberate exclusion (arbitrary row
//! CHOICE under `LIMIT` / `OFFSET` without a total order), which is counted in the summary
//! line rather than silently skipped.
//!
//! Oxigraph is an independent, mature SPARQL implementation, so an agreement over
//! thousands of random cases is strong evidence the optimizations (tagged ValueIds,
//! range-pruning, count-only joins, lazy count, filter pushdown, sort-merge OPTIONAL,
//! LIMIT early-termination) preserve SPARQL semantics. A mismatch prints a full
//! deterministic repro (seed + query + the graph).
//!
//! The ADJUDICATED cross-engine divergence classes are exempt from blind Oxigraph
//! agreement — and they live machine-readably in `bench/differential-divergences.json`
//! (bead sq-0iqzw), which this comparator CONSUMES (see `DivergenceAllowlist`):
//!   * `cross-family-eq-type-error` (sq-eibog): `=` / `!=` between literals of
//!     DIFFERENT comparison families (numeric vs string, xsd:integer vs `"s2"`, …).
//!     SPARQL 1.1 §17.4.1.7 makes such a comparison a TYPE ERROR (the row is filtered
//!     out); Oxigraph leniently resolves it to a boolean and KEEPS the row. sparq
//!     follows the spec (it passes all 15 W3C `sparql10/expr-equals` tests). For that
//!     shape the oracle re-derives the SPEC-CORRECT count itself (see
//!     `spec_filter_count`) and flags only a residual, genuine divergence — not
//!     Oxigraph's leniency.
//!   * `bnode-iri-inequality` (sq-ai2wa): `=` / `!=` pairing a blank node with an
//!     IRI — identity vs type-error reading (inert for this generator; see the JSON).
//!
//! A known-class mismatch is SKIPPED-WITH-COUNT (surfaced in the summary line); any
//! mismatch outside the listed classes FAILS. A missing/empty allowlist runs STRICT.
//!
//! Usage: `sparq-bench fuzz <seed_start> <count> [category]`
//! category ∈ `CATEGORIES` (below) or `all` (default) — lets a workflow shard the
//! space across agents (`.github/workflows/differential.yml` runs one shard per
//! category).

use std::collections::BTreeSet;

use oxigraph::store::Store;
// The ENGINE-INDEPENDENT comparators (bead sq-qcnn.4). See the VALUE-LEVEL CROSS-ORACLE
// COMPARISON section below for why every value decision has to be made here rather than by
// sparq's own numeric tower / term comparator.
use sparq_difftest::{
    canonical_key, graph_isomorphic, multiset_equal, order_by_equal, solutions_have_blank_nodes,
    solutions_isomorphic, Solution,
};

use crate::neutral::{neutral_solution, neutral_triple};

// ── ADJUDICATED-DIVERGENCE ALLOWLIST (bead sq-0iqzw) ─────────────────────────────
//
// The adjudicated cross-engine divergence classes live machine-readably in
// `bench/differential-divergences.json` (id + adjudication bead + spec clause). The
// comparator CONSUMES that file: a mismatch inside a listed class is SKIPPED-WITH-
// COUNT (surfaced in the summary line), anything else FAILS. Removing an entry from
// the file re-enables the strict differential for that class, so the JSON — not this
// code — is the source of truth for WHAT is adjudicated (the code only carries the
// per-class detectors). A missing/unreadable/malformed file runs STRICT (toward
// flagging, never toward skipping). [FABLE-5]

struct DivergenceAllowlist {
    /// sq-eibog: cross-family literal `=`/`!=` type errors (Oxigraph is lenient;
    /// handled by the spec-re-derivation sub-oracle, NOT a blind skip).
    cross_family_eq_type_error: bool,
    /// sq-ai2wa: blank-node-vs-IRI `=`/`!=` (identity vs type-error reading).
    bnode_iri_inequality: bool,
    /// Where the allowlist was loaded from (for the summary line).
    source: String,
}

impl DivergenceAllowlist {
    fn strict(source: String) -> Self {
        DivergenceAllowlist {
            cross_family_eq_type_error: false,
            bnode_iri_inequality: false,
            source,
        }
    }

    /// Load from `SPARQ_FUZZ_DIVERGENCES` (a CI/agent override), else the committed
    /// repo default resolved relative to this crate's manifest (works from any cwd).
    fn load() -> Self {
        let path = std::env::var("SPARQ_FUZZ_DIVERGENCES").unwrap_or_else(|_| {
            concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../bench/differential-divergences.json"
            )
            .to_string()
        });
        match std::fs::read_to_string(&path) {
            Ok(s) => Self::from_json(&s, &path),
            Err(e) => {
                eprintln!(
                    "warning: divergence allowlist {path} unreadable ({e}) — running \
                     STRICT (every mismatch fails)"
                );
                Self::strict(path)
            }
        }
    }

    /// Parse the allowlist JSON. An unknown class id is IGNORED with a loud warning
    /// (fail-STRICT: the comparator has no detector for it, so that class keeps
    /// failing rather than being silently "absorbed" by nothing); malformed JSON is
    /// also strict.
    fn from_json(s: &str, source: &str) -> Self {
        let mut out = Self::strict(source.to_string());
        let v: serde_json::Value = match serde_json::from_str(s) {
            Ok(v) => v,
            Err(e) => {
                eprintln!(
                    "warning: divergence allowlist {source} is not valid JSON ({e}) — \
                     running STRICT"
                );
                return out;
            }
        };
        for class in v["classes"].as_array().map(Vec::as_slice).unwrap_or(&[]) {
            match class["id"].as_str() {
                Some("cross-family-eq-type-error") => out.cross_family_eq_type_error = true,
                Some("bnode-iri-inequality") => out.bnode_iri_inequality = true,
                // `update-*` is the UPDATE differential's namespace in this shared
                // registry: update_fuzz.rs owns those detectors and pins them with its
                // own committed-allowlist test. Not unknown — just not this
                // comparator's, so no warning and (correctly) no effect here.
                Some(other) if other.starts_with("update-") => {}
                Some(other) => eprintln!(
                    "warning: divergence allowlist {source} lists unknown class id \
                     {other:?} — no detector for it; that class stays STRICT"
                ),
                None => eprintln!(
                    "warning: divergence allowlist {source} has a class without a \
                     string `id` — ignored (strict)"
                ),
            }
        }
        out
    }
}

/// Deterministic SplitMix64 — no clock/entropy, so every case is reproducible from
/// its seed (`Date::now`/`rand` are unavailable in the wasm sibling crate too).
struct Rng(u64);
impl Rng {
    fn new(seed: u64) -> Self {
        Rng(seed.wrapping_mul(0x9e37_79b9_7f4a_7c15).wrapping_add(1))
    }
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n.max(1)
    }
    fn chance(&mut self, num: u64, den: u64) -> bool {
        self.below(den) < num
    }
}

/// A random graph. `ex:age` is ALWAYS a canonical non-negative `xsd:integer`
/// (an all-inline column → range-pruning active). `ex:val` is deliberately MIXED
/// (xsd:int / xsd:decimal / negative / non-canonical integer / string) → the
/// range-pruning guard must fall back to a full scan and still be correct.
///
/// [SONNET-4.6] sq-qcnn.6 — the TYPED-LITERAL columns (`ex:when` / `ex:day` / `ex:dur` /
/// `ex:flag` / `ex:huge` / `ex:dbl`) extend the data generator past the numeric-and-string
/// space the columns above cover, into the datatype families where the two engines'
/// STORES legitimately disagree about the lexical they hand back. That disagreement is the
/// point: Oxigraph RE-CANONICALISES a literal when it loads a graph — measured on this
/// generator's own output, `"1"^^xsd:boolean` comes back as `"true"` and
/// `"1.5E3"^^xsd:double` as `"1500"` — while sparq stores the term exactly as written, so
/// a projection of one of these columns only compares EQUAL if `sparq-difftest` really is
/// canonicalising by VALUE (`canonical_lexical`: exact numeric expansion, UTC-normalised
/// instant, canonical duration, `true`/`false` — the temporal and duration families go
/// through the same rule even where the two stores happen to agree lexically today).
/// Every family below is therefore an end-to-end check on the
/// engine-independent comparator as much as on the engine, and each is pinned by
/// `typed_literal_columns_cover_every_datatype_family` /
/// `typed_columns_agree_by_value_despite_lexical_recanonicalisation`.
///
/// The families deliberately include the pairs an `f64`- or `i128`-shaped oracle would
/// wrongly equate or drop — two integers BEYOND `i128` differing only in their last digit,
/// `INF` / `-INF` / `NaN`, and (from `ex:val`) 18-significant-digit decimals — and the
/// INCOMPARABLE duration pair (`xsd:yearMonthDuration` vs `xsd:dayTimeDuration`), whose
/// value spaces have no common order. See `gen_query`'s invariants for why the duration
/// column is projected but never ORDERED or compared against a constant.
fn gen_graph(rng: &mut Rng) -> String {
    let n = 3 + rng.below(14) as usize; // 3..=16 subjects
    let mut s = String::from(
        "@prefix ex: <http://ex/> .\n@prefix xsd: <http://www.w3.org/2001/XMLSchema#> .\n",
    );
    for i in 0..n {
        let subj = format!("ex:n{i}");
        if rng.chance(3, 4) {
            s.push_str(&format!("{subj} a ex:T .\n"));
        }
        // ex:age — canonical non-negative integer in a small range so FILTER
        // thresholds land on real boundaries. All-inline column.
        if rng.chance(7, 8) {
            let age = rng.below(120);
            s.push_str(&format!("{subj} ex:age {age} .\n"));
        }
        // ex:val — MIXED datatype column.
        if rng.chance(3, 4) {
            // High-precision decimals (>15 sig. digits) where two distinct values can
            // share an f64 — the decimal analogue of the >2^53 integer case.
            let hp_dec = [
                "0.123456789012345678",
                "0.123456789012345679",
                "1.000000000000000001",
                "0.299999999999999999",
            ];
            let v = match rng.below(9) {
                0 => format!("{}", rng.below(120)), // canonical integer (inline)
                1 => format!("\"{}\"^^xsd:int", rng.below(120)), // xsd:int (not inline)
                2 => format!("\"{}.5\"^^xsd:decimal", rng.below(120)), // decimal
                3 => format!("\"-{}\"^^xsd:integer", rng.below(60)), // negative integer (not inline)
                4 => format!("\"0{}\"^^xsd:integer", 1 + rng.below(9)), // non-canonical (leading zero)
                5 => format!("\"{}.0\"^^xsd:double", rng.below(120)),   // double
                6 => format!("\"{}\"^^xsd:integer", 9007199254740990u64 + rng.below(6)), // near 2^53 — f64-inexact
                7 => format!("\"{}\"^^xsd:decimal", hp_dec[rng.below(4) as usize]), // high-precision decimal
                _ => format!("\"s{}\"", rng.below(5)), // plain string (non-numeric)
            };
            s.push_str(&format!("{subj} ex:val {v} .\n"));
        }
        // ex:name — string, sometimes language-tagged.
        if rng.chance(3, 4) {
            if rng.chance(1, 3) {
                s.push_str(&format!("{subj} ex:name \"nm{}\"@en .\n", rng.below(6)));
            } else {
                s.push_str(&format!("{subj} ex:name \"nm{}\" .\n", rng.below(6)));
            }
        }
        // ── sq-qcnn.6 TYPED-LITERAL columns ──────────────────────────────────
        // ex:when — `xsd:dateTime`, ALWAYS timezoned (`Z` / `±hh:mm`). Keeping the
        // whole column timezoned is what makes it safe to COMPARE against a
        // timezoned constant: a comparison mixing a timezoned and an untimezoned
        // dateTime resolves through the IMPLICIT timezone of XPath's dynamic context,
        // which is not fixed by the query.
        if rng.chance(1, 2) {
            let tz = ["Z", "+05:30", "-08:00", "+00:00"][rng.below(4) as usize];
            let (y, mo, d) = (2000 + rng.below(40), 1 + rng.below(12), 1 + rng.below(28));
            let (h, mi) = (rng.below(24), rng.below(60));
            s.push_str(&format!(
                "{subj} ex:when \"{y:04}-{mo:02}-{d:02}T{h:02}:{mi:02}:00{tz}\"^^xsd:dateTime .\n"
            ));
        }
        // ex:day — the DATE-granularity and UNTIMEZONED temporals: `xsd:date` with and
        // without a timezone, plus a BARE `xsd:dateTime`. Projected only (see above).
        if rng.chance(1, 3) {
            let tz = ["Z", "+05:30", "-08:00"][rng.below(3) as usize];
            let (y, mo, d) = (2000 + rng.below(40), 1 + rng.below(12), 1 + rng.below(28));
            let (h, mi) = (rng.below(24), rng.below(60));
            let v = match rng.below(3) {
                0 => format!("\"{y:04}-{mo:02}-{d:02}\"^^xsd:date"), // bare date
                1 => format!("\"{y:04}-{mo:02}-{d:02}{tz}\"^^xsd:date"), // timezoned date
                _ => format!("\"{y:04}-{mo:02}-{d:02}T{h:02}:{mi:02}:00\"^^xsd:dateTime"),
            };
            s.push_str(&format!("{subj} ex:day {v} .\n"));
        }
        // ex:dur — the three duration datatypes, INCLUDING the incomparable pair:
        // `xsd:yearMonthDuration` and `xsd:dayTimeDuration` partition `xsd:duration`'s
        // value space into two axes with no common order (XSD 1.1 §3.4.26–28).
        if rng.chance(1, 3) {
            let v = match rng.below(6) {
                0 => format!(
                    "\"P{}Y{}M\"^^xsd:yearMonthDuration",
                    1 + rng.below(5),
                    1 + rng.below(11)
                ),
                1 => format!("\"P{}M\"^^xsd:yearMonthDuration", 1 + rng.below(30)),
                2 => format!(
                    "\"P{}DT{}H\"^^xsd:dayTimeDuration",
                    1 + rng.below(10),
                    1 + rng.below(23)
                ),
                3 => format!("\"PT{}M\"^^xsd:dayTimeDuration", 1 + rng.below(120)),
                4 => format!(
                    "\"P{}Y{}M{}DT{}H\"^^xsd:duration",
                    1 + rng.below(3),
                    1 + rng.below(11),
                    1 + rng.below(10),
                    1 + rng.below(23)
                ),
                _ => format!("\"-P{}D\"^^xsd:dayTimeDuration", 1 + rng.below(9)),
            };
            s.push_str(&format!("{subj} ex:dur {v} .\n"));
        }
        // ex:flag — `xsd:boolean` in BOTH lexical forms. `"1"`/`"0"` are the
        // non-canonical ones Oxigraph rewrites to `true`/`false` on load, so this
        // column is what pins the comparator's boolean value-canonicalisation.
        if rng.chance(1, 2) {
            let v = ["true", "false", "\"1\"^^xsd:boolean", "\"0\"^^xsd:boolean"]
                [rng.below(4) as usize];
            s.push_str(&format!("{subj} ex:flag {v} .\n"));
        }
        // ex:huge — `xsd:integer` values BEYOND `i128`, including a pair differing only
        // in their last digit: any oracle that narrows an integer to a machine word (or
        // to an `f64`) either drops these rows or equates the pair.
        if rng.chance(1, 3) {
            let v = [
                "170141183460469231731687303715884105728", // 2^127 — one past i128::MAX
                "170141183460469231731687303715884105729", // …+1, differs in the last digit
                "-170141183460469231731687303715884105729", // one past i128::MIN
                "99999999999999999999999999999999999999999", // 41 digits
            ][rng.below(4) as usize];
            s.push_str(&format!("{subj} ex:huge \"{v}\"^^xsd:integer .\n"));
        }
        // ex:dbl — `xsd:double` including the three XSD specials (`INF` / `-INF` /
        // `NaN`), signed zero, and an `xsd:float` (a DIFFERENT datatype the comparator
        // must not silently merge with double).
        if rng.chance(1, 2) {
            let v = match rng.below(8) {
                0 => "\"INF\"^^xsd:double".to_string(),
                1 => "\"-INF\"^^xsd:double".to_string(),
                2 => "\"NaN\"^^xsd:double".to_string(),
                3 => format!("\"{}.5E2\"^^xsd:double", rng.below(100)),
                4 => "\"0.0E0\"^^xsd:double".to_string(),
                5 => "\"-0.0E0\"^^xsd:double".to_string(),
                6 => format!("\"{}E-3\"^^xsd:double", 1 + rng.below(999)),
                _ => format!("\"{}\"^^xsd:float", rng.below(100)),
            };
            s.push_str(&format!("{subj} ex:dbl {v} .\n"));
        }
        // ex:p — edges (chain/star/cycle fodder).
        let edges = rng.below(4);
        for _ in 0..edges {
            s.push_str(&format!("{subj} ex:p ex:n{} .\n", rng.below(n as u64)));
        }
    }
    s
}

/// A numeric comparison operator and a threshold (chosen near the data range so the
/// boundary cases of range-pruning are exercised).
fn gen_filter(rng: &mut Rng, var: &str) -> String {
    let op = ["<", "<=", ">", ">=", "=", "!="][rng.below(6) as usize];
    // Signed threshold straddling the data range AND zero — negative thresholds are
    // non-sargable (parsed as UnaryMinus), forcing the residual compare path that a
    // non-numeric operand must turn into a type error, not a string comparison.
    // 1-in-6: a threshold near 2^53, where an f64 numeric model loses integer precision.
    if rng.chance(1, 6) {
        let t = 9007199254740990u64 + rng.below(6);
        return format!("FILTER({var} {op} {t})");
    }
    let t = rng.below(160) as i64 - 40; // -40..119
    format!("FILTER({var} {op} {t})")
}

/// Every query category the generator can emit. `all` picks one uniformly at
/// random per seed; `.github/workflows/differential.yml` runs ONE NIGHTLY SHARD PER
/// ENTRY (running only `all` hides category-dense bugs — see that workflow's header),
/// so adding a category here means adding a shard there too. `categories_have_a_shard`
/// (below) pins the two lists together.
const CATEGORIES: &[&str] = &[
    "bgp", "filter", "equality", "optional", "union", "minus", "limit", "distinct", "order",
    // sq-j80vk: the SPARQL 1.1 surfaces that previously had ZERO standing
    // wrong-answer differential vs the Oxigraph oracle.
    "path", "aggregate", "subquery", "exists", "values", "bind", "graph",
    // sq-avd75: the `OPTIONAL … FILTER(!bound(?v))` negation idiom — the TRIGGER
    // shape of rewrite (b) of the `algebra-rewrite` pass (#1735), which this crate
    // builds ON (see Cargo.toml). Without it the pass's anti-join half had no
    // standing oracle at all.
    "antijoin",
    // sq-qcnn.6: the TYPED-LITERAL surfaces (`gen_graph`'s dateTime/date/duration/
    // boolean/beyond-i128-integer/double-with-specials columns), the SPARQL 1.1 string
    // functions (§17.4.3), and the two non-`SELECT` RESULT FORMS — which `compare_ask` /
    // `compare_graph` were wired for by sq-qcnn.5 but which nothing generated.
    "typed", "strfn", "ask", "construct",
];

/// A random query in the chosen category. **Every category must keep these
/// invariants** (sq-j80vk) or the comparator's oracles mis-fire:
///
/// * **Always-valid SPARQL 1.1**, restricted to the surface sparq supports —
///   Oxigraph must parse it too. (A surface sparq declines is skipped as
///   `unsupported`, which is fair; a WRONG ANSWER is not.)
/// * **Never a `LIMIT` inside a sub-select that is then joined**, and never a top-level
///   `LIMIT`/`OFFSET` whose `ORDER BY` is absent *or* non-total unless the shape is
///   deliberately cardinality-only: SPARQL leaves the row CHOICE arbitrary there, so two
///   conformant engines may legitimately return different (equally valid) rows.
///   `compare_select` routes such a query to the COUNTED `skipped(row-choice)` bucket, so a
///   new shape that needs a window either carries a total tiebreaker (and stays
///   value-checked, as the `limit` category's second shape does) or knowingly gives up its
///   value-level oracle.
/// * **No `=` / `!=` outside the `equality` / `filter` categories.** Those exact
///   texts are what `parse_eq_filter` recognises as the adjudicated sq-eibog shape;
///   a look-alike elsewhere would be re-derived by a sub-oracle that does not model
///   the surrounding query.
/// * **Every `ORDER BY` must be MODELLED by `order_by_vars`** — a list of plain variables,
///   optionally `ASC(…)`/`DESC(…)`-wrapped, all of them projected. An unmodelled clause
///   silently downgrades the case to the order-INSENSITIVE multiset comparison, losing that
///   shape's ORDER oracle with no counter to show it. (`ORDER BY` is no longer confined to
///   the `order` category: the comparator derives the sort-key equivalence classes from the
///   clause itself.)
/// * **No blank nodes.** Keeping them out of the generator is exactly what keeps the
///   sq-ai2wa (bnode-vs-IRI) allowlist class INERT; introducing one makes that class
///   live (its detector is already wired) and needs its own adjudication first.
/// * **Every query's RESULT FORM must be the one `expected_form` records for its
///   category** (sq-qcnn.6). `run` dispatches on `query_form` BEFORE the cardinality
///   flow, so a category that silently changed form would change which oracle judges it.
///
/// # sq-qcnn.6: what the typed-literal columns are and are NOT queried with
///
/// The typed columns exist to be COMPARED BY VALUE, and the shapes that use them are
/// chosen so that the answer is a function of the query — never of a semantic choice
/// SPARQL leaves open. Three exclusions are deliberate, each MEASURED against Oxigraph
/// rather than assumed:
///
/// * **Durations are projected, never compared or ordered.** `<` between an
///   `xsd:yearMonthDuration` and an `xsd:dayTimeDuration` has no defined answer (their
///   value spaces share no order, and SPARQL's operator table §17.3 maps no duration
///   comparison at all). The two engines resolve it differently — sparq drops the
///   incomparable rows as type errors, Oxigraph keeps them — so generating the
///   comparison would manufacture a nightly failure for an UNADJUDICATED divergence
///   rather than find a bug. Projection still exercises the whole store → materialise →
///   canonicalise path, which is what this column is for.
/// * **`MIN`/`MAX` never run over `ex:dbl`.** `NaN`'s position in the ordering is not
///   fixed (sparq's `MAX` skips it, Oxigraph's returns it), so an extremum over a column
///   containing `NaN` is not a function of the data.
/// * **No `DISTINCT` / `COUNT(DISTINCT …)` over a typed column.** Oxigraph
///   RE-CANONICALISES literals on load, so `"1"^^xsd:boolean` and `true` are ONE term in
///   its store and TWO in sparq's — a cardinality difference that belongs to the
///   harness's storage layers, not to either engine's DISTINCT. (The same is already
///   true of `ex:val`'s non-canonical integers; the `distinct` category reads the
///   all-canonical `ex:age` column instead.)
///
/// `GROUP_CONCAT` carries the same care in the `aggregate` category: it is emitted only
/// where the group is a SINGLETON by construction, or under `STRLEN`, because the ORDER
/// of a group's members — and so the concatenation — is left unspecified (§11 aggregates).
/// Its argument is always `STR(…)`-wrapped: `fn:concat` takes strings, and the engines
/// disagree about coercing a non-string (sparq coerces, Oxigraph errors the aggregate to
/// UNBOUND), which is a divergence about the argument, not about the aggregate.
fn gen_query(rng: &mut Rng, category: &str) -> String {
    let pfx = "PREFIX ex: <http://ex/>\nPREFIX xsd: <http://www.w3.org/2001/XMLSchema#>\n";
    // Pick an effective category (when "all", choose one at random).
    let cat = if category == "all" {
        CATEGORIES[rng.below(CATEGORIES.len() as u64) as usize]
    } else {
        category
    };

    let body = match cat {
        "bgp" => match rng.below(5) {
            0 => "?s ex:age ?a".to_string(),
            1 => "?s ex:p ?o . ?o ex:age ?a".to_string(), // chain join
            2 => "?s ex:age ?a . ?s ex:name ?n".to_string(), // star join
            3 => "?s ex:p ?o . ?o ex:p ?t . ?t ex:p ?s".to_string(), // triangle (WCOJ)
            _ => "?s ex:p ?o . ?o ex:name ?n . ?s ex:age ?a".to_string(),
        },
        "filter" => {
            let var = if rng.chance(1, 2) { "?a" } else { "?v" };
            let pat = if var == "?a" {
                "?s ex:age ?a"
            } else {
                "?s ex:val ?v"
            };
            let extra = if rng.chance(1, 2) {
                " . ?s ex:name ?n"
            } else {
                ""
            };
            format!("{pat}{extra} {}", gen_filter(rng, var))
        }
        "optional" => match rng.below(3) {
            0 => "?s ex:name ?n OPTIONAL { ?s ex:age ?a }".to_string(),
            1 => "?s ex:age ?a OPTIONAL { ?s ex:p ?o }".to_string(),
            _ => "?s ex:name ?n OPTIONAL { ?s ex:age ?a . FILTER(?a > 50) }".to_string(),
        },
        "equality" => {
            // `=` / `!=` against a constant of a possibly-different type, over the
            // MIXED column — exercises RDFterm-equal (known-different vs type error).
            let op = if rng.chance(1, 2) { "=" } else { "!=" };
            let rhs = match rng.below(8) {
                0 => format!("{}", rng.below(120)),              // integer
                1 => format!("\"s{}\"", rng.below(5)),           // plain string
                2 => "ex:n0".to_string(),                        // IRI
                3 => "\"true\"^^xsd:boolean".to_string(),        // boolean
                4 => format!("\"{}\"^^xsd:int", rng.below(120)), // xsd:int
                5 => format!("\"{}\"^^xsd:integer", 9007199254740990u64 + rng.below(6)), // near 2^53
                6 => format!(
                    "\"{}\"^^xsd:decimal",
                    [
                        "0.123456789012345678",
                        "0.123456789012345679",
                        "0.299999999999999999"
                    ][rng.below(3) as usize]
                ), // high-precision decimal
                _ => format!("\"{}.5\"^^xsd:decimal", rng.below(120)),                   // decimal
            };
            let var = if rng.chance(1, 2) {
                ("?v", "ex:val")
            } else {
                ("?a", "ex:age")
            };
            return format!(
                "{pfx}SELECT ?s WHERE {{ ?s {} {} FILTER({} {op} {rhs}) }}",
                var.1, var.0, var.0
            );
        }
        "union" => "{ ?s ex:age ?a } UNION { ?s ex:name ?a }".to_string(),
        "minus" => "?s ex:name ?n MINUS { ?s ex:age ?a }".to_string(),
        // ANTI-JOIN (sq-avd75): the `OPTIONAL { B } FILTER(!bound(?v))` negation
        // idiom. This is the TRIGGER shape of rewrite (b) of the `algebra-rewrite`
        // pass (#1735) — `Filter(!bound(?v), LeftJoin(A, B))` → `Minus(A, B)` — which
        // sparq-bench compiles ON, so a TRIGGER shape below is answered by sparq
        // through the REWRITTEN algebra while Oxigraph answers it through the plain
        // OPTIONAL+FILTER. That makes the pass's result-equivalence claim an oracle
        // check on random data rather than only a fixed-fixture assertion
        // (`sparq-engine/tests/rewrite_pass.rs`, which runs on the feature-matrix leg).
        //
        // The `minus` category is deliberately NOT this shape: it generates an
        // EXPLICIT `MINUS`, i.e. the rewrite's OUTPUT operator, and so can never
        // exercise the recogniser.
        //
        // Shapes 0..=2 MEET the rewrite's firing conditions (`?v` certain in B, absent
        // from A, A and B sharing a certain variable, and — since `rewrite_filter`
        // matches `LeftJoin { expression: None }` — an UNFILTERED `OPTIONAL`). Shapes 3
        // and 4 are the two DECLINE shapes: the pass must return the algebra verbatim
        // and the un-rewritten `LeftJoin` path stays oracle-checked. Keeping both sides
        // in ONE category is deliberate — a recogniser that got LOOSER would show up
        // here as a wrong answer, which is the failure the conservatism guards against.
        // `antijoin_category_is_the_rewrite_b_trigger` pins which shape is which.
        "antijoin" => match rng.below(5) {
            // Plain anti-join on the shared subject: named subjects with no age.
            0 => "?s ex:name ?n OPTIONAL { ?s ex:age ?a } FILTER(!bound(?a))".to_string(),
            // Same, over the edge column: named subjects with no outgoing `ex:p`.
            1 => "?s ex:name ?n OPTIONAL { ?s ex:p ?o } FILTER(!bound(?o))".to_string(),
            // Anti-join whose A side is itself a join: `ex:p` targets with no name.
            2 => "?s ex:p ?o OPTIONAL { ?o ex:name ?n } FILTER(!bound(?n))".to_string(),
            // DECLINE (theta): the OPTIONAL carries its own FILTER, which the parser
            // lifts into `LeftJoin { expression: Some(…) }`. `Minus(A, B)` is NOT
            // equivalent to that (the negation is "no age ABOVE the threshold", so the
            // condition has to move into B), and the pass matches `expression: None`
            // only — so it declines. Answering it correctly through the un-rewritten
            // path is exactly what the oracle checks here.
            3 => format!(
                "?s ex:name ?n OPTIONAL {{ ?s ex:age ?a . FILTER(?a > {}) }} FILTER(!bound(?a))",
                rng.below(120)
            ),
            // DECLINE (no shared variable): `ex:absent` is never a subject in
            // `gen_graph` (which emits only `ex:n0..ex:n15`), so B is empty AND shares
            // no variable with A — every A row survives, and a pass that fired here
            // would be WRONG, not merely unhelpful.
            _ => "?s ex:age ?a OPTIONAL { ex:absent ex:name ?n } FILTER(!bound(?n))".to_string(),
        },
        // ── sq-j80vk categories ──────────────────────────────────────────────────
        // PROPERTY PATHS (SPARQL 1.1 §9). `ex:p` is the edge predicate, so the graph
        // is a small random digraph with chains and cycles — `+`/`*` closure, inverse,
        // sequence and the negated property set all have non-trivial answers on it.
        //
        // ZERO-LENGTH forms (`*`, `?`) are emitted ONLY with their start end already
        // bound to a term that OCCURS IN THE GRAPH (`?s ex:p ?x . ?x ex:p* ?o`). The
        // zero-length match of a ground endpoint that does NOT occur in the data is a
        // REAL, currently-unadjudicated cross-engine divergence (sparq yields the
        // zero-length solution — which is what the W3C property-path tests expect,
        // cf. sparq-conformance/FINDINGS.md F11 — while Oxigraph 0.5 returns nothing);
        // absorbing it here would mean widening the allowlist without an adjudication,
        // which sq-j80vk explicitly forbids. It is filed as its own follow-up instead.
        "path" => match rng.below(9) {
            0 => "?s ex:p/ex:p ?o".to_string(),      // sequence
            1 => "?s ^ex:p ?o".to_string(),          // inverse
            2 => "?s ex:p+ ?o".to_string(),          // transitive closure
            3 => "?s (ex:p|ex:name) ?o".to_string(), // alternative
            4 => "?s ex:p/ex:age ?a".to_string(),    // sequence into a value column
            5 => "?s ex:p+/ex:name ?n".to_string(),  // closure then a value column
            6 => "?s !(ex:p|ex:age) ?o".to_string(), // negated property set
            // zero-or-more / zero-or-one from a start that is certainly a graph node.
            7 => "?s ex:p ?x . ?x ex:p* ?o".to_string(),
            _ => "?s ex:p ?x . ?x ex:p? ?o".to_string(),
        },
        // AGGREGATES / GROUP BY / HAVING (§11). The aggregate's VALUE is compared
        // directly — `compare_select` matches the full solution multiset, so a bare
        // `COUNT(*)`'s single row is checked for the right NUMBER, not merely for being
        // one row. `HAVING` additionally makes the value visible to the CARDINALITY
        // differential (the number of surviving groups is a function of the aggregated
        // values), which keeps the two oracles independent. Numeric aggregates stay on
        // `ex:age` (an all-`xsd:integer` column) — SUM/AVG over the deliberately MIXED
        // `ex:val` column is a type error whose propagation is not what this category is
        // pinning.
        //
        // `AVG` is exercised in `HAVING` but never PROJECTED: XPath leaves the precision
        // of decimal division implementation-defined (`op:numeric-divide`, ≥18 digits),
        // and sparq rounds where Oxigraph truncates, so a projected `AVG` would differ
        // in its last digit between two CONFORMANT engines. Against an integer HAVING
        // threshold that last digit can never flip the comparison, so the value stays
        // oracle-checked without manufacturing a false mismatch.
        "aggregate" => {
            let k = rng.below(4);
            let t = rng.below(200);
            return match rng.below(9) {
                0 => format!("{pfx}SELECT (COUNT(*) AS ?c) WHERE {{ ?s ex:age ?a }}"),
                1 => format!("{pfx}SELECT ?s (COUNT(?o) AS ?c) WHERE {{ ?s ex:p ?o }} GROUP BY ?s"),
                2 => format!(
                    "{pfx}SELECT ?s (COUNT(?o) AS ?c) WHERE {{ ?s ex:p ?o }} GROUP BY ?s \
                     HAVING(COUNT(?o) > {k})"
                ),
                3 => format!(
                    "{pfx}SELECT ?s (SUM(?a) AS ?t) WHERE {{ ?s ex:age ?a }} GROUP BY ?s \
                     HAVING(SUM(?a) > {t})"
                ),
                4 => format!(
                    "{pfx}SELECT ?n (COUNT(DISTINCT ?s) AS ?c) WHERE {{ ?s ex:name ?n }} \
                     GROUP BY ?n HAVING(COUNT(DISTINCT ?s) > {k})"
                ),
                5 => format!(
                    "{pfx}SELECT (MIN(?a) AS ?mn) (MAX(?a) AS ?mx) WHERE {{ ?s ex:age ?a }} \
                     HAVING(MAX(?a) > {t})"
                ),
                6 => format!(
                    "{pfx}SELECT ?s (SUM(?a) AS ?sm) WHERE {{ ?s ex:p ?o . ?o ex:age ?a }} \
                     GROUP BY ?s HAVING(AVG(?a) > {t})"
                ),
                // sq-qcnn.6 — GROUP_CONCAT, in two shapes whose value is a function of
                // the query (see the header note): a SINGLETON group (a
                // subject carries at most one `ex:age`, so the concatenation has one
                // member and no order to be arbitrary about), and a multi-member group
                // read through `STRLEN`, which is invariant under the members' order.
                7 => format!(
                    "{pfx}SELECT ?s (GROUP_CONCAT(STR(?a)) AS ?g) WHERE {{ ?s ex:age ?a }} \
                     GROUP BY ?s"
                ),
                _ => format!(
                    "{pfx}SELECT ?s (STRLEN(GROUP_CONCAT(STR(?o))) AS ?l) \
                     WHERE {{ ?s ex:p ?o }} GROUP BY ?s \
                     HAVING(STRLEN(GROUP_CONCAT(STR(?o))) > {})",
                    10 + rng.below(30)
                ),
            };
        }
        // SUB-SELECTS (§12). No `LIMIT` inside a joined sub-select (see the
        // determinism invariant above). The aggregating shapes push the aggregate's
        // VALUE into the outer row count via an outer `FILTER`, the same trick
        // `HAVING` plays for the `aggregate` category.
        "subquery" => {
            let k = rng.below(4);
            return match rng.below(5) {
                0 => format!(
                    "{pfx}SELECT * WHERE {{ ?s ex:name ?n {{ SELECT ?s WHERE {{ ?s ex:age ?a }} }} }}"
                ),
                1 => format!(
                    "{pfx}SELECT * WHERE {{ {{ SELECT DISTINCT ?s WHERE {{ ?s ex:p ?o }} }} \
                     ?s ex:age ?a }}"
                ),
                2 => format!(
                    "{pfx}SELECT * WHERE {{ {{ SELECT ?s (COUNT(?o) AS ?c) WHERE {{ ?s ex:p ?o }} \
                     GROUP BY ?s }} FILTER(?c > {k}) }}"
                ),
                3 => format!(
                    "{pfx}SELECT * WHERE {{ ?s ex:age ?a \
                     OPTIONAL {{ {{ SELECT ?s ?n WHERE {{ ?s ex:name ?n }} }} }} }}"
                ),
                _ => format!(
                    "{pfx}SELECT * WHERE {{ ?s ex:name ?n \
                     {{ SELECT ?s WHERE {{ ?s ex:age ?a . FILTER(?a > {}) }} }} }}",
                    rng.below(120)
                ),
            };
        }
        // EXISTS / NOT EXISTS (§8.1.1, §17.4.1.4) — correlated, so the inner pattern
        // is evaluated with the outer row substituted in.
        "exists" => match rng.below(6) {
            0 => "?s ex:age ?a FILTER EXISTS { ?s ex:name ?n }".to_string(),
            1 => "?s ex:age ?a FILTER NOT EXISTS { ?s ex:p ?o }".to_string(),
            2 => "?s ex:name ?n FILTER NOT EXISTS { ?s ex:age ?a . FILTER(?a > 50) }".to_string(),
            3 => "?s ex:p ?o FILTER EXISTS { ?o ex:age ?a }".to_string(),
            4 => format!(
                "?s ex:age ?a FILTER(EXISTS {{ ?s ex:name ?n }} && ?a < {})",
                rng.below(120)
            ),
            _ => "?s ex:name ?n FILTER NOT EXISTS { ?s ex:p ?o . ?o ex:age ?a }".to_string(),
        },
        // VALUES / inline data (§10.2.1), incl. the multi-column and UNDEF forms.
        "values" => {
            let (a, b) = (rng.below(120), rng.below(120));
            match rng.below(4) {
                0 => "?s ex:age ?a VALUES ?s { ex:n0 ex:n1 ex:n2 }".to_string(),
                1 => format!("VALUES ?a {{ {a} {b} }} ?s ex:age ?a"),
                2 => format!("?s ex:age ?a VALUES (?s ?a) {{ (ex:n0 {a}) (ex:n1 UNDEF) }}"),
                _ => "?s ex:name ?n VALUES ?n { \"nm0\" \"nm1\"@en }".to_string(),
            }
        }
        // BIND (§10.2.2). A bare BIND cannot change the row count, so every shape
        // FILTERs on the bound variable — that makes the computed VALUE observable in
        // the count differential. `COALESCE` over an OPTIONAL covers the UNBOUND path.
        "bind" => {
            let t = rng.below(160) as i64 - 40;
            match rng.below(5) {
                0 => format!("?s ex:age ?a BIND(?a + 1 AS ?b) FILTER(?b > {t})"),
                1 => format!("?s ex:age ?a BIND(?a * 2 AS ?b) FILTER(?b < {t})"),
                2 => format!(
                    "?s ex:name ?n OPTIONAL {{ ?s ex:age ?a }} BIND(COALESCE(?a, 0) AS ?b) \
                     FILTER(?b > {t})"
                ),
                3 => format!("?s ex:age ?a BIND(IF(?a > {t}, 1, 0) AS ?b) FILTER(?b > 0)"),
                _ => format!(
                    "?s ex:age ?a . ?s ex:name ?n BIND(STRLEN(STR(?n)) AS ?l) FILTER(?l > {})",
                    rng.below(5)
                ),
            }
        }
        // GRAPH (§13.3). The harness loads ONE Turtle document, so the dataset has a
        // default graph and NO named graphs — every `GRAPH` block is therefore empty,
        // and what these shapes pin is that sparq agrees: a `GRAPH` block must NOT
        // leak the default graph's triples, and an empty named-graph side must still
        // COMPOSE correctly (UNION keeps the left rows; OPTIONAL keeps them with `?g`
        // / `?n` unbound). Named-graph DATA is out of scope here — the store-mode
        // shards (dict-spill) re-serialise through triples only, so quads in the
        // generated document would diverge for harness reasons, not engine ones.
        "graph" => match rng.below(4) {
            0 => "GRAPH ?g { ?s ex:age ?a }".to_string(),
            1 => "{ ?s ex:age ?a } UNION { GRAPH ?g { ?s ex:age ?a } }".to_string(),
            2 => "?s ex:age ?a OPTIONAL { GRAPH ?g { ?s ex:name ?n } }".to_string(),
            _ => "GRAPH ex:g1 { ?s ex:p ?o }".to_string(),
        },
        // LIMIT / OFFSET (§15.5). BOTH policies of §2.2 of
        // `research/differential-testing-value-level.md` are generated, because they cover
        // different ground (sq-qcnn.5):
        //
        //   * The BARE window (no `ORDER BY`) is the pure early-termination plan this category
        //     exists to exercise. WHICH rows survive it is implementation-defined, so only the
        //     CARDINALITY is a function of the query: the value-level oracle correctly declines
        //     it and counts it as `skipped(row-choice)`.
        //   * The TOTAL-TIEBREAKER window appends every projected variable to `ORDER BY`. With
        //     the order total, two rows in one sort-key-equivalence class are the SAME row, so
        //     which of them the window keeps cannot change the answer — the surviving rows ARE
        //     determined and `compare_select` value-checks the window itself rather than merely
        //     counting it.
        //
        // Emitting only the bare form would leave every `LIMIT` shape value-unchecked forever;
        // emitting only the tiebreaker form would replace the early-termination plan with a
        // top-k sort and lose the plan this category is named for.
        "limit" => {
            let k = rng.below(8);
            let off = rng.below(5);
            if rng.chance(1, 2) {
                return format!(
                    "{pfx}SELECT * WHERE {{ ?s ex:p ?o . ?o ex:age ?a }} \
                     ORDER BY ?s ?o ?a LIMIT {k} OFFSET {off}"
                );
            }
            return format!(
                "{pfx}SELECT * WHERE {{ ?s ex:p ?o . ?o ex:age ?a }} LIMIT {k} OFFSET {off}"
            );
        }
        "distinct" => {
            return format!("{pfx}SELECT DISTINCT ?a WHERE {{ ?s ex:age ?a }}");
        }
        // ORDER BY (§15.1). Two shapes, for the two regimes `order_by_equal` distinguishes:
        //
        //   * A TOTAL sort key (`?a` is the only projected variable), so every equivalence class
        //     is a single distinct row and the comparison is exact SEQUENCE equality — and the
        //     trailing `LIMIT` window is determined.
        //   * A sort key over a STRICT SUBSET of the projected variables. `ORDER BY` is a
        //     PARTIAL order: rows tied on `?a` may present their `?o` in any relative order,
        //     and two conformant engines may legitimately disagree there. This is the shape
        //     element-for-element sequence equality would spuriously fail (premise-correction
        //     #2 of the design record) and that the sort-key-equivalence-class comparison
        //     handles. Ties are DENSE by construction — one subject with several outgoing
        //     `ex:p` edges yields several rows sharing its single `ex:age` — so the
        //     within-class path is genuinely exercised rather than nominally present. No
        //     `LIMIT`: without a total order the window's row CHOICE would be arbitrary.
        //
        // The sort key stays on `ex:age` (an all-`xsd:integer` column) in both. Ordering on
        // `ex:name` would be unsound as an oracle: SPARQL leaves the relative order of literals
        // with DIFFERENT datatypes implementation-defined, and that column is deliberately a
        // mix of plain and language-tagged strings.
        "order" => {
            let k = 1 + rng.below(10);
            let dir = if rng.chance(1, 2) { "?a" } else { "DESC(?a)" };
            if rng.chance(2, 3) {
                return format!("{pfx}SELECT ?a WHERE {{ ?s ex:age ?a }} ORDER BY {dir} LIMIT {k}");
            }
            return format!(
                "{pfx}SELECT ?a ?o WHERE {{ ?s ex:age ?a . ?s ex:p ?o }} ORDER BY {dir}"
            );
        }
        // ── sq-qcnn.6 categories ─────────────────────────────────────────────────
        // TYPED LITERALS (§17.1 + the XSD datatypes `gen_graph`'s new columns carry).
        // The projection shapes are the load-bearing ones: they put EVERY stored literal
        // of the column in front of the value comparator unfiltered, so they are what
        // proves `sparq-difftest` absorbs Oxigraph's load-time canonicalisation without
        // absorbing a value difference. The comparison shapes stay inside the families
        // where the answer is determined (all-timezoned dateTimes, doubles, boolean EBV)
        // — see the exclusions on `gen_query`'s header for the three that are not.
        "typed" => match rng.below(10) {
            0 => "?s ex:when ?w".to_string(),
            1 => "?s ex:day ?w".to_string(),
            2 => "?s ex:dur ?d".to_string(),
            3 => "?s ex:huge ?h".to_string(),
            4 => "?s ex:dbl ?d".to_string(),
            // The typed column JOINED with the integer column — the value has to survive
            // a join, not merely a scan.
            5 => "?s ex:when ?w . ?s ex:age ?a".to_string(),
            // Boolean EBV, both polarities.
            6 => {
                if rng.chance(1, 2) {
                    "?s ex:flag ?f FILTER(?f)".to_string()
                } else {
                    "?s ex:flag ?f FILTER(!?f)".to_string()
                }
            }
            // An all-timezoned dateTime column against a timezoned constant: every
            // comparison is between two instants, so the answer is determined.
            7 => format!(
                "?s ex:when ?w FILTER(?w < \"{}-01-01T00:00:00Z\"^^xsd:dateTime)",
                2000 + rng.below(40)
            ),
            // Doubles, where the constant straddles zero and the column carries
            // `INF`/`-INF`/`NaN` (`NaN` compares false against everything, in both).
            8 => format!("?s ex:dbl ?d FILTER(?d > {}.0)", rng.below(200) as i64 - 100),
            // `DATATYPE` over the duration column, which is the one column carrying three
            // DIFFERENT datatypes over a shared value space: this projects the datatype
            // IRI as a TERM, so the subtype has to survive the store round-trip and come
            // back out of a function, not merely sit on the literal.
            _ => "?s ex:dur ?d BIND(DATATYPE(?d) AS ?t)".to_string(),
        },
        // STRING FUNCTIONS (§17.4.3), over `ex:name` — the column that is deliberately a
        // mix of plain and language-tagged strings, so both readings are exercised: the
        // LANG-SENSITIVE ones (`UCASE(?n)` on the raw literal returns the argument's
        // language tag; `LANG(?n)` returns the tag itself) and the `STR(…)`-wrapped
        // forms, which drop it. Every FILTER is on a value the function COMPUTES, so the
        // computed string is visible to the cardinality differential and not merely
        // projected.
        "strfn" => {
            let k = rng.below(6);
            match rng.below(12) {
                0 => format!("?s ex:name ?n FILTER(CONTAINS(STR(?n), \"m{k}\"))"),
                1 => format!("?s ex:name ?n FILTER(STRSTARTS(STR(?n), \"nm{k}\"))"),
                2 => format!("?s ex:name ?n FILTER(STRENDS(STR(?n), \"{k}\"))"),
                3 => format!("?s ex:name ?n FILTER(REGEX(STR(?n), \"^nm[0-{k}]$\"))"),
                4 => format!("?s ex:name ?n FILTER(STRLEN(STR(?n)) > {k})"),
                5 => "?s ex:name ?n BIND(UCASE(?n) AS ?u) FILTER(STRSTARTS(STR(?u), \"NM\"))"
                    .to_string(),
                6 => format!(
                    "?s ex:name ?n BIND(LCASE(STR(?n)) AS ?u) FILTER(STRLEN(?u) > {k})"
                ),
                7 => "?s ex:name ?n BIND(SUBSTR(STR(?n), 2, 2) AS ?u) \
                      FILTER(STRSTARTS(?u, \"m\"))"
                    .to_string(),
                8 => format!(
                    "?s ex:name ?n BIND(CONCAT(STR(?n), \"-x\") AS ?u) FILTER(STRLEN(?u) > {k})"
                ),
                9 => "?s ex:name ?n BIND(STRAFTER(STR(?n), \"nm\") AS ?u) \
                      FILTER(STRLEN(?u) > 0)"
                    .to_string(),
                10 => format!(
                    "?s ex:name ?n BIND(STRBEFORE(STR(?n), \"{k}\") AS ?u) FILTER(STRLEN(?u) > 0)"
                ),
                // `LANG` over the mixed plain/`@en` column: the empty tag of a plain
                // literal is what makes this shape select a strict subset.
                _ => "?s ex:name ?n BIND(LANG(?n) AS ?l) FILTER(STRLEN(?l) > 0)".to_string(),
            }
        }
        // ASK (§16.3) — compared by its BOOLEAN through `compare_ask`. The thresholds
        // straddle the data range so a shard sees both answers; an `ASK` category whose
        // every seed answered `true` would be a differential that cannot fail.
        "ask" => {
            let t = rng.below(120);
            return match rng.below(6) {
                0 => format!("{pfx}ASK WHERE {{ ?s ex:age ?a FILTER(?a > {t}) }}"),
                1 => format!(
                    "{pfx}ASK WHERE {{ ?s ex:p ?o . ?o ex:age ?a FILTER(?a > {t}) }}"
                ),
                2 => format!(
                    "{pfx}ASK WHERE {{ ?s ex:name ?n \
                     FILTER NOT EXISTS {{ ?s ex:age ?a . FILTER(?a < {t}) }} }}"
                ),
                3 => format!(
                    "{pfx}ASK WHERE {{ ?s ex:p+ ?o . ?o ex:age ?a FILTER(?a > {t}) }}"
                ),
                4 => format!(
                    "{pfx}ASK WHERE {{ ?s ex:when ?w . ?s ex:age ?a FILTER(?a > {t}) }}"
                ),
                _ => format!(
                    "{pfx}ASK WHERE {{ {{ SELECT ?s (COUNT(?o) AS ?c) WHERE {{ ?s ex:p ?o }} \
                     GROUP BY ?s }} FILTER(?c > {}) }}",
                    rng.below(4)
                ),
            };
        }
        // CONSTRUCT / DESCRIBE (§16.2, §16.4) — compared as a canonical TRIPLE SET
        // through `compare_graph`, never as a triple tally.
        //
        // `DESCRIBE`'s graph is explicitly left to the query service to determine, so
        // its agreement is not something the spec guarantees. It is
        // emitted anyway, and deliberately: both engines implement the outgoing-CBD
        // reading, and the generator is BNODE-FREE — which is where the two common
        // readings (with and without blank-node closure) would part company — so on this
        // data they describe the same graph. A divergence here would be an
        // implementation-CHOICE difference to adjudicate into
        // `bench/differential-divergences.json`, not a wrong answer; that is the reason
        // it is one arm out of seven rather than a category of its own.
        "construct" => {
            return match rng.below(7) {
                0 => format!("{pfx}CONSTRUCT {{ ?s ex:age ?a }} WHERE {{ ?s ex:age ?a }}"),
                1 => format!("{pfx}CONSTRUCT {{ ?s ex:knows ?o }} WHERE {{ ?s ex:p ?o }}"),
                // A two-triple template over a join — the constructed graph is bigger
                // than the solution sequence that produced it.
                2 => format!(
                    "{pfx}CONSTRUCT {{ ?o ex:reached ?s . ?o ex:age ?a }} \
                     WHERE {{ ?s ex:p ?o . ?o ex:age ?a }}"
                ),
                // The MIXED column, so a constructed object carries the non-canonical
                // numeric lexicals through the graph comparator.
                3 => format!("{pfx}CONSTRUCT {{ ?s ex:v ?v }} WHERE {{ ?s ex:val ?v }}"),
                // …and the typed columns (sq-qcnn.6).
                4 => format!("{pfx}CONSTRUCT {{ ?s ex:t ?d }} WHERE {{ ?s ex:dbl ?d }}"),
                // The short form, whose template IS its pattern.
                5 => format!("{pfx}CONSTRUCT WHERE {{ ?s ex:name ?n }}"),
                _ => format!(
                    "{pfx}DESCRIBE ?s WHERE {{ ?s ex:age ?a . FILTER(?a > {}) }}",
                    rng.below(120)
                ),
            };
        }
        _ => "?s ex:age ?a".to_string(),
    };
    format!("{pfx}SELECT * WHERE {{ {body} }}")
}

/// Oxigraph's solution CARDINALITY, for the count differential. `SELECT` only.
///
/// [OPUS-5] sq-qcnn.5: the non-`SELECT` arms deliberately REFUSE rather than return a count.
/// They used to read `Boolean(_) => 1` and `Graph(g) => g.count()`, which made an `ASK`
/// oracle doubly blind (`false` and `true` are both "one") and reduced a `CONSTRUCT` to a
/// triple tally that cannot see a graph landing on the wrong triples. `run` dispatches those
/// forms to `compare_ask` / `compare_graph` BEFORE this is reached, and refusing them here is
/// what keeps that routing the only way they can ever be compared.
// clippy: differential oracle pins oxigraph's legacy Store::query semantics
#[allow(deprecated)]
fn oxi_count(store: &Store, q: &str) -> Result<usize, String> {
    match store.query(q).map_err(|e| e.to_string())? {
        oxigraph::sparql::QueryResults::Solutions(s) => Ok(s.count()),
        oxigraph::sparql::QueryResults::Boolean(_) => {
            Err("an ASK result is compared by its BOOLEAN, not a count (see compare_ask)".into())
        }
        oxigraph::sparql::QueryResults::Graph(_) => Err(
            "a graph result is compared by its canonical TRIPLE SET, not a count (see \
             compare_graph)"
                .into(),
        ),
    }
}

// ── DOCUMENTED-DIVERGENCE ORACLE: cross-family `=` / `!=` type errors (sq-eibog) ──
//
// Oxigraph and sparq DISAGREE, by design, on `=` / `!=` between two literals whose
// datatypes are in different comparison families (e.g. `"71"^^xsd:integer != "s2"`,
// or `?age != "s4"` where `?age` is `xsd:integer`). This is NOT a sparq bug: it is a
// KNOWN leniency in Oxigraph's reference behaviour.
//
// SPARQL 1.1 §17.3 (Operator Mapping) + §17.4.1.7 (RDFterm-equal): when no
// type-specific `=` operator applies (both numeric / both xsd:string / both boolean /
// both dateTime), `A = B` falls back to `RDFterm-equal`, which **"produces a type
// error if the arguments are both literal but are not the same RDF term"**. `A != B`
// is `fn:not(RDFterm-equal(A, B))`, so it errors too, and a FILTER whose expression
// errors on a row DROPS that row (§17.4, effective boolean value of an error is
// undefined → the row is eliminated). sparq implements this (it PASSES all 15 W3C
// `sparql10/expr-equals` evaluation tests, incl. `eq-3`/`eq-4`/`eq2-1`, whose
// authoritative `!=` result `result-eq2-2.ttl` EXCLUDES every numeric-vs-string pair).
// Oxigraph instead resolves cross-family `=` to FALSE (so `!=` to TRUE) and KEEPS the
// row — a lenient, non-conformant reading. Blindly trusting Oxigraph's count here
// would flag spec-correct sparq output as a mismatch.
//
// The oracle stays sound: when the two counts differ, we RE-DERIVE the spec-correct
// count independently — take the ORIGINAL query with only its `FILTER(...)` clause
// removed (so the exact pre-filter solution multiset, incl. any join and duplicates,
// is preserved), read each solution's bound term, and re-apply the spec's `=`/`!=`
// rule term-by-term here (a self-contained reference, NOT sparq's evaluator). We flag
// a mismatch ONLY when sparq disagrees with THAT spec-correct count. A genuine
// over/under-exclusion bug therefore still trips the fuzzer; only the Oxigraph-leniency
// delta is absorbed.

/// The single-variable `=` / `!=` FILTER shape the generator emits over the mixed
/// column: the filtered variable, the operator, and the constant right-hand side.
struct EqFilter {
    var: String,   // "v" or "a" (the graph variable under test)
    negated: bool, // true for `!=`, false for `=`
    rhs: String,   // the literal/IRI constant, verbatim from the query
}

/// Parse the generated `filter` / `equality` query IFF it is a single-variable
/// `=` / `!=` over the mixed column against a CONSTANT — the only shape on which
/// Oxigraph's cross-family leniency can diverge. Returns `None` for any other query
/// (those keep the strict Oxigraph-count differential).
fn parse_eq_filter(q: &str) -> Option<EqFilter> {
    let (op, negated) = if q.contains("!=") {
        ("!=", true)
    } else if q.contains(" = ") {
        ("=", false)
    } else {
        return None;
    };
    // Only the equality/filter shapes bind the tested var to ex:val / ex:age.
    let var = if q.contains("ex:val ?v") {
        "v"
    } else if q.contains("ex:age ?a") {
        "a"
    } else {
        return None;
    };
    // Extract `FILTER(?var OP RHS)` — the generator always emits exactly this text.
    let needle = format!("FILTER(?{var} {op} ");
    let start = q.find(&needle)? + needle.len();
    let rest = &q[start..];
    let end = rest.find(')')?;
    let rhs = rest[..end].trim().to_string();
    Some(EqFilter {
        var: var.to_string(),
        negated,
        rhs,
    })
}

/// The bound `?var` term of EVERY pre-FILTER solution of the ORIGINAL query — i.e. the
/// query with its `FILTER(...)` clause removed — so the spec-correct FILTER can be
/// re-applied per solution independently of both engines. Removing only the FILTER (not
/// rebuilding a fresh pattern) preserves the exact solution MULTISET, including any join
/// to a second pattern (`?s ex:name ?n`) and duplicate rows, so the re-derived count is
/// exact for every generated equality shape — single-pattern or 2-pattern-join.
// clippy: differential oracle pins oxigraph's legacy Store::query semantics
#[allow(deprecated)]
fn all_var_terms(store: &Store, q: &str, f: &EqFilter) -> Option<Vec<oxigraph::model::Term>> {
    // Delete the `FILTER(...)` clause from the original query text (the generator emits
    // exactly one, always well-formed and balanced), keeping all patterns intact.
    let fstart = q.find("FILTER(")?;
    let after = &q[fstart + "FILTER(".len()..];
    let close = after.find(')')?; // the generated FILTER body has no nested parens
    let stripped = format!("{}{}", &q[..fstart], &after[close + 1..]);
    // Widen the projection to `*` so the filtered variable is exposed even when the
    // original query only projects `?s` (`SELECT ?s WHERE …`). This preserves the
    // solution MULTISET/cardinality of the WHERE clause exactly (SELECT projection
    // does not deduplicate without DISTINCT), so the per-row count stays exact.
    let star_projected = stripped.replacen("SELECT ?s WHERE", "SELECT * WHERE", 1);
    match store.query(&star_projected).ok()? {
        oxigraph::sparql::QueryResults::Solutions(s) => Some(
            s.filter_map(|sol| sol.ok())
                .filter_map(|sol| sol.get(f.var.as_str()).cloned())
                .collect(),
        ),
        _ => None,
    }
}

/// The comparison FAMILY of an Oxigraph literal term for spec-correct `=`: two
/// literals decide (TRUE/FALSE) only within the same family; anything else that is
/// not the same term is a TYPE ERROR. `None` = not a literal (IRI / bnode / triple).
fn oxi_family(t: &oxigraph::model::Term) -> Option<&'static str> {
    use oxigraph::model::Term;
    let lit = match t {
        Term::Literal(l) => l,
        _ => return None, // IRI / bnode — handled by identity in `spec_eq`
    };
    let dt = lit.datatype().as_str().to_string();
    if lit.language().is_some() {
        return Some("lang");
    }
    Some(match dt.as_str() {
        "http://www.w3.org/2001/XMLSchema#integer"
        | "http://www.w3.org/2001/XMLSchema#decimal"
        | "http://www.w3.org/2001/XMLSchema#double"
        | "http://www.w3.org/2001/XMLSchema#float"
        | "http://www.w3.org/2001/XMLSchema#int"
        | "http://www.w3.org/2001/XMLSchema#long"
        | "http://www.w3.org/2001/XMLSchema#short"
        | "http://www.w3.org/2001/XMLSchema#byte"
        | "http://www.w3.org/2001/XMLSchema#nonNegativeInteger"
        | "http://www.w3.org/2001/XMLSchema#nonPositiveInteger"
        | "http://www.w3.org/2001/XMLSchema#negativeInteger"
        | "http://www.w3.org/2001/XMLSchema#positiveInteger"
        | "http://www.w3.org/2001/XMLSchema#unsignedInt"
        | "http://www.w3.org/2001/XMLSchema#unsignedLong"
        | "http://www.w3.org/2001/XMLSchema#unsignedShort"
        | "http://www.w3.org/2001/XMLSchema#unsignedByte" => "num",
        "http://www.w3.org/2001/XMLSchema#string" => "str",
        "http://www.w3.org/2001/XMLSchema#boolean" => "bool",
        "http://www.w3.org/2001/XMLSchema#dateTime"
        | "http://www.w3.org/2001/XMLSchema#dateTimeStamp" => "dateTime",
        "http://www.w3.org/2001/XMLSchema#date" => "date",
        d if d.starts_with("http://www.w3.org/2001/XMLSchema#") => "otherXsd",
        _ => "unknown",
    })
}

/// SPARQL 1.1 `A = B` as three values: `Some(true)`, `Some(false)`, or `None` (type
/// error). Self-contained reference for the generated constant-vs-term comparison —
/// deliberately independent of sparq's evaluator so it is a real oracle. Only the
/// families the generator can produce (num / str / bool / dateTime / date / otherXsd
/// / lang / IRI) need be decided; every genuinely cross-family literal pair is a
/// type error per §17.4.1.7.
fn spec_eq(a: &oxigraph::model::Term, b: &oxigraph::model::Term) -> Option<bool> {
    // sameTerm decides even for unknown datatypes.
    if a == b {
        return Some(true);
    }
    let (fa, fb) = (oxi_family(a), oxi_family(b));
    // At least one non-literal (IRI / bnode) and not the same term → known different.
    if fa.is_none() || fb.is_none() {
        return Some(false);
    }
    let (fa, fb) = (fa.unwrap(), fb.unwrap());
    if fa != fb {
        return None; // cross-family literals, not same term → TYPE ERROR
    }
    use oxigraph::model::Term::Literal;
    let (la, lb) = match (a, b) {
        (Literal(x), Literal(y)) => (x, y),
        _ => unreachable!("both are literals (families matched)"),
    };
    match fa {
        // EXACT numeric value equality — NOT via f64. The generator deliberately emits
        // integers/decimals beyond 2^53 (e.g. 9007199254740992..994) that collapse to a
        // single f64; sparq compares them exactly and an f64 oracle would wrongly report
        // them equal and flag sparq. `numeric_eq_exact` compares the decimal expansions.
        "num" => numeric_eq_exact(la.value(), lb.value()),
        "str" => Some(la.value() == lb.value()),
        "bool" => {
            let norm = |v: &str| matches!(v, "true" | "1");
            let ok = |v: &str| matches!(v, "true" | "false" | "0" | "1");
            if ok(la.value()) && ok(lb.value()) {
                Some(norm(la.value()) == norm(lb.value()))
            } else {
                None
            }
        }
        // The remaining families (dateTime / date / otherXsd / lang / unknown) never
        // occur in the mixed FILTER columns (`ex:val` / `ex:age`) or as a generated
        // equality constant, so no same-family, non-identical pair of them ever reaches
        // this oracle. A value-equal-and-identical pair was already decided by the
        // `a == b` early return; anything else here is left UNDECIDED (`None`), which is
        // the conservative choice — it can only ever DROP a row this oracle never sees.
        _ => None,
    }
}

/// EXACT numeric value equality of two XSD numeric lexicals (integer / decimal /
/// double), compared as arbitrary-precision decimal expansions — never through f64.
/// This matches SPARQL `op:numeric-equal` after XPath type promotion (int/decimal/double
/// share a value space) WITHOUT the f64-collapse that would wrongly equate distinct
/// integers/decimals above 2^53 (the very case the fuzzer stresses). Returns `None` on
/// an ill-formed lexical (a SPARQL type error).
fn numeric_eq_exact(x: &str, y: &str) -> Option<bool> {
    Some(decimal_expansion(x)? == decimal_expansion(y)?)
}

/// Normalize an XSD numeric lexical to a canonical `(negative, integer_digits,
/// fraction_digits)` with no leading/trailing insignificant zeros, so two expansions
/// are equal IFF the values are equal. Handles a leading sign, a fraction point, and a
/// decimal exponent (`e`/`E`, for xsd:double/float). Signed zero normalizes to `+0`.
fn decimal_expansion(lex: &str) -> Option<(bool, String, String)> {
    let lex = lex.trim();
    // Split off a decimal exponent (xsd:double / float).
    let (mantissa, exp) = match lex.split_once(['e', 'E']) {
        Some((m, e)) => (m, e.parse::<i64>().ok()?),
        None => (lex, 0),
    };
    let (neg, digits) = match mantissa.strip_prefix('-') {
        Some(r) => (true, r),
        None => (false, mantissa.strip_prefix('+').unwrap_or(mantissa)),
    };
    let (int_part, frac_part) = match digits.split_once('.') {
        Some((i, f)) => (i, f),
        None => (digits, ""),
    };
    if int_part.is_empty() && frac_part.is_empty() {
        return None;
    }
    if !int_part.chars().all(|c| c.is_ascii_digit())
        || !frac_part.chars().all(|c| c.is_ascii_digit())
    {
        return None;
    }
    // Build the full digit string and the position of the decimal point (from the left).
    let mut all: String = int_part.chars().chain(frac_part.chars()).collect();
    // point index within `all` counting from the start of int_part, then shift by exp.
    let point = int_part.len() as i64 + exp;
    // Reconstruct integer/fraction around the (possibly shifted) point.
    let (new_int, new_frac) = if point <= 0 {
        let zeros = "0".repeat((-point) as usize);
        (String::from("0"), format!("{zeros}{all}"))
    } else if (point as usize) >= all.len() {
        all.push_str(&"0".repeat(point as usize - all.len()));
        (all, String::new())
    } else {
        let f = all.split_off(point as usize);
        (all, f)
    };
    // Strip insignificant zeros.
    let int_norm = new_int.trim_start_matches('0');
    let int_norm = if int_norm.is_empty() { "0" } else { int_norm };
    let frac_norm = new_frac.trim_end_matches('0');
    let is_zero = int_norm == "0" && frac_norm.is_empty();
    Some((neg && !is_zero, int_norm.to_string(), frac_norm.to_string()))
}

/// Parse the generated RHS constant (verbatim query text) into an Oxigraph `Term`.
/// Handles the exact spellings `gen_filter` / the `equality` category emit: a bare
/// integer, a `"…"`-quoted string, a `"…"^^xsd:TYPE` typed literal, and an `ex:` IRI.
fn parse_rhs_term(rhs: &str) -> Option<oxigraph::model::Term> {
    use oxigraph::model::{Literal, NamedNode, Term};
    let xsd = |t: &str| NamedNode::new(format!("http://www.w3.org/2001/XMLSchema#{t}")).unwrap();
    if let Some(rest) = rhs.strip_prefix('"') {
        // "lex" or "lex"^^xsd:TYPE
        let close = rest.find('"')?;
        let lex = &rest[..close];
        let after = &rest[close + 1..];
        if let Some(dt) = after.strip_prefix("^^xsd:") {
            return Some(Term::Literal(Literal::new_typed_literal(lex, xsd(dt))));
        }
        return Some(Term::Literal(Literal::new_simple_literal(lex)));
    }
    if let Some(local) = rhs.strip_prefix("ex:") {
        return Some(Term::NamedNode(
            NamedNode::new(format!("http://ex/{local}")).unwrap(),
        ));
    }
    // bare integer / boolean keyword
    if rhs.parse::<i64>().is_ok() {
        return Some(Term::Literal(Literal::new_typed_literal(
            rhs,
            xsd("integer"),
        )));
    }
    None
}

/// The SPEC-CORRECT solution count for a single-variable `=` / `!=` FILTER over the
/// mixed column — the spec's own answer, computed independently of both engines.
/// `None` when the shape is not a recognised constant equality filter (caller keeps
/// the strict Oxigraph differential in that case).
fn spec_filter_count(store: &Store, q: &str) -> Option<usize> {
    let f = parse_eq_filter(q)?;
    let rhs = parse_rhs_term(&f.rhs)?;
    let terms = all_var_terms(store, q, &f)?;
    let kept = terms
        .iter()
        .filter(|t| match spec_eq(t, &rhs) {
            // `=` keeps eq==true; `!=` keeps eq==false; a type error drops the row.
            Some(eq) => eq != f.negated,
            None => false,
        })
        .count();
    Some(kept)
}

/// sq-ai2wa detector — deliberately NARROW (never widen beyond the adjudication in
/// bench/differential-divergences.json): the mismatching case must be the recognised
/// single-variable constant `=`/`!=` FILTER shape AND pair a BLANK NODE on one side
/// with an IRI on the other. The current generator emits no blank nodes and Oxigraph
/// shares sparq's identity reading of non-literal `=`/`!=`, so this cannot fire
/// today; it exists so generator/oracle growth (or an Oxigraph behaviour change)
/// surfaces as an ADJUDICATED skip-with-count instead of a false positive. [FABLE-5]
fn is_bnode_iri_inequality(store: &Store, q: &str) -> bool {
    use oxigraph::model::Term;
    let Some(f) = parse_eq_filter(q) else {
        return false;
    };
    let Some(rhs) = parse_rhs_term(&f.rhs) else {
        return false;
    };
    let Some(terms) = all_var_terms(store, q, &f) else {
        return false;
    };
    terms.iter().any(|t| {
        matches!(
            (t, &rhs),
            (Term::BlankNode(_), Term::NamedNode(_)) | (Term::NamedNode(_), Term::BlankNode(_))
        )
    })
}

// ── VALUE-LEVEL CROSS-ORACLE COMPARISON (bead sq-qcnn.5) ─────────────────────────
//
// Every comparator below routes BOTH engines' answers through `sparq-difftest` (bead
// sq-qcnn.4) — the ENGINE-INDEPENDENT comparator library that depends on no sparq crate
// and owns every value decision: exact arbitrary-precision integer / decimal equality
// (num-bigint / bigdecimal), canonical `xsd:double` incl. INF / NaN, UTC-normalised
// temporals, and RDFC-1.0 blank-node canonical labelling. That independence is
// load-bearing: canonicalising BOTH sides through sparq's own numeric tower or term
// comparator would apply any bug there identically to both sides, where it CANCELS — the
// differential would go blind exactly where it must see. `crate::neutral` is a purely
// structural carrier→model bridge and decides no equality (see its module docs, which also
// document the one harness-forced datatype fold Oxigraph's store makes necessary).
//
// What each result FORM is compared BY. Reducing any of them to a COUNT is precisely the
// blindness this bead removes:
//
//   * `SELECT` (no modelled ORDER BY)  → solution MULTISET equality (`multiset_equal`) —
//                                        duplicates significant (SPARQL is bag semantics).
//   * `SELECT … ORDER BY`              → equality up to permutation WITHIN each
//                                        sort-key-equivalence class (`order_by_equal`).
//                                        `ORDER BY` is a PARTIAL order, so demanding
//                                        element-for-element sequence equality is WRONG in
//                                        general: rows tied on every sort key may appear in
//                                        any relative order across two conformant engines.
//                                        Where the sort key IS total over the projection,
//                                        `order_by_equal` degrades to exactly that strict
//                                        sequence equality on its own.
//   * `SELECT` projecting a bnode      → blank-node ISOMORPHISM (`solutions_isomorphic`):
//                                        bnode labels are engine-local, so equal answers
//                                        can carry different ones and only a bijection is
//                                        well defined.
//   * `ASK`                            → the BOOLEAN itself. A count maps `true` AND
//                                        `false` to one row, so an ASK oracle built on
//                                        `oxi_count` was DOUBLY blind.
//   * `CONSTRUCT` / `DESCRIBE`         → the canonical TRIPLE SET up to blank-node
//                                        isomorphism (`graph_isomorphic`), never a triple
//                                        count (which cannot see a graph landing on the
//                                        wrong triples).
//
// The one deliberate non-comparison is `LIMIT` / `OFFSET` WITHOUT a total order over the
// projected variables: SPARQL leaves the row CHOICE arbitrary there (§2.2 of the design
// record), so only the cardinality is a function of the query. Such a case stays
// cardinality-only and is COUNTED as `skipped(row-choice)` — never silently green. The
// generator also emits the same shape WITH a total tiebreaker, which is fully value-checked
// (see the `limit` category).

/// The SPARQL result FORM of a generated query — what decides which comparator is SOUND.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Form {
    /// A solution sequence.
    Select,
    /// A boolean.
    Ask,
    /// An RDF graph (`CONSTRUCT` / `DESCRIBE`).
    Graph,
}

/// Classify a generated query's result form from its text.
///
/// Text-level classification is sound for THIS generator and only for it: every emitted query
/// puts its query form's keyword at the start of a line (the `PREFIX` prologue always ends in a
/// newline) and no generated literal contains one. A sub-`SELECT` sits INSIDE the outer form's
/// line, so the outer keyword is always the one found first.
fn query_form(q: &str) -> Form {
    for line in q.lines() {
        let t = line.trim_start();
        if t.starts_with("ASK") {
            return Form::Ask;
        }
        if t.starts_with("CONSTRUCT") || t.starts_with("DESCRIBE") {
            return Form::Graph;
        }
        if t.starts_with("SELECT") {
            return Form::Select;
        }
    }
    Form::Select
}

/// The `ORDER BY` sort variables of a generated query, in key order.
///
/// * `Some(vars)` — the whole clause is a list of plain variables, each optionally wrapped in
///   `ASC(…)` / `DESC(…)`, so the sort-key equivalence classes are exactly the distinct value
///   tuples of `vars` and [`order_by_equal`] applies. Sort DIRECTION is deliberately dropped:
///   the equivalence classes are the same either way, and both engines' sequences are compared
///   in the order they were produced, so a wrong direction still shows up as a mismatch.
/// * `Some(vec![])` — there is no `ORDER BY` at all (the un-ordered multiset comparison).
/// * `None` — the clause contains something this extractor does not model (an expression, a
///   function call). The caller must then fall back to the ORDER-INSENSITIVE multiset
///   comparison, which is always sound, rather than GUESS a sort key: claiming a finer
///   partition than the query's would manufacture false mismatches.
fn order_by_vars(q: &str) -> Option<Vec<String>> {
    let Some(i) = q.find("ORDER BY") else {
        return Some(Vec::new());
    };
    let rest = &q[i + "ORDER BY".len()..];
    // The clause runs to the end of the query or to the first modifier that can follow it.
    let end = ["LIMIT", "OFFSET"]
        .iter()
        .filter_map(|kw| rest.find(kw))
        .min()
        .unwrap_or(rest.len());
    let mut out = Vec::new();
    for tok in rest[..end].split_whitespace() {
        let inner = tok
            .strip_prefix("ASC(")
            .or_else(|| tok.strip_prefix("DESC("))
            .map(|s| s.trim_end_matches(')'))
            .unwrap_or(tok);
        let var = inner.strip_prefix('?')?;
        if var.is_empty() || !var.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            return None;
        }
        out.push(var.to_string());
    }
    Some(out)
}

/// The PROJECTION the two answers agree on — or the header mismatch, as a repro message.
///
/// The result header is part of the answer, not metadata about it: a projected-but-unbound
/// variable appears ONLY there (`rows` records bound pairs), so two answers with identical row
/// maps but different headers are DIFFERENT answers — one engine projected a variable the other
/// did not. Comparing the rows alone cannot see that, which is why the header sets are required
/// to match here, BEFORE any window classification or row comparison. Projection ORDER is not
/// compared: `SELECT *` leaves it to the implementation, and the row maps this harness compares
/// are keyed by variable name, so order cannot affect any verdict it reaches.
///
/// The agreed header, not the bound pairs of the returned rows, is the right set for
/// [`is_total_order`]. The rows reaching that decision have already been SLICED by
/// `LIMIT`/`OFFSET`, so a projected variable that happens to be unbound in every SURVIVING row
/// has vanished from them — even though it may be bound, to different values, on the tied
/// candidate rows the window chose between. Reading coverage off the sliced rows would
/// therefore call such a window "totally ordered" and value-compare a row choice SPARQL leaves
/// implementation-defined, recording a non-testable case as a passing value comparison.
///
/// The bound pairs are folded in afterwards only as a defensive consistency check — a row
/// binding a variable its own header omits is an engine defect this comparator has no verdict
/// for, and widening the coverage set can only route MORE windows to the honest
/// `skipped(row-choice)` bucket, never fewer.
fn result_vars(a: &Answer, b: &Answer) -> Result<BTreeSet<String>, String> {
    let header = |ans: &Answer| -> BTreeSet<String> { ans.vars.iter().cloned().collect() };
    let (ha, hb) = (header(a), header(b));
    if ha != hb {
        let only = |x: &BTreeSet<String>, y: &BTreeSet<String>| -> Vec<String> {
            x.difference(y).cloned().collect()
        };
        return Err(format!(
            "result HEADER differs (sparq {:?} / oxigraph {:?})\n  only in sparq: {:?}\n  \
             only in oxi  : {:?}",
            a.vars,
            b.vars,
            only(&ha, &hb),
            only(&hb, &ha)
        ));
    }
    Ok(ha
        .into_iter()
        .chain(
            a.rows
                .iter()
                .chain(&b.rows)
                .flat_map(|sol| sol.keys().cloned()),
        )
        .collect())
}

/// Is the sort key TOTAL over the projection — i.e. does it cover every variable either engine
/// projects (see [`result_vars`])?
///
/// If it does, any two rows in the same sort-key-equivalence class agree on every projected
/// variable and are therefore the SAME row, so WHICH of them a `LIMIT` / `OFFSET` window keeps
/// cannot change the answer: the windowed result becomes a function of the query and the full
/// value-level comparison applies (§2.2 of the design record — the "total tiebreaker" option).
/// An empty sort key over a non-empty projection is not total.
fn is_total_order(sort_vars: &[String], projected: &BTreeSet<String>) -> bool {
    !projected.is_empty() && projected.iter().all(|v| sort_vars.contains(v))
}

/// One engine's `SELECT` answer in the neutral model: the solution SEQUENCE with its ORDER
/// preserved, because the `ORDER BY` comparison needs it (the multiset comparator does not
/// care, and sorts internally).
type Solutions = Vec<Solution>;

/// One engine's `SELECT` answer together with its result HEADER.
///
/// The header is carried alongside the rows because it is the only place a
/// PROJECTED-BUT-UNBOUND variable survives: such a variable is invisible in `rows`, yet it can
/// still distinguish the tied candidate rows a `LIMIT` window chose between, so
/// [`is_total_order`] must see it. See [`result_vars`].
struct Answer {
    /// Every variable the engine reports in the result header, in projection order.
    vars: Vec<String>,
    /// The solutions, in the order the engine produced them.
    rows: Solutions,
}

/// sparq's `SELECT` answer, bridged into the neutral model. `None` when sparq declines the
/// query (an unsupported surface — fair to skip; a WRONG ANSWER is not).
fn sparq_solutions(g: &sparq_core::Graph, q: &str) -> Option<Answer> {
    let r = sparq_engine::query(g, q).ok()?;
    Some(Answer {
        vars: r.vars.iter().map(|v| v.as_str().to_string()).collect(),
        rows: r
            .rows
            .iter()
            .map(|row| {
                neutral_solution(
                    r.vars
                        .iter()
                        .zip(row.iter())
                        .filter_map(|(v, t)| t.as_ref().map(|t| (v.as_str(), t))),
                )
            })
            .collect(),
    })
}

/// Oxigraph's `SELECT` answer, bridged into the neutral model. `None` when the query did not
/// produce a solution sequence (an `ASK` / `CONSTRUCT`, which the form dispatch routes
/// elsewhere) or a solution failed to decode.
// clippy: differential oracle pins oxigraph's legacy Store::query semantics
#[allow(deprecated)]
fn oxi_solutions(store: &Store, q: &str) -> Option<Answer> {
    match store.query(q).ok()? {
        oxigraph::sparql::QueryResults::Solutions(s) => {
            // The header must be read BEFORE the iterator is consumed.
            let vars: Vec<String> = s.variables().iter().map(|v| v.as_str().to_string()).collect();
            let mut rows: Solutions = Vec::new();
            for sol in s {
                let sol = sol.ok()?;
                rows.push(neutral_solution(sol.iter().map(|(v, t)| (v.as_str(), t))));
            }
            Some(Answer { vars, rows })
        }
        _ => None,
    }
}

/// What the value-level cross-oracle comparison actually DID for one case. Every variant gets
/// its OWN counter in the summary line, because these outcomes mean different things: a
/// row-choice skip is a case that is not value-differential-testable at all, whereas a triage
/// outcome is an answer nobody compared. Folding them together would report a coverage gap as
/// if it were the documented non-testable case.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Compared {
    /// `SELECT` without a modelled `ORDER BY`: the solution MULTISET was compared.
    Multiset,
    /// `SELECT … ORDER BY`: compared up to permutation within each sort-key-equivalence class.
    Ordered,
    /// `SELECT` projecting a blank node: compared up to blank-node ISOMORPHISM (RDFC-1.0).
    Isomorphic,
    /// `ASK`: the boolean itself was compared.
    AskBoolean,
    /// `CONSTRUCT` / `DESCRIBE`: the canonical triple SET was compared.
    GraphIsomorphic,
    /// `LIMIT` / `OFFSET` without a total order over the projected variables — cardinality-only
    /// BY DESIGN, since SPARQL leaves the row CHOICE arbitrary there.
    SkippedRowChoice,
    /// An engine declined the query, or its answer was not the expected result form: nothing
    /// was compared (the caller counts this as `skipped(unsupported)`).
    Unsupported,
    /// RDFC-1.0 canonical labelling DECLINED — an RDF-1.2 triple term (which the RDFC-1.0
    /// implementation has no model for), a non-RDF term, or the HNDQ call limit on a poison
    /// graph. Counted triage carrying the reason; NEVER read as agreement.
    TriageIso(String),
}

/// The value-canonical `(variable, key)` pairs `sparq-difftest`'s comparators key a solution
/// on, rendered for the repro message — so the printed diff is stated in exactly the terms the
/// verdict was reached in.
fn solution_keys(sol: &Solution) -> Vec<(String, String)> {
    sol.iter()
        .map(|(v, t)| (v.clone(), canonical_key(t)))
        .collect()
}

/// The first solution present in `a` but not in `b` **as a multiset**, for the repro message.
fn first_extra_row(a: &Solutions, b: &Solutions) -> Option<String> {
    let mut rest: Vec<Vec<(String, String)>> = b.iter().map(solution_keys).collect();
    for sol in a {
        let key = solution_keys(sol);
        match rest.iter().position(|r| *r == key) {
            Some(i) => {
                rest.remove(i);
            }
            None => return Some(format!("{key:?}")),
        }
    }
    None
}

/// A repro-sized multiset diff: the row counts plus one row unique to each side.
fn multiset_detail(what: &str, sparq: &Solutions, oxi: &Solutions) -> String {
    format!(
        "{what} (sparq {} rows / oxi {} rows)\n  only in sparq: {}\n  only in oxi  : {}",
        sparq.len(),
        oxi.len(),
        first_extra_row(sparq, oxi).unwrap_or_else(|| "-".to_string()),
        first_extra_row(oxi, sparq).unwrap_or_else(|| "-".to_string())
    )
}

/// A repro-sized `ORDER BY` diff. The multisets may well AGREE here (the divergence being the
/// ORDER alone), so the first row whose sort key differs is named explicitly.
fn order_detail(sparq: &Solutions, oxi: &Solutions, sort_vars: &[&str]) -> String {
    let keys = |rows: &Solutions| -> Vec<Vec<Option<String>>> {
        rows.iter()
            .map(|sol| {
                sort_vars
                    .iter()
                    .map(|v| sol.get(*v).map(canonical_key))
                    .collect()
            })
            .collect()
    };
    let (ka, kb) = (keys(sparq), keys(oxi));
    let at = match ka.iter().zip(&kb).position(|(a, b)| a != b) {
        Some(i) => format!("; first differing sort key at row {i}: sparq={:?} oxi={:?}", ka[i], kb[i]),
        None => String::new(),
    };
    multiset_detail(
        &format!("ORDER BY result differs (sort key {sort_vars:?}{at})"),
        sparq,
        oxi,
    )
}

/// FULL-BINDING differential for a `SELECT`: sparq's complete solution multiset — or, under
/// `ORDER BY`, its sort-key-equivalence-class-respecting sequence — must equal Oxigraph's.
/// Every projected binding, duplicates included, not merely the CARDINALITY: the count
/// differential alone cannot see a same-cardinality WRONG ANSWER (a path landing on the wrong
/// endpoints, a `COUNT` / `MIN` / `MAX` off by one, a `BIND` / `VALUES` / sub-select binding
/// the wrong term), which is exactly what the `path`, `aggregate`, `subquery`, `values` and
/// `bind` categories generate.
fn compare_select(g: &sparq_core::Graph, store: &Store, q: &str) -> Result<Compared, String> {
    let (Some(sparq), Some(oxi)) = (sparq_solutions(g, q), oxi_solutions(store, q)) else {
        return Ok(Compared::Unsupported);
    };
    compare_answers(&sparq, &oxi, q)
}

/// The comparison itself, on the two answers already bridged into the neutral model — split out
/// so it can be driven from a hand-built pair (which is the only way to state a HEADER defect:
/// a real engine will not produce one on demand).
fn compare_answers(sparq: &Answer, oxi: &Answer, q: &str) -> Result<Compared, String> {
    // The header is part of the answer: check it FIRST, so a projected-variable disagreement is
    // a mismatch rather than something the window classification or the row comparison — both
    // of which see only bound pairs — silently accepts. See [`result_vars`].
    let projected = result_vars(sparq, oxi)?;
    // A sort variable OUTSIDE the projection cannot be read off the compared rows, so its
    // equivalence classes are not computable here — treat the clause as unmodelled and fall
    // back to the (always-sound) order-insensitive comparison rather than guess.
    let sort_vars = order_by_vars(q).filter(|vs| vs.iter().all(|v| projected.contains(v)));
    let windowed = q.contains("LIMIT") || q.contains("OFFSET");
    let total = sort_vars
        .as_ref()
        .is_some_and(|vs| is_total_order(vs, &projected));
    if windowed && !total {
        return Ok(Compared::SkippedRowChoice);
    }
    let (sparq, oxi) = (&sparq.rows, &oxi.rows);
    // Blank-node labels are engine-local, so a bnode-bearing answer is comparable only up to a
    // bijection. RDFC-1.0 canonicalises the whole TABLE at once (so identity shared between
    // rows is part of what is compared), which necessarily discards row ORDER — an ordered
    // bnode answer therefore has its order un-checked, and lands in its own counter.
    if solutions_have_blank_nodes(sparq) || solutions_have_blank_nodes(oxi) {
        return match solutions_isomorphic(sparq, oxi) {
            Ok(true) => Ok(Compared::Isomorphic),
            Ok(false) => Err(multiset_detail(
                "solution table is NOT isomorphic (no bijection of the blank nodes matches it)",
                sparq,
                oxi,
            )),
            Err(e) => Ok(Compared::TriageIso(e.to_string())),
        };
    }
    match &sort_vars {
        Some(vs) if !vs.is_empty() => {
            let refs: Vec<&str> = vs.iter().map(String::as_str).collect();
            if order_by_equal(sparq, oxi, &refs) {
                Ok(Compared::Ordered)
            } else {
                Err(order_detail(sparq, oxi, &refs))
            }
        }
        _ => {
            if multiset_equal(sparq, oxi) {
                Ok(Compared::Multiset)
            } else {
                Err(multiset_detail("solution MULTISET differs", sparq, oxi))
            }
        }
    }
}

/// `ASK` differential: the BOOLEAN itself. A cardinality oracle is doubly blind here — it maps
/// both `true` and `false` to one row — so this is the only sound comparison for the form.
// clippy: differential oracle pins oxigraph's legacy Store::query semantics
#[allow(deprecated)]
fn compare_ask(g: &sparq_core::Graph, store: &Store, q: &str) -> Result<Compared, String> {
    let Ok(sparq) = sparq_engine::ask(g, q) else {
        return Ok(Compared::Unsupported);
    };
    let oxi = match store.query(q) {
        Ok(oxigraph::sparql::QueryResults::Boolean(b)) => b,
        _ => return Ok(Compared::Unsupported),
    };
    if sparq != oxi {
        return Err(format!("ASK boolean differs: sparq={sparq} oxigraph={oxi}"));
    }
    Ok(Compared::AskBoolean)
}

/// `CONSTRUCT` / `DESCRIBE` differential: the canonical TRIPLE SET, up to blank-node
/// isomorphism (a constructed graph's bnode labels are engine-local, and `CONSTRUCT`
/// templates mint fresh ones). A triple COUNT cannot see a graph landing on the wrong triples,
/// which is the blindness this replaces.
// clippy: differential oracle pins oxigraph's legacy Store::query semantics
#[allow(deprecated)]
fn compare_graph(g: &sparq_core::Graph, store: &Store, q: &str) -> Result<Compared, String> {
    let Ok(sparq_triples) = sparq_engine::construct_or_describe(g, q) else {
        return Ok(Compared::Unsupported);
    };
    let mut oxi_triples: Vec<[sparq_difftest::Term; 3]> = Vec::new();
    match store.query(q) {
        Ok(oxigraph::sparql::QueryResults::Graph(triples)) => {
            for t in triples {
                let Ok(t) = t else {
                    return Ok(Compared::Unsupported);
                };
                oxi_triples.push(neutral_triple(&t));
            }
        }
        _ => return Ok(Compared::Unsupported),
    }
    let sparq_graph: Vec<[sparq_difftest::Term; 3]> =
        sparq_triples.iter().map(neutral_triple).collect();
    match graph_isomorphic(&sparq_graph, &oxi_triples) {
        Ok(true) => Ok(Compared::GraphIsomorphic),
        Ok(false) => Err(format!(
            "CONSTRUCT/DESCRIBE graph differs — no bijection of the blank nodes matches it \
             (sparq {} triples / oxi {} triples)",
            sparq_graph.len(),
            oxi_triples.len()
        )),
        Err(e) => Ok(Compared::TriageIso(e.to_string())),
    }
}

pub fn run(seed_start: u64, count: u64, category: &str) {
    // An unknown category would otherwise fall through `gen_query`'s catch-all arm
    // and quietly fuzz a single BGP shape — i.e. a typo in the CI shard matrix would
    // report a green shard that never exercised its surface. Fail loudly instead.
    if category != "all" && !CATEGORIES.contains(&category) {
        eprintln!(
            "error: unknown fuzz category {category:?} — known: all, {}",
            CATEGORIES.join(", ")
        );
        std::process::exit(2);
    }
    // The adjudicated-divergence allowlist (bench/differential-divergences.json) —
    // loaded ONCE; the active state is printed so every CI log shows exactly which
    // classes were skip-with-count vs strict for this run.
    let allow = DivergenceAllowlist::load();
    println!(
        "divergence allowlist ({}): cross-family-eq-type-error={} bnode-iri-inequality={}",
        allow.source,
        if allow.cross_family_eq_type_error {
            "adjudicated (sq-eibog)"
        } else {
            "STRICT"
        },
        if allow.bnode_iri_inequality {
            "adjudicated (sq-ai2wa)"
        } else {
            "STRICT"
        },
    );
    let mut checked = 0u64;
    let mut skipped_unsupported = 0u64;
    let mut adjudicated_cross_family = 0u64;
    let mut adjudicated_bnode_iri = 0u64;
    let mut full_mismatch = 0u64;
    let mut count_mismatch = 0u64;
    // [OPUS-5] sq-qcnn.5: ONE COUNTER PER `Compared` VARIANT. These outcomes are not
    // interchangeable — a row-choice skip is a case that is not value-differential-testable at
    // all, a triage outcome is an answer nobody compared, and the four compared kinds say WHICH
    // oracle ran — so a run that quietly stopped comparing values shows up as a number rather
    // than as green.
    let mut compared_multiset = 0u64;
    let mut compared_ordered = 0u64;
    let mut compared_iso = 0u64;
    let mut compared_ask = 0u64;
    let mut compared_graph = 0u64;
    let mut skipped_row_choice = 0u64;
    let mut skipped_undecoded = 0u64;
    let mut triage_iso = 0u64;
    let mut first_repro: Option<String> = None;

    for seed in seed_start..seed_start + count {
        let mut rng = Rng::new(seed);
        let ttl = gen_graph(&mut rng);
        let q = gen_query(&mut rng, category);

        let g = match sparq_core::Graph::load_str(&ttl, "turtle") {
            Ok(g) => g,
            Err(e) => {
                report_repro(
                    &mut first_repro,
                    seed,
                    &q,
                    &ttl,
                    &format!("sparq load error: {e}"),
                );
                full_mismatch += 1;
                continue;
            }
        };
        // With SPARQ_FUZZ_COMPRESS=1, validate the BLOCK-COMPRESSED store end-to-end:
        // identical results to Oxigraph prove the compressed scan path is correct.
        let g = if std::env::var("SPARQ_FUZZ_COMPRESS").is_ok() {
            g.into_compressed()
        } else {
            g
        };
        // With SPARQ_FUZZ_MMAP=1, save the graph and reopen it MEMORY-MAPPED (perms +
        // numeric cache + the mmap dictionary) — validates the out-of-core read paths
        // (mmap'd term lookup + materialisation) against Oxigraph. One reused temp dir,
        // overwritten each seed (the prior `g2` has dropped, releasing its mmaps).
        let g = if std::env::var("SPARQ_FUZZ_MMAP").is_ok() {
            let dir = std::env::temp_dir().join(format!("sparq_fuzz_mmap_{}", std::process::id()));
            g.save(&dir).expect("save");
            sparq_core::Graph::open(&dir).expect("open")
        } else {
            g
        };
        // With SPARQ_FUZZ_DICTSPILL=1, REBUILD the store through the spilled-dictionary
        // external build (`dict-spill`) with a TINY budget — constant dedup-cache epoch
        // clears and many-run external sorts on every case — and reopen it memory-mapped.
        // Agreement with Oxigraph over thousands of cases validates the spilled id
        // assignment + streamed dictionary end-to-end. The parsed graph is re-serialized
        // to N-Triples (the only format the spill path accepts; the generated Turtle is
        // triple-term-free, so terms round-trip exactly).
        let g = if std::env::var("SPARQ_FUZZ_DICTSPILL").is_ok() {
            let scan = g.store.scan(&[None, None, None]);
            let mut nt = String::new();
            for r in scan.rows.iter() {
                let spo = scan.to_spo(r);
                nt.push_str(&format!(
                    "{} {} {} .\n",
                    g.dict.term(spo[0]),
                    g.dict.term(spo[1]),
                    g.dict.term(spo[2])
                ));
            }
            let dir = std::env::temp_dir().join(format!("sparq_fuzz_dsp_{}", std::process::id()));
            std::fs::remove_dir_all(&dir).ok();
            let cfg = sparq_core::dictspill::SpillConfig {
                mem_budget: 1,
                disk_floor: 0,
            };
            sparq_core::Graph::build_external_spill(nt.as_bytes(), "ntriples", &dir, 64, &cfg)
                .expect("dict-spill build");
            sparq_core::Graph::open(&dir).expect("open")
        } else {
            g
        };
        let store = Store::new().unwrap();
        if let Err(e) = store.load_from_reader(oxigraph::io::RdfFormat::Turtle, ttl.as_bytes()) {
            // Both engines parse the same Turtle; a divergence here is itself a bug.
            report_repro(
                &mut first_repro,
                seed,
                &q,
                &ttl,
                &format!("oxigraph load error: {e}"),
            );
            full_mismatch += 1;
            continue;
        }

        // [OPUS-5] sq-qcnn.5: the RESULT FORM decides which comparison is SOUND, so it is
        // dispatched FIRST. `ASK` and `CONSTRUCT`/`DESCRIBE` are compared by VALUE — the
        // boolean itself, the canonical triple set — and never enter the cardinality flow,
        // where an `ASK` would collapse to "one row" whatever it answered and a graph to a
        // triple tally. [SONNET-4.6] sq-qcnn.6 landed the generator side: the `ask` and
        // `construct` categories now emit those two forms, so this dispatch is live rather
        // than merely wired, and `expected_form` (in the tests) pins which category reaches
        // which comparator.
        let form = query_form(&q);
        if form != Form::Select {
            let outcome = if form == Form::Ask {
                compare_ask(&g, &store, &q)
            } else {
                compare_graph(&g, &store, &q)
            };
            match outcome {
                Ok(Compared::Unsupported) => skipped_unsupported += 1,
                Ok(Compared::AskBoolean) => {
                    checked += 1;
                    compared_ask += 1;
                }
                Ok(Compared::GraphIsomorphic) => {
                    checked += 1;
                    compared_graph += 1;
                }
                Ok(Compared::TriageIso(why)) => {
                    checked += 1;
                    triage_iso += 1;
                    eprintln!("TRIAGE-ISO seed={seed} {why}");
                }
                Ok(other) => unreachable!("{other:?} is not an ASK/graph outcome"),
                Err(detail) => {
                    checked += 1;
                    full_mismatch += 1;
                    report_repro(&mut first_repro, seed, &q, &ttl, &detail);
                }
            }
            continue;
        }

        let oxi = match oxi_count(&store, &q) {
            Ok(n) => n,
            Err(_) => {
                skipped_unsupported += 1;
                continue;
            }
        };

        let sparq_full = match sparq_engine::query(&g, &q) {
            Ok(r) => r.len(),
            Err(_) => {
                // sparq doesn't support this surface — fair to skip (not a wrong answer).
                skipped_unsupported += 1;
                continue;
            }
        };
        checked += 1;

        if sparq_full != oxi {
            // Before flagging, consult the ADJUDICATED divergence classes loaded from
            // bench/differential-divergences.json (a class absent from that file
            // keeps the strict differential — the file is the source of truth).
            //
            // sq-eibog (cross-family `=`/`!=` type error) is NOT a blind skip: if
            // this is a constant equality filter over the mixed column, re-derive the
            // SPEC-CORRECT count and flag only when sparq disagrees with THAT (a
            // genuine bug), not with Oxigraph's lenient count. Any non-equality shape
            // has `spec_filter_count == None` and keeps the strict differential.
            let spec = if allow.cross_family_eq_type_error {
                spec_filter_count(&store, &q)
            } else {
                None
            };
            match spec {
                Some(spec) if sparq_full == spec => {
                    // sparq matches the spec; Oxigraph is lenient here — the
                    // adjudicated sq-eibog class. Skip WITH COUNT (summary line).
                    adjudicated_cross_family += 1;
                }
                Some(spec) => {
                    full_mismatch += 1;
                    report_repro(
                        &mut first_repro,
                        seed,
                        &q,
                        &ttl,
                        &format!(
                            "sparq query().len()={sparq_full} != SPEC-correct={spec} \
                             (oxigraph={oxi}, lenient cross-family =/!=)"
                        ),
                    );
                }
                None if allow.bnode_iri_inequality && is_bnode_iri_inequality(&store, &q) => {
                    // sq-ai2wa: bnode-vs-IRI `=`/`!=` — adjudicated identity-vs-type-
                    // error ambiguity. Skip WITH COUNT (inert for this generator; see
                    // the JSON entry).
                    adjudicated_bnode_iri += 1;
                }
                None => {
                    full_mismatch += 1;
                    report_repro(
                        &mut first_repro,
                        seed,
                        &q,
                        &ttl,
                        &format!("sparq query().len()={sparq_full} != oxigraph={oxi}"),
                    );
                }
            }
        }

        // FULL-BINDING differential — the answer VALUES, not just how many there are.
        // `compare_select` picks the comparator the query's shape makes sound: the solution
        // MULTISET, or — under `ORDER BY` — equality up to permutation within each
        // sort-key-equivalence class, or blank-node isomorphism.
        //
        // Run only when the two cardinalities already agree: a cardinality difference has just
        // been adjudicated above (the sq-eibog / sq-ai2wa classes differ in COUNT, so
        // re-reporting them here would double-count one divergence), and a genuine count
        // mismatch has already been reported.
        if sparq_full == oxi {
            match compare_select(&g, &store, &q) {
                Ok(Compared::Multiset) => compared_multiset += 1,
                Ok(Compared::Ordered) => compared_ordered += 1,
                Ok(Compared::Isomorphic) => compared_iso += 1,
                Ok(Compared::SkippedRowChoice) => skipped_row_choice += 1,
                // Both engines just answered this query (the cardinality differential above
                // needed both answers), so reaching here means an answer could not be DECODED
                // into the neutral model — its own anomaly, kept out of the row-choice bucket
                // so it can never be mistaken for the documented non-testable case.
                Ok(Compared::Unsupported) => skipped_undecoded += 1,
                Ok(Compared::TriageIso(why)) => {
                    triage_iso += 1;
                    eprintln!("TRIAGE-ISO seed={seed} {why}");
                }
                Ok(other) => unreachable!("{other:?} is not a SELECT outcome"),
                Err(detail) => {
                    full_mismatch += 1;
                    report_repro(&mut first_repro, seed, &q, &ttl, &detail);
                }
            }
        }

        // JSON-path differential: serialising directly from ids (query_json, incl. the
        // streaming single-pattern / 2-pattern-join paths) must yield the SAME multiset
        // of bindings as building the QueryResult then serialising. Order-independent.
        if let Ok(qj) = sparq_engine::query_json(&g, &q) {
            let via_result =
                sparq_engine::json::to_sparql_json(&sparq_engine::query(&g, &q).unwrap());
            if bindings_multiset(&qj) != bindings_multiset(&via_result) {
                full_mismatch += 1;
                report_repro(
                    &mut first_repro,
                    seed,
                    &q,
                    &ttl,
                    "query_json bindings multiset != to_sparql_json(query())",
                );
            }
        }

        // Count-path differential: the lazy/count-only count must equal the
        // materialized solution count.
        if let Ok(c) = sparq_engine::count(&g, &q) {
            if c != sparq_full {
                count_mismatch += 1;
                report_repro(
                    &mut first_repro,
                    seed,
                    &q,
                    &ttl,
                    &format!("sparq count()={c} != sparq query().len()={sparq_full}"),
                );
            }
        }
    }

    // [OPUS-5] sq-qcnn.5: one field per `Compared` variant, so the log records WHICH
    // value-level oracle ran on how many cases — not merely that "bindings" were checked.
    let value_compared =
        compared_multiset + compared_ordered + compared_iso + compared_ask + compared_graph;
    println!(
        "fuzz[{category}] seeds {seed_start}..{} : checked={checked} skipped(unsupported)={skipped_unsupported} \
         value_compared={value_compared} compared(multiset)={compared_multiset} \
         compared(order-by)={compared_ordered} compared(iso)={compared_iso} \
         compared(ask)={compared_ask} compared(construct)={compared_graph} \
         skipped(row-choice)={skipped_row_choice} skipped(undecoded)={skipped_undecoded} \
         triage(iso)={triage_iso} \
         adjudicated(cross-family-eq)={adjudicated_cross_family} adjudicated(bnode-iri)={adjudicated_bnode_iri} \
         full_mismatch={full_mismatch} count_mismatch={count_mismatch}",
        seed_start + count
    );
    if let Some(r) = first_repro {
        println!("\nFIRST FAILING CASE:\n{r}");
        std::process::exit(1);
    }
    // NON-VACUITY GUARD (sq-j80vk): a shard that skipped every case as unsupported
    // compared NOTHING while still reporting success — which is exactly how a
    // generator regression would hide (each new category is only worth a shard while
    // both engines actually answer it). A run that asked for seeds and checked none
    // is a harness failure, not a pass.
    if count > 0 && checked == 0 {
        println!(
            "ERROR: fuzz[{category}] checked 0 of {count} seeds — the differential was \
             VACUOUS (every generated query was rejected by an engine)."
        );
        std::process::exit(1);
    }
    // VALUE-LEVEL NON-VACUITY GUARD (sq-qcnn.5). `checked` counts CARDINALITY comparisons; a
    // run can have plenty of those and still never compare a VALUE — which is exactly the
    // blindness this wiring removes, and it would report green. Every category emits at least
    // one value-comparable shape (`limit`'s window carries a total tiebreaker on half its
    // draws), so a substantial run with zero value comparisons is a harness regression.
    // Gated on a sample big enough for the per-category shape mix to show up, so a deliberate
    // one-seed repro run is never failed by it.
    const VALUE_VACUITY_FLOOR: u64 = 20;
    if checked >= VALUE_VACUITY_FLOOR && value_compared == 0 {
        println!(
            "ERROR: fuzz[{category}] compared {checked} CARDINALITIES and zero VALUES — the \
             value-level differential was VACUOUS (a same-cardinality wrong answer would have \
             passed)."
        );
        std::process::exit(1);
    }
}

/// Parses SPARQL-JSON results into a canonical, order-independent multiset of bindings
/// (each binding's keys sorted; the rows sorted) so two serialisations can be compared
/// regardless of row / key order.
fn bindings_multiset(json: &str) -> Vec<String> {
    let v: serde_json::Value = serde_json::from_str(json).expect("valid SPARQL JSON");
    let mut rows: Vec<String> = v["results"]["bindings"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .map(|b| {
                    let mut kv: Vec<(String, String)> = b
                        .as_object()
                        .unwrap()
                        .iter()
                        .map(|(k, val)| (k.clone(), val.to_string()))
                        .collect();
                    kv.sort();
                    format!("{kv:?}")
                })
                .collect()
        })
        .unwrap_or_default();
    rows.sort();
    rows
}

fn report_repro(slot: &mut Option<String>, seed: u64, q: &str, ttl: &str, msg: &str) {
    // One line per failing seed (machine-greppable), so differential runs across harness
    // MODES (baseline / mmap / compress / dict-spill) can compare failing-seed SETS, not
    // just counts — a mode is clean iff its set equals the baseline's.
    eprintln!("MISMATCH seed={seed}");
    if slot.is_none() {
        *slot = Some(format!(
            "seed={seed}\n{msg}\n--- query ---\n{q}\n--- graph ---\n{ttl}"
        ));
    }
}

#[cfg(test)]
mod tests {
    //! Regression tests for the differential-oracle fix (sq-eibog): the fuzzer's
    //! `=`/`!=`-over-mixed-literals class must be adjudicated against the SPARQL 1.1
    //! spec, NOT against Oxigraph's lenient cross-family reading. Anchored on the
    //! reproducing fuzz cases AND the W3C `sparql10/expr-equals` data.
    use super::*;
    use crate::neutral::neutral_term;
    use oxigraph::model::{Literal, NamedNode, Term};

    fn xsd(t: &str) -> NamedNode {
        NamedNode::new(format!("http://www.w3.org/2001/XMLSchema#{t}")).unwrap()
    }
    fn num(t: &str, v: &str) -> Term {
        Term::Literal(Literal::new_typed_literal(v, xsd(t)))
    }
    fn s(v: &str) -> Term {
        Term::Literal(Literal::new_simple_literal(v))
    }
    fn iri(local: &str) -> Term {
        Term::NamedNode(NamedNode::new(format!("http://ex/{local}")).unwrap())
    }

    /// SPARQL 1.1 §17.4.1.7: cross-family literal `=` (numeric vs string, etc.) is a
    /// TYPE ERROR — `spec_eq` returns `None`, so the row is dropped from BOTH `=` and
    /// `!=`. This is the crux of the fuzz divergence.
    #[test]
    fn cross_family_equality_is_a_type_error() {
        // integer vs plain string → error (the seed=15 / seed=182 class)
        assert_eq!(spec_eq(&num("integer", "71"), &s("s2")), None);
        assert_eq!(spec_eq(&num("int", "84"), &s("s2")), None);
        assert_eq!(spec_eq(&num("double", "15.0"), &s("s2")), None);
        // string vs integer constant → error (the seed=1320 `?v != 47` class)
        assert_eq!(spec_eq(&s("s1"), &num("integer", "47")), None);
        // integer vs the near-2^53 integer used by the generator — SAME family, decided
        assert_eq!(spec_eq(&num("integer", "1"), &num("int", "1")), Some(true));
        assert_eq!(
            spec_eq(&num("integer", "9007199254740992"), &num("integer", "7")),
            Some(false)
        );
    }

    /// EXACT numeric equality (the >2^53 case the fuzzer stresses): distinct integers
    /// that share an f64 must NOT be reported equal. Cross-type int/decimal/double value
    /// equality must hold. Ill-formed lexicals are type errors.
    #[test]
    fn exact_numeric_equality_beyond_f64() {
        // These four are DISTINCT integers that all collapse to the same f64.
        assert_eq!(
            numeric_eq_exact("9007199254740992", "9007199254740993"),
            Some(false)
        );
        assert_eq!(
            numeric_eq_exact("9007199254740992", "9007199254740994"),
            Some(false)
        );
        assert_eq!(
            numeric_eq_exact("9007199254740992", "9007199254740992"),
            Some(true)
        );
        // High-precision decimals that share an f64.
        assert_eq!(
            numeric_eq_exact("0.123456789012345678", "0.123456789012345679"),
            Some(false)
        );
        // Cross-type value equality (integer vs decimal vs double), incl. leading zeros,
        // signs, and exponents.
        assert_eq!(numeric_eq_exact("1", "1.0"), Some(true));
        assert_eq!(numeric_eq_exact("01", "1"), Some(true));
        assert_eq!(numeric_eq_exact("1", "1.0e0"), Some(true));
        assert_eq!(numeric_eq_exact("100", "1.0e2"), Some(true));
        assert_eq!(numeric_eq_exact("0.5", "5.0e-1"), Some(true));
        assert_eq!(numeric_eq_exact("-0", "0"), Some(true));
        assert_eq!(numeric_eq_exact("-23", "23"), Some(false));
        assert_eq!(numeric_eq_exact("15.0", "15"), Some(true));
        // Ill-formed → error.
        assert_eq!(numeric_eq_exact("abc", "1"), None);
        assert_eq!(numeric_eq_exact("1", ""), None);
        // The same case, end to end through spec_eq (the seed=11746 class).
        assert_eq!(
            spec_eq(
                &num("integer", "9007199254740993"),
                &num("integer", "9007199254740992")
            ),
            Some(false)
        );
    }

    /// Same-family and identity cases still DECIDE (true/false), so genuine
    /// over/under-exclusion in those would still trip the fuzzer.
    #[test]
    fn same_family_and_identity_decide() {
        assert_eq!(spec_eq(&s("s3"), &s("s2")), Some(false)); // strings compare
        assert_eq!(spec_eq(&s("s2"), &s("s2")), Some(true));
        assert_eq!(spec_eq(&iri("n0"), &s("s2")), Some(false)); // IRI vs literal: identity
        assert_eq!(spec_eq(&iri("n0"), &iri("n0")), Some(true));
        // unknown-datatype literal, same term → true (sameTerm rescues); different → error
        let my = |v: &str| {
            Term::Literal(Literal::new_typed_literal(
                v,
                NamedNode::new("http://ex/myType").unwrap(),
            ))
        };
        assert_eq!(spec_eq(&my("zzz"), &my("zzz")), Some(true));
        assert_eq!(spec_eq(&my("zzz"), &my("aaa")), None);
        assert_eq!(spec_eq(&my("zzz"), &s("zzz")), None); // cross-family (unknown vs str)
    }

    /// The W3C `sparql10/expr-equals` `!=` case (`result-eq2-2.ttl`) over `data-eq.ttl`.
    /// The bead-relevant, SPEC-UNAMBIGUOUS property — on which §17.4.1.7, the W3C
    /// reference result, and sparq's evaluator all AGREE — is that **every
    /// numeric-vs-non-numeric literal pair is a TYPE ERROR** (absent from the `!=`
    /// result): no numeric literal appears anywhere in `result-eq2-2.ttl`'s 12 rows.
    ///
    /// (The DAWG `!=` reference additionally makes the plain-string-vs-UNKNOWN-datatype
    /// corner — `"zzz" != "zzz"^^:myType` — a decided TRUE, giving 12. sparq's engine
    /// treats that specific corner as an open-world TYPE ERROR instead — a defensible
    /// reading that the *approved* manifest never exercises, since `:eq-2-2` there points
    /// at the `=` query. That corner is irrelevant to this fuzzer: the generator emits no
    /// unknown-datatype literals. This oracle deliberately mirrors sparq's engine so it
    /// never wrongly flags; we assert the agreed numeric-exclusion property below.)
    #[test]
    fn w3c_expr_equals_numeric_pairs_are_type_errors() {
        let numerics = [
            num("integer", "1"),
            num("integer", "01"),
            num("double", "1.0e0"),
            num("double", "1.0"),
            num("double", "1"),
        ];
        let non_numeric_literals = [
            Term::Literal(Literal::new_typed_literal(
                "zzz",
                NamedNode::new("http://example.org/things#myType").unwrap(),
            )),
            s("zzz"),
            s("1"),
        ];
        // Numeric vs any DIFFERENT-value numeric decides; numeric vs any non-numeric
        // literal is a TYPE ERROR (absent from BOTH `=` and `!=`), matching the W3C
        // reference which contains no numeric literal in its `!=` result.
        for a in &numerics {
            for b in &non_numeric_literals {
                assert_eq!(spec_eq(a, b), None, "{a} vs {b} must be a type error");
                assert_eq!(
                    spec_eq(b, a),
                    None,
                    "{b} vs {a} must be a type error (symmetric)"
                );
            }
        }
        // Numeric value-equality still DECIDES (all these are value 1).
        assert_eq!(
            spec_eq(&num("integer", "1"), &num("double", "1.0e0")),
            Some(true)
        );
    }

    /// End-to-end: the exact reproducing case (seed=15). sparq must equal the
    /// SPEC-correct count (2 — the two plain strings), NOT Oxigraph's lenient 8. The
    /// oracle's `spec_filter_count` must AGREE with sparq (so it is NOT flagged).
    #[test]
    fn seed15_numeric_vs_string_ne_matches_spec_not_oxigraph() {
        let ttl = r#"@prefix ex: <http://ex/> .
@prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
ex:n0 ex:val "84"^^xsd:int .
ex:n1 ex:val "15.0"^^xsd:double .
ex:n3 ex:val 71 .
ex:n4 ex:val "s3" .
ex:n5 ex:val "9007199254740992"^^xsd:integer .
ex:n6 ex:val "s1" .
ex:n11 ex:val "56"^^xsd:int .
ex:n13 ex:val 7 .
"#;
        let q = "PREFIX ex: <http://ex/>\nPREFIX xsd: <http://www.w3.org/2001/XMLSchema#>\nSELECT ?s WHERE { ?s ex:val ?v FILTER(?v != \"s2\") }";
        let g = sparq_core::Graph::load_str(ttl, "turtle").unwrap();
        let sparq_n = sparq_engine::query(&g, q).unwrap().len();
        let store = Store::new().unwrap();
        store
            .load_from_reader(oxigraph::io::RdfFormat::Turtle, ttl.as_bytes())
            .unwrap();
        let oxi = oxi_count(&store, q).unwrap();
        let spec = spec_filter_count(&store, q).expect("recognised equality filter");
        assert_eq!(spec, 2, "only the two plain strings survive != \"s2\"");
        assert_eq!(sparq_n, 2, "sparq is spec-correct");
        assert_eq!(oxi, 8, "oxigraph is lenient (keeps the 6 numeric rows)");
        assert_eq!(sparq_n, spec, "oracle: sparq matches spec → NOT flagged");
        assert_ne!(
            sparq_n, oxi,
            "would be a false-positive under the naive oracle"
        );
    }

    /// All-`xsd:integer` column vs a string constant (`?age != "s4"`, seed=182 class):
    /// EVERY row errors → spec count 0. sparq must return 0, not Oxigraph's full count.
    #[test]
    fn all_integer_column_vs_string_constant_is_empty() {
        let ttl =
            "@prefix ex: <http://ex/> .\nex:n0 ex:age 14 .\nex:n1 ex:age 42 .\nex:n2 ex:age 39 .\n";
        let q = "PREFIX ex: <http://ex/>\nSELECT ?s WHERE { ?s ex:age ?a FILTER(?a != \"s4\") }";
        let g = sparq_core::Graph::load_str(ttl, "turtle").unwrap();
        let sparq_n = sparq_engine::query(&g, q).unwrap().len();
        let store = Store::new().unwrap();
        store
            .load_from_reader(oxigraph::io::RdfFormat::Turtle, ttl.as_bytes())
            .unwrap();
        let spec = spec_filter_count(&store, q).unwrap();
        assert_eq!(spec, 0);
        assert_eq!(sparq_n, 0);
        assert_eq!(oxi_count(&store, q).unwrap(), 3, "oxigraph keeps all 3");
    }

    /// Integer CONSTANT vs the mixed column (`?v != 47`, seed=1320 class): numeric rows
    /// compare numerically (decided), string/unknown rows error (dropped).
    #[test]
    fn integer_constant_vs_mixed_column() {
        let ttl = r#"@prefix ex: <http://ex/> .
@prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
ex:n0 ex:val 47 .
ex:n1 ex:val 7 .
ex:n2 ex:val "47"^^xsd:int .
ex:n3 ex:val "s1" .
ex:n4 ex:val "s2" .
"#;
        let q = "PREFIX ex: <http://ex/>\nPREFIX xsd: <http://www.w3.org/2001/XMLSchema#>\nSELECT ?s WHERE { ?s ex:val ?v FILTER(?v != 47) }";
        let g = sparq_core::Graph::load_str(ttl, "turtle").unwrap();
        let sparq_n = sparq_engine::query(&g, q).unwrap().len();
        let store = Store::new().unwrap();
        store
            .load_from_reader(oxigraph::io::RdfFormat::Turtle, ttl.as_bytes())
            .unwrap();
        let spec = spec_filter_count(&store, q).unwrap();
        // 47 == 47 (int) and 47 == "47"^^xsd:int → those two are FALSE for !=; 7 → TRUE;
        // "s1"/"s2" → type error (dropped). So exactly 1 row survives (the `7`).
        assert_eq!(spec, 1);
        assert_eq!(sparq_n, 1);
    }

    /// GUARD: the oracle must NOT be a blanket "trust sparq". A genuine over-exclusion
    /// on a SAME-FAMILY comparison (where the spec DOES decide) is still caught: if
    /// sparq under-counted here, `spec_filter_count` (which errs only cross-family)
    /// would disagree and the case would be flagged.
    #[test]
    fn same_family_string_filter_stays_strict() {
        let ttl = "@prefix ex: <http://ex/> .\nex:n0 ex:val \"s1\" .\nex:n1 ex:val \"s2\" .\nex:n2 ex:val \"s3\" .\n";
        let q = "PREFIX ex: <http://ex/>\nSELECT ?s WHERE { ?s ex:val ?v FILTER(?v != \"s2\") }";
        let store = Store::new().unwrap();
        store
            .load_from_reader(oxigraph::io::RdfFormat::Turtle, ttl.as_bytes())
            .unwrap();
        // Pure-string column: spec and oxigraph AGREE (2), so no leniency to absorb.
        assert_eq!(spec_filter_count(&store, q).unwrap(), 2);
        assert_eq!(oxi_count(&store, q).unwrap(), 2);
    }

    /// The COMMITTED allowlist (bench/differential-divergences.json) must parse and
    /// enable exactly the adjudicated classes — pins the JSON ids ⟷ this comparator's
    /// detectors so neither can drift silently (sq-0iqzw). [FABLE-5]
    #[test]
    fn committed_allowlist_enables_exactly_the_adjudicated_classes() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../bench/differential-divergences.json"
        );
        let s = std::fs::read_to_string(path).expect("committed allowlist readable");
        let a = DivergenceAllowlist::from_json(&s, path);
        assert!(a.cross_family_eq_type_error, "sq-eibog class must be listed");
        assert!(a.bnode_iri_inequality, "sq-ai2wa class must be listed");
        // …and the file lists NOTHING this comparator lacks a detector for (an
        // undetectable entry would be a claimed-but-unenforced allowlisting).
        let v: serde_json::Value = serde_json::from_str(&s).unwrap();
        for c in v["classes"].as_array().expect("classes array") {
            let id = c["id"].as_str().expect("string id");
            assert!(
                // `update-*` classes belong to update_fuzz.rs, whose own
                // committed-allowlist test pins them the same way.
                id.starts_with("update-")
                    || ["cross-family-eq-type-error", "bnode-iri-inequality"].contains(&id),
                "class {id:?} in the committed file has no detector in fuzz.rs"
            );
            assert!(
                c["bead"].as_str().is_some_and(|b| b.starts_with("sq-")),
                "class {id:?} must cite its adjudication bead"
            );
        }
    }

    /// An empty / malformed / unknown-id allowlist means STRICT — the FILE, not this
    /// binary, is the source of truth for what is adjudicated; anything the file
    /// does not (recognisably) list fails the differential again.
    #[test]
    fn empty_or_malformed_allowlist_is_strict() {
        let a = DivergenceAllowlist::from_json(r#"{"version":1,"classes":[]}"#, "test");
        assert!(!a.cross_family_eq_type_error);
        assert!(!a.bnode_iri_inequality);
        let a = DivergenceAllowlist::from_json("not json", "test");
        assert!(!a.cross_family_eq_type_error && !a.bnode_iri_inequality);
        // An unknown id is ignored (no detector => that class stays strict), while a
        // recognised sibling entry still applies.
        let a = DivergenceAllowlist::from_json(
            r#"{"classes":[{"id":"some-future-class"},{"id":"bnode-iri-inequality"}]}"#,
            "test",
        );
        assert!(!a.cross_family_eq_type_error);
        assert!(a.bnode_iri_inequality);
    }

    /// sq-ai2wa detector: fires ONLY for a bnode-vs-IRI `=`/`!=` pairing (the narrow
    /// adjudicated class), never for literal comparisons (those are sq-eibog's spec
    /// sub-oracle or a genuine bug) and never for non-equality shapes.
    #[test]
    fn bnode_iri_detector_is_narrow() {
        let ttl = "@prefix ex: <http://ex/> .\nex:n0 ex:val _:b0 .\nex:n1 ex:val \"s1\" .\n";
        let store = Store::new().unwrap();
        store
            .load_from_reader(oxigraph::io::RdfFormat::Turtle, ttl.as_bytes())
            .unwrap();
        let q_iri =
            "PREFIX ex: <http://ex/>\nSELECT ?s WHERE { ?s ex:val ?v FILTER(?v != ex:n0) }";
        assert!(
            is_bnode_iri_inequality(&store, q_iri),
            "bnode-vs-IRI != must be detected"
        );
        let q_str =
            "PREFIX ex: <http://ex/>\nSELECT ?s WHERE { ?s ex:val ?v FILTER(?v != \"s2\") }";
        assert!(
            !is_bnode_iri_inequality(&store, q_str),
            "a literal RHS is NOT the sq-ai2wa class"
        );
        let q_range = "PREFIX ex: <http://ex/>\nSELECT ?s WHERE { ?s ex:age ?a FILTER(?a > 3) }";
        assert!(!is_bnode_iri_inequality(&store, q_range));
    }

    // ── sq-j80vk: invariants of the EXPANDED generator surface ──────────────────

    /// One `(graph, query)` case, built exactly as `run` builds it for a seed.
    fn case(seed: u64, category: &str) -> (String, String) {
        let mut rng = Rng::new(seed);
        let ttl = gen_graph(&mut rng);
        let q = gen_query(&mut rng, category);
        (ttl, q)
    }

    fn oxi_store(ttl: &str) -> Store {
        let store = Store::new().unwrap();
        store
            .load_from_reader(oxigraph::io::RdfFormat::Turtle, ttl.as_bytes())
            .unwrap();
        store
    }

    /// The RESULT FORM each category is contracted to emit — the same dispatch `run`
    /// makes on `query_form` before choosing a comparator. Written as data rather than
    /// derived from a generated query, so a category that CHANGED form (and so changed
    /// which oracle judges it) fails `generator_invariants_hold_for_every_category`
    /// instead of silently moving to another comparator. [SONNET-4.6] sq-qcnn.6
    fn expected_form(cat: &str) -> Form {
        match cat {
            "ask" => Form::Ask,
            "construct" => Form::Graph,
            _ => Form::Select,
        }
    }

    /// EVERY category must emit SPARQL 1.1 that the INDEPENDENT oracle (Oxigraph)
    /// parses AND evaluates — a query only sparq understands is not a differential at
    /// all. Pinning that sparq answers it too is what keeps a category from silently
    /// degrading into an all-`skipped(unsupported)` shard that reports green while
    /// comparing nothing.
    ///
    /// Each form is checked through the entry point `run` actually uses for it: a
    /// `CONSTRUCT` is NOT evaluable through `sparq_engine::query` at all (that entry
    /// point answers `SELECT`/`ASK` only), so asking it there would report a category
    /// as broken that the fuzzer answers correctly.
    #[test]
    fn every_category_is_evaluable_by_both_engines() {
        for cat in CATEGORIES {
            for seed in 0..60u64 {
                let (ttl, q) = case(seed, cat);
                let store = oxi_store(&ttl);
                let g = sparq_core::Graph::load_str(&ttl, "turtle").unwrap();
                match expected_form(cat) {
                    Form::Select => {
                        oxi_count(&store, &q).unwrap_or_else(|e| {
                            panic!("category {cat} seed {seed}: oxigraph rejected\n{q}\n{e}")
                        });
                        sparq_engine::query(&g, &q).unwrap_or_else(|e| {
                            panic!("category {cat} seed {seed}: sparq rejected\n{q}\n{e}")
                        });
                    }
                    Form::Ask => {
                        sparq_engine::ask(&g, &q).unwrap_or_else(|e| {
                            panic!("category {cat} seed {seed}: sparq rejected\n{q}\n{e}")
                        });
                        assert_eq!(
                            compare_ask(&g, &store, &q),
                            Ok(Compared::AskBoolean),
                            "category {cat} seed {seed}: not compared as a boolean\n{q}"
                        );
                    }
                    Form::Graph => {
                        sparq_engine::construct_or_describe(&g, &q).unwrap_or_else(|e| {
                            panic!("category {cat} seed {seed}: sparq rejected\n{q}\n{e}")
                        });
                        assert_eq!(
                            compare_graph(&g, &store, &q),
                            Ok(Compared::GraphIsomorphic),
                            "category {cat} seed {seed}: not compared as a triple set\n{q}"
                        );
                    }
                }
            }
        }
    }

    /// NON-VACUITY: a category whose queries always return ZERO rows compares 0 to 0
    /// forever. Every category must bind rows on a healthy share of seeds. (`graph`
    /// is the deliberate floor — the harness's dataset has no named graphs, so its
    /// two bare-`GRAPH` shapes are empty BY DESIGN and only the UNION / OPTIONAL
    /// compositions bind rows.)
    ///
    /// For the non-`SELECT` forms "binds a row" is read as "the answer is not the empty
    /// one": a non-empty constructed graph, and — the strictly stronger reading, because
    /// an `ASK` has only two possible answers — an `ASK` category that produces BOTH
    /// booleans. An `ASK` shard answering `true` on every seed would agree with any
    /// oracle that also always said `true`, which is exactly the blindness sq-qcnn.5
    /// removed from the comparator and that this keeps out of the generator.
    #[test]
    fn every_category_binds_rows_on_a_healthy_share_of_seeds() {
        const SEEDS: u64 = 120;
        for cat in CATEGORIES {
            let mut non_empty = 0u64;
            let (mut asked_true, mut asked_false) = (0u64, 0u64);
            for seed in 0..SEEDS {
                let (ttl, q) = case(seed, cat);
                let g = sparq_core::Graph::load_str(&ttl, "turtle").unwrap();
                match expected_form(cat) {
                    Form::Select => {
                        if sparq_engine::query(&g, &q).map(|r| r.len()).unwrap_or(0) > 0 {
                            non_empty += 1;
                        }
                    }
                    Form::Ask => match sparq_engine::ask(&g, &q) {
                        Ok(true) => {
                            asked_true += 1;
                            non_empty += 1;
                        }
                        Ok(false) => asked_false += 1,
                        Err(_) => {}
                    },
                    Form::Graph => {
                        if sparq_engine::construct_or_describe(&g, &q)
                            .map(|t| t.len())
                            .unwrap_or(0)
                            > 0
                        {
                            non_empty += 1;
                        }
                    }
                }
            }
            assert!(
                non_empty * 4 >= SEEDS,
                "category {cat}: only {non_empty}/{SEEDS} seeds bind ANY row — that \
                 shard's differential is near-vacuous"
            );
            if expected_form(cat) == Form::Ask {
                assert!(
                    asked_true > 0 && asked_false > 0,
                    "category {cat}: {asked_true} true / {asked_false} false over {SEEDS} \
                     seeds — an ASK shard that never changes its answer cannot catch a \
                     comparator that always agrees"
                );
            }
        }
    }

    /// The oracle-safety invariants documented on `gen_query`. They are what keeps
    /// each adjudicated-divergence sub-oracle (`parse_eq_filter` / `spec_filter_count`
    /// for sq-eibog, `order_by_vars`, the sq-ai2wa bnode detector) applicable ONLY to
    /// the shapes it actually models — a category that broke one would manufacture
    /// FALSE mismatches rather than find real ones.
    #[test]
    fn generator_invariants_hold_for_every_category() {
        for cat in CATEGORIES {
            for seed in 0..300u64 {
                let (_, q) = case(seed, cat);
                if !matches!(*cat, "equality" | "filter") {
                    assert!(
                        !q.contains("!=") && !q.contains(" = "),
                        "category {cat}: an `=`/`!=` text outside equality/filter is \
                         re-derived by the sq-eibog sub-oracle, which does not model \
                         this query\n{q}"
                    );
                }
                // [OPUS-5] sq-qcnn.5: `ORDER BY` is no longer confined to the `order`
                // category — `compare_select` derives the sort-key equivalence classes from
                // the clause itself. What must hold instead is that `order_by_vars` MODELS
                // every clause the generator emits: an unmodelled clause silently downgrades
                // the case to the order-INSENSITIVE multiset comparison, so that shape would
                // lose its ORDER oracle with no counter to show it.
                assert!(
                    order_by_vars(&q).is_some(),
                    "category {cat}: `order_by_vars` does not model this ORDER BY, so the \
                     case would silently lose its ORDER oracle\n{q}"
                );
                // [SONNET-4.6] sq-qcnn.6: the RESULT FORM decides which comparator judges
                // the case (`run` dispatches on `query_form` before the cardinality flow),
                // so each category must keep emitting the form `expected_form` records for
                // it. A category that drifted — an `ask` shape that came out a `SELECT`,
                // say — would be counted rather than compared as a boolean, with nothing
                // in the summary line to show it.
                assert_eq!(
                    query_form(&q),
                    expected_form(cat),
                    "category {cat}: unexpected result form\n{q}"
                );
                assert!(
                    !q.contains("_:"),
                    "category {cat}: a blank node makes the sq-ai2wa allowlist class \
                     LIVE — that needs its own adjudication first\n{q}"
                );
                if *cat == "subquery" {
                    assert!(
                        !q.contains("LIMIT"),
                        "category {cat}: a LIMIT inside a joined sub-select makes the \
                         row CHOICE arbitrary, so two conformant engines may return \
                         different (equally valid) counts\n{q}"
                    );
                }
            }
        }
    }

    /// The nightly shard matrix (`.github/workflows/differential.yml`) must carry ONE
    /// SHARD PER CATEGORY: running only `all` hides category-dense bugs (that
    /// workflow's header records the sq-lr2ii case), so a category without its own
    /// shard gets a fraction of the standing exploration it is supposed to get.
    #[test]
    fn every_category_has_a_nightly_shard() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../.github/workflows/differential.yml"
        );
        let wf = std::fs::read_to_string(path).expect("differential.yml readable");
        for cat in CATEGORIES {
            assert!(
                wf.contains(&format!("category: {cat},")),
                "no nightly shard for category {cat:?} in {path}"
            );
        }
    }

    /// sq-avd75: the `antijoin` category exists for ONE reason — to put rewrite (b)
    /// of the `algebra-rewrite` pass (#1735) under the nightly Oxigraph oracle. It
    /// only does that if the shapes it emits are shapes the recogniser ACTUALLY
    /// fires on: a grammar that merely *looks* like the `OPTIONAL … FILTER(!bound)`
    /// idiom but that the (deliberately conservative) pass declines would fuzz the
    /// un-rewritten path forever while reporting a green `antijoin` shard.
    ///
    /// So this asserts the split directly, on the PLAN: the three trigger shapes must
    /// reach an anti-join (`Minus` in `EXPLAIN`), and the two shapes the pass is
    /// documented to DECLINE — a theta `OPTIONAL` (its FILTER is lifted into
    /// `LeftJoin { expression: Some(…) }`, where `Minus(A, B)` is not equivalent) and
    /// an `A`/`B` pair sharing no variable — must NOT. Firing on either would be
    /// unsound, not merely unhelpful, so both directions are pinned.
    ///
    /// MUTATION GUARD: weaken a trigger shape so a firing condition fails (drop the
    /// shared `?s`, bind the `!bound` variable in A, put a FILTER in its OPTIONAL) and
    /// the `fired` half goes red; drop either decline shape and the `declined` half
    /// loses its case — assert the counts separately so neither can silently vanish.
    #[test]
    fn antijoin_category_is_the_rewrite_b_trigger() {
        let (mut fired, mut declined_theta, mut declined_disjoint) = (0u64, 0u64, 0u64);
        for seed in 0..200u64 {
            let (ttl, q) = case(seed, "antijoin");
            assert!(
                q.contains("OPTIONAL {") && q.contains("FILTER(!bound(?"),
                "seed {seed}: not the OPTIONAL+!bound idiom\n{q}"
            );
            let g = sparq_core::Graph::load_str(&ttl, "turtle").unwrap();
            let ex = sparq_engine::explain(&g, &q)
                .unwrap_or_else(|e| panic!("seed {seed}: EXPLAIN failed: {e}\n{q}"));
            // The two DECLINE shapes, by their markers: a SECOND `FILTER` (the one
            // inside the OPTIONAL — the theta shape) and the `ex:absent` subject (the
            // no-shared-variable shape).
            let theta = q.matches("FILTER").count() == 2;
            let disjoint = q.contains("ex:absent");
            if theta || disjoint {
                assert!(
                    !ex.contains("Minus"),
                    "seed {seed}: the pass must DECLINE this shape — `Minus(A, B)` is \
                     not equivalent to it\n{q}\n{ex}"
                );
                if theta {
                    declined_theta += 1;
                } else {
                    declined_disjoint += 1;
                }
            } else {
                assert!(
                    ex.contains("Minus"),
                    "seed {seed}: the trigger shape did NOT reach an anti-join, so this \
                     shard fuzzes the un-rewritten path and rewrite (b) stays \
                     oracle-less\n{q}\n{ex}"
                );
                fired += 1;
            }
        }
        assert!(fired > 0, "no seed exercised the REWRITTEN anti-join path");
        assert!(declined_theta > 0, "no seed exercised the theta DECLINE path");
        assert!(
            declined_disjoint > 0,
            "no seed exercised the no-shared-variable DECLINE path"
        );
    }

    // ── sq-qcnn.6: the TYPED-LITERAL data surface ───────────────────────────────

    /// The typed columns are only worth their cost if the generator actually REACHES
    /// every datatype family the bead names. Each probe below is a distinct family (and,
    /// for the ones whose point is a specific hazard, a distinct HAZARD: the two
    /// beyond-`i128` integers that differ only in their last digit; the three doubles XSD
    /// spells specially; the boolean lexical Oxigraph rewrites on load).
    ///
    /// MUTATION GUARD: delete any arm from `gen_graph`'s typed columns — or narrow one,
    /// e.g. drop `NaN` from the double arm or the `+1` twin from the huge-integer array —
    /// and exactly that probe goes red. The probes are matched on the emitted Turtle, so
    /// they cannot be satisfied by anything but the generator emitting that literal.
    ///
    /// Each probe is a set of fragments that must ALL appear on ONE triple line — a
    /// whole-document `contains` is too weak here, because `^^xsd:date` is a PREFIX of
    /// `^^xsd:dateTime`. The `ex:day` column, whose whole point is the UNTIMEZONED forms,
    /// is not probed by substring at all: a `+00:00` timezone ends in the same `:00"` a
    /// bare dateTime does, and a `+05:30`-timezoned date ends in the same `"^^xsd:date .`
    /// a bare one does, so that column's lexicals are CLASSIFIED (below) instead.
    #[test]
    fn typed_literal_columns_cover_every_datatype_family() {
        const PROBES: &[(&str, &[&str])] = &[
            ("timezoned dateTime (Z)", &[":00Z\"^^xsd:dateTime"]),
            ("offset-timezoned dateTime", &[":00+05:30\"^^xsd:dateTime"]),
            ("negative-offset dateTime", &[":00-08:00\"^^xsd:dateTime"]),
            ("xsd:yearMonthDuration", &["\"^^xsd:yearMonthDuration"]),
            ("xsd:dayTimeDuration", &["\"^^xsd:dayTimeDuration"]),
            ("negative dayTimeDuration", &["\"-P"]),
            ("xsd:duration", &["\"^^xsd:duration ."]),
            ("boolean (canonical true)", &["ex:flag true"]),
            ("boolean (canonical false)", &["ex:flag false"]),
            ("boolean (non-canonical 1)", &["ex:flag \"1\"^^xsd:boolean"]),
            ("boolean (non-canonical 0)", &["ex:flag \"0\"^^xsd:boolean"]),
            ("integer beyond i128::MAX", &["\"170141183460469231731687303715884105728\""]),
            ("its last-digit twin", &["\"170141183460469231731687303715884105729\""]),
            ("integer beyond i128::MIN", &["\"-170141183460469231731687303715884105729\""]),
            ("41-digit integer", &["\"99999999999999999999999999999999999999999\""]),
            ("double INF", &["\"INF\"^^xsd:double"]),
            ("double -INF", &["\"-INF\"^^xsd:double"]),
            ("double NaN", &["\"NaN\"^^xsd:double"]),
            ("negative zero double", &["\"-0.0E0\"^^xsd:double"]),
            ("xsd:float", &["\"^^xsd:float"]),
        ];
        let mut seen = vec![0u64; PROBES.len()];
        for seed in 0..400u64 {
            let ttl = gen_graph(&mut Rng::new(seed));
            for (i, (_, parts)) in PROBES.iter().enumerate() {
                if ttl
                    .lines()
                    .any(|line| parts.iter().all(|p| line.contains(p)))
                {
                    seen[i] += 1;
                }
            }
        }
        for (i, (what, parts)) in PROBES.iter().enumerate() {
            assert!(
                seen[i] > 0,
                "the generator never emitted {what} ({parts:?}) in 400 seeds — that \
                 datatype family has no standing differential"
            );
        }

        // `ex:day`, classified on the LEXICAL rather than by substring. Its three arms
        // differ only in what follows the `YYYY-MM-DD` date: nothing (bare date), a
        // timezone (`Z` / `±hh:mm`), or a `T`-prefixed time (bare dateTime).
        let (mut bare_date, mut tz_date, mut bare_datetime) = (0u64, 0u64, 0u64);
        for seed in 0..400u64 {
            let ttl = gen_graph(&mut Rng::new(seed));
            for line in ttl.lines() {
                let Some(rest) = line.split_once(" ex:day \"").map(|(_, r)| r) else {
                    continue;
                };
                let lex = rest.split('"').next().expect("a closing quote");
                assert!(lex.len() >= 10, "an ex:day lexical starts with a date: {lex}");
                match &lex[10..] {
                    "" => bare_date += 1,
                    tail if tail.starts_with('T') => bare_datetime += 1,
                    _ => tz_date += 1,
                }
            }
        }
        assert!(bare_date > 0, "no BARE (untimezoned) xsd:date was generated");
        assert!(tz_date > 0, "no TIMEZONED xsd:date was generated");
        assert!(
            bare_datetime > 0,
            "no BARE (untimezoned) xsd:dateTime was generated — the untimezoned temporal \
             is exactly what `ex:day` exists for"
        );
    }

    /// The load-bearing property of the typed columns: the two engines' STORES hand back
    /// DIFFERENT lexicals for the same RDF value (Oxigraph re-canonicalises on load,
    /// sparq does not), and the answers must still compare EQUAL — by value, through
    /// `sparq-difftest`, not by string.
    ///
    /// Both halves are asserted, and the first is what stops this from being vacuous: if
    /// no case ever differed lexically the comparison would be proving nothing about
    /// canonicalisation. The mutation guard runs the other way — a changed VALUE must
    /// still fail — so "compares equal" cannot degrade into "compares everything equal".
    #[test]
    fn typed_columns_agree_by_value_despite_lexical_recanonicalisation() {
        const COLUMNS: &[&str] = &["when", "day", "dur", "flag", "huge", "dbl"];
        const SEEDS: u64 = 150;
        let mut bound = vec![0u64; COLUMNS.len()];
        let mut lexically_differed = 0u64;
        // The RAW answer as each store spells it — the pre-canonicalisation view.
        let raw = |rows: &Solutions| -> BTreeSet<String> {
            rows.iter().map(|r| format!("{:?}", r)).collect()
        };
        for seed in 0..SEEDS {
            let ttl = gen_graph(&mut Rng::new(seed));
            let g = sparq_core::Graph::load_str(&ttl, "turtle").unwrap();
            let store = oxi_store(&ttl);
            for (i, col) in COLUMNS.iter().enumerate() {
                let q =
                    format!("PREFIX ex: <http://ex/>\nSELECT ?s ?v WHERE {{ ?s ex:{col} ?v }}");
                let sparq = sparq_solutions(&g, &q).expect("sparq answers the projection");
                let oxi = oxi_solutions(&store, &q).expect("oxigraph answers the projection");
                if !sparq.rows.is_empty() {
                    bound[i] += 1;
                }
                if raw(&sparq.rows) != raw(&oxi.rows) {
                    lexically_differed += 1;
                }
                assert_eq!(
                    compare_select(&g, &store, &q),
                    Ok(Compared::Multiset),
                    "seed {seed}: ex:{col} does not compare equal by VALUE\n{ttl}"
                );
            }
        }
        for (i, col) in COLUMNS.iter().enumerate() {
            assert!(
                bound[i] * 4 >= SEEDS,
                "ex:{col} bound rows on only {}/{SEEDS} seeds — that column is too sparse \
                 to be a standing differential",
                bound[i]
            );
        }
        assert!(
            lexically_differed > 0,
            "no generated case ever differed LEXICALLY between the two stores, so this \
             test proved nothing about value-canonicalisation"
        );
        // MUTATION GUARD: equality here is by value, not a blanket pass. Change one bound
        // VALUE and the multiset comparison must fail.
        let ttl = "@prefix ex: <http://ex/> .\n@prefix xsd: <http://www.w3.org/2001/XMLSchema#> .\n\
                   ex:n0 ex:flag \"1\"^^xsd:boolean .\n";
        let g = sparq_core::Graph::load_str(ttl, "turtle").unwrap();
        let store = oxi_store(ttl);
        let q = "PREFIX ex: <http://ex/>\nSELECT ?v WHERE { ?s ex:flag ?v }";
        let sparq = sparq_solutions(&g, q).unwrap();
        let oxi = oxi_solutions(&store, q).unwrap();
        assert_ne!(
            raw(&sparq.rows),
            raw(&oxi.rows),
            "the fixture must be one the two stores spell differently (\"1\" vs \"true\")"
        );
        assert!(multiset_equal(&sparq.rows, &oxi.rows));
        let mut wrong = sparq.rows.clone();
        wrong[0].insert(
            "v".to_string(),
            sparq_difftest::Term::Literal {
                lexical: "false".to_string(),
                datatype: "http://www.w3.org/2001/XMLSchema#boolean".to_string(),
                lang: None,
            },
        );
        assert!(
            !multiset_equal(&wrong, &oxi.rows),
            "`true` and `false` are different VALUES — the comparator must not absorb that"
        );
    }

    /// Every `GROUP_CONCAT` a generated query contains is either over a SINGLETON group or
    /// read through an order-invariant operation. The order of a group's members is left
    /// unspecified (§11), so a projected multi-member concatenation is not a function of
    /// the query — it is excluded from the generator rather than compared and hoped for.
    ///
    /// The positive half runs on a fixture that really does contain a MULTI-MEMBER group
    /// (`ex:n0` has three outgoing `ex:p` edges — `BINDING_TTL`'s chain gives every subject
    /// exactly one, which would have made the `STRLEN` case vacuous), so "the generator's
    /// shapes agree across engines" is asserted where order could have bitten.
    ///
    /// The exclusion itself is pinned STRUCTURALLY, on [`group_concat_is_order_invariant`]:
    /// the excluded raw multi-member projection must classify as unsafe, and no category
    /// may emit a query that does. Asserting instead that the excluded shape actually
    /// DISAGREES across the two engines would be asserting a nondeterminism — both engines
    /// may legitimately pick the same unspecified order — so it is not asserted here.
    #[test]
    fn group_concat_is_generated_only_in_order_independent_shapes() {
        const TTL: &str = r#"@prefix ex: <http://ex/> .
ex:n0 ex:p ex:n1 . ex:n0 ex:p ex:n2 . ex:n0 ex:p ex:n3 .
ex:n1 ex:p ex:n2 .
ex:n0 ex:age 10 . ex:n1 ex:age 20 . ex:n2 ex:age 30 .
"#;
        let g = sparq_core::Graph::load_str(TTL, "turtle").unwrap();
        let store = oxi_store(TTL);
        // Precondition: the `ex:p` grouping really is multi-member, so the `STRLEN` shape
        // below is exercising order-independence and not a one-element concatenation.
        let members = "PREFIX ex: <http://ex/>\n\
             SELECT ?s (COUNT(?o) AS ?c) WHERE { ?s ex:p ?o } GROUP BY ?s";
        let biggest = sparq_engine::query(&g, members)
            .unwrap()
            .rows
            .iter()
            .filter_map(|r| r[1].as_ref().map(|t| t.to_string()))
            .filter(|c| c.starts_with("\"3\""))
            .count();
        assert_eq!(biggest, 1, "the fixture must contain a three-member group");

        let singleton = "PREFIX ex: <http://ex/>\n\
             SELECT ?s (GROUP_CONCAT(STR(?a)) AS ?g) WHERE { ?s ex:age ?a } GROUP BY ?s";
        let strlen = "PREFIX ex: <http://ex/>\n\
             SELECT ?s (STRLEN(GROUP_CONCAT(STR(?o))) AS ?l) WHERE { ?s ex:p ?o } GROUP BY ?s";
        for q in [singleton, strlen] {
            assert_eq!(
                compare_select(&g, &store, q),
                Ok(Compared::Multiset),
                "an order-independent GROUP_CONCAT shape must compare equal\n{q}"
            );
        }
        // The classifier is red on the shape the generator deliberately does NOT emit:
        // the same multi-member group, projected raw.
        let multi = "PREFIX ex: <http://ex/>\n\
             SELECT ?s (GROUP_CONCAT(STR(?o)) AS ?g) WHERE { ?s ex:p ?o } GROUP BY ?s";
        assert!(
            !group_concat_is_order_invariant(multi),
            "a raw multi-member GROUP_CONCAT projection must classify as order-dependent"
        );
        assert!(
            group_concat_is_order_invariant(singleton) && group_concat_is_order_invariant(strlen),
            "the two emitted shapes must classify as order-invariant"
        );
        // …and no category may emit an unsafe one.
        for cat in CATEGORIES {
            for seed in 0..300u64 {
                let (_, q) = case(seed, cat);
                assert!(
                    group_concat_is_order_invariant(&q),
                    "category {cat} seed {seed}: a GROUP_CONCAT that is neither \
                     order-independent nor over a singleton group\n{q}"
                );
            }
        }
    }

    /// True when EVERY `GROUP_CONCAT` in `q` (a query may carry one in `SELECT` and another
    /// in `HAVING`) sits in a shape whose value is a function of the query rather than of
    /// the members' unspecified order: read through `STRLEN`, or over the singleton `ex:age`
    /// group. Vacuously true for a query with no `GROUP_CONCAT` at all.
    fn group_concat_is_order_invariant(q: &str) -> bool {
        let singleton_group = q.contains("WHERE { ?s ex:age ?a } GROUP BY ?s");
        q.match_indices("GROUP_CONCAT")
            .all(|(i, _)| singleton_group || q[..i].ends_with("STRLEN("))
    }

    // ── the VALUE-LEVEL differential (sq-qcnn.5: `compare_select` / `compare_ask` /
    //    `compare_graph`, all through `sparq-difftest`) ─────────────────────────────

    /// A graph with chains (`ex:p`), a value column and names — enough for the
    /// path / aggregate / subquery / values / bind shapes to bind real rows.
    ///
    /// [SONNET-4.6] sq-qcnn.6: it also carries at least one literal from each typed family the
    /// generator emits, including the lexicals Oxigraph re-canonicalises on load
    /// (`"1"^^xsd:boolean`, `"1.5E3"^^xsd:double`) and the specials, so the `typed`
    /// category is value-compared against real rows here rather than against an
    /// accidentally-empty answer.
    const BINDING_TTL: &str = r#"@prefix ex: <http://ex/> .
@prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
ex:n0 ex:p ex:n1 . ex:n0 ex:age 10 . ex:n0 ex:name "nm0" .
ex:n1 ex:p ex:n2 . ex:n1 ex:age 20 . ex:n1 ex:name "nm1" .
ex:n2 ex:p ex:n3 . ex:n2 ex:age 30 . ex:n2 ex:name "nm2" .
ex:n3 ex:age 40 . ex:n3 ex:name "nm3" .
ex:n0 ex:when "2024-03-05T12:30:00Z"^^xsd:dateTime .
ex:n1 ex:when "2011-07-02T01:02:00+05:30"^^xsd:dateTime .
ex:n0 ex:day "2024-03-05"^^xsd:date .
ex:n1 ex:day "2019-11-30T23:59:00"^^xsd:dateTime .
ex:n0 ex:dur "P1Y2M"^^xsd:yearMonthDuration .
ex:n1 ex:dur "P3DT4H"^^xsd:dayTimeDuration .
ex:n2 ex:dur "P1Y2M3DT4H"^^xsd:duration .
ex:n0 ex:flag true . ex:n1 ex:flag "0"^^xsd:boolean . ex:n2 ex:flag "1"^^xsd:boolean .
ex:n0 ex:huge "170141183460469231731687303715884105728"^^xsd:integer .
ex:n1 ex:huge "170141183460469231731687303715884105729"^^xsd:integer .
ex:n0 ex:dbl "INF"^^xsd:double . ex:n1 ex:dbl "-INF"^^xsd:double .
ex:n2 ex:dbl "NaN"^^xsd:double . ex:n3 ex:dbl "1.5E3"^^xsd:double .
"#;

    fn binding_case(q: &str) -> (Solutions, Solutions) {
        let g = sparq_core::Graph::load_str(BINDING_TTL, "turtle").unwrap();
        let store = oxi_store(BINDING_TTL);
        (
            sparq_solutions(&g, q).expect("sparq solutions").rows,
            oxi_solutions(&store, q).expect("oxigraph solutions").rows,
        )
    }

    /// One neutral solution, from `(variable, N-Triples-ish term)` pairs.
    fn nsol(pairs: &[(&str, Term)]) -> Solution {
        neutral_solution(pairs.iter().map(|(v, t)| (*v, t)))
    }
    fn nint(v: &str) -> Term {
        Term::Literal(Literal::new_typed_literal(v, xsd("integer")))
    }
    fn niri(local: &str) -> Term {
        Term::NamedNode(NamedNode::new(format!("http://ex/{local}")).unwrap())
    }

    /// MUTATION PROOF that the oracle is not vacuous: a bare `COUNT(*)` returns ONE
    /// row whatever the count is, so the cardinality differential cannot see a wrong
    /// count at all. Perturbing only the VALUE — same row, same variable, same
    /// cardinality — must make the comparator fail. Were the cross-check still
    /// count-only, this assertion would not hold.
    #[test]
    fn same_cardinality_wrong_aggregate_value_is_caught() {
        let q = "PREFIX ex: <http://ex/>\nSELECT (COUNT(*) AS ?c) WHERE { ?s ex:age ?a }";
        let (sparq, oxi) = binding_case(q);
        assert_eq!(sparq.len(), 1, "a bare aggregate is always exactly one row");
        assert!(
            multiset_equal(&sparq, &oxi),
            "both engines agree on the real answer"
        );

        // sparq "returns" 3 where the fixture's answer is 4 — one row either way.
        let wrong = vec![nsol(&[("c", nint("3"))])];
        assert!(!multiset_equal(&wrong, &sparq), "the mutation must change the VALUE");
        assert_eq!(wrong.len(), sparq.len(), "the mutation preserves cardinality");
        assert!(
            !multiset_equal(&wrong, &oxi),
            "a wrong aggregate VALUE at the right cardinality must fail the oracle"
        );
    }

    /// The same mutation proof for a BINDING rather than an aggregate: a property path
    /// that lands on the wrong endpoint keeps the row count and changes only `?o`.
    #[test]
    fn same_cardinality_wrong_path_endpoint_is_caught() {
        let q = "PREFIX ex: <http://ex/>\nSELECT * WHERE { ?s ex:p/ex:p ?o }";
        let (sparq, oxi) = binding_case(q);
        assert!(!sparq.is_empty(), "the fixture must bind rows");
        assert!(
            multiset_equal(&sparq, &oxi),
            "both engines agree on the real answer"
        );

        // Redirect one endpoint to a node that IS in the graph (so only the VALUE is
        // wrong, not the shape of the answer).
        let mut wrong = sparq.clone();
        let redirected = niri("n0");
        assert_ne!(
            canonical_key(wrong[0].get("o").expect("?o is projected")),
            canonical_key(&neutral_term(&redirected)),
            "the mutation must change the VALUE"
        );
        wrong[0].insert("o".to_string(), neutral_term(&redirected));
        assert_eq!(wrong.len(), sparq.len(), "the mutation preserves cardinality");
        assert!(
            !multiset_equal(&wrong, &oxi),
            "a wrong path ENDPOINT at the right cardinality must fail the oracle"
        );
    }

    /// SPARQL is bag semantics: same cardinality, same DISTINCT rows, different
    /// multiplicities is still a wrong answer, so the comparator must not degrade
    /// into set comparison.
    #[test]
    fn duplicate_multiplicity_is_significant() {
        let row = |local: &str| nsol(&[("x", niri(local))]);
        let a = vec![row("n0"), row("n0"), row("n1")];
        let b = vec![row("n0"), row("n1"), row("n1")];
        assert_eq!(a.len(), b.len());
        assert!(!multiset_equal(&a, &b));
        // ...and the repro diff names a row unique to each side rather than shrugging.
        let detail = multiset_detail("solution MULTISET differs", &a, &b);
        assert!(detail.contains("only in sparq"), "{detail}");
        assert!(detail.contains("only in oxi"), "{detail}");
    }

    /// `order_by_vars` must MODEL exactly the clause shapes the generator emits — plain
    /// variables, `ASC(…)`, `DESC(…)`, in key order — and DECLINE (return `None`) anything
    /// else, because a guessed sort key that is FINER than the query's would manufacture
    /// false mismatches on legitimately-permuted tie runs.
    #[test]
    fn order_by_vars_models_the_generated_clauses_and_declines_the_rest() {
        let q = |suffix: &str| format!("SELECT ?a ?o WHERE {{ ?s ex:age ?a }} {suffix}");
        assert_eq!(order_by_vars(&q("")).unwrap(), Vec::<String>::new());
        assert_eq!(order_by_vars(&q("ORDER BY ?a")).unwrap(), vec!["a"]);
        assert_eq!(order_by_vars(&q("ORDER BY DESC(?a)")).unwrap(), vec!["a"]);
        assert_eq!(order_by_vars(&q("ORDER BY ASC(?a)")).unwrap(), vec!["a"]);
        // Key ORDER is significant and preserved; modifiers after the clause are excluded.
        assert_eq!(
            order_by_vars(&q("ORDER BY ?o DESC(?a) LIMIT 3 OFFSET 1")).unwrap(),
            vec!["o", "a"]
        );
        assert_eq!(
            order_by_vars(&q("ORDER BY ?a ?o OFFSET 2")).unwrap(),
            vec!["a", "o"]
        );
        // Unmodelled: an expression, a function call, a bare constant.
        assert!(order_by_vars(&q("ORDER BY (?a + 1)")).is_none());
        assert!(order_by_vars(&q("ORDER BY STRLEN(STR(?o))")).is_none());
        assert!(order_by_vars(&q("ORDER BY 3")).is_none());
    }

    /// `is_total_order` decides whether a `LIMIT` window is differential-testable at VALUE
    /// level: only a sort key covering every PROJECTED variable makes tied rows IDENTICAL
    /// rows, so that which of them the window keeps cannot change the answer.
    #[test]
    fn is_total_order_requires_covering_every_projected_variable() {
        let projected: BTreeSet<String> = ["a", "o"].iter().map(|s| s.to_string()).collect();
        assert!(is_total_order(&["a".into(), "o".into()], &projected));
        assert!(
            is_total_order(&["o".into(), "a".into(), "s".into()], &projected),
            "a SUPERSET of the projection is still total over it"
        );
        assert!(
            !is_total_order(&["a".into()], &projected),
            "a strict SUBSET leaves tie runs whose row choice is arbitrary"
        );
        assert!(!is_total_order(&[], &projected), "no ORDER BY is not a total order");
        // A projection nothing ever binds cannot be ordered totally by an empty key either.
        assert!(!is_total_order(&[], &BTreeSet::new()));
    }

    /// PREMISE-CORRECTION #2 of `research/differential-testing-value-level.md`, as a mutation
    /// guard. `ORDER BY` is a PARTIAL order: two conformant engines may permute rows that are
    /// tied on every sort key, so element-for-element sequence equality — what the previous
    /// `check_ordered` demanded — is WRONG in general and would flag a spurious mismatch.
    /// `order_by_equal` must accept a within-tie-run permutation and still reject a CROSS-run
    /// reordering (a real order violation) and a changed VALUE.
    #[test]
    fn order_by_compares_up_to_within_tie_run_permutation() {
        let row = |a: &str, o: &str| nsol(&[("a", nint(a)), ("o", niri(o))]);
        let sparq = vec![row("10", "n1"), row("10", "n2"), row("20", "n3")];
        // Tied on ?a=10, ?o permuted: EQUAL (this is the case the old check would have failed).
        let oxi = vec![row("10", "n2"), row("10", "n1"), row("20", "n3")];
        assert!(order_by_equal(&sparq, &oxi, &["a"]));
        // Cross-run reordering: a genuine ORDER violation.
        let reordered = vec![row("20", "n3"), row("10", "n1"), row("10", "n2")];
        assert!(!order_by_equal(&sparq, &reordered, &["a"]));
        // A changed VALUE inside a tie run is still caught (the run must match as a multiset).
        let wrong_value = vec![row("10", "n1"), row("10", "n9"), row("20", "n3")];
        assert!(!order_by_equal(&sparq, &wrong_value, &["a"]));
        // With the sort key made TOTAL, the comparison is exact sequence equality: the very
        // same permutation that was legal above is now a mismatch.
        assert!(!order_by_equal(&sparq, &oxi, &["a", "o"]));
        assert!(order_by_equal(&sparq, &sparq, &["a", "o"]));
        // The repro message names the sort key and the first differing row.
        let detail = order_detail(&sparq, &reordered, &["a"]);
        assert!(detail.contains("first differing sort key at row 0"), "{detail}");
    }

    /// End-to-end on both `order` shapes over a real pair of engines: the total-sort-key shape
    /// and the SUBSET-sort-key shape must BOTH be compared (not skipped), and the subset shape
    /// must actually contain tie runs — otherwise the equivalence-class path is nominal.
    #[test]
    fn both_order_shapes_are_value_compared_and_the_subset_shape_has_ties() {
        let g = sparq_core::Graph::load_str(BINDING_TTL, "turtle").unwrap();
        let store = oxi_store(BINDING_TTL);
        // Total sort key over the projection (+ LIMIT): compared, and windowed.
        let total = "PREFIX ex: <http://ex/>\nSELECT ?a WHERE { ?s ex:age ?a } ORDER BY ?a LIMIT 2";
        assert_eq!(compare_select(&g, &store, total), Ok(Compared::Ordered));
        // Subset sort key: `?o` is projected but NOT sorted on, so tie runs on `?a` may
        // permute their `?o` across engines. This fixture happens to give each subject one
        // `ex:p` edge (hence no ties), so the presence of real tie runs is asserted below on
        // the generator's own random graphs, where a subject can have several edges.
        let subset =
            "PREFIX ex: <http://ex/>\nSELECT ?a ?o WHERE { ?s ex:age ?a . ?s ex:p ?o } ORDER BY ?a";
        assert_eq!(compare_select(&g, &store, subset), Ok(Compared::Ordered));

        // The GENERATOR's `order` category must produce both shapes, and the subset one must
        // exhibit a real tie run (two rows sharing a sort key) on a healthy share of seeds.
        let (mut total_shapes, mut subset_shapes, mut with_ties) = (0u64, 0u64, 0u64);
        for seed in 0..120u64 {
            let (ttl, q) = case(seed, "order");
            if q.contains("?o") {
                subset_shapes += 1;
                let g = sparq_core::Graph::load_str(&ttl, "turtle").unwrap();
                let rows = sparq_solutions(&g, &q)
                    .expect("sparq answers the order shape")
                    .rows;
                let keys: Vec<Option<String>> =
                    rows.iter().map(|r| r.get("a").map(canonical_key)).collect();
                if keys.windows(2).any(|w| w[0] == w[1]) {
                    with_ties += 1;
                }
            } else {
                total_shapes += 1;
            }
        }
        assert!(total_shapes > 0, "the total-sort-key shape is never generated");
        assert!(subset_shapes > 0, "the subset-sort-key shape is never generated");
        assert!(
            with_ties * 4 >= subset_shapes,
            "only {with_ties}/{subset_shapes} subset-sort seeds contain a TIE RUN — the \
             sort-key-equivalence-class path is barely exercised"
        );
    }

    /// `LIMIT` policy (§2.2 of the design record), both halves. The BARE window is
    /// cardinality-only and lands in the COUNTED row-choice bucket; the same window WITH a
    /// total tiebreaker is fully value-compared. The generator must emit both.
    #[test]
    fn a_limit_window_is_value_compared_only_with_a_total_tiebreaker() {
        let g = sparq_core::Graph::load_str(BINDING_TTL, "turtle").unwrap();
        let store = oxi_store(BINDING_TTL);
        let bare = "PREFIX ex: <http://ex/>\nSELECT * WHERE { ?s ex:p ?o . ?o ex:age ?a } \
                    LIMIT 2 OFFSET 1";
        assert_eq!(
            compare_select(&g, &store, bare),
            Ok(Compared::SkippedRowChoice),
            "without a total order the surviving rows are not a function of the query"
        );
        let tiebroken = "PREFIX ex: <http://ex/>\nSELECT * WHERE { ?s ex:p ?o . ?o ex:age ?a } \
                         ORDER BY ?s ?o ?a LIMIT 2 OFFSET 1";
        assert_eq!(
            compare_select(&g, &store, tiebroken),
            Ok(Compared::Ordered),
            "a total tiebreaker determines the window, so the VALUES are comparable"
        );
        // A NON-total order plus a window stays a row-choice skip (the tie run could straddle
        // the window boundary), which is what makes the tiebreaker load-bearing rather than
        // decorative.
        let partial = "PREFIX ex: <http://ex/>\nSELECT * WHERE { ?s ex:p ?o . ?o ex:age ?a } \
                       ORDER BY ?a LIMIT 2";
        assert_eq!(
            compare_select(&g, &store, partial),
            Ok(Compared::SkippedRowChoice)
        );
        // The generator emits BOTH shapes for the `limit` category.
        let (mut bare_n, mut tiebroken_n) = (0u64, 0u64);
        for seed in 0..120u64 {
            let (_, q) = case(seed, "limit");
            if q.contains("ORDER BY") {
                tiebroken_n += 1;
            } else {
                bare_n += 1;
            }
        }
        assert!(bare_n > 0, "the bare early-termination window is never generated");
        assert!(
            tiebroken_n > 0,
            "the total-tiebreaker window is never generated, so no LIMIT shape is ever \
             value-checked"
        );
    }

    /// [SONNET-4.6] A window's TOTALITY must be read off the result HEADER, never off the
    /// bindings of the ALREADY-SLICED rows.
    ///
    /// A projected variable unbound in every SURVIVING row has vanished from those rows — even
    /// though it may have been bound, to a different value, on a tied candidate row the window
    /// chose against. Reading coverage off the sliced rows loses it, calls the sort key TOTAL
    /// over a projection it does not cover, and value-compares a window whose row choice
    /// SPARQL leaves to the implementation.
    ///
    /// Both halves are asserted: the hand-built pair states that hazard exactly (a surviving
    /// row that dropped the discriminator beside a tied candidate that kept it), and the
    /// end-to-end fixture then pins the classification `compare_select` actually reaches, on a
    /// real pair of engines, for a query whose surviving rows leave `?o` unbound.
    #[test]
    fn window_totality_comes_from_the_header_not_the_sliced_rows() {
        // The exact hazard, stated on the two views of one answer: a `SELECT ?a ?o` whose
        // surviving row binds only `?a`, next to a tied candidate row binding `?o` that the
        // window could equally well have kept.
        let header = || vec!["a".to_string(), "o".to_string()];
        let survivor = Answer {
            vars: header(),
            rows: vec![nsol(&[("a", nint("7"))])],
        };
        let tied_candidate = Answer {
            vars: header(),
            rows: vec![nsol(&[("a", nint("7")), ("o", niri("m2"))])],
        };
        let sliced: BTreeSet<String> = survivor
            .rows
            .iter()
            .flat_map(|sol| sol.keys().cloned())
            .collect();
        // Read off the SLICED rows alone the discriminator has vanished and `ORDER BY ?a`
        // looks total — the defect. Read off the HEADER it survives, and the window is
        // correctly refused.
        assert_eq!(sliced, ["a".to_string()].into_iter().collect());
        assert!(is_total_order(&["a".to_string()], &sliced));
        let projected = result_vars(&survivor, &survivor).expect("the headers agree");
        assert_eq!(
            projected,
            ["a".to_string(), "o".to_string()].into_iter().collect(),
            "a projected-but-unbound variable must survive in the coverage set"
        );
        assert!(!is_total_order(&["a".to_string()], &projected));
        // ...and it survives whether or not the ROWS bind it: coverage comes from the agreed
        // header, so the survivor's rows dropping `?o` cannot shrink it.
        assert_eq!(result_vars(&survivor, &tied_candidate), Ok(projected));

        // End to end. Both candidate rows tie on `?a`; `?o` is projected and unbound in every
        // surviving row, so the window must land in the COUNTED row-choice bucket.
        let ttl = "@prefix ex: <http://ex/> .\nex:m0 ex:age 7 .\nex:m1 ex:age 7 .\n";
        let g = sparq_core::Graph::load_str(ttl, "turtle").unwrap();
        let store = oxi_store(ttl);
        let windowed = "PREFIX ex: <http://ex/>\nSELECT ?a ?o \
                        WHERE { ?s ex:age ?a OPTIONAL { ?s ex:p ?o } } ORDER BY ?a LIMIT 1";
        // Precondition: nothing in the compared rows records that `?o` was projected at all,
        // so a bound-pair coverage set would have called `ORDER BY ?a` total here.
        for answer in [
            sparq_solutions(&g, windowed).expect("sparq answers"),
            oxi_solutions(&store, windowed).expect("oxigraph answers"),
        ] {
            assert!(answer.vars.contains(&"o".to_string()), "`?o` is projected");
            assert!(
                answer.rows.iter().all(|r| !r.contains_key("o")),
                "no surviving row binds `?o`"
            );
        }
        assert_eq!(
            compare_select(&g, &store, windowed),
            Ok(Compared::SkippedRowChoice),
            "the sort key does not cover the projection, so the row choice is arbitrary"
        );
        // The rule bites ONLY on the window: drop `LIMIT` and the same query is still fully
        // value-compared, so this is not a blanket skip.
        let unwindowed = "PREFIX ex: <http://ex/>\nSELECT ?a ?o \
                          WHERE { ?s ex:age ?a OPTIONAL { ?s ex:p ?o } } ORDER BY ?a";
        assert_eq!(
            compare_select(&g, &store, unwindowed),
            Ok(Compared::Ordered)
        );
    }

    /// [SONNET-4.6] A HEADER-only disagreement is a MISMATCH, not a pass and not a skip.
    ///
    /// A projected-but-unbound variable exists ONLY in the result header — `rows` records bound
    /// pairs — so an engine that omits (or invents) a projected variable can produce row maps
    /// identical to the other engine's. Every comparator downstream of the header sees only
    /// those rows: the multiset and `ORDER BY` paths would return a passing verdict, and the
    /// window path would return the `skipped(row-choice)` bucket. All three read as "no defect
    /// here", so the headers must be compared BEFORE any of them.
    ///
    /// The mutation is stated directly: one `Answer` drops `?o` from its header while both row
    /// maps stay byte-identical.
    #[test]
    fn a_header_only_projection_difference_is_a_mismatch() {
        // `?o` is projected; no row binds it (the `OPTIONAL` never matched). The rows below are
        // therefore identical on both sides — the header is the ONLY difference.
        let rows = || vec![nsol(&[("a", nint("7"))]), nsol(&[("a", nint("7"))])];
        let full = Answer {
            vars: vec!["a".to_string(), "o".to_string()],
            rows: rows(),
        };
        let dropped = Answer {
            vars: vec!["a".to_string()],
            rows: rows(),
        };
        assert!(
            multiset_equal(&full.rows, &dropped.rows),
            "premise: the ROWS agree, so only a header comparison can see this defect"
        );

        let where_ = "WHERE { ?s ex:age ?a OPTIONAL { ?s ex:p ?o } }";
        for (path, q) in [
            ("multiset", format!("SELECT ?a ?o {where_}")),
            ("ordered", format!("SELECT ?a ?o {where_} ORDER BY ?a")),
            // Without the header check this one reaches `skipped(row-choice)` — a COUNTED
            // non-testable case, which is exactly as wrong as a pass.
            ("window", format!("SELECT ?a ?o {where_} ORDER BY ?a LIMIT 1")),
        ] {
            for (a, b) in [(&full, &dropped), (&dropped, &full)] {
                let err = compare_answers(a, b, &q)
                    .expect_err(&format!("{path}: a dropped projected variable must not pass"));
                assert!(err.contains("result HEADER differs"), "{path}: {err}");
                assert!(
                    err.contains("\"o\""),
                    "{path}: the repro must name the dropped variable: {err}"
                );
            }
        }

        // Not a blanket `Err`: agreeing headers over the same rows still compare normally, and
        // projection ORDER alone is not a mismatch (`SELECT *` leaves it implementation-defined,
        // and the compared row maps are keyed by variable name).
        let permuted = Answer {
            vars: vec!["o".to_string(), "a".to_string()],
            rows: rows(),
        };
        for other in [&full, &permuted] {
            assert_eq!(
                compare_answers(&full, other, &format!("SELECT ?a ?o {where_}")),
                Ok(Compared::Multiset)
            );
        }
    }

    /// The value-level oracle must actually RUN on every category rather than skip it — a
    /// comparator that returned `SkippedRowChoice` for everything would be a silently green
    /// differential. This is the per-category counterpart of `run`'s value-vacuity guard.
    #[test]
    fn every_category_is_value_compared() {
        let g = sparq_core::Graph::load_str(BINDING_TTL, "turtle").unwrap();
        let store = oxi_store(BINDING_TTL);
        for cat in CATEGORIES {
            let mut compared = 0;
            for seed in 0..60u64 {
                let mut rng = Rng::new(seed);
                let _ = gen_graph(&mut rng); // keep the generator's draw sequence
                let q = gen_query(&mut rng, cat);
                // [SONNET-4.6] sq-qcnn.6: the non-`SELECT` forms have no cardinality flow
                // to gate on — they are value-compared directly by `run`, so they are
                // value-compared directly here.
                match expected_form(cat) {
                    Form::Ask => {
                        assert_eq!(
                            compare_ask(&g, &store, &q),
                            Ok(Compared::AskBoolean),
                            "category {cat} seed {seed}: the ASK was not compared\n{q}"
                        );
                        compared += 1;
                        continue;
                    }
                    Form::Graph => {
                        assert_eq!(
                            compare_graph(&g, &store, &q),
                            Ok(Compared::GraphIsomorphic),
                            "category {cat} seed {seed}: the graph was not compared\n{q}"
                        );
                        compared += 1;
                        continue;
                    }
                    Form::Select => {}
                }
                // Mirror `run`: the value comparison only runs once the CARDINALITIES agree,
                // because a cardinality difference is the adjudicated sq-eibog / sq-ai2wa
                // channel (re-reporting it here would double-count one divergence). Without
                // that gate the `equality` shapes would surface Oxigraph's documented
                // cross-family leniency as a value mismatch.
                let (Ok(sparq_n), Ok(oxi_n)) = (
                    sparq_engine::query(&g, &q).map(|r| r.len()),
                    oxi_count(&store, &q),
                ) else {
                    continue;
                };
                if sparq_n != oxi_n {
                    continue;
                }
                match compare_select(&g, &store, &q) {
                    Ok(Compared::Multiset | Compared::Ordered) => compared += 1,
                    Ok(Compared::SkippedRowChoice) => {}
                    Ok(other) => panic!(
                        "category {cat} seed {seed}: unexpected outcome {other:?} — the \
                         generator is documented as bnode-free and SELECT-only\n{q}"
                    ),
                    Err(e) => panic!("category {cat} seed {seed}: {e}\n{q}"),
                }
            }
            assert!(
                compared > 0,
                "category {cat}: NOTHING was value-compared — the value-level differential \
                 is vacuous for that shard"
            );
        }
    }

    /// [OPUS-5] sq-qcnn.5: a blank-node answer is no longer routed to a bare triage bucket —
    /// it is COMPARED, up to blank-node ISOMORPHISM (RDFC-1.0 canonical labelling via
    /// `sparq-difftest`), because bnode labels are engine-local and only a bijection is well
    /// defined. Both directions are pinned: agreeing answers with DIFFERENT labels must
    /// compare equal, and a genuinely different answer must still fail.
    #[test]
    fn a_blank_node_answer_is_compared_up_to_isomorphism() {
        // The generator is bnode-free, so this shape is written by hand: `_:b` is a real
        // blank node in BOTH engines' answers, with engine-local labels.
        let ttl = "@prefix ex: <http://ex/> .\n_:b ex:age 5 .\nex:n0 ex:age 6 .\n";
        let g = sparq_core::Graph::load_str(ttl, "turtle").unwrap();
        let store = oxi_store(ttl);
        let bnode_q = "PREFIX ex: <http://ex/>\nSELECT ?s ?a WHERE { ?s ex:age ?a }";
        // Precondition: both engines really do bind a blank node here, so the assertion
        // below is about the routing and not about an accidentally-empty answer.
        let sparq = sparq_solutions(&g, bnode_q).unwrap().rows;
        let oxi = oxi_solutions(&store, bnode_q).unwrap().rows;
        assert!(solutions_have_blank_nodes(&sparq));
        assert!(solutions_have_blank_nodes(&oxi));
        assert_eq!(
            compare_select(&g, &store, bnode_q),
            Ok(Compared::Isomorphic),
            "a bnode answer must be COMPARED up to a bijection, not merely counted"
        );
        // MUTATION GUARD: isomorphism is not a blanket pass. Change the bnode row's VALUE and
        // no bijection can match the two tables any more.
        let mut wrong = sparq.clone();
        let row = wrong
            .iter_mut()
            .find(|r| matches!(r.get("s"), Some(sparq_difftest::Term::Blank(_))))
            .expect("one row binds the blank node");
        row.insert("a".to_string(), neutral_term(&nint("99")));
        assert_eq!(
            solutions_isomorphic(&wrong, &oxi),
            Ok(false),
            "a wrong value on the blank-node row must NOT be absorbed by relabelling"
        );

        // A ground answer over the same graph is compared as a plain multiset — so the
        // isomorphism path is reached by the blank node, not by anything else about the case.
        let ground_q = "PREFIX ex: <http://ex/>\nSELECT ?a WHERE { ex:n0 ex:age ?a }";
        assert_eq!(
            compare_select(&g, &store, ground_q),
            Ok(Compared::Multiset)
        );
        // ...and a bare `LIMIT` still lands in the DISTINCT row-choice bucket.
        let limit_q = "PREFIX ex: <http://ex/>\nSELECT ?a WHERE { ?s ex:age ?a } LIMIT 1";
        assert_eq!(
            compare_select(&g, &store, limit_q),
            Ok(Compared::SkippedRowChoice)
        );
    }

    /// `query_form` must route each result form to the comparator that is sound for it. A
    /// misclassified `ASK` would be counted instead of compared, which is the exact blindness
    /// this bead removes.
    #[test]
    fn query_form_classifies_every_result_form() {
        let pfx = "PREFIX ex: <http://ex/>\n";
        assert_eq!(query_form(&format!("{pfx}SELECT * WHERE {{ ?s ?p ?o }}")), Form::Select);
        assert_eq!(query_form(&format!("{pfx}ASK WHERE {{ ?s ex:age ?a }}")), Form::Ask);
        assert_eq!(
            query_form(&format!("{pfx}CONSTRUCT {{ ?s ex:age ?a }} WHERE {{ ?s ex:age ?a }}")),
            Form::Graph
        );
        assert_eq!(query_form(&format!("{pfx}DESCRIBE ex:n0")), Form::Graph);
        // A sub-SELECT sits inside the outer form's line, so the OUTER keyword wins.
        assert_eq!(
            query_form(&format!(
                "{pfx}ASK WHERE {{ {{ SELECT ?s WHERE {{ ?s ex:age ?a }} }} }}"
            )),
            Form::Ask
        );
    }

    /// `ASK` was DOUBLY blind under the old cardinality oracle: `oxi_count` mapped
    /// `Boolean(_)` to `1`, so `ASK` false and `ASK` true were indistinguishable. Pinned two
    /// ways — the boolean comparison decides both answers, and `oxi_count` now REFUSES the
    /// form rather than quietly returning a count.
    #[test]
    fn ask_is_compared_by_its_boolean_not_a_count() {
        let g = sparq_core::Graph::load_str(BINDING_TTL, "turtle").unwrap();
        let store = oxi_store(BINDING_TTL);
        let yes = "PREFIX ex: <http://ex/>\nASK WHERE { ?s ex:age ?a }";
        let no = "PREFIX ex: <http://ex/>\nASK WHERE { ?s ex:missing ?a }";
        // Both engines agree, and they agree on DIFFERENT booleans for the two queries.
        assert!(sparq_engine::ask(&g, yes).unwrap());
        assert!(!sparq_engine::ask(&g, no).unwrap());
        assert_eq!(compare_ask(&g, &store, yes), Ok(Compared::AskBoolean));
        assert_eq!(compare_ask(&g, &store, no), Ok(Compared::AskBoolean));
        // MUTATION GUARD on the blindness itself: a count-based oracle cannot tell these two
        // apart, so it must not be reachable. `oxi_count` refuses the form.
        assert!(
            oxi_count(&store, yes).is_err(),
            "an ASK must never be reduced to a count (that made false look like true)"
        );
        assert!(oxi_count(&store, no).is_err());
    }

    /// `CONSTRUCT` was triple-COUNT-only, which cannot see a graph landing on the WRONG
    /// triples. It is now compared as a canonical triple SET (up to blank-node isomorphism).
    /// The mutation is deliberately count-preserving so only the set comparison can catch it.
    #[test]
    fn construct_is_compared_by_its_triple_set_not_a_count() {
        let g = sparq_core::Graph::load_str(BINDING_TTL, "turtle").unwrap();
        let store = oxi_store(BINDING_TTL);
        let q = "PREFIX ex: <http://ex/>\n\
                 CONSTRUCT { ?s ex:age ?a } WHERE { ?s ex:age ?a }";
        assert_eq!(
            compare_graph(&g, &store, q),
            Ok(Compared::GraphIsomorphic),
            "both engines construct the same graph"
        );
        assert!(
            oxi_count(&store, q).is_err(),
            "a graph result must never be reduced to a triple count"
        );

        // MUTATION GUARD: swap one object for another that is already in the graph. The triple
        // COUNT is unchanged, so only the SET comparison can see it.
        let truth: Vec<[sparq_difftest::Term; 3]> = sparq_engine::construct_or_describe(&g, q)
            .unwrap()
            .iter()
            .map(neutral_triple)
            .collect();
        assert!(!truth.is_empty(), "the fixture must construct triples");
        let mut wrong = truth.clone();
        wrong[0][2] = neutral_term(&nint("999"));
        assert_eq!(wrong.len(), truth.len(), "the mutation preserves the triple COUNT");
        assert_eq!(
            graph_isomorphic(&wrong, &truth),
            Ok(false),
            "a wrong constructed TRIPLE at the right count must fail the oracle"
        );
    }

    /// Non-equality shapes are not recognised → the strict Oxigraph differential is
    /// kept (`spec_filter_count` returns `None`), so ordering / join / range-pruning
    /// bugs are unaffected by this fix.
    #[test]
    fn non_equality_shapes_keep_strict_differential() {
        let store = Store::new().unwrap();
        store
            .load_from_reader(
                oxigraph::io::RdfFormat::Turtle,
                b"@prefix ex: <http://ex/> .\nex:n0 ex:age 5 .\n".as_slice(),
            )
            .unwrap();
        assert!(spec_filter_count(
            &store,
            "PREFIX ex: <http://ex/>\nSELECT * WHERE { ?s ex:age ?a FILTER(?a > 3) }"
        )
        .is_none());
        assert!(spec_filter_count(
            &store,
            "PREFIX ex: <http://ex/>\nSELECT * WHERE { ?s ex:age ?a }"
        )
        .is_none());
    }
}
