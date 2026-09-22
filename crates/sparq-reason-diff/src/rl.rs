//! [FABLE-5] sq-xg6uu — the OWL 2 RL differential harness proper: canonicalizer,
//! documented normalization, golden-vector loader, multiset differ, and the
//! per-case disposition ledger.
//!
//! # Contract
//!
//! Both sides — sparq-reason's closure and the pinned `owlrl` golden vector — are
//! rendered to a **sorted canonical N-Triples multiset** (one triple per line, oxrdf
//! canonical term rendering) and then passed through the SAME [`normalize`] step.
//! The comparison is the multiset symmetric difference. A case passes iff the
//! difference is empty, OR the case carries an explicit `PERMANENT` disposition
//! whose pinned `oracle-only:` / `sparq-only:` line sets equal the observed
//! difference EXACTLY (so a dispositioned case still fails loud the moment the
//! divergence drifts — grows, shrinks, or vanishes).
//!
//! # Normalization (documented, applied to BOTH sides)
//!
//! The raw `owlrl` closure carries two classes of case-independent output that
//! sparq-reason deliberately never materializes (the same "no axiomatic /
//! reflexive triples" posture its RDFS profile documents):
//!
//! 1. **eq-ref reflexivity** — `x owl:sameAs x` for every node in the graph.
//!    Information-free; dropped from both sides.
//! 2. **W3C vocabulary self-description** — triples whose SUBJECT is in the
//!    `rdf:` / `rdfs:` / `xsd:` / `owl:` namespaces (owlrl's built-in datatype and
//!    annotation-property typings plus the `owl:Thing`/`owl:Nothing` scaffolding,
//!    ~100 triples emitted even for an empty input under
//!    `axiomatic_triples=False`). Dropped from both sides.
//!
//! Everything else — including entailments ABOUT user terms via W3C-vocabulary
//! predicates/objects (e.g. `ex:C rdfs:subClassOf ex:D`) — is compared verbatim.
//!
//! # Corpus constraints (enforced fail-loud, not silently worked around)
//!
//! * **No user data at a W3C-vocabulary subject.** Rule 2 is a blanket
//!   subject-position drop, so anything a CASE puts there is invisible to the
//!   differ and a real RL divergence at that subject would be silently masked.
//!   ENFORCED as of sq-ovl6f (it was a comment-only convention before):
//!   [`lint_corpus_document`] rejects an authored `.nt` that ASSERTS one, and
//!   [`sparq_rl_closure_lines`] rejects sparq's CLOSURE containing one — which is
//!   the strictly stronger check, because a case can assert no vocabulary subject
//!   and still ENTAIL onto one (`ex:a owl:sameAs owl:Thing` asserts none, yet
//!   eq-rep rewrites every `ex:a` triple onto `owl:Thing`).
//!   KNOWN RESIDUAL: the check cannot be run on the ORACLE side, whose
//!   vocabulary-subject block is legitimately ~100 triples with no pinned
//!   empty-graph baseline to subtract, so an ORACLE-only entailment at a
//!   vocabulary subject stays masked (owlrl's `owl:Nothing rdfs:subClassOf <user
//!   class>` is one such, present in the corpus today).
//! * **No blank nodes.** Canonical multiset comparison across two tool stacks has
//!   no stable bnode labels; [`canonical_lines`] errors on any bnode rather than
//!   guessing an alignment. RDF lists / restrictions in the corpus use NAMED nodes.
//! * Literals stay in canonical lexical form (sparq's dictionary inlines small
//!   integers and re-renders them canonically, so `"01"^^xsd:integer` would
//!   round-trip as `"1"^^xsd:integer` and produce a spurious diff).

use sparq_core::dict::{Dict, Id};
use sparq_core::Graph;
use std::path::Path;

