// sq-hmd7l.24 (epic sq-hmd7l) — the W3C RIF conformance PASS-RATE runner
// (bench registry id `rif-conformance`). This is the CONFORMANCE-ONLY arm of the RIF
// axis: the comparative-benchmarking design record's verdict for RIF is NOT-COMPARABLE
// (no runnable open-source RIF peer exists to race — fuxi is dead, Jena dropped RIF,
// RIFle is unmaintained, RDFox consumes datalog, not RIF), so this axis emits a
// pass-rate over the W3C RIF test suite and NOTHING ELSE. No wall-clock is measured or
// printed here by design — perf of RIF-translated rules is covered on the
// materialization axis (sq-hmd7l.7), never on this one. See
// research/gap-rif-2026-07.md for the codified verdict.
//
// Two modes:
//
// * `--smoke` — the SELF-ASSERTING lane over the VENDORED subset in `bench/rif/suite/`
//   (sparq-AUTHORED cases in the W3C archive's directory taxonomy — the real W3C files
//   carry no redistribution license, even a subset is a prohibited derivative; see
//   scripts/fetch-inference-suites.sh). Every case's verdict is asserted EXACTLY
//   against the pinned `bench/rif/expected.tsv` (pinned SKIPs included — a case
//   graduating skip→pass fails the smoke until re-pinned). Exit 1 on any drift.
// * (default) — the FULL-SUITE pass-rate over the FETCHED W3C archive under
//   `tests/w3c/rif-core/` (gitignored; scripts/fetch-inference-suites.sh), emitted
//   with a PER-DIALECT breakdown (dialect = archive dir prefix, e.g. `Core`), the
//   named skip taxonomy (the denominator's honesty) and an HONEST fail listing —
//   failures are listed one by one, never rounded up into the pass count. When the
//   archive is absent the runner prints the fetch instruction and exits 0 (the
//   graceful-skip posture of the `rif-wg-core` test lane — a fresh offline checkout
//   stays green; the RATCHET for the Core archive lives in
//   crates/sparq-conformance/tests/rif_wg_core_suite.rs, not here).
//
// The oracle is a condensed re-statement of the `rif-wg-core` lane's rules
// (crates/sparq-conformance/tests/rif_wg_core_suite.rs — the authoritative,
// ratchet-carrying copy): import → validate → closure → conclusion satisfaction, the
// NET vacuity rule (an un-importable premise is a SKIP, NEVER a vacuous pass — in
// either polarity), and the genuine-violation criterion for the reject kinds (a
// rejection merely because a construct is unsupported never launders into a
// "correctly rejected").

fn main() {
    let code = runner::run(std::env::args().skip(1).collect());
    std::process::exit(code);
}

mod runner {
    use sparq_core::dict::{Dict, Id};
    use sparq_reason::rif::{Atom, Document, RifError, Term};
    use sparq_reason::rif_xml::{self, ImportError};
    use std::collections::BTreeMap;
    use std::path::{Path, PathBuf};

    // ------------------------------------------------------------------ test kinds

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum TestKind {
        PositiveEntailment,
        NegativeEntailment,
        PositiveSyntax,
        NegativeSyntax,
        ImportRejection,
    }

