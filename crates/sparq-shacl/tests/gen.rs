//! sq-55c1 — seeded generator of VALID SHACL shapes + data graphs for
//! the differential fuzzer (`diff_fuzz.rs`).
//!
//! Deterministic SplitMix64 (the same RNG `sparq-bench/src/fuzz.rs` uses), so any
//! case reproduces from its seed alone. A `Scenario` is one node shape with a
//! `sh:targetClass`, a handful of property shapes (each carrying one or more
//! constraint components from the in-scope set below), plus a small data graph of
//! instances of that class — each instance independently nudged toward conforming
//! or violating one of the property shapes.
//!
//! ## In-scope constraint components (the comparable subset — what is GENERATED)
//! These are generated and their reports compared against the reference engine:
//!   sh:minCount, sh:maxCount (cardinality, on every SIMPLE-path property shape)
//!   and one value component per property shape drawn from: sh:datatype,
//!   sh:nodeKind, sh:class, sh:pattern, sh:in, sh:minLength, sh:maxLength,
//!   sh:hasValue, sh:minInclusive, sh:maxInclusive, sh:minExclusive,
//!   sh:maxExclusive.
//!
//! (sq-0hj7) Extended to also generate, as the property shape's value
//! component:
//!   - the **logical components** sh:and / sh:or / sh:not / sh:xone, each over a
//!     small list of inline member node-shapes that carry one of the simple value
//!     components above. The generator drives each value node to a known
//!     per-member conform/violate decision, then derives the composite's expected
//!     outcome (∧ / ∨ / ¬ / exactly-one) — the differential only compares the
//!     (focus, component, path) violation set, so the bespoke per-member messages
//!     don't matter.
//!   - **sh:node** nested-shape references: an inline member node-shape carrying a
//!     simple value component, applied to the value node. This also stresses the
//!     (focus, shape) conformance memo in the evaluator.
//!
//! ## Complex SHAPE paths (sq-0hj7)
//! The property shape's `sh:path` is now a [`PathSpec`], not just a bare
//! predicate: sequence / inverse / oneOrMore / zeroOrMore / zeroOrOne forms are
//! generated alongside simple predicates, and the data graph is wired so values
//! are reachable along the chosen path. The `(focus, component, path)` comparison
//! key already maps any non-`Predicate` path to the `_:path` sentinel on both
//! sides (sparq's `norm_path` and the pySHACL adapter's `_path_key`), so complex
//! paths compare by presence + the (focus, component) pair — exactly the bead's
//! "relax the path-key to the `_:path` sentinel" disposition. Path forms whose
//! value-node set INCLUDES the focus itself (zeroOrOne / zeroOrMore) are only
//! emitted WITHOUT a value component (count-free), so the focus node is never
//! checked against a value constraint it wasn't meant to satisfy.
//!
//! ## Deliberately OUT of scope (documented; tracked as TODO beads where useful)
//!   - sh:closed, sh:qualifiedValueShape, sh:disjoint/equals/lessThan*,
//!     sh:uniqueLang, sh:languageIn — future generator extensions (their report
//!     shapes / pySHACL message-paths need extra normalisation).
//!   - sh:sparql / SPARQL constraint COMPONENTS — pySHACL and sparq both support
//!     them but their report focus/value semantics differ enough to need bespoke
//!     comparison; left for a follow-up bead.
//!   - sh:severity other than the default sh:Violation — the differential compares
//!     the conformance bit + violated-component set, which is severity-agnostic by
//!     construction here (everything is a Violation).

#![allow(dead_code)] // shared test module: not every helper is used by every test

/// Deterministic SplitMix64 — no clock/entropy, so every case reproduces from
/// its seed. Same construction as `sparq-bench/src/fuzz.rs`.
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Rng(seed.wrapping_mul(0x9e37_79b9_7f4a_7c15).wrapping_add(1))
    }
    // Named `next_u64` (not `next`) so it cannot be confused with
    // `Iterator::next` (clippy::should_implement_trait).
    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }
    pub fn below(&mut self, n: u64) -> u64 {
        self.next_u64() % n.max(1)
    }
    pub fn chance(&mut self, num: u64, den: u64) -> bool {
        self.below(den) < num
    }
    pub fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        &items[self.below(items.len() as u64) as usize]
    }
}

const EX: &str = "http://example.org/";