/// The four W3C vocabulary namespaces whose SUBJECT-position triples are dropped by
/// [`normalize`] (owlrl's vocabulary self-description block; see the module docs).
pub const W3C_VOCAB_PREFIXES: [&str; 4] = [
    "<http://www.w3.org/1999/02/22-rdf-syntax-ns#",
    "<http://www.w3.org/2000/01/rdf-schema#",
    "<http://www.w3.org/2001/XMLSchema#",
    "<http://www.w3.org/2002/07/owl#",
];

const SAME_AS: &str = "<http://www.w3.org/2002/07/owl#sameAs>";

/// Renders interned triples as canonical N-Triples lines (oxrdf term rendering,
/// `<s> <p> <o> .`), sorted bytewise, duplicates RETAINED (multiset semantics).
///
/// Errors on any blank node — the corpus is bnode-free by design and a bnode here
/// means the fixture (or the reasoner) broke that contract; there is no stable
/// cross-tool label alignment to fall back on.
pub fn canonical_lines(dict: &Dict, triples: &[[Id; 3]]) -> Result<Vec<String>, String> {
    let mut lines = Vec::with_capacity(triples.len());
    for &[s, p, o] in triples {
        let (s, p, o) = (dict.term(s), dict.term(p), dict.term(o));
        let line = format!("{s} {p} {o} .");
        if line.contains("_:") {
            return Err(format!(
                "blank node in triple `{line}` — the reason-diff corpus is bnode-free \
                 by contract (no stable cross-tool bnode alignment); use named nodes"
            ));
        }
        lines.push(line);
    }
    lines.sort_unstable();
    Ok(lines)
}

/// The documented two-rule normalization (module docs), applied identically to the
/// golden vector and to sparq's closure. Input need not be sorted; output preserves
/// order (callers pass sorted input).
pub fn normalize(lines: &[String]) -> Vec<String> {
    lines
        .iter()
        .filter(|l| !is_w3c_vocab_subject(l) && !is_reflexive_same_as(l))
        .cloned()
        .collect()
}

fn is_w3c_vocab_subject(line: &str) -> bool {
    W3C_VOCAB_PREFIXES.iter().any(|p| line.starts_with(p))
}

fn is_reflexive_same_as(line: &str) -> bool {
    // Canonical line shape: `<s> <p> <o> .` — split on the single spaces the
    // renderer emits. IRIs cannot contain unescaped spaces in N-Triples.
    let mut parts = line.splitn(3, ' ');
    let (Some(s), Some(p), Some(rest)) = (parts.next(), parts.next(), parts.next()) else {
        return false;
    };
    p == SAME_AS && rest.strip_suffix(" .") == Some(s)
}

/// Fail-loud enforcement of the module docs' **no user data at a W3C-vocabulary
/// subject** corpus constraint over one side's canonical lines. `what` names the
/// artifact so the report says what to go and fix.
fn deny_w3c_vocab_subjects(what: &str, lines: &[String]) -> Result<(), String> {
    let offenders: Vec<&String> = lines.iter().filter(|l| is_w3c_vocab_subject(l)).collect();
    if offenders.is_empty() {
        return Ok(());
    }
    let mut msg = format!(
        "{what}: {} triple(s) at a W3C-vocabulary SUBJECT. `normalize` drops every \
         such line from BOTH sides (that is where owlrl's vocabulary self-description \
         block lives), so a corpus case may not put its own data there — asserted or \
         entailed — because a real RL divergence at that subject would be MASKED \
         instead of failing:\n",
        offenders.len()
    );
    for l in offenders {
        msg.push_str("  ");
        msg.push_str(l);
        msg.push('\n');
    }
    Err(msg)
}

/// Corpus lint: enforces the W3C-vocabulary-subject constraint on one corpus `.nt`
/// document AS AUTHORED — the check a future corpus edit must survive, reported
/// against the file the editor actually touched.
///
/// This is the WEAKER half of the invariant on purpose: it sees only what the
/// document asserts. [`sparq_rl_closure_lines`] applies the same check to sparq's
/// closure and so also covers the entailed channel (see the module docs).
pub fn lint_corpus_document(nt_text: &str) -> Result<(), String> {
    let (dict, triples) = Graph::parse_to_triples(nt_text, "ntriples")?;
    deny_w3c_vocab_subjects("corpus document", &canonical_lines(&dict, &triples)?)
}

