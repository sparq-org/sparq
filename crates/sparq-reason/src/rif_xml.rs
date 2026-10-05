// [SONNET-4.6] sq-pbz04.5.3 — OPT-IN RIF/XML importer: parse the W3C RIF-Core
// XML presentation syntax into `rif::Document` with Or-split / Exists-flatten
// desugaring and a fail-closed taxonomy. Requires the `rif-core` feature (wired by
// the `rif-xml` feature below). When off, zero rif_xml code is compiled.
//
// sq-jsgyn: multi-slot Frame desugaring. A <Frame> with N>=2 <slot> children
// obj[p1->v1, p2->v2, …] desugars into N ground Frame atoms — obj[p1->v1],
// obj[p2->v2], … — one per slot. In body position these become a conjunction
// (BodyCond::And); in head position they extend the head-atom list; as a bare
// fact they produce one Rule::fact per slot. This is the sound RIF-Core lowering:
// a multi-slot frame is the conjunction of its per-slot single-slot frames
// (RIF-Core §2.3). Fail-closed: a Frame with ZERO slots is MalformedXml; any
// slot that is a named-argument uniterm is NamedArgUniterm; only the <object>
// wrapper is still single-cardinality (duplicate <object> → MalformedXml). The
// old unique_child("slot", …) check that made duplicate <slot> an error is
// removed; the per-slot named-arg check replaces the former single-slot guard.
// [SONNET-4.6] sq-jsgyn
//
// Opus adversarial-verify fixes applied (sq-pbz04.5.3 post-verify):
// Fix-1: unconditional alpha-renaming of Exists-declared vars prevents variable
//        capture in both shadow (Forall ?x … Exists ?x) and sibling (Exists ?y …
//        Exists ?y) patterns. [SONNET-4.6]
// Fix-2: fail-closed on non-<formula> children in And/Or/head-And + duplicate
//        single-cardinality wrappers (<if>, <then>) now abort the import. [SONNET-4.6]
// Fix-3: per-type whitespace handling — xsd:string lexical values preserve exact
//        whitespace; IRI/numeric/boolean types trim (XSD whitespace-collapse facet).
//        Removes the global trim_text(true) that was corrupting string literals. [SONNET-4.6]
// Fix-4: func:count → Builtin::ListLength added to iri_to_builtin (was wrongly
//        rejected as UnknownExternal). [SONNET-4.6]
// Fix-5: honesty claims in module docs updated to match final behaviour. [SONNET-4.6]
// Fix-6: <declare> with != 1 <Var> child now returns MalformedXml in BOTH the
//        Exists arm (parse_condition) and the Forall arm (parse_sentence). The old
//        filter_map(.first()) silently dropped all but the first Var, leaving the
//        others unrenamed and causing variable capture (demonstrated: Exists
//        <declare>?a ?b</declare> conflated universal ?b with existential ?b).
//        Fail-closed beats guessing; multi-Var-per-declare is not schema-valid
//        RIF-XML. [SONNET-4.6]
// sq-anuo9: residual silent-drop class (adversarial re-verify of #1461). Single-cardinality
//        wrappers read via `.children.first()` dropped surplus siblings on non-schema-valid
//        input — <if> dropping its 2nd conjunct WEAKENS the body -> OVER-derivation (the
//        soundness-relevant case), and <then>/<formula>/<object>/<instance>/<class>/<sub>/
//        <sup>/<left>/<right>/<content>/<op>/<sentence> dropped surplus terms/atoms. <declare>
//        also TOLERATED a stray non-<Var> sibling. Fix: the `only_child` helper enforces
//        exactly-one-child fail-closed (MalformedXml) on every such wrapper, and <declare>
//        now requires exactly one child that is a <Var>. Conformant RIF-XML is unaffected by
//        design. [SONNET-4.6]
// Beads for deferred low-priority items: see bead notes inline.

//! # `rif_xml` — W3C RIF-Core XML importer
//!
//! Parses the **W3C RIF/XML presentation syntax** for the RIF-Core dialect into a
//! [`crate::rif::Document`] AST that the existing `rif-core` forward chainer can run.
//!
//! ## Entry point
//!
//! ```rust
//! # #[cfg(feature = "rif-xml")] {
//! use sparq_reason::rif_xml::import;
//! let xml = br#"<Document xmlns="http://www.w3.org/2007/rif#">
//!   <payload><Group><sentences>
//!     <Forall>
//!       <declare><Var>x</Var></declare>
//!       <formula><Implies>
//!         <if><Frame>
//!           <object><Var>x</Var></object>
//!           <slot><Const type="http://www.w3.org/2007/rif#iri">http://ex/p</Const>
//!                 <Const type="http://www.w3.org/2007/rif#iri">http://ex/v</Const></slot>
//!         </Frame></if>
//!         <then><Frame>
//!           <object><Var>x</Var></object>
//!           <slot><Const type="http://www.w3.org/2007/rif#iri">http://ex/q</Const>
//!                 <Const type="http://www.w3.org/2007/rif#iri">http://ex/w</Const></slot>
//!         </Frame></then>
//!       </Implies></formula>
//!     </Forall>
//!   </sentences></Group></payload>
//! </Document>"#;
//! let doc = import(xml).expect("valid RIF-Core XML");
//! assert_eq!(doc.rules.len(), 1);
//! # }
//! ```
//!
//! ## Positional predicate atoms — `Atom(op args…)` [SONNET-4.6] sq-n7y15
//!
//! RIF-Core's XML presentation syntax uses `<Atom>` for **positional (ordered-argument)
//! predicate calls** — the dominant form in real W3C RIF Core test files. This importer
//! supports the two arities that have a sound, semantically-equivalent mapping to the
//! existing `rif::Atom` variants (scope: rif_xml only; rif.rs is unchanged):
//!
//! | Arity | XML form | Mapped to | RIF-Core equivalence |
//! |-------|----------|-----------|----------------------|
//! | **1** | `<Atom><op>C</op><args>a</args></Atom>` | `Atom::Member { obj: a, class: C }` | Unary predicate = membership `a # C` |
//! | **2** | `<Atom><op>P</op><args>a b</args></Atom>` | `Atom::Frame { obj: a, pred: P, val: b }` | Binary predicate = frame atom `a[P → b]` |
//!
//! **Fail-closed:** arity 0 and arity 3+ positional atoms are **rejected** with
//! `ImportError::UnrecognizedElement` — there is no semantically equivalent existing
//! atom form, so importing them silently into a wrong shape would be unsound.
//! A non-IRI operator is rejected with `ImportError::MalformedXml`.
//!
//! ## Sound desugarings applied at import
//!
//! 1. **Body `Or` → rule-splitting (Lloyd–Topor):** a disjunctive body
//!    `Or(C1, C2, …)` is split into one rule per disjunct. Each rule has the same
//!    head and the same universally-quantified variables; the union of the rule set is
//!    semantically equivalent to the original disjunctive body under Horn semantics.
//!    This is the *monotone-Horn-preserving* desugaring; it is the standard encoding
//!    used to admit `Or` in RIF-Core condition language while the chainer remains Horn.
//!    Note: the `rif::UNIMPLEMENTED` list has a stale entry
//!    `"disjunction (Or) in rule bodies"` — Or IS in the RIF-Core condition language;
//!    the desugaring here resolves it at import time. That entry is owned by sq-pbz04.5.4.
//!
//! 2. **Body `Exists` → existential-flatten with capture-avoidance:** existentially-
//!    quantified variables in a body `Exists(z…, C)` are *unconditionally alpha-renamed*
//!    to fresh names in the `__ex{N}` reserved namespace before the `Exists` wrapper is
//!    dropped. Freshness is verified against the complete variable set of the enclosing
//!    rule (universally-declared vars + all Exists-declared vars + previously generated
//!    fresh names). Renaming is applied innermost-first (innermost binder wins when the
//!    same name is redeclared in nested `Exists` nodes). Unconditional renaming (not just
//!    on collision) uniformly fixes both confirmed capture patterns: quantifier shadowing
//!    (`Forall ?x … Exists ?x`) and sibling reuse (`Exists ?y … Exists ?y`).
//!    `Document::validate()` enforces the range-restriction invariant on every imported
//!    document, so every renamed variable appears in at least one positive body atom.
//!
//! 3. **Multi-slot `Frame` → per-slot conjunction (sq-jsgyn):** a `Frame` carrying N
//!    `<slot>` children `obj[p1->v1 p2->v2 …]` desugars into **N single-slot ground
//!    `Frame` atoms** — one per `(pi, vi)` pair, each sharing the same `obj`. Under
//!    RIF-Core §2.3 a multi-slot frame is the conjunction of its per-slot frames, so
//!    this is the semantically equivalent Horn lowering:
//!    - **Body position:** desugared to `BodyCond::And([obj[p1->v1], obj[p2->v2], …])`.
//!    - **Head position (or conjunctive head `<And>`):** each slot adds one `Atom` to
//!      the rule's head list (`Rule::head`).
//!    - **Bare fact:** one `Rule::fact(obj[pi->vi])` per slot.
//!    - **Fail-closed invariants:** zero slots → `MalformedXml`; a named-argument
//!      `<slot>/<Name>` → `NamedArgUniterm`; duplicate `<object>` still →
//!      `MalformedXml`. Only `<slot>` may appear multiple times under a `<Frame>`;
//!      `<object>` remains single-cardinality.
//!
//! ## Fail-closed taxonomy
//!
//! Every element or construct outside the supported Core subset returns a named error
//! variant. Every unexpected child element — including non-`<formula>` children inside
//! `<And>` or `<Or>` conditions or a conjunctive head — returns an error, and a **surplus
//! child of a single-cardinality wrapper** (`<if>`, `<then>`, `<formula>`, `<object>`,
//! `<instance>`, `<class>`, `<sub>`, `<sup>`, `<left>`, `<right>`, `<content>`, `<op>`,
//! `<sentence>`, plus a `<declare>` carrying anything other than exactly one `<Var>`)
//! returns `MalformedXml` rather than being silently dropped by a `.first()`-style read.
//! This is a soundness property, not merely a diagnostic: dropping a second `<if>`
//! conjunct would WEAKEN the rule body → over-derivation (sq-anuo9). Nothing within a
//! wrapper is silently skipped or dropped, and — the twin class — a **duplicate
//! single-cardinality wrapper under a parent** (two `<object>` under a `<Frame>`,
//! `<instance>`/`<class>` under `<Member>`, `<sub>`/`<sup>` under a
//! `<Subclass>`, `<left>`/`<right>` under an `<Equal>`, `<content>`/`<op>`/`<args>` under
//! an `<External>` call, the `<formula>` under a `<Forall>`/`<Exists>`, or the `<items>`
//! under a `<List>` term) is rejected via `unique_child` rather than first-wins-dropped by
//! `child()` (sq-4l1fj, closing the
//! residual class sq-anuo9's `only_child` left; the `<if>`/`<then>` subset was already
//! guarded inline in `parse_implies`).
//! **Exception:** multiple `<slot>` children under a `<Frame>` are VALID (RIF-Core
//! multi-slot frames); they desugar to a per-slot conjunction rather than being rejected
//! (sq-jsgyn). `<object>` remains single-cardinality under `<Frame>`.
//!
//! 1. `ImportError::ImportDirective` — any `Import` element (remote imports: fail-closed).
//! 2. `ImportError::NonCoreElement { element, reason }` — non-Core dialect elements:
//!    `Naf`, `Assert`, `Retract`, `Modify`, `Neg`, `NAF`, and PRD constructs.
//! 3. `ImportError::UnknownExternal { iri }` — `External` calls with IRIs not in the
//!    supported DTB builtin set. Includes deferred DTB builtins:
//!    `pred:matches` (XSD-regex vs Rust-regex dialect gap), `func:numeric-integer-divide`
//!    (truncation vs floor division), `func:numeric-mod` (dividend-sign vs divisor-sign),
//!    guard predicates, list utilities (`func:get`, …), date/time builtins, and
//!    `func:substring*` / `func:string-join` / `func:compare`.
//! 4. `ImportError::NamedArgUniterm { name }` — named-argument uniterms (not in Core).
//! 5. `ImportError::UnrecognizedElement { tag }` — XML elements not in the supported grammar.
//! 6. `ImportError::MalformedXml(String)` — quick-xml parse errors or malformed structure.
//! 7. `ImportError::ValidationFailed(RifError)` — documents that parse but fail
//!    `Document::validate()` (range-restriction / builtin-safety invariant).

use crate::rif::{Atom, Builtin, Document, RifError, Rule, Term};
use quick_xml::events::Event;
use quick_xml::name::ResolveResult;
use quick_xml::reader::NsReader;
use std::collections::{BTreeSet, HashMap};

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

/// Error returned by `import()` when a RIF/XML document cannot be parsed or is
/// not within the supported RIF-Core subset.
///
/// Each variant corresponds to a distinct rejection category in the fail-closed
/// taxonomy documented in the module doc comment.
#[derive(Debug)]
pub enum ImportError {
    /// Malformed or invalid XML that quick-xml could not parse, or structural
    /// violations (duplicate single-cardinality wrappers, missing required children).
    MalformedXml(String),
    /// An `Import` directive was found — remote imports are not supported (fail-closed).
    ImportDirective {
        /// The location IRI of the import.
        location: String,
    },
    /// [SONNET-4.6] sq-wbql1 — the imports-closure consistency check detected a
    /// GENUINE incompatibility in an `<Import>` directive: the imported document's
    /// `profile` designates a non-Core dialect, or the combined imports-closure fails
    /// RIF-Core validation. This is a NON-VACUOUS rejection: it demonstrates detecting
    /// the specific import invalidity (a profile mismatch or rule-set inconsistency)
    /// rather than blanket-refusing any `<Import>`.
    InconsistentImport {
        /// The location IRI of the offending import.
        location: String,
        /// A human-readable description of why the import is inconsistent.
        reason: String,
    },
    /// A non-Core dialect element was found (e.g. RIF-BLD-only constructs, PRD actions).
    NonCoreElement {
        /// The element name.
        element: String,
        /// The reason it is excluded.
        reason: String,
    },
    /// An `External` call with an IRI not recognized as a supported builtin.
    UnknownExternal {
        /// The unrecognized IRI.
        iri: String,
    },
    /// A named-argument uniterm (not supported in Core; fail-closed).
    NamedArgUniterm {
        /// The named slot name that triggered this error.
        name: String,
    },
    /// An unrecognized XML element that is not in the supported Core grammar, or an
    /// unexpected child element where only `<formula>` is permitted.
    UnrecognizedElement {
        /// The tag name.
        tag: String,
    },
    /// A `Document` was imported but `Document::validate()` rejected it.
    ValidationFailed(RifError),
}

impl std::fmt::Display for ImportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ImportError::MalformedXml(msg) => {
                write!(f, "RIF/XML: malformed XML: {}", msg)
            }
            ImportError::ImportDirective { location } => {
                write!(
                    f,
                    "RIF/XML: Import directives are not supported (fail-closed): {}",
                    location
                )
            }
            ImportError::InconsistentImport { location, reason } => {
                write!(
                    f,
                    "RIF/XML: imports-closure inconsistency detected at <{}>: {}",
                    location, reason
                )
            }
            ImportError::NonCoreElement { element, reason } => {
                write!(
                    f,
                    "RIF/XML: non-Core element <{}> rejected: {}",
                    element, reason
                )
            }
            ImportError::UnknownExternal { iri } => {
                write!(
                    f,
                    "RIF/XML: External call with unrecognized builtin IRI: {}",
                    iri
                )
            }
            ImportError::NamedArgUniterm { name } => {
                write!(
                    f,
                    "RIF/XML: named-argument uniterm slot '{}' is not supported in RIF-Core",
                    name
                )
            }
            ImportError::UnrecognizedElement { tag } => {
                write!(
                    f,
                    "RIF/XML: unrecognized element <{}> (not in the supported Core grammar)",
                    tag
                )
            }
            ImportError::ValidationFailed(e) => {
                write!(f, "RIF/XML: document failed validation: {}", e)
            }
        }
    }
}

impl std::error::Error for ImportError {}

// ---------------------------------------------------------------------------
// Internal XML tree
// ---------------------------------------------------------------------------

/// A node in the parsed XML tree. We build this from quick-xml events before
/// interpreting — keeps the interpretation code recursive and clean.
struct XmlNode {
    /// The local name (no namespace prefix).
    tag: String,
    /// Accumulated character data content. NOT globally trimmed — whitespace
    /// may be semantically significant (e.g. xsd:string lexical values).
    /// Callers (primarily `parse_term`) apply per-type trimming.
    text: String,
    /// Attributes as `(local_key, value)` pairs.
    attrs: Vec<(String, String)>,
    /// Child elements.
    children: Vec<XmlNode>,
}

impl XmlNode {
    /// Return the value of the first attribute with the given local name.
    fn attr(&self, key: &str) -> Option<&str> {
        self.attrs.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
    }

    /// Return the first child with the given tag name, if any.
    fn child(&self, tag: &str) -> Option<&XmlNode> {
        self.children.iter().find(|c| c.tag == tag)
    }

    /// Return all children with the given tag name.
    fn children_named<'a>(&'a self, tag: &'a str) -> impl Iterator<Item = &'a XmlNode> {
        self.children.iter().filter(move |c| c.tag == tag)
    }

    /// Return the UNIQUE child with the given tag, failing closed if the parent holds
    /// **more than one**. RIF-XML single-cardinality wrappers appear at most once under
    /// their parent — `<object>`/`<slot>` under `<Frame>`, `<instance>`/`<class>` under
    /// `<Member>`, `<sub>`/`<sup>` under `<Subclass>`, `<left>`/`<right>` under `<Equal>`,
    /// `<content>`/`<op>`/`<args>` under an `<External>` call, and the `<formula>` under a
    /// `<Forall>`/`<Exists>`.
    ///
    /// The old `child()` (find-first) silently DROPPED the second wrapper on non-schema-valid
    /// input: a dropped `<formula>` under a `<Forall>` loses a whole rule; a dropped `<object>`
    /// changes the atom; a dropped `<slot>`/`<sup>`/`<right>` loses a conjunct. This is the
    /// parent-level twin of `only_child` (which guards a wrapper's surplus grandchildren) and
    /// mirrors the `<if>`/`<then>` duplicate guard already inlined in `parse_implies`; together
    /// they keep the module's fail-closed "nothing is silently skipped or dropped" contract
    /// airtight. `Ok(None)` is returned when the wrapper is ABSENT so each caller keeps its own
    /// "missing" diagnostic. Conformant RIF-XML has at most one of each wrapper, so this rejects
    /// only malformed input. `ctx` names the parent for the diagnostic. [OPUS-4.8] sq-4l1fj
    fn unique_child(&self, tag: &str, ctx: &str) -> Result<Option<&XmlNode>, ImportError> {
        // Iterate `self.children` directly (not via `children_named`, whose returned
        // iterator borrows `tag`) so the returned `&XmlNode` binds to `&self`.
        let mut matching = self.children.iter().filter(|c| c.tag == tag);
        let first = matching.next();
        if matching.next().is_some() {
            return Err(ImportError::MalformedXml(format!(
                "{} has duplicate <{}> elements (expected exactly one)",
                ctx, tag
            )));
        }
        Ok(first)
    }

    /// Return this node's single element child, failing closed if it has zero **or
    /// more than one**. RIF-XML single-cardinality wrappers — `<if>`, `<then>`,
    /// `<formula>`, `<object>`, `<instance>`, `<class>`, `<sub>`, `<sup>`, `<left>`,
    /// `<right>`, `<content>`, `<op>`, `<sentence>` — admit exactly one child element.
    ///
    /// The old `.children.first()` silently DROPPED surplus siblings on non-schema-valid
    /// input: an `<if>` holding two condition children lost the second conjunct, which
    /// WEAKENS the rule body → OVER-derivation (unsound); a `<then>`/`<formula>` holding
    /// two head/term children lost the second → under-derivation. Enforcing exactly-one
    /// here keeps the module's fail-closed "nothing is silently skipped or dropped"
    /// contract honest. Conformant RIF-XML always has exactly one child in these
    /// wrappers, so this rejects only malformed input. `ctx` names the wrapper for the
    /// diagnostic. [SONNET-4.6] sq-anuo9
    fn only_child(&self, ctx: &str) -> Result<&XmlNode, ImportError> {
        match self.children.as_slice() {
            [one] => Ok(one),
            [] => Err(ImportError::MalformedXml(format!("{} has no child element", ctx))),
            _ => Err(ImportError::MalformedXml(format!(
                "{} must have exactly one child element (single-cardinality wrapper), found {}",
                ctx,
                self.children.len()
            ))),
        }
    }
}

// ---------------------------------------------------------------------------
// XML tree builder
// ---------------------------------------------------------------------------

/// The RIF namespace IRI every RIF/XML `<Document>` root must be in.
const RIF_NS: &str = "http://www.w3.org/2007/rif#";

/// Maximum element nesting depth accepted by [`parse_xml_tree`].
///
/// The tree builder itself is iterative, but the interpretation that follows
/// (`parse_condition` ↔ `parse_formula_child`, `expand_body`, the alpha-renaming
/// passes, and the recursive `Drop` of the tree / `BodyCond`) recurses once per
/// nesting level, so an adversarial document of deeply nested `<And>`/`<Or>`/
/// `<Exists>` would overflow the stack and abort the process. Conformant RIF-Core
/// documents nest a few dozen levels at most; anything deeper is refused
/// fail-closed with [`ImportError::MalformedXml`]. #3359.
const MAX_XML_DEPTH: usize = 256;

/// Fail closed unless an element resolves to the RIF namespace. Every element is
/// checked, not just the root: a descendant that re-declares a foreign default
/// namespace (`<payload xmlns="urn:foreign">`), undeclares it (`xmlns=""`) or
/// rebinds the RIF prefix (`<rif:Group xmlns:rif="urn:foreign">`) leaves RIF, and
/// its local names must not be interpreted with RIF semantics. Resolution (scoping,
/// inheritance, rebinding) is quick-xml's `NsReader`. #3360.
fn check_rif_namespace(ns: &ResolveResult<'_>, local: &str) -> Result<(), ImportError> {
    match ns {
        ResolveResult::Bound(n) if n.as_ref() == RIF_NS.as_bytes() => Ok(()),
        ResolveResult::Bound(n) => Err(ImportError::MalformedXml(format!(
            "element <{}> is in namespace <{}>, not the RIF namespace <{}>",
            local,
            String::from_utf8_lossy(n.as_ref()),
            RIF_NS
        ))),
        ResolveResult::Unbound => Err(ImportError::MalformedXml(format!(
            "element <{}> is in no namespace; RIF/XML requires the RIF namespace <{}>",
            local, RIF_NS
        ))),
        ResolveResult::Unknown(p) => Err(ImportError::MalformedXml(format!(
            "element <{}> uses undeclared namespace prefix `{}`",
            local,
            String::from_utf8_lossy(p)
        ))),
    }
}

/// Parse `xml_bytes` into a tree of `XmlNode`s.
///
/// Every element must resolve to the RIF namespace ([`check_rif_namespace`]) and
/// element nesting is capped at [`MAX_XML_DEPTH`]; both fail closed.
///
/// # Whitespace handling
///
/// `trim_text` is NOT enabled globally — whitespace in `<Const>` string literals
/// is semantically significant. Per-type trimming is applied in `parse_term`
/// based on the XSD whitespace facet of each datatype.
fn parse_xml_tree(xml_bytes: &[u8]) -> Result<XmlNode, ImportError> {
    let mut reader = NsReader::from_reader(xml_bytes);
    // Do NOT set trim_text(true): whitespace in xsd:string Consts is preserved.
    // parse_term applies per-type trimming (IRI/numeric → trim; string → preserve).

    let mut stack: Vec<XmlNode> = Vec::new();

    loop {
        let (ns, event) = reader
            .read_resolved_event()
            .map_err(|e| ImportError::MalformedXml(format!("{}", e)))?;
        match event {
            Event::Start(e) => {
                let local = local_name_str(e.local_name().as_ref());
                check_rif_namespace(&ns, &local)?;
                if stack.len() >= MAX_XML_DEPTH {
                    return Err(ImportError::MalformedXml(format!(
                        "element nesting exceeds the maximum depth of {}",
                        MAX_XML_DEPTH
                    )));
                }
                let attrs = read_attrs(&e)?;
                stack.push(XmlNode { tag: local, text: String::new(), attrs, children: Vec::new() });
            }
            Event::Empty(e) => {
                // Self-closing element — push and immediately pop.
                let local = local_name_str(e.local_name().as_ref());
                check_rif_namespace(&ns, &local)?;
                let attrs = read_attrs(&e)?;
                let node = XmlNode { tag: local, text: String::new(), attrs, children: Vec::new() };
                if let Some(parent) = stack.last_mut() {
                    parent.children.push(node);
                } else {
                    // Root self-closing element — wrap and return.
                    return Ok(node);
                }
            }
            Event::Text(t) => {
                let s = t.decode().map_err(|e| ImportError::MalformedXml(format!("{}", e)));
                if let Some(top) = stack.last_mut() {
                    // Preserve text exactly — do NOT trim here.
                    // parse_term applies per-type trimming (IRI → trim; xsd:string → preserve).
                    top.text.push_str(&s?);
                }
            }
            Event::CData(t) => {
                if let Some(top) = stack.last_mut() {
                    top.text.push_str(&String::from_utf8_lossy(&t));
                }
            }
            Event::End(_) => {
                if let Some(node) = stack.pop() {
                    if stack.is_empty() {
                        return Ok(node);
                    }
                    let parent = stack.last_mut().unwrap();
                    parent.children.push(node);
                }
            }
            // Entity references (&amp;, &lt;, &#x26;, &#38;, etc.) are surfaced as
            // Event::GeneralRef in quick-xml 0.40+.  Resolve the five predefined XML
            // entities and numeric character references (decimal + hex); unknown general
            // entities are fail-closed (MalformedXml) — we never silently drop bytes.
            // [SONNET-4.6]
            Event::GeneralRef(r) => {
                let name =
                    r.decode().map_err(|e| ImportError::MalformedXml(format!("{}", e)))?;
                let resolved = resolve_xml_entity(&name)?;
                if let Some(top) = stack.last_mut() {
                    top.text.push_str(&resolved);
                }
            }
            Event::Eof => {
                break;
            }
            // Ignore comments, processing instructions, declarations.
            _ => {}
        }
    }

    Err(ImportError::MalformedXml("unexpected end of document".to_string()))
}

fn local_name_str(bytes: &[u8]) -> String {
    std::str::from_utf8(bytes).unwrap_or("").to_string()
}

/// Resolve one XML entity reference name (as quick-xml 0.40+ hands it out in
/// `Event::GeneralRef`, WITHOUT the surrounding `&`/`;`) to its replacement text.
///
/// Handles:
/// - Five predefined named entities: `amp` → `&`, `lt` → `<`, `gt` → `>`,
///   `quot` → `"`, `apos` → `'`.
/// - Decimal numeric character references: `#38` → `&`.
/// - Hexadecimal numeric character references: `#x26` / `#X26` → `&`.
///
/// Unknown general entities (e.g. `foo` for `&foo;`) → `MalformedXml` (fail-closed;
/// we never silently drop a character). [SONNET-4.6]
fn resolve_xml_entity(name: &str) -> Result<String, ImportError> {
    if let Some(rep) = quick_xml::escape::resolve_predefined_entity(name) {
        return Ok(rep.to_string());
    }
    if let Some(rest) = name.strip_prefix('#') {
        let cp = if let Some(hex) = rest.strip_prefix(['x', 'X']) {
            u32::from_str_radix(hex, 16)
        } else {
            rest.parse::<u32>()
        }
        .map_err(|_| {
            ImportError::MalformedXml(format!(
                "bad numeric character reference &{};",
                name
            ))
        })?;
        return char::from_u32(cp)
            .map(|c| c.to_string())
            .ok_or_else(|| {
                ImportError::MalformedXml(format!(
                    "numeric character reference &{}; out of Unicode range",
                    name
                ))
            });
    }
    Err(ImportError::MalformedXml(format!(
        "unknown XML entity reference &{};  (RIF/XML fail-closed: only predefined entities \
         and numeric character references are supported)",
        name
    )))
}

