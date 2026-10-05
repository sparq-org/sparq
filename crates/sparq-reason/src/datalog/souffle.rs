//! [SONNET-4.6] sq-xzb9p — the **external-engine differential arm** (test-only):
//! translate a [`Program`] into Soufflé Datalog, run the real
//! [Soufflé](https://souffle-lang.github.io/) engine over the same facts, and compare
//! its closure with ours.
//!
//! # Why an EXTERNAL arm when `oracle` already exists
//!
//! [`super::oracle`] is an independent *implementation* — but it is our code, written
//! by the same hands, from the same reading of the same design record. A shared
//! misreading of stratified semantics would agree with itself. Soufflé is the
//! reference implementation of stratified Datalog with negation, written by people who
//! never saw this repository, so an agreement is evidence the *semantics* are right and
//! not merely that our two evaluators match.
//!
//! Critically, the translation does **not** consult [`super::stratify`]: it emits one
//! Soufflé relation per dependency node and lets Soufflé compute its OWN stratification.
//! A layered translation keyed on our strata would have re-encoded our answer into the
//! question and could never catch a stratification bug; this one can, in both
//! directions — Soufflé must accept what we accept and reject what we reject.
//!
//! # The fragment this arm covers (deliberately narrow — see the honesty note)
//!
//! Triple atoms with a CONSTANT predicate, recursion, single-atom `NOT`, grouped
//! `NOT`/`NOT EXISTS`, and multi-atom heads. Everything else is a loud
//! [`translate`] error naming the construct:
//!
//! * **`AGGREGATE` and `FILTER`** are out of scope *on purpose*. Their semantics are
//!   term-level and XSD-typed (the shared substrate numeric tower, XPath operand
//!   promotion, the pinned float fold order). Soufflé's domain is untyped symbols and
//!   machine numbers, so expressing them would mean re-implementing the numeric tower
//!   inside the translator — at which point the "external reference" would be running
//!   largely on our own code and would stop being independent evidence. Those paths
//!   keep the in-tree [`super::oracle`] differential, which does test them.
//! * **Variable predicates / variable-class `rdf:type`** are out of scope: they map to
//!   the conservative `Top`/`TypeAny` nodes, which have no per-relation counterpart.
//!
//! # Encoding
//!
//! * One relation per dependency node, matching [`super::stratify`]'s granularity:
//!   `p<i>_<name>(s, o)` per constant predicate, and `c<i>_<name>(s)` per `rdf:type`
//!   CLASS. Per-class relations are what let Soufflé corroborate the class-granularity
//!   decision — a predicate-granular encoding would reject programs we accept.
//! * A term is the symbol of its canonical `oxrdf` rendering (`<http://ex/s>`,
//!   `"y"`, `"3"^^<…integer>`), so symbol equality is dictionary-id equality.
//! * Every relation is both `.input` and `.output`: a predicate may be EDB, IDB, or
//!   both, and outputting the EDB too makes the comparison total rather than
//!   derivation-only.
//! * A `NOT` group becomes an auxiliary relation projecting exactly the variables the
//!   group shares with the rule's positive body; variables local to the group are
//!   existentially quantified by that projection. This is precisely the dialect's
//!   "wildcard bindings join every atom in that group and are discarded outside it",
//!   and it keeps every negated atom grounded (an uncorrelated group projects to a
//!   nullary relation).
//!
//! # Running it
//!
//! Soufflé is an EXTERNAL BINARY, never a cargo dependency — this arm adds no crate to
//! the graph and needs no `cargo-vet` audit. When the binary is absent the fixtures
//! print why and skip, exactly as `sparq-metamorph`'s live-endpoint probes do; a
//! skipped fixture reports "not checked", never "confirmed". Set
//! `SPARQ_DATALOG_SOUFFLE_REQUIRED=1` (as the optional CI lane does) to turn absence
//! into a failure, and `SPARQ_DATALOG_SOUFFLE=/path/to/souffle` to name a binary that
//! is not on `PATH`.
//!
//! ```sh
//! cargo test -p sparq-reason --features datalog datalog::tests::souffle -- --nocapture
//! ```

use super::{bound_slots, Atom, DTerm, Program, Rule};
use rustc_hash::{FxHashMap, FxHashSet};
use sparq_core::dict::{Dict, Id};
use std::path::PathBuf;