/// A loaded golden vector: the pinned capture-tool header plus the RAW (pre-
/// normalization) canonical closure lines.
#[derive(Debug)]
pub struct Golden {
    /// The `# tool: …` header line — the pinned external-oracle version this vector
    /// was captured with. REQUIRED: a vector without it is rejected (the pin IS the
    /// claim).
    pub tool: String,
    /// Raw closure lines, sorted.
    pub lines: Vec<String>,
}

/// Loads a `bench/reason-diff/rl/<case>.expected` golden vector. `#`-prefixed lines
/// are header/comment; exactly one must be a `# tool:` pin. Triple lines are
/// re-sorted defensively (the differ needs sorted input regardless of how the file
/// was edited).
pub fn load_golden(path: &Path) -> Result<Golden, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut tool = None;
    let mut lines = Vec::new();
    for raw in text.lines() {
        let line = raw.trim_end();
        if line.is_empty() {
            continue;
        }
        if let Some(rest) = line.strip_prefix("# tool:") {
            tool = Some(rest.trim().to_string());
        }
        if line.starts_with('#') {
            continue;
        }
        lines.push(line.to_string());
    }
    let tool = tool.ok_or_else(|| {
        format!(
            "{}: missing `# tool:` header — every golden vector must pin the \
             external-oracle version it was captured with",
            path.display()
        )
    })?;
    lines.sort_unstable();
    Ok(Golden { tool, lines })
}

/// An explicit PERMANENT divergence disposition for one case (the sq-pbz04.1 ledger
/// precedent): a written reason plus the EXACT pinned symmetric difference.
#[derive(Debug, PartialEq, Eq)]
pub struct Disposition {
    /// The written reason (required, non-empty).
    pub reason: String,
    /// Triples entailed by the oracle but not by sparq (sorted).
    pub oracle_only: Vec<String>,
    /// Triples entailed by sparq but not by the oracle (sorted).
    pub sparq_only: Vec<String>,
}

/// Loads a `bench/reason-diff/rl/<case>.disposition` ledger entry. Format (line
/// oriented, `#` comments allowed):
///
/// ```text
/// disposition: PERMANENT
/// reason: <one line, required>
/// oracle-only: <canonical N-Triples line>
/// sparq-only: <canonical N-Triples line>
/// ```
pub fn load_disposition(path: &Path) -> Result<Disposition, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let (mut kind, mut reason) = (None, None);
    let (mut oracle_only, mut sparq_only) = (Vec::new(), Vec::new());
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(v) = line.strip_prefix("disposition:") {
            kind = Some(v.trim().to_string());
        } else if let Some(v) = line.strip_prefix("reason:") {
            reason = Some(v.trim().to_string());
        } else if let Some(v) = line.strip_prefix("oracle-only:") {
            oracle_only.push(v.trim().to_string());
        } else if let Some(v) = line.strip_prefix("sparq-only:") {
            sparq_only.push(v.trim().to_string());
        } else {
            return Err(format!("{}: unrecognized line `{line}`", path.display()));
        }
    }
    match kind.as_deref() {
        Some("PERMANENT") => {}
        other => {
            return Err(format!(
                "{}: `disposition:` must be exactly `PERMANENT` (got {other:?}) — \
                 anything fixable belongs in a bead, not the ledger",
                path.display()
            ));
        }
    }
    let reason = reason.filter(|r| !r.is_empty()).ok_or_else(|| {
        format!(
            "{}: a PERMANENT disposition requires a non-empty `reason:`",
            path.display()
        )
    })?;
    if oracle_only.is_empty() && sparq_only.is_empty() {
        return Err(format!(
            "{}: a disposition must pin at least one divergent triple",
            path.display()
        ));
    }
    oracle_only.sort_unstable();
    sparq_only.sort_unstable();
    Ok(Disposition {
        reason,
        oracle_only,
        sparq_only,
    })
}