    impl TestKind {
        fn dir(self) -> &'static str {
            match self {
                TestKind::PositiveEntailment => "PositiveEntailmentTest",
                TestKind::NegativeEntailment => "NegativeEntailmentTest",
                TestKind::PositiveSyntax => "PositiveSyntaxTest",
                TestKind::NegativeSyntax => "NegativeSyntaxTest",
                TestKind::ImportRejection => "ImportRejectionTest",
            }
        }
        const ALL: &'static [TestKind] = &[
            TestKind::PositiveEntailment,
            TestKind::NegativeEntailment,
            TestKind::PositiveSyntax,
            TestKind::NegativeSyntax,
            TestKind::ImportRejection,
        ];
    }

    /// Per-test verdict. `Skip` carries the NAMED taxonomy bucket; `Fail` is a genuine
    /// wrong answer within the supported subset.
    #[derive(Clone, Debug, PartialEq, Eq)]
    enum Outcome {
        Pass,
        Skip(&'static str),
        Fail(String),
    }

    impl Outcome {
        /// The stable verdict token pinned in `bench/rif/expected.tsv`.
        fn token(&self) -> String {
            match self {
                Outcome::Pass => "pass".to_string(),
                Outcome::Skip(bucket) => (*bucket).to_string(),
                Outcome::Fail(_) => "fail".to_string(),
            }
        }
    }

    /// Map an `ImportError` to the named skip-taxonomy bucket (mirrors the
    /// `rif-wg-core` lane's `skip_bucket` — the denominator's honesty).
    fn skip_bucket(e: &ImportError) -> &'static str {
        match e {
            ImportError::ImportDirective { .. } | ImportError::InconsistentImport { .. } => {
                "skip:imports"
            }
            ImportError::NonCoreElement { .. } => "skip:non-core",
            ImportError::UnknownExternal { .. } => "skip:unsupported-builtin",
            ImportError::NamedArgUniterm { .. }
            | ImportError::UnrecognizedElement { .. }
            | ImportError::MalformedXml(_)
            | ImportError::ValidationFailed(_) => "skip:condition-shape",
        }
    }

    /// Is `re` a RIF-Core violation the validator GENUINELY DETECTS (safety /
    /// range-restriction / syntactic) — as opposed to an unsupported-capability
    /// fail-close (`DistinctGroundEqual`, `Nonmonotonic`) that rejects a document it
    /// simply cannot handle? Only the former is a non-vacuous "correctly rejected".
    fn is_genuine_core_violation(re: &RifError) -> bool {
        match re {
            RifError::UnboundHeadVar { .. }
            | RifError::UnboundBuiltinInput { .. }
            | RifError::BadBuiltinArity { .. }
            | RifError::BuiltinInHead { .. }
            | RifError::EqualInConclusion => true,
            RifError::DistinctGroundEqual { .. } | RifError::Nonmonotonic { .. } => false,
        }
    }

    // ----------------------------------------------------- entity pre-resolution

    /// Resolve the DTD internal-subset `<!ENTITY name "value">` declarations the W3C
    /// RIF/XML files use (`&rif;`, `&xs;`, …), then strip the `<!DOCTYPE …>` block.
    /// Sound + self-contained: the entities are declared IN the document's own
    /// internal subset (no external DTD, no network).
    fn resolve_internal_entities(src: &str) -> String {
        let mut entities: Vec<(String, String)> = Vec::new();
        for chunk in src.split("<!ENTITY").skip(1) {
            let decl_end = chunk.find('>').unwrap_or(chunk.len());
            let decl = chunk[..decl_end].trim();
            let mut it = decl.splitn(2, char::is_whitespace);
            let name = it.next().unwrap_or("").trim().to_string();
            let value = it
                .next()
                .unwrap_or("")
                .trim()
                .trim_matches(|c| c == '"' || c == '\'')
                .to_string();
            if !name.is_empty() {
                entities.push((name, value));
            }
        }
        let mut out = src.to_string();
        if let Some(start) = out.find("<!DOCTYPE") {
            if let Some(rel) = out[start..].find("]>") {
                out.replace_range(start..start + rel + 2, "");
            } else if let Some(rel) = out[start..].find('>') {
                out.replace_range(start..start + rel + 1, "");
            }
        }
        for (name, value) in &entities {
            out = out.replace(&format!("&{};", name), value);
        }
        out
    }

    fn read_rif(dir: &Path, name: &str) -> Option<String> {
        let raw = std::fs::read_to_string(dir.join(name)).ok()?;
        Some(resolve_internal_entities(&raw))
    }

    fn rif_name(id: &str, suffix: &str) -> String {
        format!("{}-{}.rif", id, suffix)
    }

    /// Wrap a bare conclusion element (a W3C conclusion file is a bare
    /// `<Frame>`/`<Atom>`/`<Member>`/`<Subclass>` root, not a `<Document>`) in the
    /// standard scaffold so the importer parses it as a ground fact.
    fn wrap_conclusion(src: &str) -> String {
        let body = if src.trim_start().starts_with("<?xml") {
            match src.find("?>") {
                Some(pos) => &src[pos + 2..],
                None => src,
            }
        } else {
            src
        };
        format!(
            "<Document xmlns=\"http://www.w3.org/2007/rif#\">\
             <payload><Group><sentence>{}</sentence></Group></payload></Document>",
            body.trim()
        )
    }

    // ------------------------------------------------------- conclusion oracle

    /// Lower a ground existential-free conclusion `Document` to its positive atoms, or
    /// `None` if it is outside that shape (→ `skip:condition-shape`). Encodability of
    /// every term is REQUIRED here — the LOAD-BEARING guard against a NET false-pass
    /// (an un-encodable non-conclusion must not read as "not entailed").
    fn conclusion_ground_triples(doc: &Document) -> Option<Vec<Atom>> {
        let mut atoms = Vec::new();
        for rule in &doc.rules {
            if !rule.body.is_empty() {
                return None;
            }
            for head in &rule.head {
                match head {
                    Atom::Frame { .. } | Atom::Member { .. } | Atom::Subclass { .. } => {
                        if !atom_is_encodable_ground(head) {
                            return None;
                        }
                        atoms.push(head.clone());
                    }
                    Atom::Equal { .. } | Atom::Builtin { .. } => return None,
                }
            }
        }
        if atoms.is_empty() {
            None
        } else {
            Some(atoms)
        }
    }

    fn term_is_encodable(t: &Term) -> bool {
        matches!(t, Term::Iri(_) | Term::Lit { .. })
    }

    fn atom_is_encodable_ground(a: &Atom) -> bool {
        match a {
            Atom::Frame { obj, pred, val } => {
                term_is_encodable(obj) && term_is_encodable(pred) && term_is_encodable(val)
            }
            Atom::Member { obj, class } => term_is_encodable(obj) && term_is_encodable(class),
            Atom::Subclass { sub, sup } => term_is_encodable(sub) && term_is_encodable(sup),
            Atom::Equal { .. } | Atom::Builtin { .. } => false,
        }
    }

    const RDF_TYPE: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";
    const RDFS_SUBCLASS_OF: &str = "http://www.w3.org/2000/01/rdf-schema#subClassOf";

    fn atom_to_triple_terms(a: &Atom) -> Option<[Term; 3]> {
        match a {
            Atom::Frame { obj, pred, val } => Some([obj.clone(), pred.clone(), val.clone()]),
            Atom::Member { obj, class } => Some([
                obj.clone(),
                Term::Iri(RDF_TYPE.to_string()),
                class.clone(),
            ]),
            Atom::Subclass { sub, sup } => Some([
                sub.clone(),
                Term::Iri(RDFS_SUBCLASS_OF.to_string()),
                sup.clone(),
            ]),
            _ => None,
        }
    }

    fn term_to_id(dict: &mut Dict, t: &Term) -> Option<Id> {
        use oxrdf::{Literal, NamedNode, Term as OxTerm};
        match t {
            Term::Iri(iri) => Some(dict.intern(&OxTerm::NamedNode(NamedNode::new_unchecked(
                iri.clone(),
            )))),
            Term::Lit { lex, datatype } => {
                let lit = Literal::new_typed_literal(
                    lex.clone(),
                    NamedNode::new_unchecked(datatype.clone()),
                );
                Some(dict.intern(&OxTerm::Literal(lit)))
            }
            Term::Var(_) | Term::List(_) => None,
        }
    }

    fn numeric_value(t: &Term) -> Option<sparq_substrate::numeric::Num> {
        use oxrdf::{Literal, NamedNode};
        if let Term::Lit { lex, datatype } = t {
            let lit =
                Literal::new_typed_literal(lex.clone(), NamedNode::new_unchecked(datatype.clone()));
            sparq_substrate::numeric::as_numeric(&lit)
        } else {
            None
        }
    }

    /// Does the closure satisfy a ground conclusion triple? Subject/predicate compare
    /// by exact dict identity; the object VALUE-AWARE when numeric (restricted to the
    /// exact-rational tiers via `Num::to_dec()` — float/double keep exact identity, as
    /// in the `rif-wg-core` lane), else exact identity.
    fn closure_satisfies(dict: &mut Dict, closure: &[[Id; 3]], triple: &[Term; 3]) -> bool {
        let sid = match term_to_id(dict, &triple[0]) {
            Some(i) => i,
            None => return false,
        };
        let pid = match term_to_id(dict, &triple[1]) {
            Some(i) => i,
            None => return false,
        };
        let want_num = numeric_value(&triple[2]);
        let oid = term_to_id(dict, &triple[2]);
        for row in closure {
            if row[0] != sid || row[1] != pid {
                continue;
            }
            if let Some(oid) = oid {
                if row[2] == oid {
                    return true;
                }
            }
            if let Some(want) = want_num {
                if let (Some(want_dec), oxrdf::Term::Literal(l)) = (want.to_dec(), dict.term(row[2]))
                {
                    if let Some(got_dec) =
                        sparq_substrate::numeric::as_numeric(&l).and_then(|n| n.to_dec())
                    {
                        if got_dec.cmp(want_dec) == Some(std::cmp::Ordering::Equal) {
                            return true;
                        }
                    }
                }
            }
        }
        false
    }

    fn closure_entails_conclusion(dict: &mut Dict, closure: &[[Id; 3]], atoms: &[Atom]) -> bool {
        atoms.iter().all(|a| {
            atom_to_triple_terms(a)
                .map(|t| closure_satisfies(dict, closure, &t))
                .unwrap_or(false)
        })
    }

    // ----------------------------------------------------------- per-test drivers

    fn import_or_skip(src: &str) -> Result<Document, &'static str> {
        rif_xml::import(src.as_bytes()).map_err(|e| skip_bucket(&e))
    }

    /// (Positive|Negative)EntailmentTest. The NET vacuity rule lives here: an
    /// un-importable NET premise is a SKIP, never a vacuous pass.
    fn run_entailment(kind: TestKind, dir: &Path, id: &str) -> Outcome {
        let premise_src = match read_rif(dir, &rif_name(id, "premise")) {
            Some(s) => s,
            None => return Outcome::Skip("skip:condition-shape"),
        };
        let doc = match import_or_skip(&premise_src) {
            Ok(d) => d,
            Err(bucket) => return Outcome::Skip(bucket),
        };
        let mut dict = Dict::default();
        let closure = match doc.closure(&mut dict) {
            Ok(c) => c,
            Err(_) => return Outcome::Skip("skip:condition-shape"),
        };
        let concl_suffix = match kind {
            TestKind::PositiveEntailment => "conclusion",
            TestKind::NegativeEntailment => "nonconclusion",
            _ => unreachable!("run_entailment only for entailment kinds"),
        };
        let concl_src = match read_rif(dir, &rif_name(id, concl_suffix)) {
            Some(s) => s,
            None => return Outcome::Skip("skip:condition-shape"),
        };
        let concl_doc = match import_or_skip(&wrap_conclusion(&concl_src)) {
            Ok(d) => d,
            Err(_) => return Outcome::Skip("skip:condition-shape"),
        };
        let atoms = match conclusion_ground_triples(&concl_doc) {
            Some(a) => a,
            None => return Outcome::Skip("skip:condition-shape"),
        };
        let entailed = closure_entails_conclusion(&mut dict, &closure, &atoms);
        match (kind, entailed) {
            (TestKind::PositiveEntailment, true) | (TestKind::NegativeEntailment, false) => {
                Outcome::Pass
            }
            (TestKind::PositiveEntailment, false) => {
                Outcome::Fail(format!("{}: conclusion NOT entailed by closure", id))
            }
            (TestKind::NegativeEntailment, true) => {
                Outcome::Fail(format!("{}: non-conclusion WAS entailed by closure", id))
            }
            _ => unreachable!(),
        }
    }

    /// PositiveSyntaxTest: the input SHOULD import; an out-of-subset construct is a
    /// SKIP (importer completeness gap, not a wrong answer).
    fn run_positive_syntax(dir: &Path, id: &str) -> Outcome {
        let src = match read_rif(dir, &rif_name(id, "input")) {
            Some(s) => s,
            None => return Outcome::Skip("skip:condition-shape"),
        };
        match import_or_skip(&src) {
            Ok(_) => Outcome::Pass,
            Err(bucket) => Outcome::Skip(bucket),
        }
    }

    /// NegativeSyntaxTest / ImportRejectionTest: the input SHOULD be rejected. PASS
    /// only on a GENUINE detection of a modelled Core violation (or a genuine
    /// `InconsistentImport`); an unsupported-construct / blanket-import rejection is
    /// VACUOUS and stays a named SKIP — the syntax-polarity vacuity guard.
    fn run_negative_syntax(dir: &Path, id: &str) -> Outcome {
        let src = match read_rif(dir, &rif_name(id, "input")) {
            Some(s) => s,
            None => return Outcome::Skip("skip:condition-shape"),
        };
        match rif_xml::import(src.as_bytes()) {
            Ok(_) => Outcome::Fail(format!("{}: importer ACCEPTED a should-reject input", id)),
            Err(e) => {
                let genuinely_modelled = match &e {
                    ImportError::ValidationFailed(re) => is_genuine_core_violation(re),
                    ImportError::InconsistentImport { .. } => true,
                    _ => false,
                };
                if genuinely_modelled {
                    Outcome::Pass
                } else {
                    Outcome::Skip(skip_bucket(&e))
                }
            }
        }
    }

    fn run_one(kind: TestKind, dir: &Path, id: &str) -> Outcome {
        match kind {
            TestKind::PositiveEntailment | TestKind::NegativeEntailment => {
                run_entailment(kind, dir, id)
            }
            TestKind::PositiveSyntax => run_positive_syntax(dir, id),
            TestKind::NegativeSyntax | TestKind::ImportRejection => run_negative_syntax(dir, id),
        }
    }

    // ------------------------------------------------------------------- walking

    fn repo_root() -> PathBuf {
        // CARGO_MANIFEST_DIR is crates/sparq-reason.
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
    }

    /// Every `<root>/<TestType>/<id>/` case directory of a kind, sorted.
    fn tests_of_kind(root: &Path, kind: TestKind) -> Vec<(String, PathBuf)> {
        let base = root.join(kind.dir());
        let mut out = Vec::new();
        if let Ok(rd) = std::fs::read_dir(&base) {
            for entry in rd.flatten() {
                let p = entry.path();
                if p.is_dir() {
                    if let Some(id) = p.file_name().and_then(|s| s.to_str()) {
                        out.push((id.to_string(), p.clone()));
                    }
                }
            }
        }
        out.sort_by(|a, b| a.0.cmp(&b.0));
        out
    }

    struct Tally {
        pass: usize,
        fails: Vec<String>,
        skips: BTreeMap<&'static str, usize>,
        total: usize,
    }

    fn run_suite(root: &Path, mut per_case: impl FnMut(TestKind, &str, &Outcome)) -> Tally {
        let mut t = Tally {
            pass: 0,
            fails: Vec::new(),
            skips: BTreeMap::new(),
            total: 0,
        };
        for &kind in TestKind::ALL {
            for (id, dir) in tests_of_kind(root, kind) {
                t.total += 1;
                let outcome = run_one(kind, &dir, &id);
                per_case(kind, &id, &outcome);
                match &outcome {
                    Outcome::Pass => t.pass += 1,
                    Outcome::Skip(bucket) => *t.skips.entry(*bucket).or_default() += 1,
                    Outcome::Fail(why) => t.fails.push(why.clone()),
                }
            }
        }
        t
    }

    fn print_tally(label: &str, t: &Tally) {
        let skip_total: usize = t.skips.values().sum();
        // Pass-rate claims are backed by THIS line only — failures listed below,
        // never rounded up (the sq-hmd7l.24 invariant).
        println!(
            "TOTAL {} pass {} fail {} skip {} of {}",
            label,
            t.pass,
            t.fails.len(),
            skip_total,
            t.total
        );
        if skip_total > 0 {
            println!(
                "  skip taxonomy (the honest denominator, {} of {}):",
                skip_total, t.total
            );
            for (bucket, n) in &t.skips {
                println!("    {} {}", n, bucket);
            }
        }
        if !t.fails.is_empty() {
            println!("  FAILS (genuine wrong answers on the supported subset):");
            for f in &t.fails {
                println!("    - {}", f);
            }
        }
    }

    // ------------------------------------------------------------------- modes

    /// `--smoke`: the vendored, sparq-authored subset with PINNED per-case verdicts.
    fn run_smoke() -> i32 {
        let suite = repo_root().join("bench/rif/suite");
        let expected_path = repo_root().join("bench/rif/expected.tsv");
        let expected_raw = match std::fs::read_to_string(&expected_path) {
            Ok(s) => s,
            Err(e) => {
                eprintln!(
                    "ERROR: cannot read pinned expectations {}: {}",
                    expected_path.display(),
                    e
                );
                return 1;
            }
        };
        // kind<TAB>id<TAB>verdict lines; '#' comments.
        let mut expected: BTreeMap<(String, String), String> = BTreeMap::new();
        for line in expected_raw.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let mut f = line.split('\t');
            match (f.next(), f.next(), f.next()) {
                (Some(kind), Some(id), Some(verdict)) => {
                    expected.insert((kind.to_string(), id.to_string()), verdict.to_string());
                }
                _ => {
                    eprintln!("ERROR: malformed expected.tsv line: {}", line);
                    return 1;
                }
            }
        }

        println!(
            "rif-conformance --smoke: vendored sparq-authored subset (NOT W3C files — \
             license gate; see bench/rif/README.md)"
        );
        let mut drift: Vec<String> = Vec::new();
        let mut seen: Vec<(String, String)> = Vec::new();
        let tally = run_suite(&suite, |kind, id, outcome| {
            let key = (kind.dir().to_string(), id.to_string());
            let got = outcome.token();
            match expected.get(&key) {
                Some(want) if *want == got => {
                    println!("  {}/{}: {} (pinned) OK", kind.dir(), id, got);
                }
                Some(want) => {
                    drift.push(format!(
                        "{}/{}: got {} but expected.tsv pins {}",
                        kind.dir(),
                        id,
                        got,
                        want
                    ));
                }
                None => {
                    drift.push(format!(
                        "{}/{}: got {} but the case is NOT pinned in expected.tsv",
                        kind.dir(),
                        id,
                        got
                    ));
                }
            }
            seen.push(key);
        });
        // A pinned case that never ran (deleted/renamed fixture) is drift too.
        for key in expected.keys() {
            if !seen.contains(key) {
                drift.push(format!(
                    "{}/{}: pinned in expected.tsv but absent from bench/rif/suite",
                    key.0, key.1
                ));
            }
        }
        print_tally("rif-smoke", &tally);
        if tally.total == 0 {
            eprintln!(
                "ERROR: no vendored cases found under {} — smoke MUST assert, exit 1",
                suite.display()
            );
            return 1;
        }
        if !drift.is_empty() {
            println!("SMOKE DRIFT vs pinned expected.tsv:");
            for d in &drift {
                println!("  - {}", d);
            }
            return 1;
        }
        println!(
            "smoke OK: all {} vendored verdicts match the pinned expected.tsv",
            tally.total
        );
        0
    }

    /// Full mode: per-dialect pass-rate over the fetched W3C archive.
    fn run_full() -> i32 {
        let root = repo_root().join("tests/w3c/rif-core");
        println!(
            "rif-conformance: W3C RIF suite pass-rate (CONFORMANCE-ONLY — the RIF axis \
             verdict is NOT-COMPARABLE: no runnable open-source RIF peer exists to race; \
             no wall-clock is measured here. See research/gap-rif-2026-07.md)"
        );
        // Dialect archives are `<Dialect>_v<ver>/Approved/…` (e.g. Core_v1.22).
        let mut archives: Vec<(String, String, PathBuf)> = Vec::new();
        if let Ok(rd) = std::fs::read_dir(&root) {
            for entry in rd.flatten() {
                let p = entry.path();
                if !p.is_dir() {
                    continue;
                }
                let name = match p.file_name().and_then(|s| s.to_str()) {
                    Some(n) => n.to_string(),
                    None => continue,
                };
                let dialect = name.split("_v").next().unwrap_or(&name).to_string();
                let approved = p.join("Approved");
                if approved.is_dir() {
                    archives.push((dialect, name, approved));
                }
            }
        }
        archives.sort_by(|a, b| a.1.cmp(&b.1));
        if archives.is_empty() {
            println!(
                "SKIP: no fetched W3C RIF archive under {} — run \
                 scripts/fetch-inference-suites.sh, then re-run. (Graceful skip, exit 0: \
                 the archive is fetched, not vendored — no redistribution license. The \
                 self-asserting lane is `--smoke`; the Core pass-count RATCHET lives in \
                 crates/sparq-conformance/tests/rif_wg_core_suite.rs.)",
                root.display()
            );
            return 0;
        }
        let mut any_fail = false;
        for (dialect, archive, approved) in &archives {
            println!("\ndialect {} (pinned archive {}):", dialect, archive);
            let tally = run_suite(approved, |_, _, _| {});
            print_tally(&format!("rif-{}", dialect.to_lowercase()), &tally);
            any_fail = any_fail || !tally.fails.is_empty();
        }
        // Genuine wrong answers red the run; skips never do (they are the honest
        // denominator, not failures).
        if any_fail {
            1
        } else {
            0
        }
    }

    pub fn run(args: Vec<String>) -> i32 {
        match args.first().map(|s| s.as_str()) {
            Some("--smoke") => run_smoke(),
            None => run_full(),
            Some(other) => {
                eprintln!(
                    "unknown argument '{}' — usage: rif_conformance [--smoke]",
                    other
                );
                2
            }
        }
    }
}