const RDF_TYPE: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";

/// How a Soufflé relation's tuples map back to RDF triples.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Shape {
    /// Arity 2, `(subject, object)` under a fixed predicate.
    Pred(Id),
    /// Arity 1, `(subject)` under `rdf:type` with a fixed class.
    Class(Id),
}

/// A dependency node, mirroring `stratify`'s `Key` for the fragment we translate.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum Node {
    Pred(Id),
    Class(Id),
}

/// A translated program: the Soufflé source, the fact tables to feed it, and the
/// bookkeeping needed to read its answer back into dictionary ids.
#[derive(Debug)]
pub(super) struct Translation {
    /// The `.dl` program text (pinned by golden tests — it is the artifact reviewers read).
    pub(super) source: String,
    /// `(relation name, rows)` for every relation, in emission order. Rows are already
    /// TSV-encoded fact lines.
    facts: Vec<(String, Vec<String>)>,
    /// `(relation name, shape)` for every relation, in emission order.
    outputs: Vec<(String, Shape)>,
    /// Canonical term rendering back to the dictionary id it came from.
    by_symbol: FxHashMap<String, Id>,
    /// The dependency nodes present, so the comparison can be restricted to the
    /// triples this program can actually talk about.
    nodes: FxHashSet<Node>,
    /// `rdf:type`'s id, when the dictionary knows it.
    ty: Option<Id>,
}

/// Render a term the way Soufflé will see it: the canonical `oxrdf` form.
fn symbol(dict: &Dict, id: Id) -> String {
    dict.term(id).to_string()
}

/// Escape a symbol for a Soufflé string literal, rejecting anything that would
/// corrupt the tab-separated fact files rather than silently mis-encoding it.
fn quote(sym: &str) -> Result<String, String> {
    if let Some(bad) = sym.chars().find(|c| c.is_control()) {
        return Err(format!(
            "term {:?} contains the control character {:?}, which Soufflé's \
             tab-separated fact format cannot carry",
            sym, bad
        ));
    }
    let mut out = String::with_capacity(sym.len() + 2);
    out.push('"');
    for c in sym.chars() {
        if c == '"' || c == '\\' {
            out.push('\\');
        }
        out.push(c);
    }
    out.push('"');
    Ok(out)
}

/// A readable, collision-free Soufflé identifier fragment for an IRI: its last
/// path/fragment segment with non-alphanumerics folded to `_`. Uniqueness comes from
/// the caller's index prefix, never from this.
fn slug(dict: &Dict, id: Id) -> String {
    let term = symbol(dict, id);
    let tail = term
        .trim_start_matches('<')
        .trim_end_matches('>')
        .rsplit(['/', '#'])
        .next()
        .unwrap_or("");
    let cleaned: String = tail
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    if cleaned.is_empty() {
        "x".to_string()
    } else {
        cleaned
    }
}

/// The dependency node an atom reads from / derives into, or an error naming the
/// construct that falls outside this arm's fragment.
fn atom_node(atom: &Atom, ty: Option<Id>) -> Result<Node, String> {
    let Some(pred) = atom.pred else {
        return Err("variable predicates are outside the Soufflé arm's fragment \
                    (they map to the conservative `Top` node, which has no \
                    per-relation counterpart)"
            .to_string());
    };
    if ty == Some(pred) {
        match atom.t[2] {
            DTerm::Const(c) => Ok(Node::Class(c)),
            DTerm::Var(_) => Err("variable-class `rdf:type` atoms are outside the \
                                  Soufflé arm's fragment (the conservative `TypeAny` node)"
                .to_string()),
        }
    } else {
        Ok(Node::Pred(pred))
    }
}

/// Reject the constructs this arm deliberately does not model, naming them.
fn check_fragment(rule: &Rule) -> Result<(), String> {
    if !rule.aggregates.is_empty() {
        return Err("AGGREGATE atoms are outside the Soufflé arm's fragment by design \
                    — see the module docs (XSD-typed aggregation has no faithful \
                    untyped-symbol encoding); the in-tree oracle covers them"
            .to_string());
    }
    if !rule.filters.is_empty() {
        return Err("FILTER conditions are outside the Soufflé arm's fragment by design \
                    — see the module docs (the shared substrate numeric tower has no \
                    faithful Soufflé encoding); the in-tree oracle covers them"
            .to_string());
    }
    Ok(())
}