/// Multiset symmetric difference of two SORTED line vectors: returns
/// `(golden_only, ours_only)`, each with multiplicity.
pub fn diff_multisets(golden: &[String], ours: &[String]) -> (Vec<String>, Vec<String>) {
    let (mut g_only, mut o_only) = (Vec::new(), Vec::new());
    let (mut i, mut j) = (0, 0);
    while i < golden.len() && j < ours.len() {
        match golden[i].cmp(&ours[j]) {
            std::cmp::Ordering::Equal => {
                i += 1;
                j += 1;
            }
            std::cmp::Ordering::Less => {
                g_only.push(golden[i].clone());
                i += 1;
            }
            std::cmp::Ordering::Greater => {
                o_only.push(ours[j].clone());
                j += 1;
            }
        }
    }
    g_only.extend_from_slice(&golden[i..]);
    o_only.extend_from_slice(&ours[j..]);
    (g_only, o_only)
}

/// Parses one corpus `.nt` document, materializes sparq-reason's OWL 2 RL closure
/// via the PUBLIC `materialize` entry point, and returns the normalized canonical
/// line multiset.
///
/// Enforces the W3C-vocabulary-subject constraint on the RAW closure BEFORE
/// normalizing it away: everything sparq derives here is the case's own data, so
/// any of it landing at a vocabulary subject is a masked comparison, not owlrl's
/// system block.
pub fn sparq_rl_closure_lines(nt_text: &str) -> Result<Vec<String>, String> {
    let (mut dict, mut triples) = Graph::parse_to_triples(nt_text, "ntriples")?;
    sparq_reason::materialize(sparq_reason::Profile::OwlRl, &mut dict, &mut triples);
    let raw = canonical_lines(&dict, &triples)?;
    deny_w3c_vocab_subjects("sparq's RL closure of this case", &raw)?;
    Ok(normalize(&raw))
}

/// One case's differential outcome (pre-judgment): the observed normalized
/// symmetric difference plus the ledger entry, if any.
pub struct CaseOutcome {
    pub name: String,
    pub tool: String,
    pub oracle_only: Vec<String>,
    pub sparq_only: Vec<String>,
    pub disposition: Option<Disposition>,
}

/// Runs one case end to end: sparq closure vs normalized golden, multiset diff.
pub fn run_case(
    name: &str,
    nt_text: &str,
    golden: &Golden,
    disposition: Option<Disposition>,
) -> Result<CaseOutcome, String> {
    let ours = sparq_rl_closure_lines(nt_text).map_err(|e| format!("{name}: {e}"))?;
    let golden_norm = normalize(&golden.lines);
    let (oracle_only, sparq_only) = diff_multisets(&golden_norm, &ours);
    Ok(CaseOutcome {
        name: name.to_string(),
        tool: golden.tool.clone(),
        oracle_only,
        sparq_only,
        disposition,
    })
}