/// One in-scope constraint component on a property shape, with the Turtle it
/// contributes to the shapes graph and a generator for object values that
/// conform vs violate it.
#[derive(Clone, Debug)]
pub enum Constraint {
    Datatype(&'static str), // xsd local name (integer/string/boolean/decimal)
    NodeKind(&'static str), // sh:IRI / sh:Literal / sh:BlankNode
    Class(String),          // sh:class <iri>
    Pattern(&'static str),  // sh:pattern (a regex string)
    In(Vec<String>),        // sh:in ( ... ) — object terms (Turtle)
    MinLength(u64),
    MaxLength(u64),
    HasValue(String), // sh:hasValue <term> (object Turtle)
    MinInclusive(i64),
    MaxInclusive(i64),
    MinExclusive(i64),
    MaxExclusive(i64),
    // (sq-0hj7) Logical components sh:and/or/not/xone over inline
    // member node-shapes. Each member is an `sh:in (...)` token list (see
    // `Logical::member_lines` / the conform/violate construction below) chosen so
    // ONE value's per-member conformance is independently controllable, so the
    // composite's expected outcome is computable.
    Logical(Logical),
    // (sq-0hj7) `sh:node <inline shape>` — a nested node-shape that
    // applies an inner value constraint to the VALUE NODE. Conform/violate of the
    // composite is exactly conform/violate of the inner constraint, so we delegate.
    Node(Box<Constraint>),
}

/// (sq-0hj7) A logical component over inline `sh:in`-token member
/// node-shapes. The member encoding makes one value's per-member conformance
/// independently controllable so the composite outcome is derivable:
///
/// - `And`: every member list contains a shared `KEEP` token, so emitting `KEEP`
///   conforms to all; an out-of-list token conforms to none.
/// - `Or`: member lists are disjoint singletons (`tok0`, `tok1`, …); `tok0`
///   conforms to exactly one (so OR holds); an out-of-list token to none.
/// - `Xone`: disjoint singletons too; `tok0` conforms to exactly one (XONE holds);
///   an out-of-list token to zero (XONE violated). A single value is in at most
///   one disjoint list, so the >1-match path can't arise.
/// - `Not`: a single member; `KEEP` conforms to it (so NOT is violated), an
///   out-of-list token does not (so NOT holds).
#[derive(Clone, Debug)]
pub struct Logical {
    op: LogicalOp,
    /// Number of member shapes (1 for `Not`, 2..=3 for the list operators).
    members: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LogicalOp {
    And,
    Or,
    Not,
    Xone,
}

/// The shared token every `And` member list contains and the single `Not` member
/// list contains — emitting it conforms to that/those member(s).
const KEEP: &str = "\"keep\"";
/// A token in NO member list — emitting it conforms to no member.
const MISS: &str = "\"miss-none\"";

impl Logical {
    fn keyword(&self) -> &'static str {
        match self.op {
            LogicalOp::And => "and",
            LogicalOp::Or => "or",
            LogicalOp::Not => "not",
            LogicalOp::Xone => "xone",
        }
    }

    /// The `sh:in` token list for member index `i`.
    fn member_list(&self, i: u64) -> String {
        match self.op {
            // every And member shares KEEP (plus a distinct filler so the lists
            // aren't byte-identical) → KEEP conforms to all of them.
            LogicalOp::And => format!("{KEEP} \"and{i}\""),
            // Or/Xone: disjoint singletons.
            LogicalOp::Or | LogicalOp::Xone => format!("\"opt{i}\""),
            // Not: a single member whose list is {KEEP}.
            LogicalOp::Not => KEEP.to_string(),
        }
    }

    /// The Turtle for the logical component: `sh:<kw> ( [member] … )`, or for
    /// `sh:not` the single `sh:not [member]` form.
    fn shape_line(&self) -> String {
        let member = |i: u64| format!("[ sh:in ( {} ) ]", self.member_list(i));
        if self.op == LogicalOp::Not {
            format!("sh:not {}", member(0))
        } else {
            let members: Vec<String> = (0..self.members).map(member).collect();
            format!("sh:{} ( {} )", self.keyword(), members.join(" "))
        }
    }

    /// A value that makes the COMPOSITE conform.
    fn conforming_value(&self) -> String {
        match self.op {
            // KEEP is in every And member list / is the Or/Xone… no: for Or/Xone
            // KEEP is in NO list, so use opt0 (in exactly one).
            LogicalOp::And => KEEP.to_string(),
            LogicalOp::Or | LogicalOp::Xone => "\"opt0\"".to_string(),
            // NOT holds when the value does NOT conform to the inner member: an
            // out-of-list token does not match {KEEP}.
            LogicalOp::Not => MISS.to_string(),
        }
    }

    /// A value that makes the COMPOSITE violate.
    fn violating_value(&self) -> String {
        match self.op {
            // in no list → conforms to no member → And/Or/Xone all fail (Xone via
            // the zero-match branch).
            LogicalOp::And | LogicalOp::Or | LogicalOp::Xone => MISS.to_string(),
            // NOT is violated when the value DOES conform to the inner member.
            LogicalOp::Not => KEEP.to_string(),
        }
    }
}

impl Constraint {
    /// The Turtle predicate-object pairs this component adds to a property shape
    /// (joined into the shape body by the caller).
    fn shape_lines(&self) -> Vec<String> {
        match self {
            Constraint::Datatype(dt) => vec![format!("sh:datatype xsd:{dt}")],
            Constraint::NodeKind(nk) => vec![format!("sh:nodeKind {nk}")],
            Constraint::Class(c) => vec![format!("sh:class <{c}>")],
            Constraint::Pattern(p) => vec![format!("sh:pattern \"{p}\"")],
            Constraint::In(vals) => vec![format!("sh:in ( {} )", vals.join(" "))],
            Constraint::MinLength(n) => vec![format!("sh:minLength {n}")],
            Constraint::MaxLength(n) => vec![format!("sh:maxLength {n}")],
            Constraint::HasValue(v) => vec![format!("sh:hasValue {v}")],
            Constraint::MinInclusive(n) => vec![format!("sh:minInclusive {n}")],
            Constraint::MaxInclusive(n) => vec![format!("sh:maxInclusive {n}")],
            Constraint::MinExclusive(n) => vec![format!("sh:minExclusive {n}")],
            Constraint::MaxExclusive(n) => vec![format!("sh:maxExclusive {n}")],
            Constraint::Logical(l) => vec![l.shape_line()],
            // sh:node references an inline member node-shape carrying the inner
            // value constraint, applied to the value node.
            Constraint::Node(inner) => {
                vec![format!("sh:node [ {} ]", inner.shape_lines().join(" ; "))]
            }
        }
    }

    /// An object term (Turtle) that SATISFIES this component.
    fn conforming_value(&self, rng: &mut Rng) -> String {
        match self {
            // ~1-in-3 of the time draw a boundary-but-still-valid
            // literal (0, negatives, empty string, …) to widen coverage; these
            // are still lexically valid for the datatype, so they keep CONFORMING
            // to a bare `sh:datatype` constraint while exercising edge values.
            Constraint::Datatype(dt) => {
                let valid = !rng.chance(1, 3);
                typed_literal(dt, rng, valid)
            }
            Constraint::NodeKind(nk) => match *nk {
                "sh:IRI" => format!("<{EX}thing{}>", rng.below(5)),
                "sh:Literal" => format!("\"lit{}\"", rng.below(5)),
                "sh:BlankNode" => "[ ]".to_string(),
                _ => "\"x\"".to_string(),
            },
            // sh:class — an instance typed as the class.
            Constraint::Class(_) => "<__CLASS_INSTANCE__>".to_string(),
            Constraint::Pattern(_) => "\"abc123\"".to_string(), // matches the patterns we emit
            Constraint::In(vals) => vals[rng.below(vals.len() as u64) as usize].clone(),
            Constraint::MinLength(n) => format!("\"{}\"", "x".repeat((*n as usize).max(1) + 1)),
            Constraint::MaxLength(n) => format!("\"{}\"", "x".repeat((*n as usize).min(2))),
            Constraint::HasValue(v) => v.clone(),
            Constraint::MinInclusive(n) => format!("{}", n + rng.below(3) as i64),
            Constraint::MaxInclusive(n) => format!("{}", n - rng.below(3) as i64),
            Constraint::MinExclusive(n) => format!("{}", n + 1 + rng.below(3) as i64),
            Constraint::MaxExclusive(n) => format!("{}", n - 1 - rng.below(3) as i64),
            Constraint::Logical(l) => l.conforming_value(),
            Constraint::Node(inner) => inner.conforming_value(rng),
        }
    }

    /// An object term (Turtle) that VIOLATES this component.
    fn violating_value(&self, rng: &mut Rng) -> String {
        match self {
            // wrong datatype: a string where an integer is wanted, and vice-versa.
            Constraint::Datatype(dt) => match *dt {
                "integer" | "decimal" => "\"not-a-number\"".to_string(),
                "boolean" => "\"maybe\"".to_string(),
                _ => format!("{}", rng.below(100)), // bare integer where a string wanted
            },
            Constraint::NodeKind(nk) => match *nk {
                // wants an IRI -> give a literal; wants a literal -> give an IRI;
                // wants a blank node -> give an IRI.
                "sh:IRI" => "\"a-literal\"".to_string(),
                "sh:Literal" => format!("<{EX}an-iri>"),
                "sh:BlankNode" => format!("<{EX}an-iri>"),
                _ => "\"x\"".to_string(),
            },
            // sh:class — an instance NOT typed as the class.
            Constraint::Class(_) => format!("<{EX}untyped{}>", rng.below(5)),
            Constraint::Pattern(_) => "\"ZZZ_no_match\"".to_string(),
            Constraint::In(_) => "\"definitely-not-in-the-list\"".to_string(),
            Constraint::MinLength(n) => {
                format!("\"{}\"", "x".repeat((*n as usize).saturating_sub(1)))
            }
            Constraint::MaxLength(n) => format!("\"{}\"", "x".repeat(*n as usize + 3)),
            Constraint::HasValue(_) => format!("\"other-value-{}\"", rng.below(5)),
            Constraint::MinInclusive(n) => format!("{}", n - 1 - rng.below(3) as i64),
            Constraint::MaxInclusive(n) => format!("{}", n + 1 + rng.below(3) as i64),
            Constraint::MinExclusive(n) => format!("{}", n - rng.below(2) as i64), // n itself violates min-exclusive
            Constraint::MaxExclusive(n) => format!("{}", n + rng.below(2) as i64),
            Constraint::Logical(l) => l.violating_value(),
            Constraint::Node(inner) => inner.violating_value(rng),
        }
    }

    /// Does this constraint need a typed class instance in the data graph?
    /// `sh:node` over an inner `sh:class` propagates the inner requirement.
    fn class_iri(&self) -> Option<&str> {
        match self {
            Constraint::Class(c) => Some(c),
            Constraint::Node(inner) => inner.class_iri(),
            _ => None,
        }
    }
}

/// A typed literal of the given xsd local name. Both `valid` modes produce a
/// value that is lexically valid for the datatype (so it still conforms to a
/// bare `sh:datatype` constraint — wrong-datatype violations are minted by
/// `violating_value`). `valid=false` instead picks boundary values (zero,
/// negatives, the empty string, …) to widen the fuzz coverage.
fn typed_literal(dt: &str, rng: &mut Rng, valid: bool) -> String {
    match dt {
        "integer" if !valid => {
            // boundary integers: 0, and small negatives.
            format!("{}", -(rng.below(3) as i64))
        }
        "integer" => format!("{}", rng.below(100)),
        "decimal" if !valid => "\"-0.0\"^^xsd:decimal".to_string(),
        "decimal" => format!("\"{}.5\"^^xsd:decimal", rng.below(100)),
        "boolean" if !valid => {
            // the lexical alternatives "0"/"1" for xsd:boolean.
            if rng.chance(1, 2) {
                "\"1\"^^xsd:boolean"
            } else {
                "\"0\"^^xsd:boolean"
            }
            .to_string()
        }
        "boolean" => if rng.chance(1, 2) { "true" } else { "false" }.to_string(),
        "string" if !valid => "\"\"".to_string(), // empty string is a valid xsd:string
        "string" => format!("\"s{}\"", rng.below(10)),
        _ if !valid => "\"\"".to_string(),
        _ => format!("\"v{}\"", rng.below(10)),
    }
}

/// (sq-0hj7) The `sh:path` of a property shape: a simple predicate or
/// one of the complex forms. Each form knows how to (a) render itself as a Turtle
/// path expression for `sh:path`, and (b) emit the data-graph triple(s) that make
/// a given `value` reachable from a `subject` along it.
///
/// The complex forms are restricted to those whose value-node set does NOT include
/// the focus node itself — `Inverse`, `Sequence`, `OneOrMore` — so a value
/// constraint can be attached without the focus being checked against it. The
/// `ZeroOrOne` / `ZeroOrMore` forms (where the focus IS a value node) are only
/// generated count-free + value-constraint-free (see `gen_path`).
#[derive(Clone, Debug)]
enum PathSpec {
    /// `<p>` — a bare predicate path.
    Predicate(String),
    /// `[ sh:inversePath <p> ]` — the value node is the SUBJECT of `<value> <p> <subject>`.
    Inverse(String),
    /// `( <p1> <p2> )` — a two-step sequence through a fresh intermediate node.
    Sequence(String, String),
    /// `[ sh:oneOrMorePath <p> ]` — one hop suffices to make a value reachable.
    OneOrMore(String),
    /// `[ sh:zeroOrOnePath <p> ]` — focus is also a value node (count/value-free).
    ZeroOrOne(String),
    /// `[ sh:zeroOrMorePath <p> ]` — focus is also a value node (count/value-free).
    ZeroOrMore(String),
}

impl PathSpec {
    /// `true` for the simple predicate path (where the `(focus, component, path)`
    /// key carries the bare predicate IRI on both engines); `false` for the
    /// complex forms (which both engines normalise to the `_:path` sentinel).
    fn is_simple(&self) -> bool {
        matches!(self, PathSpec::Predicate(_))
    }

    /// The Turtle path expression for `sh:path`.
    fn to_turtle(&self) -> String {
        match self {
            PathSpec::Predicate(p) => format!("<{p}>"),
            PathSpec::Inverse(p) => format!("[ sh:inversePath <{p}> ]"),
            PathSpec::Sequence(a, b) => format!("( <{a}> <{b}> )"),
            PathSpec::OneOrMore(p) => format!("[ sh:oneOrMorePath <{p}> ]"),
            PathSpec::ZeroOrOne(p) => format!("[ sh:zeroOrOnePath <{p}> ]"),
            PathSpec::ZeroOrMore(p) => format!("[ sh:zeroOrMorePath <{p}> ]"),
        }
    }

    /// Emit the data triple(s) (subject, predicate, object Turtle) that make
    /// `value` reachable from `subject` along this path. `fresh` mints the unique
    /// intermediate node a sequence path needs.
    fn emit(&self, subject: &str, value: &str, fresh: &mut impl FnMut() -> String) -> Vec<String> {
        match self {
            PathSpec::Predicate(p)
            | PathSpec::OneOrMore(p)
            | PathSpec::ZeroOrOne(p)
            | PathSpec::ZeroOrMore(p) => {
                vec![format!("<{subject}> <{p}> {value} .")]
            }
            // inverse: value --p--> subject (so subject reaches value via ^p).
            // The value MUST be an IRI/blank to be a subject; complex inverse paths
            // are only generated with IRI-yielding value constraints (see gen_path),
            // but for a literal value we still need a subject, so route through a
            // fresh node and assert literal equality is impossible — guarded upstream.
            PathSpec::Inverse(p) => vec![format!("{value} <{p}> <{subject}> .")],
            PathSpec::Sequence(a, b) => {
                let mid = fresh();
                vec![
                    format!("<{subject}> <{a}> <{mid}> ."),
                    format!("<{mid}> <{b}> {value} ."),
                ]
            }
        }
    }
}

/// One property shape: a path, a min/maxCount pair, and a value constraint. The
/// generator decides per-instance whether to satisfy or violate.
struct PropShape {
    path: PathSpec,
    min_count: Option<u64>,
    max_count: Option<u64>,
    value: Option<Constraint>,
}

impl PropShape {
    fn shape_turtle(&self) -> String {
        let mut lines = vec![format!("sh:path {}", self.path.to_turtle())];
        if let Some(n) = self.min_count {
            lines.push(format!("sh:minCount {n}"));
        }
        if let Some(n) = self.max_count {
            lines.push(format!("sh:maxCount {n}"));
        }
        if let Some(c) = &self.value {
            lines.extend(c.shape_lines());
        }
        format!(
            "    sh:property [\n      {} ;\n    ]",
            lines.join(" ;\n      ")
        )
    }
}

/// A full generated scenario: a node shape, its property shapes, and a data graph.
pub struct Scenario {
    target_class: String,
    props: Vec<PropShape>,
    instances: Vec<Instance>,
    /// Supporting type declarations the data graph needs (typed class instances
    /// for sh:class conforming values, etc.).
    support: Vec<String>,
}

/// One data-graph instance of the target class. The value objects are wired to
/// the focus subject along each property shape's path during generation (a
/// complex path emits intermediate triples), so we store the fully-rendered
/// triple lines rather than `(predicate, object)` pairs. (sq-0hj7)
struct Instance {
    subject: String,
    /// Fully-rendered `<s> <p> o .` triple lines (path-aware).
    triples: Vec<String>,
}

impl Scenario {
    pub fn generate(rng: &mut Rng) -> Scenario {
        let class_n = rng.below(1000);
        let target_class = format!("{EX}C{class_n}");
        let n_props = 1 + rng.below(4); // 1..=4 property shapes

        let mut props = Vec::new();
        let mut support = Vec::new();
        let mut used_preds = std::collections::HashSet::new();
        let mut fresh_id = 0u64;
        for i in 0..n_props {
            // distinct base predicate per property shape so cardinality is
            // unambiguous (the path may compose several distinct predicates).
            let mut pred = format!("{EX}p{i}");
            while !used_preds.insert(pred.clone()) {
                pred = format!("{EX}p{}_{}", i, rng.below(1000));
            }
            // (sq-0hj7) choose a path form first — it constrains which
            // value-constraint families are sound (e.g. an inverse path needs an
            // IRI-valued constraint, and focus-including paths must stay value-free).
            let path = gen_path(rng, &pred, &mut used_preds);
            let value = gen_value_for_path(rng, &path, &mut support);
            // counts: keep min<=max and small so violations are reachable. Complex
            // paths get NO cardinality constraint (their value-node counting under
            // */+/seq/inverse has subtle focus-inclusion semantics we keep out of
            // the cardinality comparison — the value-component comparison still runs).
            let (min_count, max_count) = if path.is_simple() {
                let mn = if rng.chance(2, 3) {
                    Some(rng.below(2))
                } else {
                    None
                };
                let mx = if rng.chance(1, 2) {
                    Some(1 + rng.below(2))
                } else {
                    None
                };
                (mn, mx)
            } else {
                (None, None)
            };
            props.push(PropShape {
                path,
                min_count,
                max_count,
                value,
            });
        }

        // Generate instances: each is a fresh subject of the target class, with
        // each property independently satisfied or violated.
        let n_inst = 1 + rng.below(5); // 1..=5 instances
        let mut instances = Vec::new();
        for i in 0..n_inst {
            let subject = format!("{EX}i{}_{}", class_n, i);
            let mut triples = Vec::new();
            for p in &props {
                // Decide how many values to emit for this property (0..=3), then
                // make each conform or violate the value constraint.
                let k = rng.below(4); // 0,1,2,3 values
                for _ in 0..k {
                    let obj = match &p.value {
                        Some(c) => {
                            let v = if rng.chance(1, 2) {
                                c.conforming_value(rng)
                            } else {
                                c.violating_value(rng)
                            };
                            // Resolve the class-instance placeholder into a real
                            // typed instance recorded in `support`.
                            resolve_value(v, c, &mut support, rng)
                        }
                        None => format!("\"v{}\"", rng.below(10)),
                    };
                    let mut fresh = || {
                        fresh_id += 1;
                        format!("{EX}mid{fresh_id}")
                    };
                    triples.extend(p.path.emit(&subject, &obj, &mut fresh));
                }
            }
            instances.push(Instance { subject, triples });
        }

        Scenario {
            target_class,
            props,
            instances,
            support,
        }
    }

    pub fn shapes_turtle(&self) -> String {
        let mut s = String::from(PREFIXES);
        s.push_str(&format!("<{EX}Shape> a sh:NodeShape ;\n"));
        s.push_str(&format!("    sh:targetClass <{}> ;\n", self.target_class));
        let bodies: Vec<String> = self.props.iter().map(|p| p.shape_turtle()).collect();
        s.push_str(&bodies.join(" ;\n"));
        s.push_str(" .\n");
        s
    }

    pub fn data_turtle(&self) -> String {
        let mut s = String::from(PREFIXES);
        for inst in &self.instances {
            s.push_str(&format!("<{}> a <{}> .\n", inst.subject, self.target_class));
            for triple in &inst.triples {
                s.push_str(triple);
                s.push('\n');
            }
        }
        // Supporting declarations (typed class instances for sh:class, etc.).
        for line in &self.support {
            s.push_str(line);
            s.push('\n');
        }
        s
    }
}

/// (sq-0hj7) Choose the `sh:path` form for a property shape. ~5-in-6
/// stay simple predicate paths (so cardinality + simple-path-key coverage is
/// unchanged); ~1-in-6 pick a complex form, drawing fresh distinct predicates for
/// the multi-predicate forms so the path's predicates don't collide with other
/// shapes' base predicates.
fn gen_path(
    rng: &mut Rng,
    base_pred: &str,
    used_preds: &mut std::collections::HashSet<String>,
) -> PathSpec {
    if !rng.chance(1, 6) {
        return PathSpec::Predicate(base_pred.to_string());
    }
    match rng.below(5) {
        0 => PathSpec::Inverse(base_pred.to_string()),
        1 => PathSpec::Sequence(base_pred.to_string(), fresh_pred(rng, used_preds)),
        2 => PathSpec::OneOrMore(base_pred.to_string()),
        3 => PathSpec::ZeroOrOne(base_pred.to_string()),
        _ => PathSpec::ZeroOrMore(base_pred.to_string()),
    }
}

/// Mints a fresh predicate IRI distinct from every predicate `used_preds` has
/// seen, so a sequence path's second predicate can't collide with another
/// shape's base predicate (which would blur cardinality across shapes).
fn fresh_pred(rng: &mut Rng, used_preds: &mut std::collections::HashSet<String>) -> String {
    let mut p = format!("{EX}q{}", rng.below(100000));
    while !used_preds.insert(p.clone()) {
        p = format!("{EX}q{}", rng.below(100000));
    }
    p
}

/// (sq-0hj7) Pick a value constraint that is SOUND for the chosen path
/// form:
///   - simple predicate / sequence / oneOrMore: any in-scope value component
///     (the value is the final object, so a literal is fine).
///   - inverse: the value node must be a SUBJECT, so it must be an IRI — restrict
///     to `sh:class` (both its conforming and violating values are IRIs).
///   - zeroOrOne / zeroOrMore: the focus node IS a value node, so a value
///     component would (correctly, but confusingly) also be checked against the
///     focus; we keep these path-only (no value component) so the differential
///     compares pure complex-path reachability + cardinality-free conformance.
fn gen_value_for_path(
    rng: &mut Rng,
    path: &PathSpec,
    support: &mut Vec<String>,
) -> Option<Constraint> {
    match path {
        PathSpec::Predicate(_) | PathSpec::Sequence(..) | PathSpec::OneOrMore(_) => {
            gen_constraint(rng, support)
        }
        PathSpec::Inverse(_) => {
            // IRI-valued only (value node becomes a subject).
            let cls = format!("{EX}Kind{}", rng.below(100));
            Some(Constraint::Class(cls))
        }
        PathSpec::ZeroOrOne(_) | PathSpec::ZeroOrMore(_) => None,
    }
}

/// Picks a random in-scope value constraint.
///
/// `_support` is intentionally unused: every constraint we currently
/// generate fills its data-graph support per-value later in `resolve_value`
/// (e.g. `sh:class` mints its typed instance there), so nothing is accumulated
/// up front. The parameter is kept so callers needn't change if a future
/// constraint does need up-front support declarations.
fn gen_constraint(rng: &mut Rng, _support: &mut Vec<String>) -> Option<Constraint> {
    // 1-in-6: a count-only property shape (no value constraint).
    if rng.chance(1, 6) {
        return None;
    }
    // 12..=14: the sq-0hj7 logical/node extensions (drawn ~1-in-5 of value cases).
    let choice = rng.below(15);
    Some(match choice {
        // (sq-qvqk7) Keep rustc 1.88 from inferring unsized `str` here.
        0 => Constraint::Datatype(rng.pick::<&str>(&["integer", "string", "boolean", "decimal"])),
        1 => Constraint::NodeKind(rng.pick::<&str>(&["sh:IRI", "sh:Literal", "sh:BlankNode"])),
        2 => {
            // sh:class: conforming values get a typed instance minted lazily by
            // `resolve_value` (so each conforming value points at its own typed
            // node); no class-level support declaration is needed up front.
            let cls = format!("{EX}Kind{}", rng.below(100));
            Constraint::Class(cls)
        }
        3 => Constraint::Pattern(rng.pick::<&str>(&["^[a-z]+[0-9]+$", "^a.*$", "[0-9]"])),
        4 => {
            let n = 2 + rng.below(3);
            let vals: Vec<String> = (0..n).map(|j| format!("\"opt{j}\"")).collect();
            Constraint::In(vals)
        }
        5 => Constraint::MinLength(1 + rng.below(4)),
        6 => Constraint::MaxLength(2 + rng.below(4)),
        7 => Constraint::HasValue(format!("\"required-{}\"", rng.below(5))),
        8 => Constraint::MinInclusive(rng.below(50) as i64),
        9 => Constraint::MaxInclusive(50 + rng.below(50) as i64),
        10 => Constraint::MinExclusive(rng.below(50) as i64),
        11 => Constraint::MaxExclusive(50 + rng.below(50) as i64),
        // (sq-0hj7) logical components over inline `sh:in`-token members.
        12 | 13 => {
            let op = *rng.pick(&[
                LogicalOp::And,
                LogicalOp::Or,
                LogicalOp::Not,
                LogicalOp::Xone,
            ]);
            let members = if op == LogicalOp::Not {
                1
            } else {
                2 + rng.below(2) // 2..=3 members
            };
            Constraint::Logical(Logical { op, members })
        }
        // (sq-0hj7) sh:node over an inline member shape carrying a
        // simple leaf value constraint. Restrict the leaf to families whose
        // conform/violate is unambiguous on a value node — reuse a small subset
        // (datatype / nodeKind / in / pattern / numeric range).
        _ => {
            let inner = gen_leaf_for_node(rng);
            Constraint::Node(Box::new(inner))
        }
    })
}

/// (sq-0hj7) A leaf value constraint suitable INSIDE a `sh:node` member
/// shape. We exclude `sh:class` (its conforming value is a typed-instance
/// placeholder resolved per top-level value, which the nested `resolve_value`
/// path already handles, but keeping the nested leaf class-free avoids minting
/// support inside a member shape) and the logical/node families (no nesting of
/// composites — one level is enough to stress the conformance memo).
fn gen_leaf_for_node(rng: &mut Rng) -> Constraint {
    match rng.below(7) {
        // (sq-qvqk7) Keep rustc 1.88 from inferring unsized `str` here.
        0 => Constraint::Datatype(rng.pick::<&str>(&["integer", "string", "boolean"])),
        1 => Constraint::NodeKind(rng.pick::<&str>(&["sh:IRI", "sh:Literal"])),
        2 => Constraint::Pattern(rng.pick::<&str>(&["^[a-z]+[0-9]+$", "^a.*$"])),
        3 => {
            let n = 2 + rng.below(3);
            let vals: Vec<String> = (0..n).map(|j| format!("\"opt{j}\"")).collect();
            Constraint::In(vals)
        }
        4 => Constraint::MinInclusive(rng.below(50) as i64),
        5 => Constraint::MaxInclusive(50 + rng.below(50) as i64),
        _ => Constraint::MinLength(1 + rng.below(3)),
    }
}

/// Resolves the `<__CLASS_INSTANCE__>` placeholder produced by a conforming
/// sh:class value into a real, typed instance recorded in `support`. All other
/// values pass through unchanged.
fn resolve_value(v: String, c: &Constraint, support: &mut Vec<String>, rng: &mut Rng) -> String {
    if v == "<__CLASS_INSTANCE__>" {
        if let Some(cls) = c.class_iri() {
            let inst = format!("{EX}inst_{}", rng.below(100000));
            support.push(format!("<{inst}> a <{cls}> ."));
            return format!("<{inst}>");
        }
    }
    v
}

const PREFIXES: &str = "\
@prefix sh: <http://www.w3.org/ns/shacl#> .
@prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
@prefix rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#> .
";

// ---------------------------------------------------------------------------
// (sq-0hj7) Test-support: coverage introspection + a single-case
// builder so the per-PR fast lane can prove (a) the new component families are
// actually generated, and (b) the generator's conform/violate construction is
// SOUND — i.e. sparq's own report agrees with what the generator intended for
// each value. Without these guards the extended generator could silently emit
// shapes that always (dis)agree for the wrong reason.
// ---------------------------------------------------------------------------

/// Which extended component families a scenario contains (for the coverage guard).
#[derive(Default, Debug)]
pub struct Coverage {
    pub logical_and: bool,
    pub logical_or: bool,
    pub logical_not: bool,
    pub logical_xone: bool,
    pub node: bool,
    pub complex_path: bool,
}

impl Scenario {
    /// Classify the component families present in this scenario.
    pub fn coverage(&self) -> Coverage {
        let mut c = Coverage::default();
        for p in &self.props {
            if !p.path.is_simple() {
                c.complex_path = true;
            }
            match &p.value {
                Some(Constraint::Logical(l)) => match l.op {
                    LogicalOp::And => c.logical_and = true,
                    LogicalOp::Or => c.logical_or = true,
                    LogicalOp::Not => c.logical_not = true,
                    LogicalOp::Xone => c.logical_xone = true,
                },
                Some(Constraint::Node(_)) => c.node = true,
                _ => {}
            }
        }
        c
    }
}

/// A self-contained `(shapes_ttl, data_ttl)` pair: a node shape with ONE property
/// shape carrying `path` + `value`, and ONE focus instance whose single value is
/// driven to conform (`conform=true`) or violate (`conform=false`) `value`.
///
/// Used by the soundness self-test: the generator INTENDS this case to
/// conform/violate, so sparq validating it must agree (the global conformance bit
/// must equal `conform`). This is the invariant the differential leans on, checked
/// here without a reference engine.
pub fn single_case(
    value: Constraint,
    path: PathSpecKind,
    conform: bool,
    seed: u64,
) -> (String, String) {
    let mut rng = Rng::new(seed);
    let class = format!("{EX}T{seed}");
    let base_pred = format!("{EX}pp");
    let path = match path {
        PathSpecKind::Predicate => PathSpec::Predicate(base_pred.clone()),
        PathSpecKind::Inverse => PathSpec::Inverse(base_pred.clone()),
        PathSpecKind::Sequence => PathSpec::Sequence(base_pred.clone(), format!("{EX}qq")),
        PathSpecKind::OneOrMore => PathSpec::OneOrMore(base_pred.clone()),
        PathSpecKind::ZeroOrOne => PathSpec::ZeroOrOne(base_pred.clone()),
        PathSpecKind::ZeroOrMore => PathSpec::ZeroOrMore(base_pred.clone()),
    };

    let mut shapes = String::from(PREFIXES);
    shapes.push_str(&format!("<{EX}S> a sh:NodeShape ;\n"));
    shapes.push_str(&format!("    sh:targetClass <{class}> ;\n"));
    let prop = PropShape {
        path: path.clone(),
        min_count: None,
        max_count: None,
        value: Some(value.clone()),
    };
    shapes.push_str(&prop.shape_turtle());
    shapes.push_str(" .\n");

    let subject = format!("{EX}foc");
    let raw = if conform {
        value.conforming_value(&mut rng)
    } else {
        value.violating_value(&mut rng)
    };
    let mut support = Vec::new();
    let obj = resolve_value(raw, &value, &mut support, &mut rng);
    let mut fresh_n = 0u64;
    let mut fresh = || {
        fresh_n += 1;
        format!("{EX}m{fresh_n}")
    };
    let triples = path.emit(&subject, &obj, &mut fresh);

    let mut data = String::from(PREFIXES);
    data.push_str(&format!("<{subject}> a <{class}> .\n"));
    for t in &triples {
        data.push_str(t);
        data.push('\n');
    }
    for line in &support {
        data.push_str(line);
        data.push('\n');
    }
    (shapes, data)
}

/// A value-component-FREE single case for a focus-including path form
/// (`zeroOrOne` / `zeroOrMore`), mirroring exactly how the generator emits them:
/// one focus instance with one outbound edge, no value or cardinality constraint.
/// Such a shape can only conform (nothing to violate), so the soundness test
/// asserts conformance — a regression that made the path ill-formed or spuriously
/// violating (e.g. checking the focus against something) would fail.
pub fn single_case_path_only(path: PathSpecKind, seed: u64) -> (String, String) {
    let mut rng = Rng::new(seed);
    let class = format!("{EX}TP{seed}");
    let base_pred = format!("{EX}pp");
    let path = match path {
        PathSpecKind::Predicate => PathSpec::Predicate(base_pred.clone()),
        PathSpecKind::Inverse => PathSpec::Inverse(base_pred.clone()),
        PathSpecKind::Sequence => PathSpec::Sequence(base_pred.clone(), format!("{EX}qq")),
        PathSpecKind::OneOrMore => PathSpec::OneOrMore(base_pred.clone()),
        PathSpecKind::ZeroOrOne => PathSpec::ZeroOrOne(base_pred.clone()),
        PathSpecKind::ZeroOrMore => PathSpec::ZeroOrMore(base_pred.clone()),
    };
    let mut shapes = String::from(PREFIXES);
    shapes.push_str(&format!("<{EX}S> a sh:NodeShape ;\n"));
    shapes.push_str(&format!("    sh:targetClass <{class}> ;\n"));
    let prop = PropShape {
        path: path.clone(),
        min_count: None,
        max_count: None,
        value: None,
    };
    shapes.push_str(&prop.shape_turtle());
    shapes.push_str(" .\n");

    let subject = format!("{EX}foc");
    let obj = format!("\"v{}\"", rng.below(5));
    let mut fresh_n = 0u64;
    let mut fresh = || {
        fresh_n += 1;
        format!("{EX}m{fresh_n}")
    };
    let triples = path.emit(&subject, &obj, &mut fresh);
    let mut data = String::from(PREFIXES);
    data.push_str(&format!("<{subject}> a <{class}> .\n"));
    for t in &triples {
        data.push_str(t);
        data.push('\n');
    }
    (shapes, data)
}

/// The path forms `single_case` can build (a public mirror of the private
/// `PathSpec` discriminant, so tests don't need the private predicates).
#[derive(Clone, Copy, Debug)]
pub enum PathSpecKind {
    Predicate,
    Inverse,
    Sequence,
    OneOrMore,
    ZeroOrOne,
    ZeroOrMore,
}

/// Constructors the soundness test uses to build each extended value component
/// without reaching into the private `Constraint` enum.
pub fn logical_and(members: u64) -> Constraint {
    Constraint::Logical(Logical {
        op: LogicalOp::And,
        members,
    })
}
pub fn logical_or(members: u64) -> Constraint {
    Constraint::Logical(Logical {
        op: LogicalOp::Or,
        members,
    })
}
pub fn logical_not() -> Constraint {
    Constraint::Logical(Logical {
        op: LogicalOp::Not,
        members: 1,
    })
}
pub fn logical_xone(members: u64) -> Constraint {
    Constraint::Logical(Logical {
        op: LogicalOp::Xone,
        members,
    })
}
/// `sh:node [ sh:datatype xsd:integer ]` — the simplest nested node-shape ref.
pub fn node_datatype_integer() -> Constraint {
    Constraint::Node(Box::new(Constraint::Datatype("integer")))
}
/// `sh:node [ sh:nodeKind sh:IRI ]` — an IRI-valued nested node-shape ref.
pub fn node_nodekind_iri() -> Constraint {
    Constraint::Node(Box::new(Constraint::NodeKind("sh:IRI")))
}
/// A plain `sh:datatype xsd:integer` (for the complex-path soundness cases).
pub fn datatype_integer() -> Constraint {
    Constraint::Datatype("integer")
}
/// `sh:class <iri>` — IRI-valued (used for the inverse-path soundness case).
pub fn class_kind() -> Constraint {
    Constraint::Class(format!("{EX}KindT"))
}