/// Interned relation names, assigned in first-encounter order so the emitted source is
/// deterministic.
struct Relations {
    index: FxHashMap<Node, usize>,
    order: Vec<(Node, String)>,
}

impl Relations {
    fn new() -> Self {
        Relations {
            index: FxHashMap::default(),
            order: Vec::new(),
        }
    }

    fn intern(&mut self, dict: &Dict, node: Node) -> String {
        if let Some(&i) = self.index.get(&node) {
            return self.order[i].1.clone();
        }
        let i = self.order.len();
        let name = match node {
            Node::Pred(p) => format!("p{}_{}", i, slug(dict, p)),
            Node::Class(c) => format!("c{}_{}", i, slug(dict, c)),
        };
        self.index.insert(node, i);
        self.order.push((node, name.clone()));
        name
    }
}

/// Render one atom as a Soufflé literal, e.g. `p0_edge(v1, "<http://ex/b>")`.
fn atom_literal(
    dict: &Dict,
    rels: &mut Relations,
    atom: &Atom,
    ty: Option<Id>,
    consts: &mut Vec<Id>,
) -> Result<String, String> {
    let node = atom_node(atom, ty)?;
    let name = rels.intern(dict, node);
    let mut arg = |t: &DTerm| -> Result<String, String> {
        match t {
            DTerm::Var(v) => Ok(format!("v{}", v)),
            DTerm::Const(c) => {
                consts.push(*c);
                quote(&symbol(dict, *c))
            }
        }
    };
    let args = match node {
        // The class is already in the relation name; only the subject is a column.
        Node::Class(_) => arg(&atom.t[0])?,
        Node::Pred(_) => format!("{}, {}", arg(&atom.t[0])?, arg(&atom.t[2])?),
    };
    Ok(format!("{}({})", name, args))
}

/// The variable slots an atom mentions.
fn atom_vars(atom: &Atom, out: &mut Vec<u32>) {
    for t in &atom.t {
        if let DTerm::Var(v) = t {
            if !out.contains(v) {
                out.push(*v);
            }
        }
    }
}