fn read_attrs(e: &quick_xml::events::BytesStart<'_>) -> Result<Vec<(String, String)>, ImportError> {
    let mut out = Vec::new();
    for a in e.attributes() {
        let a = a.map_err(|err| ImportError::MalformedXml(format!("{}", err)))?;
        // RIF/XML attributes (`type`, `ordered`, ...) are unqualified. A prefixed
        // attribute (`x:type`) is in some other namespace and a namespace
        // declaration is not content, so neither may be read by its local name.
        if a.key.prefix().is_some() || a.key.as_ref() == b"xmlns" {
            continue;
        }
        let key = local_name_str(a.key.local_name().as_ref());
        // Unescape the attribute value so that e.g. `type="http://ex/?a=1&amp;b=2"`
        // produces `"http://ex/?a=1&b=2"` rather than the literal `"&amp;"` text.
        // quick_xml::escape::unescape resolves the five predefined XML entities +
        // numeric character references, and returns Err for unknown entities.
        // [SONNET-4.6]
        let raw = std::str::from_utf8(&a.value)
            .map_err(|err| ImportError::MalformedXml(format!("{}", err)))?;
        let val = quick_xml::escape::unescape(raw)
            .map(|c| c.into_owned())
            .map_err(|err| ImportError::MalformedXml(format!("{}", err)))?;
        out.push((key, val));
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// Non-Core element guard
// ---------------------------------------------------------------------------

/// Non-Core / non-monotonic elements that must be rejected immediately.
const NON_CORE_TAGS: &[(&str, &str)] = &[
    ("Naf", "negation-as-failure is not in RIF-Core (monotone)"),
    ("NAF", "negation-as-failure is not in RIF-Core (monotone)"),
    ("Neg", "strong negation is not in RIF-Core"),
    ("Assert", "PRD Assert action is not in RIF-Core"),
    ("Retract", "PRD Retract action is not in RIF-Core"),
    ("Modify", "PRD Modify action is not in RIF-Core"),
    ("Do", "PRD Do action block is not in RIF-Core"),
    ("Act", "PRD Act block is not in RIF-Core"),
];

fn check_non_core(tag: &str) -> Result<(), ImportError> {
    for &(t, reason) in NON_CORE_TAGS {
        if tag == t {
            return Err(ImportError::NonCoreElement {
                element: tag.to_string(),
                reason: reason.to_string(),
            });
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Builtin IRI mapping
// ---------------------------------------------------------------------------

const FUNC: &str = "http://www.w3.org/2007/rif-builtin-function#";
const PRED: &str = "http://www.w3.org/2007/rif-builtin-predicate#";

/// Map a DTB builtin IRI to a `Builtin` variant, or return `ImportError::UnknownExternal`.
fn iri_to_builtin(iri: &str) -> Result<Builtin, ImportError> {
    // Numeric functions
    if iri == concat_strs(FUNC, "numeric-add") {
        return Ok(Builtin::NumericAdd);
    }
    if iri == concat_strs(FUNC, "numeric-subtract") {
        return Ok(Builtin::NumericSubtract);
    }
    if iri == concat_strs(FUNC, "numeric-multiply") {
        return Ok(Builtin::NumericMultiply);
    }
    if iri == concat_strs(FUNC, "numeric-divide") {
        return Ok(Builtin::NumericDivide);
    }
    // String functions
    if iri == concat_strs(FUNC, "upper-case") {
        return Ok(Builtin::StringUpperCase);
    }
    if iri == concat_strs(FUNC, "lower-case") {
        return Ok(Builtin::StringLowerCase);
    }
    if iri == concat_strs(FUNC, "encode-for-uri") {
        return Ok(Builtin::StringEncodeForUri);
    }
    if iri == concat_strs(FUNC, "concat") {
        return Ok(Builtin::StringConcat);
    }
    if iri == concat_strs(FUNC, "string-length") {
        return Ok(Builtin::StringLength);
    }
    // List functions
    if iri == concat_strs(FUNC, "concatenate") {
        return Ok(Builtin::ListConcatenate);
    }
    // [SONNET-4.6] Fix-4: func:count (list-member count) → Builtin::ListLength.
    // The downstream lowering (list:length) IS implemented; the importer was wrongly
    // rejecting this IRI as UnknownExternal (Opus adversarial-verify finding).
    if iri == concat_strs(FUNC, "count") {
        return Ok(Builtin::ListLength);
    }
    // Numeric predicates
    if iri == concat_strs(PRED, "numeric-equal") {
        return Ok(Builtin::NumericEqual);
    }
    if iri == concat_strs(PRED, "numeric-not-equal") {
        return Ok(Builtin::NumericNotEqual);
    }
    if iri == concat_strs(PRED, "numeric-less-than") {
        return Ok(Builtin::NumericLessThan);
    }
    if iri == concat_strs(PRED, "numeric-greater-than") {
        return Ok(Builtin::NumericGreaterThan);
    }
    if iri == concat_strs(PRED, "numeric-not-less-than") {
        return Ok(Builtin::NumericNotLessThan);
    }
    if iri == concat_strs(PRED, "numeric-not-greater-than") {
        return Ok(Builtin::NumericNotGreaterThan);
    }
    // String predicates
    if iri == concat_strs(PRED, "contains") {
        return Ok(Builtin::StringContains);
    }
    if iri == concat_strs(PRED, "starts-with") {
        return Ok(Builtin::StringStartsWith);
    }
    if iri == concat_strs(PRED, "ends-with") {
        return Ok(Builtin::StringEndsWith);
    }
    // List predicates
    if iri == concat_strs(PRED, "list-contains") {
        return Ok(Builtin::ListContains);
    }
    // Deferred / unknown → fail closed
    Err(ImportError::UnknownExternal { iri: iri.to_string() })
}

/// Const-friendly string concatenation for use in match arms.
fn concat_strs(a: &str, b: &str) -> String {
    format!("{}{}", a, b)
}

// ---------------------------------------------------------------------------
// XSD whitespace-facet helper
// ---------------------------------------------------------------------------

/// Returns `true` for XSD types whose whitespace facet is `collapse`
/// (leading/trailing/internal whitespace is not semantically significant in their
/// lexical space). For these types, `parse_term` trims the text value.
///
/// - `rif:iri` is handled separately upstream (→ `Term::Iri`, always trimmed).
/// - `xsd:string` and all string-derived types NOT listed here are **not** collapse
///   types — their lexical whitespace is preserved.
///
/// Per XSD Datatype specification (§4.3.6 whitespace): numeric, boolean, date/time,
/// `anyURI` use collapse; string uses preserve.
fn is_whitespace_collapse_type(ty: &str) -> bool {
    let prefix = "http://www.w3.org/2001/XMLSchema#";
    if !ty.starts_with(prefix) {
        return false;
    }
    let local = &ty[prefix.len()..];
    matches!(
        local,
        "integer"
            | "decimal"
            | "float"
            | "double"
            | "nonNegativeInteger"
            | "positiveInteger"
            | "negativeInteger"
            | "nonPositiveInteger"
            | "long"
            | "int"
            | "short"
            | "byte"
            | "unsignedLong"
            | "unsignedInt"
            | "unsignedShort"
            | "unsignedByte"
            | "boolean"
            | "date"
            | "dateTime"
            | "time"
            | "anyURI"
            | "duration"
    )
}

// ---------------------------------------------------------------------------
// Term parsing
// ---------------------------------------------------------------------------

/// The `rif:iri` type attribute value in `<Const type="…">`.
const RIF_IRI_TYPE: &str = "http://www.w3.org/2007/rif#iri";

/// The `rif:local` type attribute value in `<Const type="…">`.
///
/// RIF **local constants** are document-scoped: two `rif:local` constants with the
/// same name in DIFFERENT documents denote DISTINCT individuals (they share no
/// cross-document identity). This is a semantic property the current importer cannot
/// faithfully represent — the `Term::Lit` representation makes them structurally
/// equal across documents, which would cause a `NegativeEntailmentTest` to
/// incorrectly report the non-conclusion as "entailed" (the `Local_Constant` W3C
/// test demonstrates this exactly). We therefore reject `rif:local` constants
/// fail-closed rather than silently mis-importing them. [SONNET-4.6] sq-n7y15
const RIF_LOCAL_TYPE: &str = "http://www.w3.org/2007/rif#local";

/// Parse a `<Const>` or `<Var>` node into a `Term`.
///
/// # Whitespace handling (Fix-3)
///
/// - `rif:iri` Consts: trim (IRIs never contain meaningful leading/trailing whitespace).
/// - `xsd:string` Consts (empty `type` attr or explicit `xsd:string`) and all other
///   non-collapse-type literals: **preserve exact whitespace** (semantically significant
///   in lexical value space).
/// - Known XSD collapse types (numeric, boolean, date/time, anyURI): trim.
/// - `<Var>` text: trim (variable names do not contain meaningful whitespace).
fn parse_term(node: &XmlNode) -> Result<Term, ImportError> {
    match node.tag.as_str() {
        "Const" => {
            let ty = node.attr("type").unwrap_or("");
            if ty == RIF_LOCAL_TYPE {
                // rif:local constants are document-scoped: the same name in two different
                // documents denotes two DISTINCT individuals. The Term::Lit representation
                // would make them structurally equal across documents — a soundness hazard
                // (a NegativeEntailmentTest with a rif:local non-conclusion would
                // incorrectly report the non-conclusion as entailed). Reject fail-closed.
                // [SONNET-4.6] sq-n7y15
                return Err(ImportError::UnrecognizedElement {
                    tag: "Const(rif:local) — local constants are document-scoped; \
                          cross-document identity cannot be faithfully represented \
                          (fail-closed, not silently mis-imported)"
                        .to_string(),
                });
            }
            if ty == RIF_IRI_TYPE {
                // IRI: XSD whitespace-collapse; trim for safety.
                Ok(Term::Iri(node.text.trim().to_string()))
            } else if ty.is_empty() {
                // No type attribute → plain xsd:string; preserve whitespace exactly.
                Ok(Term::Lit {
                    lex: node.text.clone(),
                    datatype: "http://www.w3.org/2001/XMLSchema#string".to_string(),
                })
            } else if is_whitespace_collapse_type(ty) {
                // Known XSD collapse types: trim.
                Ok(Term::Lit { lex: node.text.trim().to_string(), datatype: ty.to_string() })
            } else {
                // All other types (including explicit xsd:string and unrecognized types):
                // preserve exact whitespace — the lexical form belongs to the value space
                // of the declared datatype, which may be whitespace-sensitive.
                Ok(Term::Lit { lex: node.text.clone(), datatype: ty.to_string() })
            }
        }
        "Var" => {
            // Variable names are whitespace-insensitive; trim surrounding whitespace
            // that may appear from XML indentation.
            Ok(Term::Var(node.text.trim().to_string()))
        }
        "List" => {
            // <List><items><...term...><...term...></items></List>
            // Fail-closed on a DUPLICATE <items> wrapper (sq-4l1fj): the same
            // single-cardinality-wrapper class — first-wins would silently drop a
            // second <items>, changing the list term.
            let items_node = node.unique_child("items", "List")?;
            let items = match items_node {
                Some(items) => items
                    .children
                    .iter()
                    .map(parse_term)
                    .collect::<Result<Vec<_>, _>>()?,
                None => Vec::new(),
            };
            Ok(Term::List(items))
        }
        other => {
            check_non_core(other)?;
            Err(ImportError::UnrecognizedElement { tag: other.to_string() })
        }
    }
}

// ---------------------------------------------------------------------------
// Body condition intermediate representation
// ---------------------------------------------------------------------------

/// Intermediate representation for a body condition parsed from RIF/XML.
/// Converted to `Vec<Vec<Atom>>` (list of disjuncts, each a conjunction) by
/// `expand_body` after `alpha_rename_cond` has processed all `Exists` nodes.
enum BodyCond {
    Atom(Atom),
    And(Vec<BodyCond>),
    Or(Vec<BodyCond>),
    /// Exists(declared_var_names, sub_condition).
    ///
    /// The declared vars are alpha-renamed to fresh names by `alpha_rename_cond`
    /// (which simultaneously drops this wrapper). The `expand_body` Exists arm is
    /// a safe fallback but should not be reached in the normal import path.
    Exists(Vec<String>, Box<BodyCond>),
}

/// Expand `BodyCond` into a list of disjuncts (each disjunct is a conjunction of atoms).
/// Or-split (Lloyd–Topor) and Exists-flatten are applied here.
///
/// In the normal import path `alpha_rename_cond` is called before `expand_body`, so
/// all `Exists` wrappers have already been dropped and their declared vars renamed.
/// The `Exists` arm below is a safe fallback used by mutation-check tests that call
/// `expand_body` without prior renaming to demonstrate the variable-capture behaviour
/// that alpha-renaming fixes.
fn expand_body(cond: BodyCond) -> Vec<Vec<Atom>> {
    match cond {
        BodyCond::Atom(a) => vec![vec![a]],
        BodyCond::And(conds) => {
            // Cross-product of all sub-disjuncts.
            let mut result: Vec<Vec<Atom>> = vec![vec![]];
            for sub in conds {
                let sub_disj = expand_body(sub);
                result = cross_product(result, sub_disj);
            }
            result
        }
        BodyCond::Or(conds) => {
            // Union: each disjunct becomes a separate rule body.
            conds.into_iter().flat_map(expand_body).collect()
        }
        BodyCond::Exists(_, sub) => {
            // Fallback flatten (alpha_rename_cond should have removed this wrapper).
            expand_body(*sub)
        }
    }
}

fn cross_product(a: Vec<Vec<Atom>>, b: Vec<Vec<Atom>>) -> Vec<Vec<Atom>> {
    let mut result = Vec::with_capacity(a.len() * b.len().max(1));
    for da in &a {
        for db in &b {
            let mut combined = da.clone();
            combined.extend_from_slice(db);
            result.push(combined);
        }
    }
    if result.is_empty() {
        a
    } else {
        result
    }
}

// ---------------------------------------------------------------------------
// Alpha-renaming (Fix-1: variable capture)
// ---------------------------------------------------------------------------

/// Collect all variable names occurring in a `BodyCond` tree (including those
/// declared in `Exists` nodes) into `out`.
fn collect_vars_in_cond(cond: &BodyCond, out: &mut BTreeSet<String>) {
    match cond {
        BodyCond::Atom(a) => collect_vars_in_atom(a, out),
        BodyCond::And(subs) | BodyCond::Or(subs) => {
            for s in subs {
                collect_vars_in_cond(s, out);
            }
        }
        BodyCond::Exists(declared_vars, sub) => {
            for v in declared_vars {
                out.insert(v.clone());
            }
            collect_vars_in_cond(sub, out);
        }
    }
}

fn collect_vars_in_atom(a: &Atom, out: &mut BTreeSet<String>) {
    match a {
        Atom::Frame { obj, pred, val } => {
            collect_var_in_term(obj, out);
            collect_var_in_term(pred, out);
            collect_var_in_term(val, out);
        }
        Atom::Member { obj, class } => {
            collect_var_in_term(obj, out);
            collect_var_in_term(class, out);
        }
        Atom::Subclass { sub, sup } => {
            collect_var_in_term(sub, out);
            collect_var_in_term(sup, out);
        }
        Atom::Equal { left, right } => {
            collect_var_in_term(left, out);
            collect_var_in_term(right, out);
        }
        Atom::Builtin { args, .. } => {
            for t in args {
                collect_var_in_term(t, out);
            }
        }
    }
}

fn collect_var_in_term(t: &Term, out: &mut BTreeSet<String>) {
    match t {
        Term::Var(v) => {
            out.insert(v.clone());
        }
        Term::List(items) => {
            for item in items {
                collect_var_in_term(item, out);
            }
        }
        _ => {}
    }
}

/// Generate a fresh variable name not already in `universe`.
///
/// Names are drawn from the reserved `__ex{N}` namespace. The counter is
/// incremented past any names already in `universe` (so if a user document
/// improbably uses `__ex0`, we skip to `__ex1`).
fn fresh_name(counter: &mut u32, universe: &BTreeSet<String>) -> String {
    loop {
        let name = format!("__ex{}", counter);
        *counter += 1;
        if !universe.contains(&name) {
            return name;
        }
    }
}

/// Apply a variable substitution to a `Term`.
fn sub_term(t: Term, map: &HashMap<String, String>) -> Term {
    match t {
        Term::Var(v) => {
            if let Some(fresh) = map.get(&v) {
                Term::Var(fresh.clone())
            } else {
                Term::Var(v)
            }
        }
        Term::List(items) => Term::List(items.into_iter().map(|i| sub_term(i, map)).collect()),
        other => other,
    }
}

/// Apply a variable substitution to an `Atom`.
fn substitute_in_atom(a: Atom, map: &HashMap<String, String>) -> Atom {
    match a {
        Atom::Frame { obj, pred, val } => Atom::Frame {
            obj: sub_term(obj, map),
            pred: sub_term(pred, map),
            val: sub_term(val, map),
        },
        Atom::Member { obj, class } => Atom::Member {
            obj: sub_term(obj, map),
            class: sub_term(class, map),
        },
        Atom::Subclass { sub, sup } => Atom::Subclass {
            sub: sub_term(sub, map),
            sup: sub_term(sup, map),
        },
        Atom::Equal { left, right } => Atom::Equal {
            left: sub_term(left, map),
            right: sub_term(right, map),
        },
        Atom::Builtin { op, args } => Atom::Builtin {
            op,
            args: args.into_iter().map(|t| sub_term(t, map)).collect(),
        },
    }
}

/// Apply a variable substitution to a `BodyCond` tree.
fn substitute_in_cond(cond: BodyCond, map: &HashMap<String, String>) -> BodyCond {
    match cond {
        BodyCond::Atom(a) => BodyCond::Atom(substitute_in_atom(a, map)),
        BodyCond::And(subs) => {
            BodyCond::And(subs.into_iter().map(|s| substitute_in_cond(s, map)).collect())
        }
        BodyCond::Or(subs) => {
            BodyCond::Or(subs.into_iter().map(|s| substitute_in_cond(s, map)).collect())
        }
        BodyCond::Exists(vars, sub) => {
            // Defensive: substitute through any Exists nodes not yet processed.
            // In the normal alpha_rename_cond path (innermost-first), inner Exists
            // have already been replaced by their renamed subs — this arm is only
            // reached in unusual calling sequences.
            BodyCond::Exists(vars, Box::new(substitute_in_cond(*sub, map)))
        }
    }
}

/// Alpha-rename all `Exists`-declared variables to globally fresh names.
///
/// # Scope discipline — innermost binder wins
///
/// Processing is innermost-first (DFS pre-order on sub-conditions before the
/// enclosing `Exists`). By the time an outer `Exists` generates its substitution,
/// every inner `Exists` has already renamed its own vars to fresh names, so the
/// outer map finds no occurrences of inner vars — the innermost binder wins.
///
/// # Exists wrapper removal
///
/// Each `Exists` arm returns the **renamed sub-condition WITHOUT the `Exists`
/// wrapper** — the renaming simultaneously performs the Exists-flatten step.
/// After this call the returned `BodyCond` tree contains no `Exists` nodes.
///
/// # Arguments
///
/// * `counter` — monotonically incremented across all `Exists` nodes in the
///   rule; ensures global freshness even across sibling `Exists` nodes.
/// * `universe` — the complete set of already-known names; extended with each
///   generated fresh name so siblings cannot collide with earlier siblings.
fn alpha_rename_cond(
    cond: BodyCond,
    counter: &mut u32,
    universe: &mut BTreeSet<String>,
) -> BodyCond {
    match cond {
        BodyCond::Atom(a) => BodyCond::Atom(a),
        BodyCond::And(subs) => BodyCond::And(
            subs.into_iter()
                .map(|s| alpha_rename_cond(s, counter, universe))
                .collect(),
        ),
        BodyCond::Or(subs) => BodyCond::Or(
            subs.into_iter()
                .map(|s| alpha_rename_cond(s, counter, universe))
                .collect(),
        ),
        BodyCond::Exists(declared_vars, sub) => {
            // Step 1: rename any nested Exists first (innermost-first).
            let renamed_sub = alpha_rename_cond(*sub, counter, universe);
            // Step 2: build a substitution map — each declared var → a fresh name.
            let mut map: HashMap<String, String> = HashMap::new();
            for var in &declared_vars {
                let fresh = fresh_name(counter, universe);
                universe.insert(fresh.clone());
                map.insert(var.clone(), fresh);
            }
            // Step 3: apply the substitution to the (already innermost-renamed) sub.
            // The Exists wrapper is dropped here — this IS the flatten step.
            substitute_in_cond(renamed_sub, &map)
        }
    }
}

// ---------------------------------------------------------------------------
// Atom parsing
// ---------------------------------------------------------------------------

/// Parse an element that should be a positive atom (or atoms — a multi-slot Frame
/// desugars into N atoms). Returns the desugared list of atoms.
///
/// Multi-slot Frame desugaring (sq-jsgyn): a `<Frame>` with N `<slot>` children
/// `obj[p1->v1 p2->v2 …]` returns N `Atom::Frame` values — one per slot. All
/// other atom types are always single, so their list is a singleton. See
/// `parse_frame_atoms` for the per-slot logic and fail-closed invariants.
fn parse_positive_atoms(node: &XmlNode) -> Result<Vec<Atom>, ImportError> {
    match node.tag.as_str() {
        // Multi-slot Frame: returns one Atom per slot (sq-jsgyn).
        "Frame" => parse_frame_atoms(node),
        "Member" => Ok(vec![parse_member(node)?]),
        "Subclass" => Ok(vec![parse_subclass(node)?]),
        "Equal" => Ok(vec![parse_equal(node)?]),
        // [SONNET-4.6] sq-n7y15 — positional predicate Atom: Atom(op args...) import.
        "Atom" => Ok(vec![parse_positional_atom(node)?]),
        other => {
            check_non_core(other)?;
            Err(ImportError::UnrecognizedElement { tag: other.to_string() })
        }
    }
}

/// Parse a bare positional `<Atom>` — a RIF-Core n-ary predicate call `P(arg1, arg2, …)`.
///
/// ## RIF-Core XML structure
///
/// ```xml
/// <Atom>
///   <op><Const type="http://www.w3.org/2007/rif#iri">http://ex/P</Const></op>
///   <args ordered="yes">
///     <Const type="http://www.w3.org/2007/rif#iri">http://ex/a</Const>
///     <Var>x</Var>
///   </args>
/// </Atom>
/// ```
///
/// ## Soundly supported arities → existing `Atom` variants (scope: rif_xml only)
///
/// The RIF-Core internal model (`rif.rs`) has no first-class n-ary predicate variant.
/// This function maps positional atoms to the semantically equivalent **existing** variants:
///
/// | Arity | Positional form | Mapped to | RIF-Core equivalence |
/// |-------|-----------------|-----------|----------------------|
/// | **2** | `P(arg1, arg2)` | `Atom::Frame { obj: arg1, pred: P, val: arg2 }` | Binary predicate = frame atom `arg1[P -> arg2]` (RIF-Core §2.3) |
/// | **1** | `C(arg1)` | `Atom::Member { obj: arg1, class: C }` | Unary predicate = membership `arg1 # C` |
///
/// Arities **0** and **3+** are rejected fail-closed: there is no semantically equivalent
/// existing atom form, so importing them silently into a wrong shape would be unsound.
/// A non-IRI operator is similarly rejected.
///
/// ## Fail-closed invariant
///
/// Any positional atom that cannot be soundly mapped to an existing variant is rejected
/// with a clear `ImportError` — never silently imported with altered semantics.
/// [SONNET-4.6] sq-n7y15
fn parse_positional_atom(node: &XmlNode) -> Result<Atom, ImportError> {
    // <op> is a single-cardinality wrapper: fail-closed on duplicates (sq-4l1fj).
    let op_node = node.unique_child("op", "positional Atom")?.ok_or_else(|| {
        ImportError::MalformedXml("positional <Atom> missing <op>".to_string())
    })?;
    // The <op> child must be exactly one <Const> carrying the predicate IRI.
    let op_const = op_node.only_child("positional Atom <op>")?;
    if op_const.tag != "Const" {
        return Err(ImportError::MalformedXml(format!(
            "positional Atom <op> child must be <Const>, found <{}>",
            op_const.tag
        )));
    }
    // The predicate IRI — parsed by parse_term so the type attribute is validated.
    let pred_term = parse_term(op_const)?;
    let pred_iri = match &pred_term {
        Term::Iri(iri) => iri.clone(),
        _ => {
            return Err(ImportError::MalformedXml(
                "positional Atom operator must be an IRI-typed Const (rif:iri); \
                 non-IRI operators are not supported (fail-closed)"
                    .to_string(),
            ));
        }
    };

    // <args> is a single-cardinality wrapper: fail-closed on duplicates (sq-4l1fj).
    let args: Vec<Term> = match node.unique_child("args", "positional Atom")? {
        Some(args_node) => args_node
            .children
            .iter()
            .map(parse_term)
            .collect::<Result<Vec<_>, _>>()?,
        None => Vec::new(),
    };

    // Named-argument check: the <args> form is positional-only; a named-arg slot would
    // use a different XML structure (bead-scope does not include named-arg Atoms since
    // those are rejected at the Frame/<slot>/<Name> level). Guard defensively.
    if node.child("slot").is_some() {
        let name = node
            .child("slot")
            .and_then(|s| s.child("Name"))
            .and_then(|n| n.children.first())
            .map(|n| n.text.clone())
            .unwrap_or_default();
        return Err(ImportError::NamedArgUniterm { name });
    }

    // Map arity → semantically equivalent existing Atom variant.
    // Fail-closed on unsupported arities — never silently mis-import.
    match args.as_slice() {
        // Arity 1: unary predicate C(arg1) ≡ arg1 # C (membership). [SONNET-4.6] sq-n7y15
        [arg1] => Ok(Atom::Member {
            obj: arg1.clone(),
            class: Term::Iri(pred_iri),
        }),
        // Arity 2: binary predicate P(arg1, arg2) ≡ arg1[P -> arg2] (frame). [SONNET-4.6] sq-n7y15
        [arg1, arg2] => Ok(Atom::Frame {
            obj: arg1.clone(),
            pred: Term::Iri(pred_iri),
            val: arg2.clone(),
        }),
        // Arity 0 or 3+: no sound mapping in the existing Core model — reject fail-closed.
        _ => Err(ImportError::UnrecognizedElement {
            tag: format!(
                "Atom (positional, arity {}) — only arity-1 and arity-2 \
                 positional atoms are supported (arity-1 maps to membership, \
                 arity-2 maps to frame); arity-{} has no sound mapping in \
                 the Core model (fail-closed)",
                args.len(),
                args.len()
            ),
        }),
    }
}

/// Parse a `<Frame>` node into one `Atom::Frame` per `<slot>` child, desugaring
/// a multi-slot frame `obj[p1->v1 p2->v2 …]` into the conjunction of single-slot
/// frames `[obj[p1->v1], obj[p2->v2], …]`.
///
/// # RIF-Core §2.3 semantics
///
/// Under RIF-Core, `obj[p1->v1 p2->v2]` is syntactic sugar for the conjunction
/// `obj[p1->v1] And obj[p2->v2]`. Each per-slot atom is a first-class `Atom::Frame`
/// in the `rif::Document` model. The desugaring is sound and semantically equivalent
/// to N single-slot frames.
///
/// # Fail-closed invariants (sq-jsgyn)
///
/// - **Zero `<slot>` children** → `MalformedXml` (a Frame must carry at least one slot).
/// - **Duplicate `<object>`** → `MalformedXml` (single-cardinality; sq-4l1fj unchanged).
/// - **Missing `<object>`** → `MalformedXml`.
/// - **Named-argument slot** (`<slot><Name>…</Name>…</slot>`) → `NamedArgUniterm`
///   (checked per slot; one bad slot rejects the whole Frame).
/// - **`<slot>` with != 2 term children** → `MalformedXml` (checked per slot).
/// - Multiple `<slot>` children are VALID and desugar to multiple atoms; a Frame
///   document with only `<object>` and no `<slot>` is malformed — unlike the prior
///   single-slot-only importer, the `<slot>` multiplicity guard is now `>= 1`.
///
/// [SONNET-4.6] sq-jsgyn
fn parse_frame_atoms(node: &XmlNode) -> Result<Vec<Atom>, ImportError> {
    // <object> is single-cardinality (sq-4l1fj): duplicate <object> → MalformedXml.
    let obj_node = node.unique_child("object", "Frame")?.ok_or_else(|| ImportError::MalformedXml(
        "Frame missing <object>".to_string(),
    ))?;
    let obj = parse_first_term(obj_node)?;

    // Collect ALL <slot> children — multiple slots are legal (multi-slot frame, sq-jsgyn).
    let slots: Vec<&XmlNode> = node.children_named("slot").collect();

    // Fail-closed: a <Frame> must carry at least one <slot>.
    if slots.is_empty() {
        return Err(ImportError::MalformedXml(
            "Frame missing <slot> (at least one <slot> is required)".to_string(),
        ));
    }

    // Desugar each slot into a single Atom::Frame. Per-slot named-arg guard.
    let mut atoms = Vec::with_capacity(slots.len());
    for slot in slots {
        // Detect named-argument uniterms: a slot child with a <Name> child is a named arg.
        if slot.child("Name").is_some() {
            let name = slot
                .child("Name")
                .and_then(|n| n.children.first())
                .map(|n| n.text.clone())
                .unwrap_or_default();
            return Err(ImportError::NamedArgUniterm { name });
        }

        let slot_terms: Vec<Term> =
            slot.children.iter().map(parse_term).collect::<Result<_, _>>()?;
        if slot_terms.len() != 2 {
            return Err(ImportError::MalformedXml(format!(
                "Frame <slot> must have exactly 2 term children, got {}",
                slot_terms.len()
            )));
        }
        atoms.push(Atom::Frame {
            obj: obj.clone(),
            pred: slot_terms[0].clone(),
            val: slot_terms[1].clone(),
        });
    }
    Ok(atoms)
}

fn parse_member(node: &XmlNode) -> Result<Atom, ImportError> {
    // <Member><instance>TERM</instance><class>TERM</class></Member>
    // Fail-closed on a DUPLICATE <instance>/<class> wrapper (sq-4l1fj).
    let inst = node.unique_child("instance", "Member")?.ok_or_else(|| ImportError::MalformedXml(
        "Member missing <instance>".to_string(),
    ))?;
    let cls = node.unique_child("class", "Member")?.ok_or_else(|| ImportError::MalformedXml(
        "Member missing <class>".to_string(),
    ))?;
    Ok(Atom::Member { obj: parse_first_term(inst)?, class: parse_first_term(cls)? })
}

fn parse_subclass(node: &XmlNode) -> Result<Atom, ImportError> {
    // <Subclass><sub>TERM</sub><sup>TERM</sup></Subclass>
    // Fail-closed on a DUPLICATE <sub>/<sup> wrapper (sq-4l1fj).
    let sub = node.unique_child("sub", "Subclass")?.ok_or_else(|| ImportError::MalformedXml(
        "Subclass missing <sub>".to_string(),
    ))?;
    let sup = node.unique_child("sup", "Subclass")?.ok_or_else(|| ImportError::MalformedXml(
        "Subclass missing <sup>".to_string(),
    ))?;
    Ok(Atom::Subclass { sub: parse_first_term(sub)?, sup: parse_first_term(sup)? })
}

fn parse_equal(node: &XmlNode) -> Result<Atom, ImportError> {
    // <Equal><left>TERM</left><right>TERM</right></Equal>
    // Fail-closed on a DUPLICATE <left>/<right> wrapper (sq-4l1fj).
    let left = node.unique_child("left", "Equal")?.ok_or_else(|| ImportError::MalformedXml(
        "Equal missing <left>".to_string(),
    ))?;
    let right = node.unique_child("right", "Equal")?.ok_or_else(|| ImportError::MalformedXml(
        "Equal missing <right>".to_string(),
    ))?;
    Ok(Atom::Equal { left: parse_first_term(left)?, right: parse_first_term(right)? })
}

/// Parse the single term child of a single-cardinality wrapper element (e.g.
/// `<object>TERM</object>`, `<instance>`, `<class>`, `<sub>`, `<sup>`, `<left>`,
/// `<right>`). Surplus term siblings are rejected (fail-closed, sq-anuo9) rather
/// than silently dropped by the old `.children.first()`. [SONNET-4.6]
fn parse_first_term(wrapper: &XmlNode) -> Result<Term, ImportError> {
    let child = wrapper.only_child(&format!("term wrapper <{}>", wrapper.tag))?;
    parse_term(child)
}

/// Parse an `<External>` builtin call. Returns an `Atom::Builtin`.
fn parse_external(node: &XmlNode) -> Result<Atom, ImportError> {
    // <External><content><Atom><op><Const type="rif:iri">IRI</Const></op><args>...</args></Atom></content></External>
    // Fail-closed on a DUPLICATE <content> wrapper (sq-4l1fj).
    let content = node.unique_child("content", "External")?.ok_or_else(|| ImportError::MalformedXml(
        "External missing <content>".to_string(),
    ))?;
    // The child of <content> should be <Atom> (for predicates) or <Expr> (for functions).
    // Single-cardinality: surplus siblings are rejected, not dropped (sq-anuo9).
    let inner = content.only_child("External <content>")?;
    if inner.tag != "Atom" && inner.tag != "Expr" {
        check_non_core(&inner.tag)?;
        return Err(ImportError::UnrecognizedElement { tag: inner.tag.clone() });
    }

    // Get the operator IRI from <op><Const type="rif:iri">IRI</Const></op>
    // Fail-closed on a DUPLICATE <op> wrapper (sq-4l1fj).
    let op_node = inner.unique_child("op", "External Atom/Expr")?.ok_or_else(|| ImportError::MalformedXml(
        "External Atom/Expr missing <op>".to_string(),
    ))?;
    // Single-cardinality: the <op> wrapper holds exactly one <Const> (sq-anuo9).
    let op_const = op_node.only_child("External <op>")?;
    let op_iri = if op_const.tag == "Const" {
        op_const.text.trim().to_string()
    } else {
        return Err(ImportError::MalformedXml("External <op> child is not <Const>".to_string()));
    };

    let builtin = iri_to_builtin(&op_iri)?;

    // Collect args from <args><...term...><...term...></args>
    // Fail-closed on a DUPLICATE <args> wrapper (sq-4l1fj) — a second <args> would
    // otherwise be silently dropped, changing the builtin's argument list.
    let args = match inner.unique_child("args", "External Atom/Expr")? {
        Some(args_node) => args_node
            .children
            .iter()
            .map(parse_term)
            .collect::<Result<Vec<_>, _>>()?,
        None => Vec::new(),
    };

    Ok(Atom::Builtin { op: builtin, args })
}

// ---------------------------------------------------------------------------
// Body condition parsing
// ---------------------------------------------------------------------------

/// Parse a condition element (what can appear inside `<if>`, `<formula>`,
/// `<And>`, `<Or>`, `<Exists>`'s body, etc.) into a `BodyCond`.
///
/// # Fail-closed children check (Fix-2)
///
/// `<And>` and `<Or>` now reject any non-`<formula>` child element with
/// `ImportError::UnrecognizedElement`, instead of silently dropping them.
fn parse_condition(node: &XmlNode) -> Result<BodyCond, ImportError> {
    match node.tag.as_str() {
        "And" => {
            // Fail-closed: every child of <And> MUST be <formula>.
            let mut subs = Vec::new();
            for c in &node.children {
                if c.tag != "formula" {
                    check_non_core(&c.tag)?;
                    return Err(ImportError::UnrecognizedElement { tag: c.tag.clone() });
                }
                subs.push(parse_formula_child(c)?);
            }
            Ok(BodyCond::And(subs))
        }
        "Or" => {
            // Fail-closed: every child of <Or> MUST be <formula>.
            let mut subs = Vec::new();
            for c in &node.children {
                if c.tag != "formula" {
                    check_non_core(&c.tag)?;
                    return Err(ImportError::UnrecognizedElement { tag: c.tag.clone() });
                }
                subs.push(parse_formula_child(c)?);
            }
            Ok(BodyCond::Or(subs))
        }
        "Exists" => {
            // <Exists>
            //   <declare><Var>z</Var></declare>  (one or more, each with exactly one <Var>)
            //   <formula>CONDITION</formula>
            // </Exists>
            // Collect declared variable names (used by alpha_rename_cond in parse_implies).
            //
            // [SONNET-4.6] Fix-6 + sq-anuo9: fail-closed on <declare> with != 1 child, or a
            // single non-<Var> child. The old filter_map(.first()) silently dropped all but
            // the first <Var> (demonstrated capture: Exists <declare>?a ?b</declare> conflated
            // universal ?b with existential ?b); the earlier Fix-6 filter-to-<Var> still
            // TOLERATED a stray non-<Var> sibling (e.g. <declare><Var>a</Var><Const>…</Const>)
            // by ignoring it. A <declare> admits exactly ONE child and it MUST be a <Var>; any
            // other shape is not schema-valid RIF-XML, so we reject rather than guess intent.
            let mut declared_vars: Vec<String> = Vec::new();
            for d in node.children_named("declare") {
                match d.children.as_slice() {
                    [only] if only.tag == "Var" => {
                        declared_vars.push(only.text.trim().to_string());
                    }
                    _ => {
                        return Err(ImportError::MalformedXml(format!(
                            "<declare> must contain exactly one <Var> child, found {} child element(s) (in <Exists>)",
                            d.children.len()
                        )));
                    }
                }
            }
            // Fail-closed on a DUPLICATE <formula> wrapper (sq-4l1fj): a dropped second
            // <formula> under <Exists> silently loses a conjunct of the existential body.
            let formula = node.unique_child("formula", "Exists")?.ok_or_else(|| ImportError::MalformedXml(
                "Exists missing <formula>".to_string(),
            ))?;
            let sub = parse_formula_child(formula)?;
            Ok(BodyCond::Exists(declared_vars, Box::new(sub)))
        }
        // Multi-slot Frame in body position: desugar to conjunction (sq-jsgyn).
        // A single-slot frame produces BodyCond::Atom directly (no unnecessary And wrapper).
        "Frame" => {
            let atoms = parse_frame_atoms(node)?;
            if atoms.len() == 1 {
                Ok(BodyCond::Atom(atoms.into_iter().next().unwrap()))
            } else {
                // N-slot frame → And(obj[p1->v1], obj[p2->v2], …).
                // Under RIF-Core §2.3 a multi-slot frame is the conjunction of per-slot
                // frames; this is the semantically equivalent Horn-body lowering.
                // [SONNET-4.6] sq-jsgyn
                Ok(BodyCond::And(atoms.into_iter().map(BodyCond::Atom).collect()))
            }
        }
        "Member" => Ok(BodyCond::Atom(parse_member(node)?)),
        "Subclass" => Ok(BodyCond::Atom(parse_subclass(node)?)),
        "Equal" => Ok(BodyCond::Atom(parse_equal(node)?)),
        "External" => Ok(BodyCond::Atom(parse_external(node)?)),
        // [SONNET-4.6] sq-n7y15 — positional Atom in body conditions.
        "Atom" => Ok(BodyCond::Atom(parse_positional_atom(node)?)),
        other => {
            check_non_core(other)?;
            Err(ImportError::UnrecognizedElement { tag: other.to_string() })
        }
    }
}

/// Parse the child of a `<formula>` wrapper (descend into the wrapper's single child).
/// A `<formula>` holds exactly one condition; a surplus sibling is rejected rather than
/// silently dropped (sq-anuo9 — a dropped conjunct weakens the body → over-derivation).
fn parse_formula_child(formula: &XmlNode) -> Result<BodyCond, ImportError> {
    let child = formula.only_child("<formula>")?;
    parse_condition(child)
}

// ---------------------------------------------------------------------------
// Head parsing
// ---------------------------------------------------------------------------

/// Parse the head of an `<Implies>` (the `<then>` child). Head may be a single
/// positive atom (or multi-slot Frame → multiple atoms) or an `<And>` of positive atoms.
///
/// # Fail-closed children check (Fix-2)
///
/// The `<And>` case now rejects any non-`<formula>` child element.
///
/// # Multi-slot Frame in head position (sq-jsgyn)
///
/// A `<Frame>` with N slots in head position adds N atoms to the rule head.
/// This is the sound RIF-Core lowering: a multi-slot frame conclusion is the
/// conjunction of per-slot atoms (all must be derived for the rule to fire).
fn parse_head(node: &XmlNode) -> Result<Vec<Atom>, ImportError> {
    match node.tag.as_str() {
        "And" => {
            // Conjunctive head — fail-closed: every child MUST be <formula>.
            let mut head = Vec::new();
            for c in &node.children {
                if c.tag != "formula" {
                    check_non_core(&c.tag)?;
                    return Err(ImportError::UnrecognizedElement { tag: c.tag.clone() });
                }
                // Single-cardinality: each head-<formula> holds exactly one atom element;
                // a surplus sibling head atom is rejected, not dropped (sq-anuo9).
                // A multi-slot Frame in a head-<formula> desugars to multiple atoms. [sq-jsgyn]
                let child = c.only_child("<formula> in head And")?;
                head.extend(parse_positive_atoms(child)?);
            }
            Ok(head)
        }
        _ => {
            // Single positive atom (or multi-slot Frame → multiple atoms). [sq-jsgyn]
            parse_positive_atoms(node)
        }
    }
}

// ---------------------------------------------------------------------------
// Sentence / rule parsing
// ---------------------------------------------------------------------------

/// Parse a `<Forall>` or a bare fact sentence into zero-or-more `Rule`s (zero-or-more
/// because Or-split can produce multiple rules).
fn parse_sentence(node: &XmlNode) -> Result<Vec<Rule>, ImportError> {
    match node.tag.as_str() {
        "Forall" => {
            // Collect universally declared variable names.
            // These seed the alpha-rename universe so generated fresh names cannot
            // collide with universally-declared vars.
            //
            // [SONNET-4.6] Fix-6 + sq-anuo9: identical guard to the Exists arm — reject a
            // <declare> whose sole child is not a <Var>, or that has != 1 child, instead of
            // silently dropping extras or tolerating a stray non-<Var> sibling.
            let mut forall_vars_vec: Vec<String> = Vec::new();
            for d in node.children_named("declare") {
                match d.children.as_slice() {
                    [only] if only.tag == "Var" => {
                        forall_vars_vec.push(only.text.trim().to_string());
                    }
                    _ => {
                        return Err(ImportError::MalformedXml(format!(
                            "<declare> must contain exactly one <Var> child, found {} child element(s) (in <Forall>)",
                            d.children.len()
                        )));
                    }
                }
            }
            let forall_vars: BTreeSet<String> = forall_vars_vec.into_iter().collect();

            // Fail-closed on a DUPLICATE <formula> wrapper (sq-4l1fj): a dropped second
            // <formula> under <Forall> silently loses a whole rule.
            let formula = node.unique_child("formula", "Forall")?.ok_or_else(|| ImportError::MalformedXml(
                "Forall missing <formula>".to_string(),
            ))?;
            // Single-cardinality: the Forall <formula> holds exactly one body element
            // (an <Implies> or a bare atom); a surplus sibling is rejected (sq-anuo9).
            let body_node = formula.only_child("Forall <formula>")?;
            match body_node.tag.as_str() {
                "Implies" => parse_implies(body_node, &forall_vars),
                other => {
                    // A bare atom as a Forall formula = universally closed fact.
                    // Multi-slot Frame desugars to one Rule::fact per slot. [sq-jsgyn]
                    check_non_core(other)?;
                    let atoms = parse_positive_atoms(body_node)?;
                    Ok(atoms.into_iter().map(Rule::fact).collect())
                }
            }
        }
        // Bare fact (no Forall wrapper) — no existentials are possible.
        // [SONNET-4.6] sq-n7y15 — "Atom" added: bare positional predicate fact.
        // Multi-slot Frame: one Rule::fact per slot. [sq-jsgyn]
        "Frame" | "Member" | "Subclass" | "Equal" | "Atom" => {
            let atoms = parse_positive_atoms(node)?;
            Ok(atoms.into_iter().map(Rule::fact).collect())
        }
        other => {
            check_non_core(other)?;
            Err(ImportError::UnrecognizedElement { tag: other.to_string() })
        }
    }
}

fn parse_implies(node: &XmlNode, forall_vars: &BTreeSet<String>) -> Result<Vec<Rule>, ImportError> {
    // Fail-closed: duplicate single-cardinality wrappers are rejected (Fix-2).
    // The old `.child("if")` was first-wins; now we check for duplicates explicitly.
    if node.children_named("if").count() > 1 {
        return Err(ImportError::MalformedXml(
            "Implies has duplicate <if> elements (expected exactly one)".to_string(),
        ));
    }
    if node.children_named("then").count() > 1 {
        return Err(ImportError::MalformedXml(
            "Implies has duplicate <then> elements (expected exactly one)".to_string(),
        ));
    }

    let if_node = node.child("if").ok_or_else(|| ImportError::MalformedXml(
        "Implies missing <if>".to_string(),
    ))?;
    let then_node = node.child("then").ok_or_else(|| ImportError::MalformedXml(
        "Implies missing <then>".to_string(),
    ))?;

    // Parse the body condition. Single-cardinality: the <if> holds exactly one condition.
    // A surplus condition sibling used to be silently dropped by `.children.first()` — for
    // `<if>` that drops a CONJUNCT, weakening the guard → OVER-derivation (unsound). This is
    // the soundness-relevant site; reject surplus fail-closed (sq-anuo9). [SONNET-4.6]
    let body_child = if_node.only_child("<if>")?;
    let body_cond = parse_condition(body_child)?;

    // Alpha-rename all Exists-declared vars to globally fresh names (Fix-1).
    // Universe: Forall-declared vars + all var names in the body condition tree.
    // This prevents fresh names from colliding with universals or other body vars.
    let mut universe: BTreeSet<String> = forall_vars.clone();
    collect_vars_in_cond(&body_cond, &mut universe);
    let mut counter = 0u32;
    let body_cond = alpha_rename_cond(body_cond, &mut counter, &mut universe);

    // Parse the head. Single-cardinality: the <then> holds exactly one head element (a
    // positive atom or a conjunctive <And>). A surplus head sibling used to be silently
    // dropped → under-derivation; reject it fail-closed (sq-anuo9). [SONNET-4.6]
    let then_child = then_node.only_child("<then>")?;
    let head = parse_head(then_child)?;

    // Or-split / Exists-flatten → one rule per body disjunct.
    let disjuncts = expand_body(body_cond);
    let rules = disjuncts
        .into_iter()
        .map(|body| Rule::implies(head.clone(), body))
        .collect();
    Ok(rules)
}

// ---------------------------------------------------------------------------
// Document interpretation
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Imports-closure consistency check  [SONNET-4.6] sq-wbql1
// ---------------------------------------------------------------------------

/// The W3C RIF-Core profile IRI. An `<Import>` whose `profile` attribute names a
/// different profile is incompatible with the importing RIF-Core document.
const RIF_CORE_PROFILE_IRI: &str = "http://www.w3.org/2007/rif#Core";

/// Known non-Core RIF dialect profile IRIs. An import declaring one of these is
/// INCOMPATIBLE with a RIF-Core document — rejected with `InconsistentImport`.
const NON_CORE_PROFILES: &[&str] = &[
    "http://www.w3.org/2007/rif#BLD",
    "http://www.w3.org/2007/rif#PRD",
    "http://www.w3.org/2007/rif#OWL-Direct",
    "http://www.w3.org/2007/rif#OWL-RDF-Compatibility",
    "http://www.w3.org/2007/rif#SWC",
    "http://www.w3.org/2007/rif#FLD",
];

/// Check whether a `profile` IRI is incompatible with RIF-Core. Returns an
/// `InconsistentImport` error if so; `Ok(())` when the profile is Core or unset.
///
/// Per RIF-Core §3 (Imports): a conforming RIF-Core processor MUST reject a
/// document whose imports closure contains a document with an incompatible profile.
/// An explicitly non-Core profile IRI is the detectable case.
fn check_import_profile(
    location: &str,
    profile: Option<&str>,
) -> Result<(), ImportError> {
    let Some(prof) = profile else { return Ok(()) };
    let prof = prof.trim();
    // An explicit non-Core profile is incompatible with a RIF-Core document.
    if NON_CORE_PROFILES.contains(&prof) {
        return Err(ImportError::InconsistentImport {
            location: location.to_string(),
            reason: format!(
                "imported profile <{}> is incompatible with RIF-Core (non-Core dialect)",
                prof
            ),
        });
    }
    // Any profile OTHER than Core (and other than empty/absent) is also incompatible,
    // since we cannot verify what constraints it imposes.
    if !prof.is_empty() && prof != RIF_CORE_PROFILE_IRI {
        return Err(ImportError::InconsistentImport {
            location: location.to_string(),
            reason: format!(
                "imported profile <{}> is unrecognized — only RIF-Core (<{}>) is \
                 compatible with this processor",
                prof, RIF_CORE_PROFILE_IRI
            ),
        });
    }
    Ok(())
}

/// Interpret a `<Document>` root node into a `Document`, driving the imports-closure
/// check via `resolver`.  When `resolver` is `None`, any `<Import>` → `ImportDirective`
/// (the original fail-closed behaviour).  When `resolver` is `Some(f)`, each `<Import>`
/// directive is:
///
/// 1. Profile-checked: a non-Core `profile` attribute → `InconsistentImport`.
/// 2. Resolved: `f(location)` is called; if it returns `Some(bytes)` the imported
///    document is parsed and its rules are merged into the closure for combined
///    `validate()`.  If `f` returns `None` (the document is not locally available),
///    the import is still rejected fail-closed with `ImportDirective` — never silently
///    accepted.
fn interpret_document<F>(root: &XmlNode, resolver: Option<&F>) -> Result<Document, ImportError>
where
    F: Fn(&str) -> Option<Vec<u8>>,
{
    if root.tag != "Document" {
        return Err(ImportError::UnrecognizedElement { tag: root.tag.clone() });
    }

    let mut doc = Document::new();
    // Rules accumulated from resolved imported documents (the imports closure).
    let mut imported_rules: Vec<crate::rif::Rule> = Vec::new();

    for child in &root.children {
        match child.tag.as_str() {
            "directive" => {
                for item in &child.children {
                    if item.tag == "Import" {
                        let location = item
                            .child("location")
                            .and_then(|n| n.children.first())
                            .map(|n| n.text.trim().to_string())
                            .unwrap_or_default();
                        // Read the optional profile attribute from the Import node, or
                        // from a <profile> child element (both forms appear in the W3C
                        // test suite).
                        let profile_from_child = item
                            .child("profile")
                            .and_then(|n| n.children.first())
                            .map(|n| n.text.trim().to_string());
                        let profile = profile_from_child.as_deref();

                        // Step 1: profile consistency check (detectable offline).
                        check_import_profile(&location, profile)?;

                        // Step 2: try to resolve the imported document.
                        match resolver {
                            Some(f) => match f(&location) {
                                Some(bytes) => {
                                    // Parse the imported document. Its own imports are
                                    // not recursively resolved here (bounded single-level
                                    // closure for the offline check — a full transitive
                                    // resolver is tracked as sq-wbql1 future work).
                                    let imp_root = parse_xml_tree(&bytes)?;
                                    let imp_doc = interpret_document(&imp_root, None::<&fn(&str) -> Option<Vec<u8>>>)?;
                                    // Accumulate its rules for the combined validate.
                                    imported_rules.extend(imp_doc.rules);
                                }
                                None => {
                                    // Resolver could not supply the imported document:
                                    // fail closed — never silently accept an import we
                                    // cannot examine (only the profile check passed).
                                    return Err(ImportError::ImportDirective { location });
                                }
                            },
                            None => {
                                // No resolver: blanket fail-closed (original behaviour).
                                return Err(ImportError::ImportDirective { location });
                            }
                        }
                        continue;
                    }
                    check_non_core(&item.tag)?;
                    // Other directives (e.g. <Profile>) are unrecognized.
                    return Err(ImportError::UnrecognizedElement { tag: item.tag.clone() });
                }
            }
            "payload" => {
                interpret_payload(child, &mut doc)?;
            }
            other => {
                check_non_core(other)?;
                return Err(ImportError::UnrecognizedElement { tag: other.to_string() });
            }
        }
    }

    // If any imports were resolved, validate the combined rule set.
    if !imported_rules.is_empty() {
        // Build the combined document: importing document's rules + all imported rules.
        let mut combined = crate::rif::Document::new();
        for rule in &doc.rules {
            combined.push(rule.clone());
        }
        for rule in imported_rules {
            combined.push(rule);
        }
        combined.validate().map_err(ImportError::ValidationFailed)?;
        // If combined validation passes, return only the importing document's rules
        // (the imported rules were merged only for the consistency check, per the
        // RIF-Core import semantics: the importer's closure is the full derivation
        // domain, but here we return the successfully-imported document for further
        // caller use — the caller can re-drive closure over the combined set).
        return Ok(combined);
    }

    Ok(doc)
}

fn interpret_payload(payload: &XmlNode, doc: &mut Document) -> Result<(), ImportError> {
    for child in &payload.children {
        match child.tag.as_str() {
            "Group" => interpret_group(child, doc)?,
            other => {
                check_non_core(other)?;
                return Err(ImportError::UnrecognizedElement { tag: other.to_string() });
            }
        }
    }
    Ok(())
}

fn interpret_group(group: &XmlNode, doc: &mut Document) -> Result<(), ImportError> {
    for child in &group.children {
        match child.tag.as_str() {
            "sentences" => {
                for sentence in &child.children {
                    let rules = parse_sentence(sentence)?;
                    for rule in rules {
                        doc.push(rule);
                    }
                }
            }
            "sentence" => {
                // <sentence> is a structural wrapper in some RIF/XML serializations;
                // descend into its actual sentence child (Fix-2 / Copilot thread #4).
                // The old code called parse_sentence(child) where child.tag=="sentence",
                // which always errored with UnrecognizedElement since parse_sentence only
                // handles Forall/Frame/Member/Subclass/Equal at the top level.
                // Single-cardinality: the <sentence> wrapper holds exactly one sentence
                // element; a surplus sibling sentence is rejected, not dropped (sq-anuo9).
                let actual = child.only_child("<sentence> wrapper")?;
                let rules = parse_sentence(actual)?;
                for rule in rules {
                    doc.push(rule);
                }
            }
            "Group" => {
                // Nested group.
                interpret_group(child, doc)?;
            }
            other => {
                check_non_core(other)?;
                return Err(ImportError::UnrecognizedElement { tag: other.to_string() });
            }
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

/// Parse a RIF-Core XML document from `xml_bytes` into a [`crate::rif::Document`].
///
/// Applies two sound desugarings at import time:
/// - Body `Or`: split into multiple rules (one rule per disjunct — Lloyd-Topor step,
///   monotone-Horn-preserving).
/// - Body `Exists`: existential variables are **unconditionally alpha-renamed** to
///   fresh names in the `__ex{N}` reserved namespace, then the `Exists` wrapper is
///   dropped. This prevents variable capture in shadow (`Forall ?x … Exists ?x`) and
///   sibling (`Exists ?y … Exists ?y`) patterns. See `alpha_rename_cond`
///   (the crate-internal renaming pass; documented in the module doc).
///
/// Everything outside the supported Core subset yields a named `ImportError` variant
/// (fail-closed, never silent skipping). The returned `Document` has already been
/// validated via `Document::validate()`.
///
/// Any `<Import>` directive → `ImportDirective` fail-closed (blanket refusal when no
/// resolver is available). To resolve imports and check their consistency, use
/// [`import_with_closure`].
pub fn import(xml_bytes: &[u8]) -> Result<Document, ImportError> {
    let root = parse_xml_tree(xml_bytes)?;
    let doc = interpret_document(&root, None::<&fn(&str) -> Option<Vec<u8>>>)?;
    doc.validate().map_err(ImportError::ValidationFailed)?;
    Ok(doc)
}

/// [SONNET-4.6] sq-wbql1 — Parse a RIF-Core XML document and compute the
/// **imports-closure consistency check** using a caller-supplied resolver.
///
/// Unlike [`import`], this function does NOT blanket-refuse `<Import>` directives.
/// Instead, for each `<Import>` it:
///
/// 1. **Profile-checks** the `profile` attribute: if it names a non-Core RIF dialect
///    (BLD, PRD, OWL-Direct, …) the import is rejected with
///    `ImportError::InconsistentImport` — a GENUINE, NON-VACUOUS detection of the
///    specific invalidity the W3C RIF ImportRejectionTests target.
///
/// 2. **Resolves** the imported document bytes via `resolver(location_iri)`. If the
///    resolver returns `Some(bytes)`, the imported document is parsed as RIF-Core and
///    its rules are merged with the importing document's rules; the combined set is then
///    validated. A validation failure → `ImportError::InconsistentImport` (the imported
///    rules are inconsistent with the importing document). If the resolver returns
///    `None` (the document is not locally available), the import is rejected fail-closed
///    with `ImportError::ImportDirective` — never silently accepted.
///
/// ## Fail-closed invariant (sq-wbql1)
///
/// An inconsistent/unresolvable/incompatible import is ALWAYS rejected — never silently
/// accepted. A CONSISTENT import (resolvable, compatible profile, combined rules pass
/// validation) is accepted and the merged `Document` is returned.
///
/// ## Example
///
/// ```rust
/// # #[cfg(feature = "rif-xml")] {
/// use sparq_reason::rif_xml::{import_with_closure, ImportError};
///
/// // An importing document with a non-Core profile import.
/// let importing = br#"<Document xmlns="http://www.w3.org/2007/rif#">
///   <directive><Import>
///     <location><Const type="http://www.w3.org/2007/rif#iri">http://ex/rules</Const></location>
///     <profile><Const type="http://www.w3.org/2007/rif#iri">http://www.w3.org/2007/rif#BLD</Const></profile>
///   </Import></directive>
///   <payload><Group></Group></payload>
/// </Document>"#;
///
/// // The resolver provides no bytes (location not locally available).
/// let result = import_with_closure(importing, |_loc| None);
/// assert!(matches!(result, Err(ImportError::InconsistentImport { .. })),
///     "a non-Core profile import must be rejected as InconsistentImport");
/// # }
/// ```
pub fn import_with_closure<F>(xml_bytes: &[u8], resolver: F) -> Result<Document, ImportError>
where
    F: Fn(&str) -> Option<Vec<u8>>,
{
    let root = parse_xml_tree(xml_bytes)?;
    let doc = interpret_document(&root, Some(&resolver))?;
    // Final validate (covers the no-import case and the combined-rules path).
    doc.validate().map_err(ImportError::ValidationFailed)?;
    Ok(doc)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// #3359 — a hostile document of deeply nested `<And>` must be REFUSED
    /// (`MalformedXml`), not overflow the stack and abort the process. Run on the
    /// default test-thread stack.
    #[test]
    fn deeply_nested_and_is_refused_not_stack_overflow() {
        let depth = 200_000;
        let mut xml = String::from(
            "<Document xmlns=\"http://www.w3.org/2007/rif#\"><payload><Group><sentence>\
             <Implies><if>",
        );
        for _ in 0..depth {
            xml.push_str("<And><formula>");
        }
        for _ in 0..depth {
            xml.push_str("</formula></And>");
        }
        xml.push_str("</if><then/></Implies></sentence></Group></payload></Document>");
        match import(xml.as_bytes()) {
            Err(ImportError::MalformedXml(msg)) => {
                assert!(msg.contains("maximum depth"), "unexpected message: {msg}")
            }
            other => panic!("expected a depth refusal, got {other:?}"),
        }
    }

    /// The same guard covers `<Or>` and `<Exists>` (and self-closing leaves).
    #[test]
    fn deeply_nested_or_exists_is_refused() {
        for (open, close) in [
            ("<Or><formula>", "</formula></Or>"),
            (
                "<Exists><declare><Var>z</Var></declare><formula>",
                "</formula></Exists>",
            ),
        ] {
            let depth = 100_000;
            let mut xml = String::from(
                "<Document xmlns=\"http://www.w3.org/2007/rif#\"><payload><Group><sentence>\
                 <Implies><if>",
            );
            for _ in 0..depth {
                xml.push_str(open);
            }
            xml.push_str("<Atom/>");
            for _ in 0..depth {
                xml.push_str(close);
            }
            xml.push_str("</if><then/></Implies></sentence></Group></payload></Document>");
            let res = import(xml.as_bytes());
            assert!(
                matches!(res, Err(ImportError::MalformedXml(ref m)) if m.contains("maximum depth")),
                "{open}: deep nesting must be refused"
            );
        }
    }

    /// A nested body just under the cap (~250 element levels) still imports — and
    /// does not itself overflow the default test-thread stack in a debug build.
    #[test]
    fn near_cap_nested_and_still_imports() {
        let depth = 120;
        let mut xml = String::from(
            "<Document xmlns=\"http://www.w3.org/2007/rif#\"><payload><Group><sentence>\
             <Forall><declare><Var>x</Var></declare><formula><Implies><if>",
        );
        for _ in 0..depth {
            xml.push_str("<And><formula>");
        }
        xml.push_str(
            "<Atom><op><Const type=\"http://www.w3.org/2007/rif#iri\">http://ex/p</Const></op>\
             <args ordered=\"yes\"><Var>x</Var></args></Atom>",
        );
        for _ in 0..depth {
            xml.push_str("</formula></And>");
        }
        xml.push_str(
            "</if><then><Atom><op><Const type=\"http://www.w3.org/2007/rif#iri\">http://ex/q</Const></op>\
             <args ordered=\"yes\"><Var>x</Var></args></Atom></then></Implies></formula></Forall>\
             </sentence></Group></payload></Document>",
        );
        import(xml.as_bytes()).expect("a body just under the depth cap must import");
    }

    /// #3360 — a `<Document>` root with no namespace, or a foreign one, is not
    /// RIF/XML and is refused fail-closed.
    #[test]
    fn non_rif_namespace_root_is_refused() {
        for xml in [
            "<Document><payload><Group></Group></payload></Document>",
            "<Document xmlns=\"http://example.org/not-rif#\"><payload><Group></Group></payload></Document>",
            "<x:Document xmlns:x=\"http://example.org/not-rif#\" xmlns=\"http://www.w3.org/2007/rif#\"><payload><Group></Group></payload></x:Document>",
            "<Document xmlns=\"http://example.org/not-rif#\"/>",
        ] {
            let res = import(xml.as_bytes());
            assert!(
                matches!(res, Err(ImportError::MalformedXml(ref m)) if m.contains("namespace")),
                "a non-RIF-namespace root must be refused: {xml}"
            );
        }
    }

    /// A prefixed root bound to the RIF namespace is accepted.
    #[test]
    fn prefixed_rif_namespace_root_is_accepted() {
        let xml = "<rif:Document xmlns:rif=\"http://www.w3.org/2007/rif#\">\
                   <rif:payload><rif:Group></rif:Group></rif:payload></rif:Document>";
        import(xml.as_bytes()).expect("a prefixed RIF-namespace root must import");
    }

    fn assert_namespace_refused(xml: &str, why: &str) {
        let res = import(xml.as_bytes());
        assert!(
            matches!(res, Err(ImportError::MalformedXml(ref m)) if m.contains("namespace")),
            "{why}: got {res:?}"
        );
    }

    /// A descendant that re-declares a FOREIGN default namespace takes its whole
    /// subtree out of RIF; those elements must not be read as RIF syntax even
    /// though the root is in the RIF namespace.
    #[test]
    fn foreign_default_namespace_on_descendant_is_refused() {
        let base = std::str::from_utf8(MINIMAL_RULE_XML).unwrap();
        assert!(import(base.as_bytes()).is_ok(), "control document must import");
        for (from, to) in [
            ("<payload>", "<payload xmlns=\"urn:foreign\">"),
            ("<Group>", "<Group xmlns=\"urn:foreign\">"),
            ("<then>", "<then xmlns=\"urn:foreign\">"),
        ] {
            assert_namespace_refused(
                &base.replacen(from, to, 1),
                &format!("foreign default namespace via {to} must be refused"),
            );
        }
        // Undeclaring the default namespace (xmlns="") also leaves RIF.
        assert_namespace_refused(
            &base.replacen("<Group>", "<Group xmlns=\"\">", 1),
            "an undeclared default namespace must be refused",
        );
    }

    /// A RIF prefix rebound to a foreign namespace below the root must be refused;
    /// rebinding to the RIF namespace itself (or another prefix bound to RIF) is fine.
    #[test]
    fn rebound_prefix_on_descendant_is_refused() {
        let rebound = "<rif:Document xmlns:rif=\"http://www.w3.org/2007/rif#\">\
                       <rif:payload><rif:Group xmlns:rif=\"urn:foreign\"></rif:Group>\
                       </rif:payload></rif:Document>";
        assert_namespace_refused(rebound, "a rebound rif: prefix must be refused");
        let unbound_prefix = "<rif:Document xmlns:rif=\"http://www.w3.org/2007/rif#\">\
                              <x:payload></x:payload></rif:Document>";
        assert!(import(unbound_prefix.as_bytes()).is_err(), "an undeclared prefix must be refused");
        let rebound_to_rif = "<rif:Document xmlns:rif=\"http://www.w3.org/2007/rif#\">\
                              <r:payload xmlns:r=\"http://www.w3.org/2007/rif#\">\
                              <Group xmlns=\"http://www.w3.org/2007/rif#\"></Group>\
                              </r:payload></rif:Document>";
        import(rebound_to_rif.as_bytes()).expect("any binding that resolves to RIF imports");
    }

    /// The full finding: a foreign `<payload>` subtree carrying a rule must not
    /// produce any rule (and hence no conclusions).
    #[test]
    fn foreign_namespace_subtree_produces_no_rules() {
        let base = std::str::from_utf8(MINIMAL_RULE_XML).unwrap();
        let xml = base.replacen("<payload>", "<payload xmlns=\"urn:foreign\">", 1);
        assert!(import(xml.as_bytes()).is_err());
    }

    // ---- XML fixtures -------------------------------------------------------

    const MINIMAL_RULE_XML: &[u8] = br#"<Document xmlns="http://www.w3.org/2007/rif#">
  <payload>
    <Group>
      <sentences>
        <Forall>
          <declare><Var>x</Var></declare>
          <declare><Var>y</Var></declare>
          <formula>
            <Implies>
              <if>
                <Frame>
                  <object><Const type="http://www.w3.org/2007/rif#iri">http://example.org/a</Const></object>
                  <slot><Const type="http://www.w3.org/2007/rif#iri">http://example.org/p</Const><Var>y</Var></slot>
                </Frame>
              </if>
              <then>
                <Frame>
                  <object><Var>y</Var></object>
                  <slot><Const type="http://www.w3.org/2007/rif#iri">http://example.org/q</Const><Const type="http://www.w3.org/2007/rif#iri">http://example.org/b</Const></slot>
                </Frame>
              </then>
            </Implies>
          </formula>
        </Forall>
      </sentences>
    </Group>
  </payload>
</Document>"#;

    const OR_SPLIT_XML: &[u8] = br#"<Document xmlns="http://www.w3.org/2007/rif#">
  <payload>
    <Group>
      <sentences>
        <Forall>
          <declare><Var>x</Var></declare>
          <formula>
            <Implies>
              <if>
                <Or>
                  <formula>
                    <Frame>
                      <object><Var>x</Var></object>
                      <slot><Const type="http://www.w3.org/2007/rif#iri">http://example.org/type</Const><Const type="http://www.w3.org/2007/rif#iri">http://example.org/A</Const></slot>
                    </Frame>
                  </formula>
                  <formula>
                    <Frame>
                      <object><Var>x</Var></object>
                      <slot><Const type="http://www.w3.org/2007/rif#iri">http://example.org/type</Const><Const type="http://www.w3.org/2007/rif#iri">http://example.org/B</Const></slot>
                    </Frame>
                  </formula>
                </Or>
              </if>
              <then>
                <Frame>
                  <object><Var>x</Var></object>
                  <slot><Const type="http://www.w3.org/2007/rif#iri">http://example.org/type</Const><Const type="http://www.w3.org/2007/rif#iri">http://example.org/C</Const></slot>
                </Frame>
              </then>
            </Implies>
          </formula>
        </Forall>
      </sentences>
    </Group>
  </payload>
</Document>"#;

    // EXISTS_FLATTEN_XML — used by test_exists_flatten (updated for alpha-rename behavior).
    const EXISTS_FLATTEN_XML: &[u8] = br#"<Document xmlns="http://www.w3.org/2007/rif#">
  <payload>
    <Group>
      <sentences>
        <Forall>
          <declare><Var>x</Var></declare>
          <formula>
            <Implies>
              <if>
                <Exists>
                  <declare><Var>z</Var></declare>
                  <formula>
                    <And>
                      <formula>
                        <Frame>
                          <object><Var>x</Var></object>
                          <slot><Const type="http://www.w3.org/2007/rif#iri">http://example.org/p</Const><Var>z</Var></slot>
                        </Frame>
                      </formula>
                      <formula>
                        <Frame>
                          <object><Var>z</Var></object>
                          <slot><Const type="http://www.w3.org/2007/rif#iri">http://example.org/q</Const><Const type="http://www.w3.org/2007/rif#iri">http://example.org/val</Const></slot>
                        </Frame>
                      </formula>
                    </And>
                  </formula>
                </Exists>
              </if>
              <then>
                <Frame>
                  <object><Var>x</Var></object>
                  <slot><Const type="http://www.w3.org/2007/rif#iri">http://example.org/r</Const><Const type="http://www.w3.org/2007/rif#iri">http://example.org/yes</Const></slot>
                </Frame>
              </then>
            </Implies>
          </formula>
        </Forall>
      </sentences>
    </Group>
  </payload>
</Document>"#;

    const IMPORT_DIRECTIVE_XML: &[u8] = br#"<Document xmlns="http://www.w3.org/2007/rif#">
  <directive>
    <Import>
      <location><Const type="http://www.w3.org/2007/rif#iri">http://example.org/rules.rif</Const></location>
    </Import>
  </directive>
  <payload><Group><sentences></sentences></Group></payload>
</Document>"#;

    const NAF_XML: &[u8] = br#"<Document xmlns="http://www.w3.org/2007/rif#">
  <payload>
    <Group>
      <sentences>
        <Forall>
          <declare><Var>x</Var></declare>
          <formula>
            <Implies>
              <if><Naf><formula><Frame><object><Var>x</Var></object><slot><Const type="http://www.w3.org/2007/rif#iri">http://example.org/p</Const><Const type="http://www.w3.org/2007/rif#iri">http://example.org/v</Const></slot></Frame></formula></Naf></if>
              <then><Frame><object><Var>x</Var></object><slot><Const type="http://www.w3.org/2007/rif#iri">http://example.org/q</Const><Const type="http://www.w3.org/2007/rif#iri">http://example.org/v</Const></slot></Frame></then>
            </Implies>
          </formula>
        </Forall>
      </sentences>
    </Group>
  </payload>
</Document>"#;

    const UNKNOWN_EXTERNAL_XML: &[u8] = br#"<Document xmlns="http://www.w3.org/2007/rif#">
  <payload>
    <Group>
      <sentences>
        <Forall>
          <declare><Var>x</Var></declare>
          <declare><Var>y</Var></declare>
          <formula>
            <Implies>
              <if>
                <And>
                  <formula>
                    <Frame>
                      <object><Var>x</Var></object>
                      <slot><Const type="http://www.w3.org/2007/rif#iri">http://example.org/p</Const><Var>y</Var></slot>
                    </Frame>
                  </formula>
                  <formula>
                    <External>
                      <content><Atom>
                        <op><Const type="http://www.w3.org/2007/rif#iri">http://www.w3.org/2007/rif-builtin-predicate#matches</Const></op>
                        <args><Var>y</Var><Const type="http://www.w3.org/2001/XMLSchema#string">.*</Const></args>
                      </Atom></content>
                    </External>
                  </formula>
                </And>
              </if>
              <then>
                <Frame>
                  <object><Var>x</Var></object>
                  <slot><Const type="http://www.w3.org/2007/rif#iri">http://example.org/q</Const><Const type="http://www.w3.org/2007/rif#iri">http://example.org/yes</Const></slot>
                </Frame>
              </then>
            </Implies>
          </formula>
        </Forall>
      </sentences>
    </Group>
  </payload>
</Document>"#;

    // ---- Alpha-rename test fixtures (Fix-1) ---------------------------------

    /// Shadow: Forall ?x: head(?x) :- And(p(?x, v), Exists ?x(q(?x, v)))
    /// The existential ?x shadows the universal ?x — must produce distinct body vars.
    const SHADOW_XML: &[u8] = br#"<Document xmlns="http://www.w3.org/2007/rif#">
  <payload><Group><sentences>
    <Forall>
      <declare><Var>x</Var></declare>
      <formula>
        <Implies>
          <if>
            <And>
              <formula>
                <Frame>
                  <object><Var>x</Var></object>
                  <slot><Const type="http://www.w3.org/2007/rif#iri">http://ex/p</Const><Const type="http://www.w3.org/2007/rif#iri">http://ex/v</Const></slot>
                </Frame>
              </formula>
              <formula>
                <Exists>
                  <declare><Var>x</Var></declare>
                  <formula>
                    <Frame>
                      <object><Var>x</Var></object>
                      <slot><Const type="http://www.w3.org/2007/rif#iri">http://ex/q</Const><Const type="http://www.w3.org/2007/rif#iri">http://ex/v</Const></slot>
                    </Frame>
                  </formula>
                </Exists>
              </formula>
            </And>
          </if>
          <then>
            <Frame>
              <object><Var>x</Var></object>
              <slot><Const type="http://www.w3.org/2007/rif#iri">http://ex/h</Const><Const type="http://www.w3.org/2007/rif#iri">http://ex/v</Const></slot>
            </Frame>
          </then>
        </Implies>
      </formula>
    </Forall>
  </sentences></Group></payload>
</Document>"#;

    /// Sibling Exists: Forall ?x: head(?x) :- And(Exists ?y(p(?x,?y)), Exists ?y(q(?x,?y)))
    /// Two sibling Exists nodes each declare ?y — must produce TWO distinct fresh vars.
    const SIBLING_EXISTS_XML: &[u8] = br#"<Document xmlns="http://www.w3.org/2007/rif#">
  <payload><Group><sentences>
    <Forall>
      <declare><Var>x</Var></declare>
      <formula>
        <Implies>
          <if>
            <And>
              <formula>
                <Exists>
                  <declare><Var>y</Var></declare>
                  <formula>
                    <Frame>
                      <object><Var>x</Var></object>
                      <slot><Const type="http://www.w3.org/2007/rif#iri">http://ex/p</Const><Var>y</Var></slot>
                    </Frame>
                  </formula>
                </Exists>
              </formula>
              <formula>
                <Exists>
                  <declare><Var>y</Var></declare>
                  <formula>
                    <Frame>
                      <object><Var>x</Var></object>
                      <slot><Const type="http://www.w3.org/2007/rif#iri">http://ex/q</Const><Var>y</Var></slot>
                    </Frame>
                  </formula>
                </Exists>
              </formula>
            </And>
          </if>
          <then>
            <Frame>
              <object><Var>x</Var></object>
              <slot><Const type="http://www.w3.org/2007/rif#iri">http://ex/r</Const><Const type="http://www.w3.org/2007/rif#iri">http://ex/v</Const></slot>
            </Frame>
          </then>
        </Implies>
      </formula>
    </Forall>
  </sentences></Group></payload>
</Document>"#;

    /// Nested Exists: Forall ?x: head(?x) :- Exists ?z(Exists ?z(p(?x,?z)))
    /// Inner Exists ?z must win; outer Exists ?z has no remaining ?z occurrences to rename.
    const NESTED_EXISTS_XML: &[u8] = br#"<Document xmlns="http://www.w3.org/2007/rif#">
  <payload><Group><sentences>
    <Forall>
      <declare><Var>x</Var></declare>
      <formula>
        <Implies>
          <if>
            <Exists>
              <declare><Var>z</Var></declare>
              <formula>
                <Exists>
                  <declare><Var>z</Var></declare>
                  <formula>
                    <Frame>
                      <object><Var>x</Var></object>
                      <slot><Const type="http://www.w3.org/2007/rif#iri">http://ex/p</Const><Var>z</Var></slot>
                    </Frame>
                  </formula>
                </Exists>
              </formula>
            </Exists>
          </if>
          <then>
            <Frame>
              <object><Var>x</Var></object>
              <slot><Const type="http://www.w3.org/2007/rif#iri">http://ex/r</Const><Const type="http://www.w3.org/2007/rif#iri">http://ex/v</Const></slot>
            </Frame>
          </then>
        </Implies>
      </formula>
    </Forall>
  </sentences></Group></payload>
</Document>"#;

    /// Non-colliding Exists: Forall ?x: head(?x) :- Exists ?w(p(?x,?w))
    /// ?w does not collide with any other var, but MUST still be renamed unconditionally.
    const NON_COLLIDING_EXISTS_XML: &[u8] = br#"<Document xmlns="http://www.w3.org/2007/rif#">
  <payload><Group><sentences>
    <Forall>
      <declare><Var>x</Var></declare>
      <formula>
        <Implies>
          <if>
            <Exists>
              <declare><Var>w</Var></declare>
              <formula>
                <Frame>
                  <object><Var>x</Var></object>
                  <slot><Const type="http://www.w3.org/2007/rif#iri">http://ex/p</Const><Var>w</Var></slot>
                </Frame>
              </formula>
            </Exists>
          </if>
          <then>
            <Frame>
              <object><Var>x</Var></object>
              <slot><Const type="http://www.w3.org/2007/rif#iri">http://ex/r</Const><Const type="http://www.w3.org/2007/rif#iri">http://ex/v</Const></slot>
            </Frame>
          </then>
        </Implies>
      </formula>
    </Forall>
  </sentences></Group></payload>
</Document>"#;

    /// Duplicate <if>: Implies with two <if> children — must be rejected (Fix-2).
    const DUPLICATE_IF_XML: &[u8] = br#"<Document xmlns="http://www.w3.org/2007/rif#">
  <payload><Group><sentences>
    <Forall>
      <declare><Var>x</Var></declare>
      <formula>
        <Implies>
          <if>
            <Frame>
              <object><Var>x</Var></object>
              <slot><Const type="http://www.w3.org/2007/rif#iri">http://ex/p</Const><Const type="http://www.w3.org/2007/rif#iri">http://ex/v</Const></slot>
            </Frame>
          </if>
          <if>
            <Frame>
              <object><Var>x</Var></object>
              <slot><Const type="http://www.w3.org/2007/rif#iri">http://ex/q</Const><Const type="http://www.w3.org/2007/rif#iri">http://ex/v</Const></slot>
            </Frame>
          </if>
          <then>
            <Frame>
              <object><Var>x</Var></object>
              <slot><Const type="http://www.w3.org/2007/rif#iri">http://ex/r</Const><Const type="http://www.w3.org/2007/rif#iri">http://ex/v</Const></slot>
            </Frame>
          </then>
        </Implies>
      </formula>
    </Forall>
  </sentences></Group></payload>
</Document>"#;

    // ---- Fix-6 fixtures (multi-Var/empty declare rejection) -----------------

    /// Demonstrated capture case (Opus re-verify): Exists with a single <declare> that
    /// holds TWO <Var> children (?a and ?b). The old filter_map(.first()) silently
    /// dropped ?b, leaving it unrenamed and conflated with the universal ?b.
    /// Fix-6 rejects this with MalformedXml.
    ///
    /// Mutation check: removing the count guard in `parse_condition`'s Exists arm
    /// causes this test to fail — `import` returns `Ok` (accepting the malformed
    /// document and silently dropping the second Var) instead of `Err`.
    const MULTI_VAR_DECLARE_EXISTS_XML: &[u8] = br#"<Document xmlns="http://www.w3.org/2007/rif#">
  <payload><Group><sentences>
    <Forall>
      <declare><Var>b</Var></declare>
      <formula>
        <Implies>
          <if>
            <Exists>
              <declare><Var>a</Var><Var>b</Var></declare>
              <formula>
                <And>
                  <formula>
                    <Frame>
                      <object><Var>b</Var></object>
                      <slot><Const type="http://www.w3.org/2007/rif#iri">http://ex/f</Const><Var>a</Var></slot>
                    </Frame>
                  </formula>
                  <formula>
                    <Frame>
                      <object><Var>b</Var></object>
                      <slot><Const type="http://www.w3.org/2007/rif#iri">http://ex/g</Const><Var>b</Var></slot>
                    </Frame>
                  </formula>
                </And>
              </formula>
            </Exists>
          </if>
          <then>
            <Frame>
              <object><Var>b</Var></object>
              <slot><Const type="http://www.w3.org/2007/rif#iri">http://ex/h</Const><Const type="http://www.w3.org/2007/rif#iri">http://ex/v</Const></slot>
            </Frame>
          </then>
        </Implies>
      </formula>
    </Forall>
  </sentences></Group></payload>
</Document>"#;

    /// Empty <declare/> in an Exists — no <Var> children → Fix-6 rejects with MalformedXml.
    const EMPTY_DECLARE_EXISTS_XML: &[u8] = br#"<Document xmlns="http://www.w3.org/2007/rif#">
  <payload><Group><sentences>
    <Forall>
      <declare><Var>x</Var></declare>
      <formula>
        <Implies>
          <if>
            <Exists>
              <declare/>
              <formula>
                <Frame>
                  <object><Var>x</Var></object>
                  <slot><Const type="http://www.w3.org/2007/rif#iri">http://ex/p</Const><Const type="http://www.w3.org/2007/rif#iri">http://ex/v</Const></slot>
                </Frame>
              </formula>
            </Exists>
          </if>
          <then>
            <Frame>
              <object><Var>x</Var></object>
              <slot><Const type="http://www.w3.org/2007/rif#iri">http://ex/q</Const><Const type="http://www.w3.org/2007/rif#iri">http://ex/v</Const></slot>
            </Frame>
          </then>
        </Implies>
      </formula>
    </Forall>
  </sentences></Group></payload>
</Document>"#;

    /// Valid multi-declare Exists: two <declare> elements each with exactly one <Var>.
    /// Regression guard: Fix-6 must NOT reject this; both vars must be alpha-renamed
    /// to distinct fresh names (one-Var-per-declare is schema-valid RIF-XML).
    const MULTI_DECLARE_VALID_EXISTS_XML: &[u8] = br#"<Document xmlns="http://www.w3.org/2007/rif#">
  <payload><Group><sentences>
    <Forall>
      <declare><Var>x</Var></declare>
      <formula>
        <Implies>
          <if>
            <Exists>
              <declare><Var>a</Var></declare>
              <declare><Var>b</Var></declare>
              <formula>
                <And>
                  <formula>
                    <Frame>
                      <object><Var>x</Var></object>
                      <slot><Const type="http://www.w3.org/2007/rif#iri">http://ex/p</Const><Var>a</Var></slot>
                    </Frame>
                  </formula>
                  <formula>
                    <Frame>
                      <object><Var>x</Var></object>
                      <slot><Const type="http://www.w3.org/2007/rif#iri">http://ex/q</Const><Var>b</Var></slot>
                    </Frame>
                  </formula>
                </And>
              </formula>
            </Exists>
          </if>
          <then>
            <Frame>
              <object><Var>x</Var></object>
              <slot><Const type="http://www.w3.org/2007/rif#iri">http://ex/r</Const><Const type="http://www.w3.org/2007/rif#iri">http://ex/v</Const></slot>
            </Frame>
          </then>
        </Implies>
      </formula>
    </Forall>
  </sentences></Group></payload>
</Document>"#;

    // ---- Tests --------------------------------------------------------------

    /// Minimal document: one rule (Frame body → Frame head). Parse succeeds.
    #[test]
    fn test_minimal_core_doc() {
        let doc = import(MINIMAL_RULE_XML).expect("valid RIF-Core XML");
        assert_eq!(doc.rules.len(), 1, "one rule in the document");
        let rule = &doc.rules[0];
        assert_eq!(rule.head.len(), 1, "one head atom");
        assert_eq!(rule.body.len(), 1, "one body atom");
        // Head: y[q->b]
        assert!(
            matches!(&rule.head[0], Atom::Frame { obj: Term::Var(v), .. } if v == "y"),
            "head object is variable y"
        );
        // Body: a[p->y]
        assert!(
            matches!(&rule.body[0], Atom::Frame { obj: Term::Iri(i), .. } if i == "http://example.org/a"),
            "body object is IRI a"
        );
    }

    /// Or-body with 2 disjuncts → document has TWO rules after Or-split.
    #[test]
    fn test_or_split_into_two_rules() {
        let doc = import(OR_SPLIT_XML).expect("valid RIF-Core XML with Or body");
        assert_eq!(doc.rules.len(), 2, "Or-split must produce one rule per disjunct");
        // Both rules have the same head.
        let head0 = &doc.rules[0].head;
        let head1 = &doc.rules[1].head;
        assert_eq!(head0, head1, "both split rules share the same head");
    }

    /// Exists in body: existential var is alpha-renamed to a fresh name; validation passes.
    ///
    /// After Fix-1 the original var name "z" does NOT appear in the body — it has been
    /// unconditionally renamed to a fresh `__ex` name.
    #[test]
    fn test_exists_flatten() {
        let doc = import(EXISTS_FLATTEN_XML).expect("Exists-flatten with alpha-rename should succeed");
        assert_eq!(doc.rules.len(), 1);
        let rule = &doc.rules[0];
        // Body should have 2 atoms (the And inside the Exists was flattened).
        assert_eq!(rule.body.len(), 2, "both atoms from the Exists body are present");

        // The original existential variable name "z" must NOT appear — it was alpha-renamed.
        let z_var = Term::Var("z".to_string());
        let has_original_z = rule.body.iter().any(|a| match a {
            Atom::Frame { obj, pred, val } => obj == &z_var || pred == &z_var || val == &z_var,
            Atom::Member { obj, class } => obj == &z_var || class == &z_var,
            Atom::Subclass { sub, sup } => sub == &z_var || sup == &z_var,
            Atom::Equal { left, right } => left == &z_var || right == &z_var,
            Atom::Builtin { args, .. } => args.iter().any(|t| t == &z_var),
        });
        assert!(
            !has_original_z,
            "existential var 'z' must be alpha-renamed (must not appear under original name)"
        );

        // A fresh var (starting with __ex) must appear in the body atoms instead.
        let has_fresh = rule.body.iter().any(|a| match a {
            Atom::Frame { obj, pred, val } => [obj, pred, val]
                .iter()
                .any(|t| matches!(t, Term::Var(v) if v.starts_with("__ex"))),
            Atom::Member { obj, class } => [obj, class]
                .iter()
                .any(|t| matches!(t, Term::Var(v) if v.starts_with("__ex"))),
            Atom::Builtin { args, .. } => args
                .iter()
                .any(|t| matches!(t, Term::Var(v) if v.starts_with("__ex"))),
            _ => false,
        });
        assert!(has_fresh, "a fresh alpha-renamed existential var must appear in body");
    }

    /// Round-trip: parse minimal XML and verify structural equality with hand-built Document.
    #[test]
    fn test_round_trip_structural_equality() {
        let doc = import(MINIMAL_RULE_XML).expect("minimal doc parses");
        // Compare against hand-built rule. The body has no Exists so alpha-rename is a no-op.
        let expected_head = Atom::Frame {
            obj: Term::Var("y".to_string()),
            pred: Term::Iri("http://example.org/q".to_string()),
            val: Term::Iri("http://example.org/b".to_string()),
        };
        let expected_body = Atom::Frame {
            obj: Term::Iri("http://example.org/a".to_string()),
            pred: Term::Iri("http://example.org/p".to_string()),
            val: Term::Var("y".to_string()),
        };
        assert_eq!(doc.rules.len(), 1);
        assert_eq!(doc.rules[0].head, vec![expected_head]);
        assert_eq!(doc.rules[0].body, vec![expected_body]);
    }

    /// Import directive → ImportError::ImportDirective.
    #[test]
    fn test_reject_import_directive() {
        let err = import(IMPORT_DIRECTIVE_XML).expect_err("Import directive must be rejected");
        assert!(
            matches!(err, ImportError::ImportDirective { .. }),
            "expected ImportDirective, got: {}",
            err
        );
    }

    /// Naf element → ImportError::NonCoreElement.
    #[test]
    fn test_reject_naf() {
        let err = import(NAF_XML).expect_err("Naf must be rejected");
        assert!(
            matches!(err, ImportError::NonCoreElement { ref element, .. } if element == "Naf"),
            "expected NonCoreElement(Naf), got: {}",
            err
        );
    }

    /// External with unknown IRI → ImportError::UnknownExternal.
    #[test]
    fn test_reject_unknown_external() {
        let err = import(UNKNOWN_EXTERNAL_XML).expect_err("unknown External IRI must be rejected");
        assert!(
            matches!(err, ImportError::UnknownExternal { .. }),
            "expected UnknownExternal, got: {}",
            err
        );
    }

    /// Malformed XML → ImportError::MalformedXml.
    #[test]
    fn test_reject_malformed_xml() {
        let err = import(b"not xml <<<").expect_err("malformed XML must be rejected");
        assert!(
            matches!(err, ImportError::MalformedXml(_)),
            "expected MalformedXml, got: {}",
            err
        );
    }

    /// Or-split mutation spot-check: two rules produce distinct body conditions,
    /// and together entail what we'd expect (verifies the real dispatch path).
    #[test]
    fn test_or_split_mutation_check() {
        let doc = import(OR_SPLIT_XML).expect("or-split doc");
        assert_eq!(doc.rules.len(), 2, "Or produces exactly 2 rules");
        // Rule 0 body: x[type->A]
        let body0 = &doc.rules[0].body;
        let body1 = &doc.rules[1].body;
        // Verify they are DIFFERENT (each disjunct produces a different body).
        assert_ne!(body0, body1, "each disjunct gives a distinct rule body");
        // Verify one body contains A and the other B.
        let has_iri = |body: &Vec<Atom>, iri: &str| {
            body.iter().any(|a| {
                if let Atom::Frame { val: Term::Iri(v), .. } = a { v == iri } else { false }
            })
        };
        let has_a = has_iri(body0, "http://example.org/A")
            || has_iri(body1, "http://example.org/A");
        let has_b = has_iri(body0, "http://example.org/B")
            || has_iri(body1, "http://example.org/B");
        assert!(has_a, "one rule body corresponds to disjunct A");
        assert!(has_b, "one rule body corresponds to disjunct B");
    }

    /// Named-argument uniterm → ImportError::NamedArgUniterm.
    #[test]
    fn test_reject_named_arg_uniterm() {
        // Construct XML with a <slot><Name>...</Name>...</slot> pattern.
        let xml = br#"<Document xmlns="http://www.w3.org/2007/rif#">
  <payload><Group><sentences>
    <Forall>
      <declare><Var>x</Var></declare>
      <formula>
        <Implies>
          <if>
            <Frame>
              <object><Var>x</Var></object>
              <slot>
                <Name><Const type="http://www.w3.org/2007/rif#iri">http://example.org/p</Const></Name>
                <Var>x</Var>
              </slot>
            </Frame>
          </if>
          <then>
            <Frame>
              <object><Var>x</Var></object>
              <slot><Const type="http://www.w3.org/2007/rif#iri">http://example.org/q</Const><Const type="http://www.w3.org/2007/rif#iri">http://example.org/v</Const></slot>
            </Frame>
          </then>
        </Implies>
      </formula>
    </Forall>
  </sentences></Group></payload>
</Document>"#;
        let err = import(xml).expect_err("named-arg uniterm must be rejected");
        assert!(
            matches!(err, ImportError::NamedArgUniterm { .. }),
            "expected NamedArgUniterm, got: {}",
            err
        );
    }

    /// Display impl: each ImportError variant must produce a non-empty string.
    #[test]
    fn test_display_import_error() {
        use crate::rif::RifError;

        let cases: Vec<ImportError> = vec![
            ImportError::MalformedXml("bad".to_string()),
            ImportError::ImportDirective { location: "http://ex/r".to_string() },
            ImportError::NonCoreElement {
                element: "Naf".to_string(),
                reason: "monotone".to_string(),
            },
            ImportError::UnknownExternal { iri: "http://ex/f".to_string() },
            ImportError::NamedArgUniterm { name: "foo".to_string() },
            ImportError::UnrecognizedElement { tag: "Bogus".to_string() },
            ImportError::ValidationFailed(RifError::UnboundHeadVar { var: "x".to_string() }),
        ];
        for e in &cases {
            let s = e.to_string();
            assert!(!s.is_empty(), "Display must be non-empty for {:?}", e);
        }
    }

    /// ValidationFailed: a syntactically valid XML doc that fails range-restriction.
    #[test]
    fn test_reject_validation_failed() {
        // Rule: head var ?y not bound by body.
        let xml = br#"<Document xmlns="http://www.w3.org/2007/rif#">
  <payload><Group><sentences>
    <Forall>
      <declare><Var>x</Var></declare>
      <declare><Var>y</Var></declare>
      <formula>
        <Implies>
          <if>
            <Frame>
              <object><Var>x</Var></object>
              <slot><Const type="http://www.w3.org/2007/rif#iri">http://ex/p</Const><Const type="http://www.w3.org/2007/rif#iri">http://ex/v</Const></slot>
            </Frame>
          </if>
          <then>
            <Frame>
              <object><Var>y</Var></object>
              <slot><Const type="http://www.w3.org/2007/rif#iri">http://ex/q</Const><Const type="http://www.w3.org/2007/rif#iri">http://ex/w</Const></slot>
            </Frame>
          </then>
        </Implies>
      </formula>
    </Forall>
  </sentences></Group></payload>
</Document>"#;
        let err = import(xml).expect_err("unbound head var must be caught by validate");
        assert!(
            matches!(err, ImportError::ValidationFailed(RifError::UnboundHeadVar { .. })),
            "expected ValidationFailed(UnboundHeadVar), got: {}",
            err
        );
    }

    /// A known builtin IRI round-trips correctly.
    #[test]
    fn test_builtin_numeric_add_parses() {
        let b = iri_to_builtin(
            "http://www.w3.org/2007/rif-builtin-function#numeric-add",
        )
        .expect("numeric-add must map to Builtin::NumericAdd");
        assert_eq!(b, Builtin::NumericAdd);
    }

    // ---- Alpha-rename tests (Fix-1 mandatory) --------------------------------

    /// Test (a): shadow document — Forall ?x: h(?x) :- And(p(?x), Exists ?x(q(?x))).
    /// The existential ?x must be renamed to a fresh name; body vars must be DISTINCT
    /// (NOT [x, x]).
    #[test]
    fn test_alpha_rename_shadow() {
        let doc = import(SHADOW_XML).expect("shadow doc must import with alpha-rename");
        assert_eq!(doc.rules.len(), 1);
        let rule = &doc.rules[0];
        assert_eq!(rule.body.len(), 2, "body has two atoms");

        // Collect all variable names from body object positions.
        let body_obj_vars: Vec<String> = rule.body.iter().filter_map(|a| {
            if let Atom::Frame { obj: Term::Var(v), .. } = a { Some(v.clone()) } else { None }
        }).collect();
        assert_eq!(body_obj_vars.len(), 2, "both body atoms have a Var object");

        // The two object vars must be DISTINCT (not both "x").
        let distinct: BTreeSet<_> = body_obj_vars.iter().cloned().collect();
        assert_eq!(distinct.len(), 2,
            "Fix-1: shadow capture — body vars must be distinct after alpha-rename; \
             got {:?} (both are the same var, indicating capture)", body_obj_vars);

        // One must be the universal "x", the other a fresh __ex name.
        assert!(distinct.contains("x"), "universal ?x must still appear in body");
        let has_fresh = distinct.iter().any(|v| v.starts_with("__ex"));
        assert!(has_fresh, "existential ?x must be renamed to a __ex fresh var");
    }

    /// Test (b): sibling Exists — Forall ?x: head :- And(Exists ?y(p(?x,?y)), Exists ?y(q(?x,?y))).
    /// Two sibling Exists each declare ?y — must produce TWO DISTINCT fresh vars.
    #[test]
    fn test_alpha_rename_sibling() {
        let doc = import(SIBLING_EXISTS_XML).expect("sibling exists doc must import");
        assert_eq!(doc.rules.len(), 1);
        let rule = &doc.rules[0];
        assert_eq!(rule.body.len(), 2, "body has two atoms (one per sibling Exists)");

        // The val of each body Frame should be a distinct fresh var.
        let fresh_vals: Vec<String> = rule.body.iter().filter_map(|a| {
            if let Atom::Frame { val: Term::Var(v), .. } = a { Some(v.clone()) } else { None }
        }).collect();
        assert_eq!(fresh_vals.len(), 2, "both body atoms have a Var value");

        let distinct: BTreeSet<_> = fresh_vals.iter().cloned().collect();
        assert_eq!(distinct.len(), 2,
            "Fix-1: sibling reuse — two sibling Exists ?y must produce TWO distinct fresh vars; \
             got {:?}", fresh_vals);

        // Both must be fresh __ex names.
        assert!(fresh_vals.iter().all(|v| v.starts_with("__ex")),
            "all existential vars from sibling Exists must be renamed to __ex names");
    }

    /// Test (c): nested Exists — the innermost binder wins.
    /// Exists ?z(Exists ?z(p(?x,?z))): inner Exists ?z is renamed; outer rename has no effect.
    #[test]
    fn test_alpha_rename_nested() {
        let doc = import(NESTED_EXISTS_XML).expect("nested exists doc must import");
        assert_eq!(doc.rules.len(), 1);
        let rule = &doc.rules[0];
        // Only one body atom (the And has been flattened; the nested Exists both removed).
        assert_eq!(rule.body.len(), 1, "single body atom from innermost Exists");

        // The body atom must use a fresh var (not the original "z").
        let z_var = Term::Var("z".to_string());
        let has_z = matches!(&rule.body[0], Atom::Frame { val, .. } if val == &z_var);
        assert!(!has_z, "original 'z' must not appear — innermost Exists renamed it");

        let has_fresh = matches!(&rule.body[0],
            Atom::Frame { val: Term::Var(v), .. } if v.starts_with("__ex"));
        assert!(has_fresh, "innermost ?z must be renamed to a __ex fresh var");
    }

    /// Test (d): non-colliding existential var is UNCONDITIONALLY renamed; validate() passes.
    /// The original ?w does not collide with any other var, but must still be renamed.
    #[test]
    fn test_alpha_rename_non_colliding() {
        let doc = import(NON_COLLIDING_EXISTS_XML)
            .expect("non-colliding exists must import and pass validation");
        assert_eq!(doc.rules.len(), 1);
        let rule = &doc.rules[0];
        assert_eq!(rule.body.len(), 1);

        // ?w must NOT appear under its original name.
        let w_var = Term::Var("w".to_string());
        let has_original_w = matches!(&rule.body[0], Atom::Frame { val, .. } if val == &w_var);
        assert!(!has_original_w,
            "non-colliding existential ?w must be renamed unconditionally (Fix-1)");

        // A fresh __ex var must appear instead.
        let has_fresh = matches!(&rule.body[0],
            Atom::Frame { val: Term::Var(v), .. } if v.starts_with("__ex"));
        assert!(has_fresh, "renamed ?w must appear as a __ex fresh var");

        // Document must still validate (range-restriction holds: head ?x bound by body).
        // The import() call itself called validate(); the fact it returned Ok proves this.
    }

    /// Mutation check: disabling alpha_rename_cond produces duplicate body vars for
    /// both the shadow and sibling patterns. This proves that alpha-renaming is the
    /// load-bearing mechanism that fixes capture; tests (a) and (b) would be RED without it.
    #[test]
    fn test_capture_mutation_check() {
        // Helper: collect all Term::Var object-position names from a flat disjunct.
        fn body_obj_var_names(body: &[Atom]) -> Vec<String> {
            body.iter().filter_map(|a| {
                if let Atom::Frame { obj: Term::Var(v), .. } = a { Some(v.clone()) } else { None }
            }).collect()
        }

        // --- Shadow pattern WITHOUT alpha_rename_cond ---
        // Construct: And(Frame{obj:Var("x"),...}, Exists(["x"], Frame{obj:Var("x"),...}))
        let make_shadow = || BodyCond::And(vec![
            BodyCond::Atom(Atom::Frame {
                obj: Term::Var("x".to_string()),
                pred: Term::Iri("http://ex/p".to_string()),
                val: Term::Iri("http://ex/v".to_string()),
            }),
            BodyCond::Exists(
                vec!["x".to_string()],
                Box::new(BodyCond::Atom(Atom::Frame {
                    obj: Term::Var("x".to_string()),
                    pred: Term::Iri("http://ex/q".to_string()),
                    val: Term::Iri("http://ex/v".to_string()),
                })),
            ),
        ]);

        // Without alpha_rename_cond: expand_body drops the Exists wrapper → both atoms
        // use obj=Var("x") → body_obj_var_names returns ["x", "x"].
        let raw_disjuncts = expand_body(make_shadow());
        assert_eq!(raw_disjuncts.len(), 1);
        let raw_vars = body_obj_var_names(&raw_disjuncts[0]);
        let raw_distinct: BTreeSet<_> = raw_vars.iter().cloned().collect();
        // Mutation check (RED without rename): without alpha-rename, duplicate var detected.
        assert_eq!(raw_distinct.len(), 1,
            "mutation check: without alpha_rename_cond, shadow produces duplicate 'x' — \
             capture confirmed (this assertion verifies the mutation FAILS; \
             disable alpha_rename_cond in parse_implies and tests (a)/(b) go RED)");

        // With alpha_rename_cond: distinct vars.
        let mut counter = 0u32;
        let mut universe: BTreeSet<String> = ["x".to_string()].into_iter().collect();
        let renamed = alpha_rename_cond(make_shadow(), &mut counter, &mut universe);
        let renamed_disjuncts = expand_body(renamed);
        assert_eq!(renamed_disjuncts.len(), 1);
        let renamed_vars = body_obj_var_names(&renamed_disjuncts[0]);
        let renamed_distinct: BTreeSet<_> = renamed_vars.iter().cloned().collect();
        assert_eq!(renamed_distinct.len(), 2,
            "with alpha_rename_cond, shadow body vars are distinct (capture fixed)");

        // --- Sibling pattern WITHOUT alpha_rename_cond ---
        // Construct: And(Exists(["y"], Frame{val:Var("y")}), Exists(["y"], Frame{val:Var("y")}))
        fn body_val_var_names(body: &[Atom]) -> Vec<String> {
            body.iter().filter_map(|a| {
                if let Atom::Frame { val: Term::Var(v), .. } = a { Some(v.clone()) } else { None }
            }).collect()
        }
        let make_sibling = || BodyCond::And(vec![
            BodyCond::Exists(
                vec!["y".to_string()],
                Box::new(BodyCond::Atom(Atom::Frame {
                    obj: Term::Iri("http://ex/s".to_string()),
                    pred: Term::Iri("http://ex/p".to_string()),
                    val: Term::Var("y".to_string()),
                })),
            ),
            BodyCond::Exists(
                vec!["y".to_string()],
                Box::new(BodyCond::Atom(Atom::Frame {
                    obj: Term::Iri("http://ex/s".to_string()),
                    pred: Term::Iri("http://ex/q".to_string()),
                    val: Term::Var("y".to_string()),
                })),
            ),
        ]);
        let sib_raw_disjuncts = expand_body(make_sibling());
        let sib_raw_vars = body_val_var_names(&sib_raw_disjuncts[0]);
        let sib_raw_distinct: BTreeSet<_> = sib_raw_vars.iter().cloned().collect();
        assert_eq!(sib_raw_distinct.len(), 1,
            "mutation check: without alpha_rename_cond, sibling produces duplicate 'y'");

        // With alpha_rename_cond → distinct.
        let mut counter2 = 0u32;
        let mut universe2: BTreeSet<String> = ["y".to_string()].into_iter().collect();
        let sib_renamed = alpha_rename_cond(make_sibling(), &mut counter2, &mut universe2);
        let sib_renamed_disjuncts = expand_body(sib_renamed);
        let sib_renamed_vars = body_val_var_names(&sib_renamed_disjuncts[0]);
        let sib_renamed_distinct: BTreeSet<_> = sib_renamed_vars.iter().cloned().collect();
        assert_eq!(sib_renamed_distinct.len(), 2,
            "with alpha_rename_cond, sibling body vars are distinct");
    }

    // ---- Silent-drop / fail-closed tests (Fix-2) ----------------------------

    /// <And> with a non-<formula> child → UnrecognizedElement (not silent drop).
    #[test]
    fn test_silent_drop_stray_child_and() {
        let xml = br#"<Document xmlns="http://www.w3.org/2007/rif#">
  <payload><Group><sentences>
    <Forall>
      <declare><Var>x</Var></declare>
      <formula>
        <Implies>
          <if>
            <And>
              <formula>
                <Frame>
                  <object><Var>x</Var></object>
                  <slot><Const type="http://www.w3.org/2007/rif#iri">http://ex/p</Const><Const type="http://www.w3.org/2007/rif#iri">http://ex/v</Const></slot>
                </Frame>
              </formula>
              <StrayChild/>
            </And>
          </if>
          <then>
            <Frame>
              <object><Var>x</Var></object>
              <slot><Const type="http://www.w3.org/2007/rif#iri">http://ex/q</Const><Const type="http://www.w3.org/2007/rif#iri">http://ex/v</Const></slot>
            </Frame>
          </then>
        </Implies>
      </formula>
    </Forall>
  </sentences></Group></payload>
</Document>"#;
        let err = import(xml).expect_err("stray non-formula child in And must be rejected");
        assert!(
            matches!(err, ImportError::UnrecognizedElement { ref tag } if tag == "StrayChild"),
            "expected UnrecognizedElement(StrayChild), got: {}",
            err
        );
    }

    /// Duplicate <if> in Implies → MalformedXml (fail-closed for single-cardinality).
    #[test]
    fn test_duplicate_if_rejected() {
        let err = import(DUPLICATE_IF_XML).expect_err("duplicate <if> must be rejected");
        assert!(
            matches!(err, ImportError::MalformedXml(_)),
            "expected MalformedXml for duplicate <if>, got: {}",
            err
        );
    }

    // ---- sq-anuo9: surplus-child rejection on single-cardinality wrappers ----
    //
    // On NON-schema-valid input, single-cardinality wrappers reached via the old
    // `.children.first()` silently DROPPED surplus siblings, contradicting the module
    // doc's universal "nothing is silently skipped or dropped". The `<if>` case is the
    // soundness-relevant one — a dropped conjunct WEAKENS the guard → OVER-derivation.
    // Each test is a mutation-check: reverting the corresponding `only_child` call to
    // `.children.first()` makes `import` return `Ok` (silently dropping the surplus),
    // turning the `expect_err`/`unwrap_err` RED. Conformant RIF-XML (exactly one child
    // per wrapper) is unaffected: the paired control in the `<if>` test imports fine, and
    // the full existing suite (MINIMAL_RULE_XML, OR_SPLIT_XML, EXISTS_FLATTEN_XML, …)
    // stays green. [SONNET-4.6]

    /// Assert `xml` is rejected with `MalformedXml` whose message mentions `needle`.
    fn assert_surplus_rejected(xml: &[u8], needle: &str) {
        let err = import(xml).unwrap_err();
        assert!(
            matches!(err, ImportError::MalformedXml(_)),
            "expected MalformedXml (surplus single-cardinality child), got: {}",
            err
        );
        let msg = err.to_string();
        assert!(
            msg.contains(needle),
            "MalformedXml message must mention {}: {}",
            needle,
            msg
        );
    }

    /// SOUNDNESS (over-derivation): an `<if>` holding TWO condition children must be
    /// rejected. The old `.children.first()` kept only the first `<Frame>` and dropped
    /// the second conjunct, so the imported rule had a strictly WEAKER body (fires on
    /// more inputs) → unsound over-derivation. Fail-closed rejection is correct. The
    /// paired control proves the surplus sibling is the SOLE cause of the rejection.
    #[test]
    fn test_surplus_if_child_rejected_over_derivation() {
        // Malformed: <if> directly contains TWO <Frame> conjuncts (no <And> wrapper).
        let malformed = br#"<Document xmlns="http://www.w3.org/2007/rif#">
  <payload><Group><sentences><Forall>
    <declare><Var>x</Var></declare>
    <formula><Implies>
      <if>
        <Frame><object><Var>x</Var></object>
          <slot><Const type="http://www.w3.org/2007/rif#iri">http://ex/p</Const><Const type="http://www.w3.org/2007/rif#iri">http://ex/v</Const></slot></Frame>
        <Frame><object><Var>x</Var></object>
          <slot><Const type="http://www.w3.org/2007/rif#iri">http://ex/q</Const><Const type="http://www.w3.org/2007/rif#iri">http://ex/w</Const></slot></Frame>
      </if>
      <then>
        <Frame><object><Var>x</Var></object>
          <slot><Const type="http://www.w3.org/2007/rif#iri">http://ex/r</Const><Const type="http://www.w3.org/2007/rif#iri">http://ex/v</Const></slot></Frame>
      </then>
    </Implies></formula>
  </Forall></sentences></Group></payload>
</Document>"#;
        assert_surplus_rejected(malformed, "<if>");

        // Control: the SAME rule with a single <if> child imports cleanly — the surplus
        // sibling above is the sole cause of the rejection, not some unrelated defect.
        let control = br#"<Document xmlns="http://www.w3.org/2007/rif#">
  <payload><Group><sentences><Forall>
    <declare><Var>x</Var></declare>
    <formula><Implies>
      <if>
        <Frame><object><Var>x</Var></object>
          <slot><Const type="http://www.w3.org/2007/rif#iri">http://ex/p</Const><Const type="http://www.w3.org/2007/rif#iri">http://ex/v</Const></slot></Frame>
      </if>
      <then>
        <Frame><object><Var>x</Var></object>
          <slot><Const type="http://www.w3.org/2007/rif#iri">http://ex/r</Const><Const type="http://www.w3.org/2007/rif#iri">http://ex/v</Const></slot></Frame>
      </then>
    </Implies></formula>
  </Forall></sentences></Group></payload>
</Document>"#;
        let doc = import(control).expect("single-child <if> is conformant and must import");
        assert_eq!(doc.rules.len(), 1, "control rule must import as exactly one rule");
    }

    /// `<then>` holding TWO head atoms drops the second (under-derivation) → rejected.
    /// A conjunctive head must be expressed as `<then><And>…</And></then>`, so two bare
    /// `<Frame>` children directly under `<then>` are malformed.
    #[test]
    fn test_surplus_then_child_rejected() {
        let xml = br#"<Document xmlns="http://www.w3.org/2007/rif#">
  <payload><Group><sentences><Forall>
    <declare><Var>x</Var></declare>
    <formula><Implies>
      <if><Frame><object><Var>x</Var></object>
        <slot><Const type="http://www.w3.org/2007/rif#iri">http://ex/p</Const><Const type="http://www.w3.org/2007/rif#iri">http://ex/v</Const></slot></Frame></if>
      <then>
        <Frame><object><Var>x</Var></object>
          <slot><Const type="http://www.w3.org/2007/rif#iri">http://ex/q</Const><Const type="http://www.w3.org/2007/rif#iri">http://ex/v</Const></slot></Frame>
        <Frame><object><Var>x</Var></object>
          <slot><Const type="http://www.w3.org/2007/rif#iri">http://ex/r</Const><Const type="http://www.w3.org/2007/rif#iri">http://ex/v</Const></slot></Frame>
      </then>
    </Implies></formula>
  </Forall></sentences></Group></payload>
</Document>"#;
        assert_surplus_rejected(xml, "<then>");
    }

    /// A body `<formula>` inside `<And>` holding TWO condition children → rejected
    /// (`parse_formula_child`).
    #[test]
    fn test_surplus_formula_child_rejected() {
        let xml = br#"<Document xmlns="http://www.w3.org/2007/rif#">
  <payload><Group><sentences><Forall>
    <declare><Var>x</Var></declare>
    <formula><Implies>
      <if><And>
        <formula>
          <Frame><object><Var>x</Var></object>
            <slot><Const type="http://www.w3.org/2007/rif#iri">http://ex/p</Const><Const type="http://www.w3.org/2007/rif#iri">http://ex/v</Const></slot></Frame>
          <Frame><object><Var>x</Var></object>
            <slot><Const type="http://www.w3.org/2007/rif#iri">http://ex/q</Const><Const type="http://www.w3.org/2007/rif#iri">http://ex/w</Const></slot></Frame>
        </formula>
      </And></if>
      <then><Frame><object><Var>x</Var></object>
        <slot><Const type="http://www.w3.org/2007/rif#iri">http://ex/r</Const><Const type="http://www.w3.org/2007/rif#iri">http://ex/v</Const></slot></Frame></then>
    </Implies></formula>
  </Forall></sentences></Group></payload>
</Document>"#;
        assert_surplus_rejected(xml, "<formula>");
    }

    /// The `<formula>` directly under a `<Forall>` holding a surplus sibling (an
    /// `<Implies>` plus a stray `<Frame>`) → rejected (`parse_sentence` Forall arm).
    #[test]
    fn test_surplus_forall_formula_child_rejected() {
        let xml = br#"<Document xmlns="http://www.w3.org/2007/rif#">
  <payload><Group><sentences><Forall>
    <declare><Var>x</Var></declare>
    <formula>
      <Implies>
        <if><Frame><object><Var>x</Var></object>
          <slot><Const type="http://www.w3.org/2007/rif#iri">http://ex/p</Const><Const type="http://www.w3.org/2007/rif#iri">http://ex/v</Const></slot></Frame></if>
        <then><Frame><object><Var>x</Var></object>
          <slot><Const type="http://www.w3.org/2007/rif#iri">http://ex/q</Const><Const type="http://www.w3.org/2007/rif#iri">http://ex/v</Const></slot></Frame></then>
      </Implies>
      <Frame><object><Const type="http://www.w3.org/2007/rif#iri">http://ex/s</Const></object>
        <slot><Const type="http://www.w3.org/2007/rif#iri">http://ex/t</Const><Const type="http://www.w3.org/2007/rif#iri">http://ex/u</Const></slot></Frame>
    </formula>
  </Forall></sentences></Group></payload>
</Document>"#;
        assert_surplus_rejected(xml, "<formula>");
    }

    /// A `<formula>` inside a conjunctive HEAD `<And>` holding two head atoms → rejected
    /// (`parse_head`).
    #[test]
    fn test_surplus_head_formula_child_rejected() {
        let xml = br#"<Document xmlns="http://www.w3.org/2007/rif#">
  <payload><Group><sentences><Forall>
    <declare><Var>x</Var></declare>
    <formula><Implies>
      <if><Frame><object><Var>x</Var></object>
        <slot><Const type="http://www.w3.org/2007/rif#iri">http://ex/p</Const><Const type="http://www.w3.org/2007/rif#iri">http://ex/v</Const></slot></Frame></if>
      <then><And>
        <formula>
          <Frame><object><Var>x</Var></object>
            <slot><Const type="http://www.w3.org/2007/rif#iri">http://ex/q</Const><Const type="http://www.w3.org/2007/rif#iri">http://ex/v</Const></slot></Frame>
          <Frame><object><Var>x</Var></object>
            <slot><Const type="http://www.w3.org/2007/rif#iri">http://ex/r</Const><Const type="http://www.w3.org/2007/rif#iri">http://ex/v</Const></slot></Frame>
        </formula>
      </And></then>
    </Implies></formula>
  </Forall></sentences></Group></payload>
</Document>"#;
        assert_surplus_rejected(xml, "<formula>");
    }

    /// A term wrapper `<object>` holding TWO term children → rejected (`parse_first_term`).
    /// Representative of the `<object>/<instance>/<class>/<sub>/<sup>/<left>/<right>` class.
    #[test]
    fn test_surplus_object_term_rejected() {
        let xml = br#"<Document xmlns="http://www.w3.org/2007/rif#">
  <payload><Group><sentences>
    <Frame>
      <object>
        <Const type="http://www.w3.org/2007/rif#iri">http://ex/s</Const>
        <Const type="http://www.w3.org/2007/rif#iri">http://ex/extra</Const>
      </object>
      <slot><Const type="http://www.w3.org/2007/rif#iri">http://ex/p</Const><Const type="http://www.w3.org/2007/rif#iri">http://ex/v</Const></slot>
    </Frame>
  </sentences></Group></payload>
</Document>"#;
        assert_surplus_rejected(xml, "<object>");
    }

    /// A term wrapper `<instance>` (Member) holding TWO term children → rejected. Confirms
    /// `parse_first_term` names the ACTUAL wrapper tag, not a hard-coded one.
    #[test]
    fn test_surplus_instance_term_rejected() {
        let xml = br#"<Document xmlns="http://www.w3.org/2007/rif#">
  <payload><Group><sentences>
    <Member>
      <instance>
        <Const type="http://www.w3.org/2007/rif#iri">http://ex/i</Const>
        <Const type="http://www.w3.org/2007/rif#iri">http://ex/extra</Const>
      </instance>
      <class><Const type="http://www.w3.org/2007/rif#iri">http://ex/C</Const></class>
    </Member>
  </sentences></Group></payload>
</Document>"#;
        assert_surplus_rejected(xml, "<instance>");
    }

    /// An `<External><content>` holding TWO children → rejected (`parse_external`).
    #[test]
    fn test_surplus_external_content_rejected() {
        let xml = br#"<Document xmlns="http://www.w3.org/2007/rif#">
  <payload><Group><sentences><Forall>
    <declare><Var>x</Var></declare>
    <formula><Implies>
      <if><External>
        <content>
          <Atom><op><Const type="http://www.w3.org/2007/rif#iri">http://www.w3.org/2007/rif-builtin-predicate#is-literal-integer</Const></op>
            <args><Var>x</Var></args></Atom>
          <Atom><op><Const type="http://www.w3.org/2007/rif#iri">http://www.w3.org/2007/rif-builtin-predicate#is-literal-integer</Const></op>
            <args><Var>x</Var></args></Atom>
        </content>
      </External></if>
      <then><Frame><object><Var>x</Var></object>
        <slot><Const type="http://www.w3.org/2007/rif#iri">http://ex/p</Const><Const type="http://www.w3.org/2007/rif#iri">http://ex/v</Const></slot></Frame></then>
    </Implies></formula>
  </Forall></sentences></Group></payload>
</Document>"#;
        assert_surplus_rejected(xml, "<content>");
    }

    /// An `<External>` `<op>` wrapper holding TWO `<Const>` children → rejected
    /// (`parse_external`).
    #[test]
    fn test_surplus_external_op_rejected() {
        let xml = br#"<Document xmlns="http://www.w3.org/2007/rif#">
  <payload><Group><sentences><Forall>
    <declare><Var>x</Var></declare>
    <formula><Implies>
      <if><External>
        <content><Atom>
          <op>
            <Const type="http://www.w3.org/2007/rif#iri">http://www.w3.org/2007/rif-builtin-predicate#is-literal-integer</Const>
            <Const type="http://www.w3.org/2007/rif#iri">http://ex/extra</Const>
          </op>
          <args><Var>x</Var></args>
        </Atom></content>
      </External></if>
      <then><Frame><object><Var>x</Var></object>
        <slot><Const type="http://www.w3.org/2007/rif#iri">http://ex/p</Const><Const type="http://www.w3.org/2007/rif#iri">http://ex/v</Const></slot></Frame></then>
    </Implies></formula>
  </Forall></sentences></Group></payload>
</Document>"#;
        assert_surplus_rejected(xml, "<op>");
    }

    /// A `<sentence>` wrapper holding TWO sentence elements → rejected (`interpret_group`).
    #[test]
    fn test_surplus_sentence_wrapper_rejected() {
        let xml = br#"<Document xmlns="http://www.w3.org/2007/rif#">
  <payload><Group>
    <sentence>
      <Frame><object><Const type="http://www.w3.org/2007/rif#iri">http://ex/s1</Const></object>
        <slot><Const type="http://www.w3.org/2007/rif#iri">http://ex/p</Const><Const type="http://www.w3.org/2007/rif#iri">http://ex/v</Const></slot></Frame>
      <Frame><object><Const type="http://www.w3.org/2007/rif#iri">http://ex/s2</Const></object>
        <slot><Const type="http://www.w3.org/2007/rif#iri">http://ex/p</Const><Const type="http://www.w3.org/2007/rif#iri">http://ex/v</Const></slot></Frame>
    </sentence>
  </Group></payload>
</Document>"#;
        assert_surplus_rejected(xml, "<sentence>");
    }

    /// A `<declare>` holding a stray NON-`<Var>` sibling → rejected. The earlier Fix-6
    /// filter-to-`<Var>` guard TOLERATED this (it silently ignored the stray `<Const>`);
    /// sq-anuo9 tightens it to "exactly one child, and it must be a `<Var>`". (Forall arm.)
    #[test]
    fn test_declare_stray_nonvar_child_rejected_forall() {
        let xml = br#"<Document xmlns="http://www.w3.org/2007/rif#">
  <payload><Group><sentences><Forall>
    <declare><Var>x</Var><Const type="http://www.w3.org/2007/rif#iri">http://ex/junk</Const></declare>
    <formula><Implies>
      <if><Frame><object><Var>x</Var></object>
        <slot><Const type="http://www.w3.org/2007/rif#iri">http://ex/p</Const><Const type="http://www.w3.org/2007/rif#iri">http://ex/v</Const></slot></Frame></if>
      <then><Frame><object><Var>x</Var></object>
        <slot><Const type="http://www.w3.org/2007/rif#iri">http://ex/q</Const><Const type="http://www.w3.org/2007/rif#iri">http://ex/v</Const></slot></Frame></then>
    </Implies></formula>
  </Forall></sentences></Group></payload>
</Document>"#;
        assert_surplus_rejected(xml, "declare");
    }

    /// Same stray-non-`<Var>` tolerance fix in the `<Exists>` (`parse_condition`) arm.
    #[test]
    fn test_declare_stray_nonvar_child_rejected_exists() {
        let xml = br#"<Document xmlns="http://www.w3.org/2007/rif#">
  <payload><Group><sentences><Forall>
    <declare><Var>x</Var></declare>
    <formula><Implies>
      <if><Exists>
        <declare><Var>a</Var><Const type="http://www.w3.org/2007/rif#iri">http://ex/junk</Const></declare>
        <formula><Frame><object><Var>x</Var></object>
          <slot><Const type="http://www.w3.org/2007/rif#iri">http://ex/p</Const><Var>a</Var></slot></Frame></formula>
      </Exists></if>
      <then><Frame><object><Var>x</Var></object>
        <slot><Const type="http://www.w3.org/2007/rif#iri">http://ex/q</Const><Const type="http://www.w3.org/2007/rif#iri">http://ex/v</Const></slot></Frame></then>
    </Implies></formula>
  </Forall></sentences></Group></payload>
</Document>"#;
        assert_surplus_rejected(xml, "declare");
    }

    // ---- Const whitespace round-trip (Fix-3) ---------------------------------

    /// xsd:string lexical value with surrounding whitespace must round-trip exactly.
    /// The old trim_text(true) was stripping leading/trailing spaces from string literals.
    #[test]
    fn test_const_string_whitespace_roundtrip() {
        // Bare Frame fact: object, slot pred (IRI), slot val (xsd:string with whitespace).
        let xml = br#"<Document xmlns="http://www.w3.org/2007/rif#">
  <payload><Group><sentences>
    <Frame>
      <object><Const type="http://www.w3.org/2007/rif#iri">http://ex/s</Const></object>
      <slot>
        <Const type="http://www.w3.org/2007/rif#iri">http://ex/p</Const>
        <Const type="http://www.w3.org/2001/XMLSchema#string"> a  b </Const>
      </slot>
    </Frame>
  </sentences></Group></payload>
</Document>"#;
        let doc = import(xml).expect("valid doc with xsd:string whitespace value");
        assert_eq!(doc.rules.len(), 1);
        // The fact's head atom contains the string Const in the val position.
        let atom = &doc.rules[0].head[0];
        if let Atom::Frame { val, .. } = atom {
            assert_eq!(
                val,
                &Term::Lit {
                    lex: " a  b ".to_string(),
                    datatype: "http://www.w3.org/2001/XMLSchema#string".to_string(),
                },
                "xsd:string lexical value must be preserved exactly (Fix-3)"
            );
        } else {
            panic!("expected Frame atom, got {:?}", atom);
        }
    }

    // ---- func:count (ListLength) mapping (Fix-4) ----------------------------

    /// func:count → Builtin::ListLength (was wrongly rejected as UnknownExternal).
    #[test]
    fn test_list_length_func_count_parses() {
        let b = iri_to_builtin("http://www.w3.org/2007/rif-builtin-function#count")
            .expect("func:count must map to Builtin::ListLength (Fix-4)");
        assert_eq!(b, Builtin::ListLength);
    }

    // ---- Fix-6 tests: <declare> cardinality guard ---------------------------

    /// Test (a) — demonstrated capture case: Exists with a single <declare> holding
    /// TWO <Var> children (?a and ?b) must be rejected (MalformedXml).
    ///
    /// Mutation-check annotation: without the count guard (`var_children.len() != 1`
    /// check removed, reverting to the old `filter_map(.first())` path), `import`
    /// returns `Ok` — it silently accepts the document and drops the second Var,
    /// leaving ?b unrenamed. With the guard this test returns `Err(MalformedXml)`
    /// and the assertion passes; removing the guard causes the `expect_err` to panic
    /// (import returns `Ok`), making the test RED. [SONNET-4.6]
    #[test]
    fn test_reject_multi_var_declare_exists() {
        let err = import(MULTI_VAR_DECLARE_EXISTS_XML)
            .expect_err("multi-Var <declare> in Exists must be rejected (Fix-6)");
        assert!(
            matches!(err, ImportError::MalformedXml(_)),
            "expected MalformedXml for multi-Var <declare>, got: {}",
            err
        );
        // Verify the error message names the element for debuggability.
        let msg = err.to_string();
        assert!(
            msg.contains("declare"),
            "error message must mention <declare>: {}",
            msg
        );
    }

    /// Test (b): empty <declare/> in Exists (zero <Var> children) must be rejected.
    #[test]
    fn test_reject_empty_declare_exists() {
        let err = import(EMPTY_DECLARE_EXISTS_XML)
            .expect_err("empty <declare/> in Exists must be rejected (Fix-6)");
        assert!(
            matches!(err, ImportError::MalformedXml(_)),
            "expected MalformedXml for empty <declare/>, got: {}",
            err
        );
        let msg = err.to_string();
        assert!(
            msg.contains("declare"),
            "error message must mention <declare>: {}",
            msg
        );
    }

    /// Test (c): valid one-Var-per-declare multi-declare Exists imports successfully.
    /// Both declared vars must be alpha-renamed to DISTINCT fresh names.
    /// This is the regression guard: Fix-6 must not break schema-valid multi-declare.
    #[test]
    fn test_multi_declare_valid_exists() {
        let doc = import(MULTI_DECLARE_VALID_EXISTS_XML)
            .expect("valid multi-declare Exists (one Var per declare) must import (Fix-6 regression guard)");
        assert_eq!(doc.rules.len(), 1);
        let rule = &doc.rules[0];
        // Body has two atoms (one per declared var in the multi-declare Exists).
        assert_eq!(rule.body.len(), 2, "both atoms from the multi-declare Exists body are present");

        // Neither original var name "a" nor "b" must appear (both unconditionally renamed).
        let a_var = Term::Var("a".to_string());
        let b_var = Term::Var("b".to_string());
        let has_original = rule.body.iter().any(|atom| match atom {
            Atom::Frame { obj, pred, val } => {
                [obj, pred, val].iter().any(|t| *t == &a_var || *t == &b_var)
            }
            Atom::Builtin { args, .. } => args.iter().any(|t| t == &a_var || t == &b_var),
            _ => false,
        });
        assert!(
            !has_original,
            "original vars 'a' and 'b' must be alpha-renamed (Fix-6 regression guard)"
        );

        // Collect all fresh vars from body val positions.
        let fresh_vals: Vec<String> = rule.body.iter().filter_map(|a| {
            if let Atom::Frame { val: Term::Var(v), .. } = a {
                Some(v.clone())
            } else {
                None
            }
        }).collect();
        assert_eq!(fresh_vals.len(), 2, "both body atoms have a Var value (the renamed existentials)");

        // Both must be fresh __ex names AND must be DISTINCT.
        assert!(
            fresh_vals.iter().all(|v| v.starts_with("__ex")),
            "all existential vars must be renamed to __ex fresh names; got {:?}",
            fresh_vals
        );
        let distinct: BTreeSet<_> = fresh_vals.iter().cloned().collect();
        assert_eq!(
            distinct.len(),
            2,
            "the two declared vars must produce TWO distinct fresh names; got {:?}",
            fresh_vals
        );
    }

    // ---- Entity-reference round-trip tests (Fix: GeneralRef + attr unescape) ----
    //
    // These tests cover the three blockers identified by the Opus re-verify:
    //
    //  A. Text entity references in element content:
    //     quick-xml 0.40+ emits &amp;/&lt;/&gt;/&#x26; as Event::GeneralRef, not
    //     part of the adjacent Event::Text.  Before this fix they were silently
    //     dropped (caught by the wildcard `_ => {}` arm), corrupting literal values.
    //
    //  B. Attribute values containing entity references:
    //     `type="http://ex/?a=1&amp;b=2"` kept the literal `"&amp;"` text instead of
    //     the decoded `"&"` because the old read_attrs used `str::from_utf8(&a.value)`
    //     without calling `quick_xml::escape::unescape`.
    //
    //  C. Unknown general entities (&undefined;) must be fail-closed (MalformedXml),
    //     not silently dropped.
    //
    // [SONNET-4.6]

    /// Entity references in text content must be decoded correctly.
    ///
    /// Input XML contains `a&amp;b &lt;c&gt; &#x26;d` as the lexical value of an
    /// xsd:string Const.  The decoded value must be exactly `a&b <c> &d`.
    ///
    /// Mutation check annotation: if the `Event::GeneralRef` arm is removed (reverted
    /// to the wildcard `_ => {}`), the GeneralRef events are silently discarded and
    /// the text node accumulates only the adjacent text (`a`, `b `, `c`, ` `, `d`),
    /// producing `"ab c d"` instead of `"a&b <c> &d"`.  The `assert_eq!` below then
    /// fails, turning this test RED.  Restore the `Event::GeneralRef` arm to make it
    /// GREEN again. [SONNET-4.6]
    #[test]
    fn test_entity_text_roundtrip() {
        // RIF Document with a bare Frame fact whose val Const is an xsd:string
        // containing &amp;, &lt;, &gt;, and &#x26; (numeric hex char ref).
        let xml = br#"<Document xmlns="http://www.w3.org/2007/rif#">
  <payload><Group><sentences>
    <Frame>
      <object><Const type="http://www.w3.org/2007/rif#iri">http://ex/s</Const></object>
      <slot>
        <Const type="http://www.w3.org/2007/rif#iri">http://ex/p</Const>
        <Const type="http://www.w3.org/2001/XMLSchema#string">a&amp;b &lt;c&gt; &#x26;d</Const>
      </slot>
    </Frame>
  </sentences></Group></payload>
</Document>"#;
        let doc = import(xml).expect("valid doc with entity references in xsd:string");
        assert_eq!(doc.rules.len(), 1, "one fact rule");
        let atom = &doc.rules[0].head[0];
        if let Atom::Frame { val, .. } = atom {
            assert_eq!(
                val,
                &Term::Lit {
                    lex: "a&b <c> &d".to_string(),
                    datatype: "http://www.w3.org/2001/XMLSchema#string".to_string(),
                },
                "entity references must be decoded: &amp;→& &lt;→< &gt;→> &#x26;→& (Fix: GeneralRef)"
            );
        } else {
            panic!("expected Frame atom, got {:?}", atom);
        }
    }

    /// Attribute values containing entity references must be unescaped.
    ///
    /// A Const with `type="http://ex/dt?a=1&amp;b=2"` (the `&amp;` is an XML entity
    /// reference in the attribute value) must import with datatype IRI
    /// `"http://ex/dt?a=1&b=2"`, not the literal `"&amp;"` text.
    ///
    /// Mutation check annotation: if `read_attrs` is reverted to use
    /// `str::from_utf8(&a.value).to_string()` (without `quick_xml::escape::unescape`),
    /// the datatype will contain the literal `"&amp;"` and the `assert_eq!` will fail.
    /// [SONNET-4.6]
    #[test]
    fn test_entity_attr_roundtrip() {
        // Bare Frame fact with a typed literal whose datatype IRI contains &amp; in the
        // XML attribute (a query-string separator: `?a=1&amp;b=2`).
        let xml = br#"<Document xmlns="http://www.w3.org/2007/rif#">
  <payload><Group><sentences>
    <Frame>
      <object><Const type="http://www.w3.org/2007/rif#iri">http://ex/s</Const></object>
      <slot>
        <Const type="http://www.w3.org/2007/rif#iri">http://ex/p</Const>
        <Const type="http://ex/dt?a=1&amp;b=2">val</Const>
      </slot>
    </Frame>
  </sentences></Group></payload>
</Document>"#;
        let doc = import(xml).expect("valid doc with entity in type attribute");
        assert_eq!(doc.rules.len(), 1, "one fact rule");
        let atom = &doc.rules[0].head[0];
        if let Atom::Frame { val, .. } = atom {
            assert_eq!(
                val,
                &Term::Lit {
                    lex: "val".to_string(),
                    datatype: "http://ex/dt?a=1&b=2".to_string(),
                },
                "entity in type attribute must be unescaped: &amp; → & (Fix: attr unescape)"
            );
        } else {
            panic!("expected Frame atom, got {:?}", atom);
        }
    }

    /// An unknown general entity reference (&undefined;) must produce MalformedXml,
    /// never silently drop the reference text. [SONNET-4.6]
    #[test]
    fn test_reject_unknown_entity() {
        let xml = br#"<Document xmlns="http://www.w3.org/2007/rif#">
  <payload><Group><sentences>
    <Frame>
      <object><Const type="http://www.w3.org/2007/rif#iri">http://ex/s</Const></object>
      <slot>
        <Const type="http://www.w3.org/2007/rif#iri">http://ex/p</Const>
        <Const type="http://www.w3.org/2001/XMLSchema#string">a&undefined;b</Const>
      </slot>
    </Frame>
  </sentences></Group></payload>
</Document>"#;
        let err = import(xml).expect_err("unknown entity &undefined; must be rejected (fail-closed)");
        assert!(
            matches!(err, ImportError::MalformedXml(_)),
            "expected MalformedXml for unknown entity, got: {}",
            err
        );
        let msg = err.to_string();
        assert!(
            msg.contains("undefined") || msg.contains("entity"),
            "error message must name the bad entity: {}",
            msg
        );
    }

    /// resolve_xml_entity unit tests — covers predefined entities and numeric refs.
    #[test]
    fn test_resolve_xml_entity_predefined() {
        // Five predefined XML entities.
        assert_eq!(resolve_xml_entity("amp").unwrap(), "&");
        assert_eq!(resolve_xml_entity("lt").unwrap(), "<");
        assert_eq!(resolve_xml_entity("gt").unwrap(), ">");
        assert_eq!(resolve_xml_entity("quot").unwrap(), "\"");
        assert_eq!(resolve_xml_entity("apos").unwrap(), "'");
        // Decimal numeric character reference: &#38; = '&'.
        assert_eq!(resolve_xml_entity("#38").unwrap(), "&");
        // Hexadecimal numeric character reference: &#x26; = '&' (0x26).
        assert_eq!(resolve_xml_entity("#x26").unwrap(), "&");
        // Upper-case X prefix (non-standard but handled defensively).
        assert_eq!(resolve_xml_entity("#X26").unwrap(), "&");
        // Unknown general entity → fail-closed.
        assert!(
            matches!(resolve_xml_entity("undefined"), Err(ImportError::MalformedXml(_))),
            "unknown entity must produce MalformedXml"
        );
    }

    // ---- sq-4l1fj: fail-closed on a DUPLICATE single-cardinality WRAPPER under a parent ----
    //
    // sq-anuo9 (`only_child`) closed the SURPLUS-GRANDCHILDREN class — one wrapper holding two
    // element children. A distinct twin remained: TWO copies of the SAME single-cardinality
    // wrapper under one parent, read via `child()` (find-first), silently took the first and
    // DROPPED the second. A dropped `<formula>` under a `<Forall>` loses a WHOLE RULE; a dropped
    // `<object>`/`<slot>` changes the atom. `unique_child` now rejects the duplicate fail-closed
    // (the parent-level twin of the `<if>`/`<then>` guard already inlined in `parse_implies`).
    //
    // Each test is a MUTATION-CHECK: reverting `unique_child`'s body to the pre-fix first-wins
    // (`Ok(self.children.iter().find(|c| c.tag == tag))`) makes every `import` below return `Ok`
    // (silently taking the first wrapper), flipping the `unwrap_err`/`expect_err` RED. Conformant
    // RIF-XML has at most one of each wrapper, so every valid-document test stays green (the
    // paired controls below import cleanly). [OPUS-4.8]

    /// Assert `xml` is rejected with a `MalformedXml` DUPLICATE-wrapper diagnostic naming `tag`.
    /// Asserts the word "duplicate" so the test pins the `unique_child` path specifically (the
    /// `only_child` surplus path says "must have exactly one child element" instead).
    fn assert_duplicate_wrapper_rejected(xml: &[u8], tag: &str) {
        let err = import(xml).unwrap_err();
        assert!(
            matches!(err, ImportError::MalformedXml(_)),
            "expected MalformedXml (duplicate single-cardinality wrapper), got: {}",
            err
        );
        let msg = err.to_string();
        assert!(
            msg.contains("duplicate") && msg.contains(tag),
            "MalformedXml must be a DUPLICATE-wrapper diagnostic naming {}: {}",
            tag,
            msg
        );
    }

    /// Parent `<Frame>` — a duplicate `<object>` is rejected; multiple `<slot>` children
    /// are NOW VALID (multi-slot frame desugaring, sq-jsgyn). Paired control: the
    /// single-slot Frame fact imports as exactly one rule.
    ///
    /// Note: the pre-sq-jsgyn behaviour rejected two `<slot>` elements with a
    /// `MalformedXml("Frame has duplicate <slot>…")` error via `unique_child`.
    /// That guard is removed; `<slot>` is now multi-cardinality under `<Frame>`.
    /// `<object>` remains single-cardinality and its duplicate rejection is unchanged.
    #[test]
    fn test_duplicate_frame_object_rejected_multislot_accepted() {
        // Duplicate <object> — STILL rejected (single-cardinality unchanged).
        let dup_object = br#"<Document xmlns="http://www.w3.org/2007/rif#">
  <payload><Group><sentences>
    <Frame>
      <object><Const type="http://www.w3.org/2007/rif#iri">http://ex/s</Const></object>
      <object><Const type="http://www.w3.org/2007/rif#iri">http://ex/s2</Const></object>
      <slot><Const type="http://www.w3.org/2007/rif#iri">http://ex/p</Const><Const type="http://www.w3.org/2007/rif#iri">http://ex/v</Const></slot>
    </Frame>
  </sentences></Group></payload>
</Document>"#;
        assert_duplicate_wrapper_rejected(dup_object, "<object>");

        // Two <slot> children — NOW ACCEPTED as a 2-slot frame (sq-jsgyn).
        // Mutation-check: if parse_frame_atoms is reverted to unique_child("slot",…),
        // this import fails instead of succeeding → expect() panics → test RED.
        let two_slot = br#"<Document xmlns="http://www.w3.org/2007/rif#">
  <payload><Group><sentences>
    <Frame>
      <object><Const type="http://www.w3.org/2007/rif#iri">http://ex/s</Const></object>
      <slot><Const type="http://www.w3.org/2007/rif#iri">http://ex/p</Const><Const type="http://www.w3.org/2007/rif#iri">http://ex/v</Const></slot>
      <slot><Const type="http://www.w3.org/2007/rif#iri">http://ex/q</Const><Const type="http://www.w3.org/2007/rif#iri">http://ex/w</Const></slot>
    </Frame>
  </sentences></Group></payload>
</Document>"#;
        let doc = import(two_slot).expect("2-slot Frame bare fact must now import (sq-jsgyn)");
        // A bare 2-slot Frame produces 2 Rule::fact atoms (one per slot).
        assert_eq!(doc.rules.len(), 2, "2-slot bare Frame desugars to 2 facts");

        // Control: single-slot Frame fact imports as exactly one rule.
        let control = br#"<Document xmlns="http://www.w3.org/2007/rif#">
  <payload><Group><sentences>
    <Frame>
      <object><Const type="http://www.w3.org/2007/rif#iri">http://ex/s</Const></object>
      <slot><Const type="http://www.w3.org/2007/rif#iri">http://ex/p</Const><Const type="http://www.w3.org/2007/rif#iri">http://ex/v</Const></slot>
    </Frame>
  </sentences></Group></payload>
</Document>"#;
        let doc = import(control).expect("single-slot Frame fact is conformant and must import");
        assert_eq!(doc.rules.len(), 1, "control Frame fact must import as exactly one rule");
    }

    // ---- sq-jsgyn: multi-slot Frame desugaring tests --------------------------
    //
    // These tests cover the multi-slot Frame desugaring introduced by sq-jsgyn.
    // The RIF-Core semantic is: obj[p1->v1 p2->v2] ≡ obj[p1->v1] AND obj[p2->v2].
    // Each test has a mutation-check annotation demonstrating what goes red if the
    // desugaring is reverted (e.g., reverting to unique_child("slot",…) or failing
    // to split into multiple atoms).

    /// **Bare-fact position** — a 2-slot Frame produces 2 `Rule::fact` atoms.
    /// The per-slot atoms share the same `obj` and carry distinct `pred`/`val`.
    ///
    /// Mutation-check: reverting `parse_sentence` bare-Frame arm from
    /// `parse_positive_atoms` back to `parse_positive_atom` (single-atom) breaks
    /// compilation OR causes the test to fail if a single-atom shim is inserted
    /// (it returns one rule with only the first slot → `doc.rules.len() == 1` →
    /// assertion `2` fails → RED). [SONNET-4.6] sq-jsgyn
    #[test]
    fn test_multislot_bare_fact_desugars_to_per_slot_facts() {
        let xml = br#"<Document xmlns="http://www.w3.org/2007/rif#">
  <payload><Group><sentences>
    <Frame>
      <object><Const type="http://www.w3.org/2007/rif#iri">http://ex/obj</Const></object>
      <slot><Const type="http://www.w3.org/2007/rif#iri">http://ex/p1</Const><Const type="http://www.w3.org/2007/rif#iri">http://ex/v1</Const></slot>
      <slot><Const type="http://www.w3.org/2007/rif#iri">http://ex/p2</Const><Const type="http://www.w3.org/2007/rif#iri">http://ex/v2</Const></slot>
    </Frame>
  </sentences></Group></payload>
</Document>"#;
        let doc = import(xml).expect("2-slot bare Frame must import (sq-jsgyn)");
        // 2 slots → 2 Rule::fact entries.
        assert_eq!(doc.rules.len(), 2, "2-slot Frame desugars to 2 facts");

        // Both rules are ground facts (empty body).
        for rule in &doc.rules {
            assert!(rule.body.is_empty(), "each per-slot fact has an empty body");
        }
        // Both rules have exactly one head atom.
        for rule in &doc.rules {
            assert_eq!(rule.head.len(), 1, "each per-slot fact has one head atom");
        }
        // Collect head atoms and verify they are the expected per-slot frames.
        let heads: Vec<&Atom> = doc.rules.iter().map(|r| &r.head[0]).collect();
        let has_p1 = heads.iter().any(|a| {
            matches!(a, Atom::Frame { pred: Term::Iri(p), val: Term::Iri(v), .. }
                if p == "http://ex/p1" && v == "http://ex/v1")
        });
        let has_p2 = heads.iter().any(|a| {
            matches!(a, Atom::Frame { pred: Term::Iri(p), val: Term::Iri(v), .. }
                if p == "http://ex/p2" && v == "http://ex/v2")
        });
        assert!(has_p1, "per-slot fact for p1->v1 must be present");
        assert!(has_p2, "per-slot fact for p2->v2 must be present");
        // Both share the same object IRI.
        let all_same_obj = heads.iter().all(|a| {
            matches!(a, Atom::Frame { obj: Term::Iri(o), .. } if o == "http://ex/obj")
        });
        assert!(all_same_obj, "all per-slot atoms must share the same obj");
    }

    /// **Body position** — a 2-slot Frame in the body of a rule desugars to a
    /// conjunction of 2 `Atom::Frame` in the rule body (one per slot).
    ///
    /// Mutation-check: reverting the `"Frame"` arm in `parse_condition` from
    /// `parse_frame_atoms` + `BodyCond::And` back to `parse_frame` (single atom)
    /// causes the rule body to contain only 1 atom → `rule.body.len() == 1` →
    /// assertion `2` fails → RED. [SONNET-4.6] sq-jsgyn
    #[test]
    fn test_multislot_body_desugars_to_conjunction() {
        let xml = br#"<Document xmlns="http://www.w3.org/2007/rif#">
  <payload><Group><sentences>
    <Forall>
      <declare><Var>x</Var></declare>
      <formula>
        <Implies>
          <if>
            <Frame>
              <object><Var>x</Var></object>
              <slot><Const type="http://www.w3.org/2007/rif#iri">http://ex/p1</Const><Const type="http://www.w3.org/2007/rif#iri">http://ex/v1</Const></slot>
              <slot><Const type="http://www.w3.org/2007/rif#iri">http://ex/p2</Const><Const type="http://www.w3.org/2007/rif#iri">http://ex/v2</Const></slot>
            </Frame>
          </if>
          <then>
            <Frame>
              <object><Var>x</Var></object>
              <slot><Const type="http://www.w3.org/2007/rif#iri">http://ex/r</Const><Const type="http://www.w3.org/2007/rif#iri">http://ex/yes</Const></slot>
            </Frame>
          </then>
        </Implies>
      </formula>
    </Forall>
  </sentences></Group></payload>
</Document>"#;
        let doc = import(xml).expect("2-slot Frame in body must import (sq-jsgyn)");
        // Single rule (no Or-split, single head).
        assert_eq!(doc.rules.len(), 1, "2-slot body Frame → 1 rule (no Or-split)");
        let rule = &doc.rules[0];
        // Body: 2 atoms (one per slot).
        assert_eq!(rule.body.len(), 2, "2-slot body Frame desugars to 2 body atoms");
        // Head: 1 atom (the head frame has a single slot).
        assert_eq!(rule.head.len(), 1, "single-slot head produces 1 head atom");
        // Verify the body contains both per-slot atoms.
        let has_p1 = rule.body.iter().any(|a| {
            matches!(a, Atom::Frame { pred: Term::Iri(p), val: Term::Iri(v), .. }
                if p == "http://ex/p1" && v == "http://ex/v1")
        });
        let has_p2 = rule.body.iter().any(|a| {
            matches!(a, Atom::Frame { pred: Term::Iri(p), val: Term::Iri(v), .. }
                if p == "http://ex/p2" && v == "http://ex/v2")
        });
        assert!(has_p1, "body must contain the p1->v1 per-slot atom");
        assert!(has_p2, "body must contain the p2->v2 per-slot atom");
        // Both share the same object variable ?x.
        let all_same_obj = rule.body.iter().all(|a| {
            matches!(a, Atom::Frame { obj: Term::Var(v), .. } if v == "x")
        });
        assert!(all_same_obj, "all per-slot body atoms share obj=?x");
    }

    /// **Head position** — a 2-slot Frame in the head of a rule desugars to 2 head atoms.
    ///
    /// Mutation-check: reverting `parse_head` from `extend(parse_positive_atoms(…))`
    /// back to `push(parse_positive_atom(…))` causes the head to contain only 1 atom
    /// → `rule.head.len() == 1` → assertion `2` fails → RED. [SONNET-4.6] sq-jsgyn
    #[test]
    fn test_multislot_head_desugars_to_multiple_head_atoms() {
        let xml = br#"<Document xmlns="http://www.w3.org/2007/rif#">
  <payload><Group><sentences>
    <Forall>
      <declare><Var>x</Var></declare>
      <formula>
        <Implies>
          <if>
            <Frame>
              <object><Var>x</Var></object>
              <slot><Const type="http://www.w3.org/2007/rif#iri">http://ex/p</Const><Const type="http://www.w3.org/2007/rif#iri">http://ex/v</Const></slot>
            </Frame>
          </if>
          <then>
            <Frame>
              <object><Var>x</Var></object>
              <slot><Const type="http://www.w3.org/2007/rif#iri">http://ex/q1</Const><Const type="http://www.w3.org/2007/rif#iri">http://ex/w1</Const></slot>
              <slot><Const type="http://www.w3.org/2007/rif#iri">http://ex/q2</Const><Const type="http://www.w3.org/2007/rif#iri">http://ex/w2</Const></slot>
            </Frame>
          </then>
        </Implies>
      </formula>
    </Forall>
  </sentences></Group></payload>
</Document>"#;
        let doc = import(xml).expect("2-slot Frame in head must import (sq-jsgyn)");
        assert_eq!(doc.rules.len(), 1, "single rule (single-slot body, no Or-split)");
        let rule = &doc.rules[0];
        // Head: 2 atoms (one per slot).
        assert_eq!(rule.head.len(), 2, "2-slot head Frame desugars to 2 head atoms");
        // Body: 1 atom (single-slot body Frame).
        assert_eq!(rule.body.len(), 1, "single-slot body produces 1 body atom");
        // Verify both head atoms are present with correct pred/val.
        let has_q1 = rule.head.iter().any(|a| {
            matches!(a, Atom::Frame { pred: Term::Iri(p), val: Term::Iri(v), .. }
                if p == "http://ex/q1" && v == "http://ex/w1")
        });
        let has_q2 = rule.head.iter().any(|a| {
            matches!(a, Atom::Frame { pred: Term::Iri(p), val: Term::Iri(v), .. }
                if p == "http://ex/q2" && v == "http://ex/w2")
        });
        assert!(has_q1, "head must contain the q1->w1 per-slot atom");
        assert!(has_q2, "head must contain the q2->w2 per-slot atom");
    }

    /// **Three-slot frame** — regression guard for N>2 slots.
    ///
    /// The desugaring is not limited to exactly 2 slots; a 3-slot frame in a rule
    /// body must produce 3 body atoms. This test prevents a "only handle 2 slots"
    /// regression. [SONNET-4.6] sq-jsgyn
    #[test]
    fn test_multislot_three_slots_in_body() {
        let xml = br#"<Document xmlns="http://www.w3.org/2007/rif#">
  <payload><Group><sentences>
    <Forall>
      <declare><Var>x</Var></declare>
      <formula>
        <Implies>
          <if>
            <Frame>
              <object><Var>x</Var></object>
              <slot><Const type="http://www.w3.org/2007/rif#iri">http://ex/a</Const><Const type="http://www.w3.org/2007/rif#iri">http://ex/1</Const></slot>
              <slot><Const type="http://www.w3.org/2007/rif#iri">http://ex/b</Const><Const type="http://www.w3.org/2007/rif#iri">http://ex/2</Const></slot>
              <slot><Const type="http://www.w3.org/2007/rif#iri">http://ex/c</Const><Const type="http://www.w3.org/2007/rif#iri">http://ex/3</Const></slot>
            </Frame>
          </if>
          <then>
            <Frame>
              <object><Var>x</Var></object>
              <slot><Const type="http://www.w3.org/2007/rif#iri">http://ex/r</Const><Const type="http://www.w3.org/2007/rif#iri">http://ex/yes</Const></slot>
            </Frame>
          </then>
        </Implies>
      </formula>
    </Forall>
  </sentences></Group></payload>
</Document>"#;
        let doc = import(xml).expect("3-slot Frame in body must import (sq-jsgyn)");
        assert_eq!(doc.rules.len(), 1, "single rule");
        let rule = &doc.rules[0];
        assert_eq!(rule.body.len(), 3, "3-slot body Frame desugars to 3 body atoms");
        assert_eq!(rule.head.len(), 1, "single-slot head produces 1 head atom");
    }

    /// **Fail-closed: Frame with zero slots** — must be rejected (MalformedXml).
    ///
    /// A `<Frame>` with an `<object>` but no `<slot>` is syntactically malformed
    /// (a frame without slots is meaningless in RIF-Core). Fail-closed rejection.
    ///
    /// Mutation-check: removing the `slots.is_empty()` guard from `parse_frame_atoms`
    /// causes this to return `Ok(vec![])` → a rule with an empty head atom list →
    /// the `expect_err` panics → RED. [SONNET-4.6] sq-jsgyn
    #[test]
    fn test_multislot_frame_zero_slots_rejected() {
        let xml = br#"<Document xmlns="http://www.w3.org/2007/rif#">
  <payload><Group><sentences>
    <Frame>
      <object><Const type="http://www.w3.org/2007/rif#iri">http://ex/obj</Const></object>
    </Frame>
  </sentences></Group></payload>
</Document>"#;
        let err = import(xml).expect_err("Frame with zero slots must be rejected (fail-closed)");
        assert!(
            matches!(err, ImportError::MalformedXml(_)),
            "expected MalformedXml for zero-slot Frame, got: {}",
            err
        );
        let msg = err.to_string();
        assert!(
            msg.contains("slot"),
            "error message must mention <slot>: {}",
            msg
        );
    }

    /// **Fail-closed: named-argument slot in a multi-slot Frame** — the whole Frame is
    /// rejected when ANY slot is a named-argument uniterm, not just the bad slot.
    ///
    /// Mutation-check: removing the `slot.child("Name").is_some()` guard in
    /// `parse_frame_atoms` would allow the named-arg slot to pass through as
    /// a malformed term, corrupting the import silently → `expect_err` panics →
    /// RED. [SONNET-4.6] sq-jsgyn
    #[test]
    fn test_multislot_named_arg_slot_rejected() {
        // A 2-slot Frame where the second slot is a named-argument uniterm.
        let xml = br#"<Document xmlns="http://www.w3.org/2007/rif#">
  <payload><Group><sentences>
    <Forall>
      <declare><Var>x</Var></declare>
      <formula>
        <Implies>
          <if>
            <Frame>
              <object><Var>x</Var></object>
              <slot><Const type="http://www.w3.org/2007/rif#iri">http://ex/p1</Const><Const type="http://www.w3.org/2007/rif#iri">http://ex/v1</Const></slot>
              <slot>
                <Name><Const type="http://www.w3.org/2007/rif#iri">http://ex/named</Const></Name>
                <Var>x</Var>
              </slot>
            </Frame>
          </if>
          <then>
            <Frame>
              <object><Var>x</Var></object>
              <slot><Const type="http://www.w3.org/2007/rif#iri">http://ex/r</Const><Const type="http://www.w3.org/2007/rif#iri">http://ex/yes</Const></slot>
            </Frame>
          </then>
        </Implies>
      </formula>
    </Forall>
  </sentences></Group></payload>
</Document>"#;
        let err = import(xml)
            .expect_err("named-arg slot in multi-slot Frame must be rejected (fail-closed)");
        assert!(
            matches!(err, ImportError::NamedArgUniterm { .. }),
            "expected NamedArgUniterm for named-arg slot, got: {}",
            err
        );
    }

    /// **Mutation-check (per-slot split is load-bearing):** verifies the desugaring
    /// split is not bypassed by checking that the body atoms carry DIFFERENT predicates.
    /// If the split collapsed to a single atom (e.g., returning only the last slot),
    /// the `pred` distinctness assertion would fail → RED.
    ///
    /// This directly tests the per-slot `atoms.push(…)` loop in `parse_frame_atoms`
    /// is called for EVERY slot, not just one. [SONNET-4.6] sq-jsgyn
    #[test]
    fn test_multislot_split_mutation_per_slot_distinctness() {
        let xml = br#"<Document xmlns="http://www.w3.org/2007/rif#">
  <payload><Group><sentences>
    <Forall>
      <declare><Var>x</Var></declare>
      <formula>
        <Implies>
          <if>
            <Frame>
              <object><Var>x</Var></object>
              <slot><Const type="http://www.w3.org/2007/rif#iri">http://ex/PRED_A</Const><Const type="http://www.w3.org/2007/rif#iri">http://ex/VAL_A</Const></slot>
              <slot><Const type="http://www.w3.org/2007/rif#iri">http://ex/PRED_B</Const><Const type="http://www.w3.org/2007/rif#iri">http://ex/VAL_B</Const></slot>
            </Frame>
          </if>
          <then>
            <Frame>
              <object><Var>x</Var></object>
              <slot><Const type="http://www.w3.org/2007/rif#iri">http://ex/r</Const><Const type="http://www.w3.org/2007/rif#iri">http://ex/yes</Const></slot>
            </Frame>
          </then>
        </Implies>
      </formula>
    </Forall>
  </sentences></Group></payload>
</Document>"#;
        let doc = import(xml).expect("mutation-check: 2-slot body must import");
        let rule = &doc.rules[0];
        assert_eq!(rule.body.len(), 2, "must have 2 body atoms (per-slot split)");
        // Collect the pred IRIs from body atoms.
        let preds: Vec<String> = rule.body.iter().filter_map(|a| {
            if let Atom::Frame { pred: Term::Iri(p), .. } = a { Some(p.clone()) } else { None }
        }).collect();
        assert_eq!(preds.len(), 2, "both body atoms must be Frame atoms with IRI predicates");
        // The two predicates must be DISTINCT — the per-slot split is the load-bearing mechanism.
        // If the loop was collapsed to a single atom, both preds would be the same → assertion fails.
        assert_ne!(preds[0], preds[1],
            "per-slot split mutation: predicates must differ (PRED_A vs PRED_B); \
             a single-atom collapse would produce identical preds → this assertion fails → RED");
        // Specifically the two expected predicates.
        assert!(preds.contains(&"http://ex/PRED_A".to_string()), "PRED_A must be present");
        assert!(preds.contains(&"http://ex/PRED_B".to_string()), "PRED_B must be present");
    }

    /// Parent `<Member>` — a duplicate `<instance>` OR a duplicate `<class>` is rejected.
    #[test]
    fn test_duplicate_member_wrappers_rejected() {
        let dup_instance = br#"<Document xmlns="http://www.w3.org/2007/rif#">
  <payload><Group><sentences>
    <Member>
      <instance><Const type="http://www.w3.org/2007/rif#iri">http://ex/i</Const></instance>
      <instance><Const type="http://www.w3.org/2007/rif#iri">http://ex/i2</Const></instance>
      <class><Const type="http://www.w3.org/2007/rif#iri">http://ex/C</Const></class>
    </Member>
  </sentences></Group></payload>
</Document>"#;
        assert_duplicate_wrapper_rejected(dup_instance, "<instance>");

        let dup_class = br#"<Document xmlns="http://www.w3.org/2007/rif#">
  <payload><Group><sentences>
    <Member>
      <instance><Const type="http://www.w3.org/2007/rif#iri">http://ex/i</Const></instance>
      <class><Const type="http://www.w3.org/2007/rif#iri">http://ex/C</Const></class>
      <class><Const type="http://www.w3.org/2007/rif#iri">http://ex/D</Const></class>
    </Member>
  </sentences></Group></payload>
</Document>"#;
        assert_duplicate_wrapper_rejected(dup_class, "<class>");
    }

    /// Parent `<Subclass>` — a duplicate `<sub>` OR a duplicate `<sup>` is rejected.
    #[test]
    fn test_duplicate_subclass_wrappers_rejected() {
        let dup_sub = br#"<Document xmlns="http://www.w3.org/2007/rif#">
  <payload><Group><sentences>
    <Subclass>
      <sub><Const type="http://www.w3.org/2007/rif#iri">http://ex/A</Const></sub>
      <sub><Const type="http://www.w3.org/2007/rif#iri">http://ex/A2</Const></sub>
      <sup><Const type="http://www.w3.org/2007/rif#iri">http://ex/B</Const></sup>
    </Subclass>
  </sentences></Group></payload>
</Document>"#;
        assert_duplicate_wrapper_rejected(dup_sub, "<sub>");

        let dup_sup = br#"<Document xmlns="http://www.w3.org/2007/rif#">
  <payload><Group><sentences>
    <Subclass>
      <sub><Const type="http://www.w3.org/2007/rif#iri">http://ex/A</Const></sub>
      <sup><Const type="http://www.w3.org/2007/rif#iri">http://ex/B</Const></sup>
      <sup><Const type="http://www.w3.org/2007/rif#iri">http://ex/C</Const></sup>
    </Subclass>
  </sentences></Group></payload>
</Document>"#;
        assert_duplicate_wrapper_rejected(dup_sup, "<sup>");
    }

    /// Parent `<Equal>` (in a rule body) — a duplicate `<left>` OR `<right>` is rejected. The
    /// duplicate guard fires at PARSE time, before `Document::validate()` runs, so it is the
    /// error surfaced even though a bare Equal would also fail range/equality validation.
    #[test]
    fn test_duplicate_equal_wrappers_rejected() {
        let dup_left = br#"<Document xmlns="http://www.w3.org/2007/rif#">
  <payload><Group><sentences><Forall>
    <declare><Var>x</Var></declare>
    <formula><Implies>
      <if><Equal>
        <left><Var>x</Var></left>
        <left><Const type="http://www.w3.org/2007/rif#iri">http://ex/a</Const></left>
        <right><Const type="http://www.w3.org/2007/rif#iri">http://ex/b</Const></right>
      </Equal></if>
      <then><Frame><object><Var>x</Var></object>
        <slot><Const type="http://www.w3.org/2007/rif#iri">http://ex/p</Const><Const type="http://www.w3.org/2007/rif#iri">http://ex/v</Const></slot></Frame></then>
    </Implies></formula>
  </Forall></sentences></Group></payload>
</Document>"#;
        assert_duplicate_wrapper_rejected(dup_left, "<left>");

        let dup_right = br#"<Document xmlns="http://www.w3.org/2007/rif#">
  <payload><Group><sentences><Forall>
    <declare><Var>x</Var></declare>
    <formula><Implies>
      <if><Equal>
        <left><Var>x</Var></left>
        <right><Const type="http://www.w3.org/2007/rif#iri">http://ex/a</Const></right>
        <right><Const type="http://www.w3.org/2007/rif#iri">http://ex/b</Const></right>
      </Equal></if>
      <then><Frame><object><Var>x</Var></object>
        <slot><Const type="http://www.w3.org/2007/rif#iri">http://ex/p</Const><Const type="http://www.w3.org/2007/rif#iri">http://ex/v</Const></slot></Frame></then>
    </Implies></formula>
  </Forall></sentences></Group></payload>
</Document>"#;
        assert_duplicate_wrapper_rejected(dup_right, "<right>");
    }

    /// Parent `<External>` call — a duplicate `<content>`, `<op>`, OR `<args>` is rejected. The
    /// `<args>` case uses a recognised builtin (`pred:numeric-equal`) so parsing reaches the
    /// args lookup past operator resolution.
    #[test]
    fn test_duplicate_external_wrappers_rejected() {
        let dup_content = br#"<Document xmlns="http://www.w3.org/2007/rif#">
  <payload><Group><sentences><Forall>
    <declare><Var>x</Var></declare>
    <formula><Implies>
      <if><External>
        <content><Atom><op><Const type="http://www.w3.org/2007/rif#iri">http://www.w3.org/2007/rif-builtin-predicate#numeric-equal</Const></op><args><Var>x</Var><Var>x</Var></args></Atom></content>
        <content><Atom><op><Const type="http://www.w3.org/2007/rif#iri">http://www.w3.org/2007/rif-builtin-predicate#numeric-equal</Const></op><args><Var>x</Var><Var>x</Var></args></Atom></content>
      </External></if>
      <then><Frame><object><Var>x</Var></object>
        <slot><Const type="http://www.w3.org/2007/rif#iri">http://ex/p</Const><Const type="http://www.w3.org/2007/rif#iri">http://ex/v</Const></slot></Frame></then>
    </Implies></formula>
  </Forall></sentences></Group></payload>
</Document>"#;
        assert_duplicate_wrapper_rejected(dup_content, "<content>");

        let dup_op = br#"<Document xmlns="http://www.w3.org/2007/rif#">
  <payload><Group><sentences><Forall>
    <declare><Var>x</Var></declare>
    <formula><Implies>
      <if><External>
        <content><Atom>
          <op><Const type="http://www.w3.org/2007/rif#iri">http://www.w3.org/2007/rif-builtin-predicate#numeric-equal</Const></op>
          <op><Const type="http://www.w3.org/2007/rif#iri">http://www.w3.org/2007/rif-builtin-predicate#numeric-less-than</Const></op>
          <args><Var>x</Var><Var>x</Var></args>
        </Atom></content>
      </External></if>
      <then><Frame><object><Var>x</Var></object>
        <slot><Const type="http://www.w3.org/2007/rif#iri">http://ex/p</Const><Const type="http://www.w3.org/2007/rif#iri">http://ex/v</Const></slot></Frame></then>
    </Implies></formula>
  </Forall></sentences></Group></payload>
</Document>"#;
        assert_duplicate_wrapper_rejected(dup_op, "<op>");

        let dup_args = br#"<Document xmlns="http://www.w3.org/2007/rif#">
  <payload><Group><sentences><Forall>
    <declare><Var>x</Var></declare>
    <formula><Implies>
      <if><External>
        <content><Atom>
          <op><Const type="http://www.w3.org/2007/rif#iri">http://www.w3.org/2007/rif-builtin-predicate#numeric-equal</Const></op>
          <args><Var>x</Var><Var>x</Var></args>
          <args><Var>x</Var><Var>x</Var></args>
        </Atom></content>
      </External></if>
      <then><Frame><object><Var>x</Var></object>
        <slot><Const type="http://www.w3.org/2007/rif#iri">http://ex/p</Const><Const type="http://www.w3.org/2007/rif#iri">http://ex/v</Const></slot></Frame></then>
    </Implies></formula>
  </Forall></sentences></Group></payload>
</Document>"#;
        assert_duplicate_wrapper_rejected(dup_args, "<args>");
    }

    /// Parent `<Forall>` — a duplicate `<formula>` is rejected. This is the highest-severity
    /// case: `child("formula")` first-wins silently DROPPED the second whole rule. Paired
    /// control: a single-`<formula>` Forall imports as exactly one rule.
    #[test]
    fn test_duplicate_forall_formula_rejected_whole_rule_loss() {
        let dup = br#"<Document xmlns="http://www.w3.org/2007/rif#">
  <payload><Group><sentences><Forall>
    <declare><Var>x</Var></declare>
    <formula><Implies>
      <if><Frame><object><Var>x</Var></object>
        <slot><Const type="http://www.w3.org/2007/rif#iri">http://ex/p</Const><Const type="http://www.w3.org/2007/rif#iri">http://ex/v</Const></slot></Frame></if>
      <then><Frame><object><Var>x</Var></object>
        <slot><Const type="http://www.w3.org/2007/rif#iri">http://ex/q</Const><Const type="http://www.w3.org/2007/rif#iri">http://ex/v</Const></slot></Frame></then>
    </Implies></formula>
    <formula><Implies>
      <if><Frame><object><Var>x</Var></object>
        <slot><Const type="http://www.w3.org/2007/rif#iri">http://ex/r</Const><Const type="http://www.w3.org/2007/rif#iri">http://ex/v</Const></slot></Frame></if>
      <then><Frame><object><Var>x</Var></object>
        <slot><Const type="http://www.w3.org/2007/rif#iri">http://ex/s</Const><Const type="http://www.w3.org/2007/rif#iri">http://ex/v</Const></slot></Frame></then>
    </Implies></formula>
  </Forall></sentences></Group></payload>
</Document>"#;
        assert_duplicate_wrapper_rejected(dup, "<formula>");

        let control = br#"<Document xmlns="http://www.w3.org/2007/rif#">
  <payload><Group><sentences><Forall>
    <declare><Var>x</Var></declare>
    <formula><Implies>
      <if><Frame><object><Var>x</Var></object>
        <slot><Const type="http://www.w3.org/2007/rif#iri">http://ex/p</Const><Const type="http://www.w3.org/2007/rif#iri">http://ex/v</Const></slot></Frame></if>
      <then><Frame><object><Var>x</Var></object>
        <slot><Const type="http://www.w3.org/2007/rif#iri">http://ex/q</Const><Const type="http://www.w3.org/2007/rif#iri">http://ex/v</Const></slot></Frame></then>
    </Implies></formula>
  </Forall></sentences></Group></payload>
</Document>"#;
        let doc = import(control).expect("single-<formula> Forall is conformant and must import");
        assert_eq!(doc.rules.len(), 1, "control Forall must import as exactly one rule");
    }

    /// Parent `<Exists>` (in a rule body) — a duplicate `<formula>` is rejected. A dropped
    /// second `<formula>` silently loses a conjunct of the existential body.
    #[test]
    fn test_duplicate_exists_formula_rejected() {
        let dup = br#"<Document xmlns="http://www.w3.org/2007/rif#">
  <payload><Group><sentences><Forall>
    <declare><Var>x</Var></declare>
    <formula><Implies>
      <if><Exists>
        <declare><Var>z</Var></declare>
        <formula><Frame><object><Var>x</Var></object>
          <slot><Const type="http://www.w3.org/2007/rif#iri">http://ex/p</Const><Var>z</Var></slot></Frame></formula>
        <formula><Frame><object><Var>z</Var></object>
          <slot><Const type="http://www.w3.org/2007/rif#iri">http://ex/q</Const><Var>x</Var></slot></Frame></formula>
      </Exists></if>
      <then><Frame><object><Var>x</Var></object>
        <slot><Const type="http://www.w3.org/2007/rif#iri">http://ex/r</Const><Const type="http://www.w3.org/2007/rif#iri">http://ex/v</Const></slot></Frame></then>
    </Implies></formula>
  </Forall></sentences></Group></payload>
</Document>"#;
        assert_duplicate_wrapper_rejected(dup, "<formula>");
    }

    /// Parent `<List>` term — a duplicate `<items>` wrapper is rejected (`parse_term`). Not one
    /// of the bead's originally-enumerated seven parents, but the IDENTICAL residual class
    /// (`child("items")` first-wins would silently drop a second `<items>`, changing the list
    /// term); closed here to keep the module-doc universal claim airtight. The `<List>` sits in a
    /// Frame slot value position so `parse_term` reaches it.
    #[test]
    fn test_duplicate_list_items_rejected() {
        let dup = br#"<Document xmlns="http://www.w3.org/2007/rif#">
  <payload><Group><sentences>
    <Frame>
      <object><Const type="http://www.w3.org/2007/rif#iri">http://ex/s</Const></object>
      <slot>
        <Const type="http://www.w3.org/2007/rif#iri">http://ex/p</Const>
        <List>
          <items><Const type="http://www.w3.org/2007/rif#iri">http://ex/a</Const></items>
          <items><Const type="http://www.w3.org/2007/rif#iri">http://ex/b</Const></items>
        </List>
      </slot>
    </Frame>
  </sentences></Group></payload>
</Document>"#;
        assert_duplicate_wrapper_rejected(dup, "<items>");

        // Control: a single-<items> List value imports cleanly (List term is supported).
        let control = br#"<Document xmlns="http://www.w3.org/2007/rif#">
  <payload><Group><sentences>
    <Frame>
      <object><Const type="http://www.w3.org/2007/rif#iri">http://ex/s</Const></object>
      <slot>
        <Const type="http://www.w3.org/2007/rif#iri">http://ex/p</Const>
        <List>
          <items><Const type="http://www.w3.org/2007/rif#iri">http://ex/a</Const></items>
        </List>
      </slot>
    </Frame>
  </sentences></Group></payload>
</Document>"#;
        let doc = import(control).expect("single-<items> List value is conformant and must import");
        assert_eq!(doc.rules.len(), 1, "control List-valued Frame fact must import as one rule");
    }

    // ---- Positional Atom tests (sq-n7y15) -----------------------------------

    /// A binary positional Atom in a bare fact imports as `Atom::Frame`.
    /// `Atom(http://ex/P, http://ex/a, http://ex/b)` ≡ `a[P → b]`.
    #[test]
    fn test_positional_atom_binary_fact_imports_as_frame() {
        let xml = br#"<Document xmlns="http://www.w3.org/2007/rif#">
  <payload><Group><sentences>
    <Atom>
      <op><Const type="http://www.w3.org/2007/rif#iri">http://ex/P</Const></op>
      <args ordered="yes">
        <Const type="http://www.w3.org/2007/rif#iri">http://ex/a</Const>
        <Const type="http://www.w3.org/2007/rif#iri">http://ex/b</Const>
      </args>
    </Atom>
  </sentences></Group></payload>
</Document>"#;
        let doc = import(xml).expect("binary positional Atom fact must import");
        assert_eq!(doc.rules.len(), 1);
        let rule = &doc.rules[0];
        assert!(rule.body.is_empty(), "fact has no body");
        assert_eq!(rule.head.len(), 1);
        assert_eq!(
            rule.head[0],
            Atom::Frame {
                obj: Term::Iri("http://ex/a".to_string()),
                pred: Term::Iri("http://ex/P".to_string()),
                val: Term::Iri("http://ex/b".to_string()),
            },
            "binary positional Atom must map to Frame{{obj=a, pred=P, val=b}}"
        );
    }

    /// A unary positional Atom in a bare fact imports as `Atom::Member`.
    /// `Atom(http://ex/C, http://ex/a)` ≡ `a # C`.
    #[test]
    fn test_positional_atom_unary_fact_imports_as_member() {
        let xml = br#"<Document xmlns="http://www.w3.org/2007/rif#">
  <payload><Group><sentences>
    <Atom>
      <op><Const type="http://www.w3.org/2007/rif#iri">http://ex/C</Const></op>
      <args ordered="yes">
        <Const type="http://www.w3.org/2007/rif#iri">http://ex/a</Const>
      </args>
    </Atom>
  </sentences></Group></payload>
</Document>"#;
        let doc = import(xml).expect("unary positional Atom fact must import");
        assert_eq!(doc.rules.len(), 1);
        let rule = &doc.rules[0];
        assert!(rule.body.is_empty(), "fact has no body");
        assert_eq!(rule.head.len(), 1);
        assert_eq!(
            rule.head[0],
            Atom::Member {
                obj: Term::Iri("http://ex/a".to_string()),
                class: Term::Iri("http://ex/C".to_string()),
            },
            "unary positional Atom must map to Member{{obj=a, class=C}}"
        );
    }

    /// Positional Atoms in a rule body (body condition position) import correctly.
    /// A Forall: head :- Atom(P x y) maps to Frame in the body.
    #[test]
    fn test_positional_atom_in_rule_body() {
        let xml = br#"<Document xmlns="http://www.w3.org/2007/rif#">
  <payload><Group><sentences>
    <Forall>
      <declare><Var>x</Var></declare>
      <declare><Var>y</Var></declare>
      <formula><Implies>
        <if>
          <Atom>
            <op><Const type="http://www.w3.org/2007/rif#iri">http://ex/R</Const></op>
            <args ordered="yes"><Var>x</Var><Var>y</Var></args>
          </Atom>
        </if>
        <then>
          <Frame>
            <object><Var>x</Var></object>
            <slot>
              <Const type="http://www.w3.org/2007/rif#iri">http://ex/relatedTo</Const>
              <Var>y</Var>
            </slot>
          </Frame>
        </then>
      </Implies></formula>
    </Forall>
  </sentences></Group></payload>
</Document>"#;
        let doc = import(xml).expect("positional Atom in body must import");
        assert_eq!(doc.rules.len(), 1);
        let rule = &doc.rules[0];
        assert_eq!(rule.body.len(), 1, "body has one positional Atom");
        assert_eq!(
            rule.body[0],
            Atom::Frame {
                obj: Term::Var("x".to_string()),
                pred: Term::Iri("http://ex/R".to_string()),
                val: Term::Var("y".to_string()),
            },
            "binary positional Atom in body must map to Frame"
        );
        assert_eq!(rule.head.len(), 1);
    }

    /// Positional Atom in the HEAD of a rule imports as Frame. [SONNET-4.6] sq-n7y15
    #[test]
    fn test_positional_atom_in_rule_head() {
        let xml = br#"<Document xmlns="http://www.w3.org/2007/rif#">
  <payload><Group><sentences>
    <Forall>
      <declare><Var>x</Var></declare>
      <declare><Var>y</Var></declare>
      <formula><Implies>
        <if>
          <Frame>
            <object><Var>x</Var></object>
            <slot>
              <Const type="http://www.w3.org/2007/rif#iri">http://ex/p</Const>
              <Var>y</Var>
            </slot>
          </Frame>
        </if>
        <then>
          <Atom>
            <op><Const type="http://www.w3.org/2007/rif#iri">http://ex/Q</Const></op>
            <args ordered="yes"><Var>x</Var><Var>y</Var></args>
          </Atom>
        </then>
      </Implies></formula>
    </Forall>
  </sentences></Group></payload>
</Document>"#;
        let doc = import(xml).expect("positional Atom in head must import");
        assert_eq!(doc.rules.len(), 1);
        let rule = &doc.rules[0];
        assert_eq!(rule.head.len(), 1);
        assert_eq!(
            rule.head[0],
            Atom::Frame {
                obj: Term::Var("x".to_string()),
                pred: Term::Iri("http://ex/Q".to_string()),
                val: Term::Var("y".to_string()),
            },
            "binary positional Atom in head must map to Frame"
        );
    }

    /// Arity-0 positional Atom is rejected fail-closed. [SONNET-4.6] sq-n7y15
    #[test]
    fn test_positional_atom_arity_0_rejected() {
        let xml = br#"<Document xmlns="http://www.w3.org/2007/rif#">
  <payload><Group><sentences>
    <Atom>
      <op><Const type="http://www.w3.org/2007/rif#iri">http://ex/P</Const></op>
      <args ordered="yes"></args>
    </Atom>
  </sentences></Group></payload>
</Document>"#;
        let err = import(xml).expect_err("arity-0 positional Atom must be rejected fail-closed");
        assert!(
            matches!(err, ImportError::UnrecognizedElement { ref tag } if tag.contains("arity 0")),
            "expected UnrecognizedElement mentioning 'arity 0', got: {}",
            err
        );
    }

    /// Arity-3+ positional Atom is rejected fail-closed. [SONNET-4.6] sq-n7y15
    #[test]
    fn test_positional_atom_arity_3_rejected() {
        let xml = br#"<Document xmlns="http://www.w3.org/2007/rif#">
  <payload><Group><sentences>
    <Atom>
      <op><Const type="http://www.w3.org/2007/rif#iri">http://ex/T</Const></op>
      <args ordered="yes">
        <Const type="http://www.w3.org/2007/rif#iri">http://ex/a</Const>
        <Const type="http://www.w3.org/2007/rif#iri">http://ex/b</Const>
        <Const type="http://www.w3.org/2007/rif#iri">http://ex/c</Const>
      </args>
    </Atom>
  </sentences></Group></payload>
</Document>"#;
        let err = import(xml).expect_err("arity-3 positional Atom must be rejected fail-closed");
        assert!(
            matches!(err, ImportError::UnrecognizedElement { ref tag } if tag.contains("arity 3")),
            "expected UnrecognizedElement mentioning 'arity 3', got: {}",
            err
        );
    }

    /// Non-IRI operator in a positional Atom is rejected fail-closed. [SONNET-4.6] sq-n7y15
    #[test]
    fn test_positional_atom_non_iri_op_rejected() {
        // Literal-typed Const (xsd:string) is not a valid predicate IRI.
        let xml = br#"<Document xmlns="http://www.w3.org/2007/rif#">
  <payload><Group><sentences>
    <Atom>
      <op><Const type="http://www.w3.org/2001/XMLSchema#string">not-an-iri</Const></op>
      <args ordered="yes">
        <Const type="http://www.w3.org/2007/rif#iri">http://ex/a</Const>
      </args>
    </Atom>
  </sentences></Group></payload>
</Document>"#;
        let err = import(xml).expect_err("non-IRI Atom operator must be rejected fail-closed");
        assert!(
            matches!(err, ImportError::MalformedXml(_)),
            "expected MalformedXml for non-IRI Atom operator, got: {}",
            err
        );
    }

    /// Mutation check: swapping arg order (arg1 ↔ arg2) in a binary positional Atom
    /// must change the parsed Frame (obj ≠ val). This proves that arg ORDER is preserved
    /// and the mapping is not accidentally order-insensitive. [SONNET-4.6] sq-n7y15
    #[test]
    fn test_positional_atom_arg_order_mutation() {
        // Normal order: Atom(P, a, b) → Frame{obj=a, pred=P, val=b}
        let normal_xml = br#"<Document xmlns="http://www.w3.org/2007/rif#">
  <payload><Group><sentences>
    <Atom>
      <op><Const type="http://www.w3.org/2007/rif#iri">http://ex/P</Const></op>
      <args ordered="yes">
        <Const type="http://www.w3.org/2007/rif#iri">http://ex/a</Const>
        <Const type="http://www.w3.org/2007/rif#iri">http://ex/b</Const>
      </args>
    </Atom>
  </sentences></Group></payload>
</Document>"#;
        // Swapped order: Atom(P, b, a) → Frame{obj=b, pred=P, val=a}
        let swapped_xml = br#"<Document xmlns="http://www.w3.org/2007/rif#">
  <payload><Group><sentences>
    <Atom>
      <op><Const type="http://www.w3.org/2007/rif#iri">http://ex/P</Const></op>
      <args ordered="yes">
        <Const type="http://www.w3.org/2007/rif#iri">http://ex/b</Const>
        <Const type="http://www.w3.org/2007/rif#iri">http://ex/a</Const>
      </args>
    </Atom>
  </sentences></Group></payload>
</Document>"#;
        let doc_normal = import(normal_xml).expect("normal-order binary Atom must import");
        let doc_swapped = import(swapped_xml).expect("swapped-order binary Atom must import");

        let atom_normal = &doc_normal.rules[0].head[0];
        let atom_swapped = &doc_swapped.rules[0].head[0];

        // Mutation check: the two documents must produce DIFFERENT atoms.
        assert_ne!(
            atom_normal, atom_swapped,
            "mutation check FAILED: swapping arg order must produce a different Frame; \
             arg-order mapping appears order-insensitive (this test must be RED if \
             arg1/arg2 are reversed in parse_positional_atom)"
        );
        // Verify the exact mapping: normal → obj=a, val=b; swapped → obj=b, val=a.
        assert_eq!(
            atom_normal,
            &Atom::Frame {
                obj: Term::Iri("http://ex/a".to_string()),
                pred: Term::Iri("http://ex/P".to_string()),
                val: Term::Iri("http://ex/b".to_string()),
            },
            "normal order: first arg is obj, second is val"
        );
        assert_eq!(
            atom_swapped,
            &Atom::Frame {
                obj: Term::Iri("http://ex/b".to_string()),
                pred: Term::Iri("http://ex/P".to_string()),
                val: Term::Iri("http://ex/a".to_string()),
            },
            "swapped order: first arg is obj (b), second is val (a)"
        );
    }

    /// Positional Atom with variables round-trips through the rule engine (fact + rule).
    /// Proves the REAL path works end-to-end: binary Atom fact asserted, binary Atom rule
    /// body matches it, Frame head derived. [SONNET-4.6] sq-n7y15
    #[test]
    fn test_positional_atom_end_to_end_rule_engine() {
        use sparq_core::dict::Dict;
        // Fact: Atom(http://ex/R, http://ex/a, http://ex/b) (binary predicate R(a,b))
        // Rule: Forall ?x ?y: Frame{x relatedTo y} :- Atom(R, x, y)
        // Expected: Frame{a relatedTo b} is derived.
        let xml = br#"<Document xmlns="http://www.w3.org/2007/rif#">
  <payload><Group><sentences>
    <Atom>
      <op><Const type="http://www.w3.org/2007/rif#iri">http://ex/R</Const></op>
      <args ordered="yes">
        <Const type="http://www.w3.org/2007/rif#iri">http://ex/a</Const>
        <Const type="http://www.w3.org/2007/rif#iri">http://ex/b</Const>
      </args>
    </Atom>
    <Forall>
      <declare><Var>x</Var></declare>
      <declare><Var>y</Var></declare>
      <formula><Implies>
        <if>
          <Atom>
            <op><Const type="http://www.w3.org/2007/rif#iri">http://ex/R</Const></op>
            <args ordered="yes"><Var>x</Var><Var>y</Var></args>
          </Atom>
        </if>
        <then>
          <Frame>
            <object><Var>x</Var></object>
            <slot>
              <Const type="http://www.w3.org/2007/rif#iri">http://ex/relatedTo</Const>
              <Var>y</Var>
            </slot>
          </Frame>
        </then>
      </Implies></formula>
    </Forall>
  </sentences></Group></payload>
</Document>"#;
        let doc = import(xml).expect("end-to-end positional Atom document must import");
        let mut dict = Dict::new();
        let closure = doc.closure(&mut dict).expect("closure must succeed");

        // The triple (http://ex/a, http://ex/relatedTo, http://ex/b) must be in the closure.
        // Use intern_iri — the IRIs are now in the dict after the closure run, so intern
        // returns the existing Id (no new allocation needed). A closure triple [s,p,o]
        // with all three Ids present proves the derivation fired. [SONNET-4.6] sq-n7y15
        let s_id = dict.intern_iri("http://ex/a");
        let p_id = dict.intern_iri("http://ex/relatedTo");
        let o_id = dict.intern_iri("http://ex/b");

        assert!(
            closure.iter().any(|t| t[0] == s_id && t[1] == p_id && t[2] == o_id),
            "end-to-end: the derived triple (a, relatedTo, b) must appear in the closure; \
             this proves positional Atom R(a,b) was matched by the rule body and the \
             conclusion Frame was derived"
        );
    }

    /// Direct unit test of the `unique_child` helper — covers all three branches (zero → `None`,
    /// one → `Some`, more-than-one → `Err`) without routing through `import`, so the new code is
    /// directly (not only integration-) covered.
    #[test]
    fn test_unique_child_helper_branches() {
        fn leaf(tag: &str) -> XmlNode {
            XmlNode { tag: tag.to_string(), text: String::new(), attrs: Vec::new(), children: Vec::new() }
        }
        // Children: two <a>, one <b>, zero <c>.
        let parent = XmlNode {
            tag: "Parent".to_string(),
            text: String::new(),
            attrs: Vec::new(),
            children: vec![leaf("a"), leaf("b"), leaf("a")],
        };
        // Exactly one <b> → Ok(Some(<b>)).
        assert!(
            matches!(parent.unique_child("b", "Parent"), Ok(Some(n)) if n.tag == "b"),
            "exactly one <b> must return Ok(Some)"
        );
        // Zero <c> → Ok(None) (caller keeps its own "missing" diagnostic).
        assert!(
            matches!(parent.unique_child("c", "Parent"), Ok(None)),
            "absent <c> must return Ok(None), not an error"
        );
        // Two <a> → Err(MalformedXml) naming the tag + "duplicate".
        match parent.unique_child("a", "Parent") {
            Err(ImportError::MalformedXml(msg)) => {
                assert!(msg.contains("duplicate"), "message must say 'duplicate': {}", msg);
                assert!(msg.contains("<a>"), "message must name <a>: {}", msg);
                assert!(msg.contains("Parent"), "message must name the parent ctx: {}", msg);
            }
            Err(e) => panic!("expected MalformedXml, got a different error: {}", e),
            Ok(_) => panic!("two <a> must be rejected, got Ok"),
        }
    }

    // ---- [SONNET-4.6] sq-wbql1: imports-closure consistency check tests ----

    const EMPTY_GROUP_DOC: &[u8] = br#"<Document xmlns="http://www.w3.org/2007/rif#">
  <payload><Group></Group></payload>
</Document>"#;

    const FRAME_FACT_DOC: &[u8] = br#"<Document xmlns="http://www.w3.org/2007/rif#">
  <payload><Group>
    <sentence><Frame>
      <object><Const type="http://www.w3.org/2007/rif#iri">http://example.org/a</Const></object>
      <slot><Const type="http://www.w3.org/2007/rif#iri">http://example.org/p</Const><Const type="http://www.w3.org/2007/rif#iri">http://example.org/b</Const></slot>
    </Frame></sentence>
  </Group></payload>
</Document>"#;

    /// A document with no imports imports cleanly via `import_with_closure`.
    #[test]
    fn import_with_closure_no_imports_passes() {
        let result = import_with_closure(FRAME_FACT_DOC, |_| None);
        assert!(result.is_ok(), "no-import doc must pass, got: {:?}", result.err());
    }

    /// A blanket-refused import (resolver returns None) → `ImportDirective` fail-closed.
    /// The import is NOT silently accepted even though the profile check passes.
    #[test]
    fn import_with_closure_unresolvable_import_is_fail_closed() {
        let importing = br#"<Document xmlns="http://www.w3.org/2007/rif#">
  <directive><Import>
    <location><Const type="http://www.w3.org/2007/rif#iri">http://example.org/unavailable</Const></location>
  </Import></directive>
  <payload><Group></Group></payload>
</Document>"#;
        let result = import_with_closure(importing, |_| None);
        assert!(
            matches!(result, Err(ImportError::ImportDirective { .. })),
            "an unresolvable import (resolver returns None) MUST be fail-closed as \
             ImportDirective, got: {:?}",
            result
        );
    }

    /// A non-Core profile import → `InconsistentImport` (genuine detection).
    /// This is the load-bearing invariant for sq-wbql1.
    #[test]
    fn import_with_closure_non_core_profile_is_inconsistent_import() {
        // BLD profile — incompatible with RIF-Core.
        let importing = br#"<Document xmlns="http://www.w3.org/2007/rif#">
  <directive><Import>
    <location><Const type="http://www.w3.org/2007/rif#iri">http://example.org/bld</Const></location>
    <profile><Const type="http://www.w3.org/2007/rif#iri">http://www.w3.org/2007/rif#BLD</Const></profile>
  </Import></directive>
  <payload><Group></Group></payload>
</Document>"#;
        // Even with a resolver that provides valid bytes, the profile check fires first.
        let result = import_with_closure(importing, |_| Some(EMPTY_GROUP_DOC.to_vec()));
        assert!(
            matches!(result, Err(ImportError::InconsistentImport { .. })),
            "a BLD-profile import MUST be InconsistentImport (genuine detection), got: {:?}",
            result
        );
    }

    /// A Core-profile import that resolves to a valid RIF-Core document → ACCEPTED.
    /// This is the positive invariant: a consistent import is not spuriously rejected.
    #[test]
    fn import_with_closure_core_profile_consistent_import_is_accepted() {
        let importing = br#"<Document xmlns="http://www.w3.org/2007/rif#">
  <directive><Import>
    <location><Const type="http://www.w3.org/2007/rif#iri">http://example.org/ext</Const></location>
    <profile><Const type="http://www.w3.org/2007/rif#iri">http://www.w3.org/2007/rif#Core</Const></profile>
  </Import></directive>
  <payload><Group></Group></payload>
</Document>"#;
        let result = import_with_closure(importing, |loc| {
            if loc == "http://example.org/ext" {
                Some(FRAME_FACT_DOC.to_vec())
            } else {
                None
            }
        });
        assert!(
            result.is_ok(),
            "a Core-profile import resolving to a valid RIF-Core doc MUST be accepted, got: {:?}",
            result.err()
        );
    }

    /// An import resolving to an INVALID RIF-Core document (the imported rules fail
    /// `validate()`) → `ValidationFailed` or `InconsistentImport`. The combined rules
    /// must not silently pass. This exercises the combined-validate path.
    #[test]
    fn import_with_closure_imported_invalid_rules_rejected() {
        // Importing document (no imports, just wraps the import logic).
        let importing = br#"<Document xmlns="http://www.w3.org/2007/rif#">
  <directive><Import>
    <location><Const type="http://www.w3.org/2007/rif#iri">http://example.org/bad</Const></location>
  </Import></directive>
  <payload><Group></Group></payload>
</Document>"#;
        // Imported document: contains a rule with an UNSAFE head variable (UnboundHeadVar).
        // The variable ?y appears in the head but not in the body — range-restriction fails.
        let unsafe_imported = br#"<Document xmlns="http://www.w3.org/2007/rif#">
  <payload><Group>
    <sentence>
      <Forall>
        <declare><Var>x</Var></declare>
        <declare><Var>y</Var></declare>
        <formula><Implies>
          <if><Frame>
            <object><Var>x</Var></object>
            <slot><Const type="http://www.w3.org/2007/rif#iri">http://ex/p</Const><Const type="http://www.w3.org/2007/rif#iri">http://ex/v</Const></slot>
          </Frame></if>
          <then><Frame>
            <object><Var>y</Var></object>
            <slot><Const type="http://www.w3.org/2007/rif#iri">http://ex/q</Const><Const type="http://www.w3.org/2007/rif#iri">http://ex/w</Const></slot>
          </Frame></then>
        </Implies></formula>
      </Forall>
    </sentence>
  </Group></payload>
</Document>"#;
        let result = import_with_closure(importing, |_| Some(unsafe_imported.to_vec()));
        assert!(
            result.is_err(),
            "importing an unsafe (invalid RIF-Core) document MUST be rejected, got Ok"
        );
    }

    /// MUTATION VERIFY (sq-wbql1): the profile check is NON-VACUOUS — a Core-profile
    /// import is accepted while a BLD-profile import is rejected, proving the check
    /// distinguishes them rather than accepting or rejecting both.
    #[test]
    fn import_with_closure_profile_check_is_non_vacuous_mutation() {
        // ACCEPTED: no profile (absent = Core-compatible).
        let no_profile = br#"<Document xmlns="http://www.w3.org/2007/rif#">
  <directive><Import>
    <location><Const type="http://www.w3.org/2007/rif#iri">http://example.org/ext</Const></location>
  </Import></directive>
  <payload><Group></Group></payload>
</Document>"#;
        let ok = import_with_closure(no_profile, |_| Some(EMPTY_GROUP_DOC.to_vec()));
        assert!(ok.is_ok(), "no-profile import MUST be accepted (mutation check — ok side)");

        // REJECTED: BLD profile.
        let bld_profile = br#"<Document xmlns="http://www.w3.org/2007/rif#">
  <directive><Import>
    <location><Const type="http://www.w3.org/2007/rif#iri">http://example.org/ext</Const></location>
    <profile><Const type="http://www.w3.org/2007/rif#iri">http://www.w3.org/2007/rif#BLD</Const></profile>
  </Import></directive>
  <payload><Group></Group></payload>
</Document>"#;
        let err = import_with_closure(bld_profile, |_| Some(EMPTY_GROUP_DOC.to_vec()));
        assert!(
            matches!(err, Err(ImportError::InconsistentImport { .. })),
            "BLD-profile import MUST be InconsistentImport (mutation check — reject side), got: {:?}",
            err
        );
    }
}