/// The fail-loud judgment (the bead's INVARIANT): a case passes iff it MATCHES the
/// pinned oracle, or its divergence equals its PERMANENT disposition EXACTLY. A
/// stale disposition (case now matches, or the divergence drifted) also fails —
/// the ledger can never rot silently.
pub fn judge(outcome: &CaseOutcome) -> Result<String, String> {
    let diverged = !outcome.oracle_only.is_empty() || !outcome.sparq_only.is_empty();
    match (&outcome.disposition, diverged) {
        (None, false) => Ok(format!("MATCH ({})", outcome.tool)),
        (None, true) => Err(format!(
            "UNdispositioned divergence vs {}:\n{}",
            outcome.tool,
            render_diff(&outcome.oracle_only, &outcome.sparq_only),
        )),
        (Some(d), false) => Err(format!(
            "stale disposition (`{}`): the case now MATCHES the oracle — delete the \
             .disposition file",
            d.reason
        )),
        (Some(d), true) => {
            if d.oracle_only == outcome.oracle_only && d.sparq_only == outcome.sparq_only {
                Ok(format!("PERMANENT divergence, dispositioned: {}", d.reason))
            } else {
                Err(format!(
                    "divergence drifted from its disposition (`{}`).\nobserved:\n{}\npinned:\n{}",
                    d.reason,
                    render_diff(&outcome.oracle_only, &outcome.sparq_only),
                    render_diff(&d.oracle_only, &d.sparq_only),
                ))
            }
        }
    }
}