/// Translate `program` plus `facts` into a runnable Soufflé problem.
///
/// # Errors
///
/// Returns `Err` naming the construct when the program uses anything outside the
/// fragment documented at the module level — never a silent partial translation.
pub(super) fn translate(
    dict: &Dict,
    program: &Program,
    facts: &[[Id; 3]],
) -> Result<Translation, String> {
    let ty = {
        let id = dict.lookup(&oxrdf::NamedNode::new_unchecked(RDF_TYPE).into());
        (id != 0).then_some(id)
    };
    let mut rels = Relations::new();
    let mut consts: Vec<Id> = Vec::new();
    let mut aux_decls: Vec<String> = Vec::new();
    let mut clauses: Vec<String> = Vec::new();

    for (ri, rule) in program.rules.iter().enumerate() {
        check_fragment(rule)?;
        let bound = bound_slots(rule);
        let mut body: Vec<String> = Vec::new();
        for atom in &rule.positive {
            body.push(atom_literal(dict, &mut rels, atom, ty, &mut consts)?);
        }
        // One auxiliary relation per NOT group: project the variables the group shares
        // with the positive body, existentially quantifying the rest.
        for (gi, group) in rule.negated.iter().enumerate() {
            let mut vars: Vec<u32> = Vec::new();
            for atom in group {
                atom_vars(atom, &mut vars);
            }
            let mut shared: Vec<u32> = vars.into_iter().filter(|v| bound.contains(v)).collect();
            shared.sort_unstable();
            let aux = format!("neg{}_{}", ri, gi);
            let params: Vec<String> = shared.iter().map(|v| format!("v{}: symbol", v)).collect();
            let args: Vec<String> = shared.iter().map(|v| format!("v{}", v)).collect();
            let mut group_body: Vec<String> = Vec::new();
            for atom in group {
                group_body.push(atom_literal(dict, &mut rels, atom, ty, &mut consts)?);
            }
            aux_decls.push(format!(".decl {}({})", aux, params.join(", ")));
            clauses.push(format!(
                "{}({}) :- {}.",
                aux,
                args.join(", "),
                group_body.join(", ")
            ));
            body.push(format!("!{}({})", aux, args.join(", ")));
        }
        // Multi-atom heads become one clause per head atom over the same body.
        for head in &rule.head {
            let h = atom_literal(dict, &mut rels, head, ty, &mut consts)?;
            clauses.push(format!("{} :- {}.", h, body.join(", ")));
        }
    }

    // Distribute the input facts into the relations the program can read.
    let nodes: FxHashSet<Node> = rels.index.keys().copied().collect();
    let mut rows: FxHashMap<usize, Vec<String>> = FxHashMap::default();
    let mut by_symbol: FxHashMap<String, Id> = FxHashMap::default();
    let remember = |dict: &Dict, id: Id, by: &mut FxHashMap<String, Id>| -> Result<(), String> {
        let sym = symbol(dict, id);
        if sym.chars().any(|c| c.is_control()) {
            return Err(format!(
                "term {:?} contains a control character, which Soufflé's \
                 tab-separated fact format cannot carry",
                sym
            ));
        }
        by.insert(sym, id);
        Ok(())
    };
    for id in consts {
        remember(dict, id, &mut by_symbol)?;
    }
    for &[s, p, o] in facts {
        let (node, row) = if ty == Some(p) {
            (Node::Class(o), vec![s])
        } else {
            (Node::Pred(p), vec![s, o])
        };
        let Some(&i) = rels.index.get(&node) else {
            continue; // No atom can read this relation, so it cannot affect the closure.
        };
        for &id in &row {
            remember(dict, id, &mut by_symbol)?;
        }
        let line: Vec<String> = row.iter().map(|&id| symbol(dict, id)).collect();
        rows.entry(i).or_default().push(line.join("\t"));
    }

    // Emit: declarations first (Soufflé needs a relation declared before use), then
    // the auxiliary declarations, then every clause, then the outputs.
    let mut source = String::new();
    source.push_str(
        "// Generated by sparq-reason datalog::souffle (sq-xzb9p). Do not edit.\n\
         // One relation per stratification node; Soufflé stratifies this itself.\n",
    );
    let mut outputs: Vec<(String, Shape)> = Vec::new();
    let mut fact_tables: Vec<(String, Vec<String>)> = Vec::new();
    for (i, (node, name)) in rels.order.iter().enumerate() {
        let (params, shape) = match *node {
            Node::Pred(p) => ("s: symbol, o: symbol", Shape::Pred(p)),
            Node::Class(c) => ("s: symbol", Shape::Class(c)),
        };
        source.push_str(&format!(".decl {}({})\n.input {}\n", name, params, name));
        outputs.push((name.clone(), shape));
        fact_tables.push((name.clone(), rows.remove(&i).unwrap_or_default()));
    }
    for decl in &aux_decls {
        source.push_str(decl);
        source.push('\n');
    }
    for clause in &clauses {
        source.push_str(clause);
        source.push('\n');
    }
    for (name, _) in &outputs {
        source.push_str(&format!(".output {}\n", name));
    }

    Ok(Translation {
        source,
        facts: fact_tables,
        outputs,
        by_symbol,
        nodes,
        ty,
    })
}

impl Translation {
    /// Whether a triple lives in a relation this program models — the projection the
    /// comparison is meaningful on (facts under unread predicates never reach Soufflé).
    pub(super) fn covers(&self, [_, p, o]: [Id; 3]) -> bool {
        let node = if self.ty == Some(p) {
            Node::Class(o)
        } else {
            Node::Pred(p)
        };
        self.nodes.contains(&node)
    }

    /// Run Soufflé at `bin` over this translation and read its closure back as triples.
    ///
    /// # Errors
    ///
    /// Returns `Err` when Soufflé rejects the program (notably: "Unable to stratify"),
    /// when it fails to run, or when it answers with a symbol that was never fed in.
    pub(super) fn run(&self, bin: &str) -> Result<FxHashSet<[Id; 3]>, String> {
        let dir = scratch_dir();
        let facts_dir = dir.join("facts");
        let out_dir = dir.join("out");
        let result = self.run_in(bin, &dir, &facts_dir, &out_dir);
        let _ = std::fs::remove_dir_all(&dir);
        result
    }

    fn run_in(
        &self,
        bin: &str,
        dir: &std::path::Path,
        facts_dir: &std::path::Path,
        out_dir: &std::path::Path,
    ) -> Result<FxHashSet<[Id; 3]>, String> {
        let io = |e: std::io::Error| format!("souffle scratch dir: {}", e);
        std::fs::create_dir_all(facts_dir).map_err(io)?;
        std::fs::create_dir_all(out_dir).map_err(io)?;
        let program_path = dir.join("program.dl");
        std::fs::write(&program_path, &self.source).map_err(io)?;
        for (name, rows) in &self.facts {
            let mut body = rows.join("\n");
            if !body.is_empty() {
                body.push('\n');
            }
            std::fs::write(facts_dir.join(format!("{}.facts", name)), body).map_err(io)?;
        }
        let output = std::process::Command::new(bin)
            .arg("-F")
            .arg(facts_dir)
            .arg("-D")
            .arg(out_dir)
            .arg(&program_path)
            .output()
            .map_err(|e| format!("failed to run {}: {}", bin, e))?;
        if !output.status.success() {
            return Err(format!(
                "souffle rejected the program ({}): {}",
                output.status,
                String::from_utf8_lossy(&output.stderr).trim()
            ));
        }
        let mut closure: FxHashSet<[Id; 3]> = FxHashSet::default();
        for (name, shape) in &self.outputs {
            let path = out_dir.join(format!("{}.csv", name));
            // Soufflé omits nothing here (every relation is `.input`), but an empty
            // relation may still be an empty file.
            let text = std::fs::read_to_string(&path).unwrap_or_default();
            for line in text.lines().filter(|l| !l.is_empty()) {
                let cols: Vec<&str> = line.split('\t').collect();
                let lookup = |sym: &str| -> Result<Id, String> {
                    self.by_symbol.get(sym).copied().ok_or_else(|| {
                        format!(
                            "souffle relation {} returned the symbol {:?}, which was \
                             never fed in — the fragment is supposed to invent no terms",
                            name, sym
                        )
                    })
                };
                let triple = match (shape, cols.as_slice()) {
                    (Shape::Class(c), [s]) => [lookup(s)?, self.ty.expect("class node"), *c],
                    (Shape::Pred(p), [s, o]) => [lookup(s)?, *p, lookup(o)?],
                    _ => {
                        return Err(format!(
                            "souffle relation {} returned {} columns, which does not \
                             match its declared shape {:?}",
                            name,
                            cols.len(),
                            shape
                        ))
                    }
                };
                closure.insert(triple);
            }
        }
        Ok(closure)
    }
}

/// A process-unique scratch directory (no `tempfile` dependency — this arm adds none).
fn scratch_dir() -> PathBuf {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    std::env::temp_dir().join(format!(
        "sparq-datalog-souffle-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ))
}

/// Locate the Soufflé binary: `SPARQ_DATALOG_SOUFFLE`, else `souffle` on `PATH`.
///
/// Returns `None` when it is not runnable — unless `SPARQ_DATALOG_SOUFFLE_REQUIRED=1`,
/// which turns absence into a panic so the optional CI lane cannot pass by skipping.
pub(super) fn binary() -> Option<String> {
    let named = std::env::var("SPARQ_DATALOG_SOUFFLE").ok();
    let bin = named.clone().unwrap_or_else(|| "souffle".to_string());
    let runnable = std::process::Command::new(&bin)
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success());
    if runnable {
        return Some(bin);
    }
    if std::env::var("SPARQ_DATALOG_SOUFFLE_REQUIRED").as_deref() == Ok("1") {
        panic!(
            "SPARQ_DATALOG_SOUFFLE_REQUIRED=1 but the Soufflé binary {:?} is not \
             runnable — the external-engine differential arm cannot report a result",
            bin
        );
    }
    println!(
        "skipping the Soufflé differential arm: {:?} is not runnable (set \
         SPARQ_DATALOG_SOUFFLE to a binary, or SPARQ_DATALOG_SOUFFLE_REQUIRED=1 to \
         make this a failure). NOT CHECKED — not confirmed.",
        bin
    );
    None
}