fn render_diff(oracle_only: &[String], sparq_only: &[String]) -> String {
    let mut s = String::new();
    for l in oracle_only {
        s.push_str("  oracle-only: ");
        s.push_str(l);
        s.push('\n');
    }
    for l in sparq_only {
        s.push_str("  sparq-only: ");
        s.push_str(l);
        s.push('\n');
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(lines: &[&str]) -> Vec<String> {
        lines.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn canonical_lines_renders_sorted_and_rejects_bnodes() {
        let (mut dict, _) = Graph::parse_to_triples("", "ntriples").unwrap();
        let a = dict.intern_iri("http://ex.org/a");
        let b = dict.intern_iri("http://ex.org/b");
        let p = dict.intern_iri("http://ex.org/p");
        let lines = canonical_lines(&dict, &[[b, p, a], [a, p, b]]).unwrap();
        assert_eq!(
            lines,
            v(&[
                "<http://ex.org/a> <http://ex.org/p> <http://ex.org/b> .",
                "<http://ex.org/b> <http://ex.org/p> <http://ex.org/a> .",
            ])
        );
        let bn = dict.intern_blank("x1");
        let err = canonical_lines(&dict, &[[bn, p, a]]).unwrap_err();
        assert!(err.contains("blank node"), "got: {err}");
    }

    #[test]
    fn normalize_drops_exactly_the_two_documented_classes() {
        let input = v(&[
            // kept: user triple, W3C vocab in PREDICATE/OBJECT position only.
            "<http://ex.org/C> <http://www.w3.org/2000/01/rdf-schema#subClassOf> <http://ex.org/D> .",
            // kept: non-reflexive sameAs.
            "<http://ex.org/a> <http://www.w3.org/2002/07/owl#sameAs> <http://ex.org/b> .",
            // dropped: eq-ref reflexivity.
            "<http://ex.org/a> <http://www.w3.org/2002/07/owl#sameAs> <http://ex.org/a> .",
            // dropped: W3C vocabulary subject (owlrl system block).
            "<http://www.w3.org/2001/XMLSchema#string> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://www.w3.org/2000/01/rdf-schema#Datatype> .",
            "<http://www.w3.org/2002/07/owl#Thing> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://www.w3.org/2002/07/owl#Class> .",
        ]);
        assert_eq!(normalize(&input), v(&[&input[0], &input[1]]));
    }

    // [OPUS-5] sq-ovl6f — the W3C-vocab-subject constraint is ENFORCED, not merely
    // documented: what `normalize` drops, the corpus may not produce.
    #[test]
    fn corpus_lint_rejects_data_asserted_at_a_w3c_vocab_subject() {
        // Vocabulary terms in PREDICATE/OBJECT position are fine — only the subject
        // position is dropped.
        let clean =
            "<http://ex.org/Dog> <http://www.w3.org/2000/01/rdf-schema#subClassOf> <http://ex.org/Animal> .\n";
        assert!(lint_corpus_document(clean).is_ok());
        assert!(sparq_rl_closure_lines(clean).is_ok());

        let asserted =
            "<http://www.w3.org/2002/07/owl#Thing> <http://ex.org/p> <http://ex.org/o> .\n";
        let err = lint_corpus_document(asserted).unwrap_err();
        assert!(err.contains("W3C-vocabulary SUBJECT"), "got: {err}");
        assert!(err.contains("<http://www.w3.org/2002/07/owl#Thing>"), "got: {err}");
        // The closure check subsumes the document check (the closure contains the
        // asserted triple), so this case fails on either path.
        assert!(sparq_rl_closure_lines(asserted).is_err());
    }

    // [OPUS-5] sq-ovl6f — the ENTAILED channel, which an input-only lint cannot see.
    #[test]
    fn closure_lint_catches_masking_the_document_lint_cannot() {
        // Asserts nothing at a vocabulary subject, so the document lint passes; eq-rep
        // then rewrites `ex:a`'s triples onto `owl:Thing`, where `normalize` would
        // silently drop them from both sides.
        let nt = "<http://ex.org/a> <http://www.w3.org/2002/07/owl#sameAs> <http://www.w3.org/2002/07/owl#Thing> .\n\
                  <http://ex.org/a> <http://ex.org/p> <http://ex.org/o> .\n";
        assert!(lint_corpus_document(nt).is_ok());
        let masked = "<http://www.w3.org/2002/07/owl#Thing> <http://ex.org/p> <http://ex.org/o> .";
        let err = sparq_rl_closure_lines(nt).unwrap_err();
        assert!(
            err.contains(masked),
            "the masked entailment must be named in the report:\n{err}"
        );
    }

    #[test]
    fn reflexive_same_as_needs_exact_subject_object_equality() {
        // Prefix-shadowing subject/object must NOT be treated as reflexive.
        let l = "<http://ex.org/a> <http://www.w3.org/2002/07/owl#sameAs> <http://ex.org/aa> ."
            .to_string();
        assert_eq!(normalize(std::slice::from_ref(&l)), vec![l]);
    }

    #[test]
    fn diff_multisets_reports_both_sides_with_multiplicity() {
        let golden = v(&["a", "b", "b", "c"]);
        let ours = v(&["b", "c", "c", "d"]);
        let (g_only, o_only) = diff_multisets(&golden, &ours);
        assert_eq!(g_only, v(&["a", "b"]));
        assert_eq!(o_only, v(&["c", "d"]));
        let (e1, e2) = diff_multisets(&golden, &golden);
        assert!(e1.is_empty() && e2.is_empty());
    }

    // [GPT-5.6] sq-kcld6 — swapping inputs swaps the multiset-diff halves.
    #[test]
    fn diff_multisets_is_antisymmetric_under_argument_swap() {
        let a = v(&[
            "<http://example.com/a> <http://example.com/p> <http://example.com/o> .",
            "<http://example.com/b> <http://example.com/p> <http://example.com/o> .",
            "<http://example.com/b> <http://example.com/p> <http://example.com/o> .",
            "<http://example.com/d> <http://example.com/p> <http://example.com/o> .",
        ]);
        let b = v(&[
            "<http://example.com/b> <http://example.com/p> <http://example.com/o> .",
            "<http://example.com/c> <http://example.com/p> <http://example.com/o> .",
            "<http://example.com/d> <http://example.com/p> <http://example.com/o> .",
            "<http://example.com/e> <http://example.com/p> <http://example.com/o> .",
            "<http://example.com/e> <http://example.com/p> <http://example.com/o> .",
        ]);

        let forward = diff_multisets(&a, &b);
        let reverse = diff_multisets(&b, &a);

        assert!(!forward.0.is_empty() && !forward.1.is_empty());
        assert_eq!(forward, (reverse.1, reverse.0));
    }

    #[test]
    fn judge_matrix_is_fail_loud() {
        let base = |oracle_only: &[&str], disposition: Option<Disposition>| CaseOutcome {
            name: "t".into(),
            tool: "owlrl==7.6.2".into(),
            oracle_only: v(oracle_only),
            sparq_only: vec![],
            disposition,
        };
        let disp = |oracle_only: &[&str]| Disposition {
            reason: "beyond sparq's RL subset".into(),
            oracle_only: v(oracle_only),
            sparq_only: vec![],
        };
        // clean match
        assert!(judge(&base(&[], None)).is_ok());
        // undispositioned divergence fails
        assert!(judge(&base(&["x"], None))
            .unwrap_err()
            .contains("UNdispositioned"));
        // exactly-pinned divergence passes, carrying the written reason
        assert!(judge(&base(&["x"], Some(disp(&["x"]))))
            .unwrap()
            .contains("beyond sparq's RL subset"));
        // drifted divergence fails
        assert!(judge(&base(&["x", "y"], Some(disp(&["x"]))))
            .unwrap_err()
            .contains("drifted"));
        // stale disposition (now matching) fails
        assert!(judge(&base(&[], Some(disp(&["x"]))))
            .unwrap_err()
            .contains("stale disposition"));
    }

    #[test]
    fn golden_loader_requires_tool_pin() {
        let dir = std::env::temp_dir().join("sparq-reason-diff-golden-test");
        std::fs::create_dir_all(&dir).unwrap();
        let good = dir.join("good.expected");
        std::fs::write(&good, "# tool: owlrl==7.6.2\n# case: x.nt\nb .\na .\n").unwrap();
        let g = load_golden(&good).unwrap();
        assert_eq!(g.tool, "owlrl==7.6.2");
        assert_eq!(g.lines, v(&["a .", "b ."])); // re-sorted
        let bad = dir.join("bad.expected");
        std::fs::write(&bad, "a .\n").unwrap();
        assert!(load_golden(&bad).unwrap_err().contains("# tool:"));
    }

    #[test]
    fn disposition_loader_rejects_non_permanent_empty_or_unreasoned() {
        let dir = std::env::temp_dir().join("sparq-reason-diff-disp-test");
        std::fs::create_dir_all(&dir).unwrap();
        let write = |name: &str, body: &str| {
            let p = dir.join(name);
            std::fs::write(&p, body).unwrap();
            p
        };
        let ok = load_disposition(&write(
            "ok.disposition",
            "# ledger entry\ndisposition: PERMANENT\nreason: r\noracle-only: t .\n",
        ))
        .unwrap();
        assert_eq!(ok.reason, "r");
        assert_eq!(ok.oracle_only, v(&["t ."]));
        assert!(load_disposition(&write(
            "k.disposition",
            "disposition: TEMP\nreason: r\noracle-only: t .\n"
        ))
        .unwrap_err()
        .contains("PERMANENT"));
        assert!(load_disposition(&write(
            "r.disposition",
            "disposition: PERMANENT\noracle-only: t .\n"
        ))
        .unwrap_err()
        .contains("reason"));
        assert!(load_disposition(&write(
            "e.disposition",
            "disposition: PERMANENT\nreason: r\n"
        ))
        .unwrap_err()
        .contains("at least one divergent triple"));
        assert!(load_disposition(&write(
            "u.disposition",
            "disposition: PERMANENT\nreason: r\nnonsense\n"
        ))
        .unwrap_err()
        .contains("unrecognized"));
    }

    #[test]
    fn sparq_rl_closure_smoke_symmetric_property() {
        // prp-symp end to end through the PUBLIC materialize entry point.
        let nt = "<http://ex.org/knows> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://www.w3.org/2002/07/owl#SymmetricProperty> .\n\
                  <http://ex.org/a> <http://ex.org/knows> <http://ex.org/b> .\n";
        let lines = sparq_rl_closure_lines(nt).unwrap();
        assert!(lines
            .contains(&"<http://ex.org/b> <http://ex.org/knows> <http://ex.org/a> .".to_string()));
    }
}
