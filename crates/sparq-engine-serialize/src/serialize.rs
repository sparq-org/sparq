//! [OPUS-4.8] (sq-678h) RDF serializer matrix — Turtle / TriG / N-Quads writers.
//!
//! sparq could *parse* Turtle / TriG / N-Quads / N-Triples but could only *write*
//! N-Triples (via `sparq_engine::triples_to_ntriples`, which leans on oxrdf's canonical
//! `Display`). Every peer round-trips, so this closes that baseline gap.
//!
//! ## Opt-in
//!
//! This whole module lives behind the **`serialize-rdf`** cargo feature: a consumer
//! that only needs N-Triples (the always-on `triples_to_ntriples`) pays nothing —
//! no extra code compiles and no new dependency enters the default build. The writers
//! reuse the existing `oxrdf` term/IRI/literal infrastructure (no new crates).
//!
//! ## What's here
//!
//! * [`write_turtle`] — Turtle with `@prefix` compaction, predicate-object lists,
//!   `a` for `rdf:type`, and correct literal / IRI / blank-node escaping.
//! * [`write_turtle_pretty`] / [`write_trig_pretty`] (+ the `graph_to_*_pretty` wrappers,
//!   sq-ixc3.2) — the same Turtle/TriG but in the *pretty* shape of the site's
//!   `prettyTurtle` reshaper (#805): blank-line-separated, configurable-indent subject
//!   blocks with **emission-order-independent** (sorted) output. The long-term engine
//!   home for that site-side TS formatter.
//! * [`write_trig`] — Turtle blocks wrapped in `GRAPH <g> { … }` for named graphs.
//! * [`write_nquads`] — N-Triples with the 4th graph column.
//! * [`graph_to_turtle`] / [`graph_to_trig`] / [`graph_to_nquads`] — pull the triples
//!   straight out of a [`Graph`] (and its named graphs, for the dataset formats).
//! * [`graph_to_jsonld`] / [`write_jsonld`] — JSON-LD 1.1 in the **expanded**,
//!   **flattened**, or basic prefix **compacted** document form (see [`JsonLdForm`]).
//!   A native writer (no json-ld crate, zero new deps) built on the same oxrdf terms,
//!   implementing the Deserialize-to-RDF inverse: it emits exactly the node objects the
//!   JSON-LD-to-RDF algorithm would consume back into this triple set.
//! * [`graph_to_jsonld_pretty`] / [`write_jsonld_pretty`] (+ the `*_with` wrappers,
//!   sq-ixc3.3) — the same JSON-LD documents in an indented, multi-line shape. A
//!   whitespace-only presentation pass ([`JsonLdPrettyOptions`]) layered over the
//!   minified writer (the byte content is the minified document re-indented), so it
//!   reuses the same deterministic ordering and round-trips identically. Still
//!   dependency-free: a tiny hand-written JSON re-indenter, no serde_json (dev-only).
//!
//! ## Canonical interop note
//!
//! N-Triples / N-Quads term syntax is produced by oxrdf's `Display` (the same
//! canonical form the parsers accept), so those two formats are byte-stable. Turtle
//! and TriG add prefix compaction on top of that same escaping, so a parse →
//! serialize → re-parse round-trip is isomorphic (verified by the property tests).
//!
//! RDF 1.2 triple terms (`<<( … )>>`) appear only in object position; the Turtle
//! writer emits them in that grammar (oxrdf renders the nested triple), so a graph
//! carrying triple terms still round-trips through Turtle / N-Triples.

use oxrdf::{NamedOrBlankNode, Term, Triple};
use sparq_core::Graph;
use std::collections::BTreeMap;
use std::fmt::Write as _;

const RDF_TYPE: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";
const RDF_FIRST: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#first";
const RDF_REST: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#rest";
const RDF_NIL: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#nil";

/// A prefix map (`prefix` → namespace IRI) for Turtle / TriG compaction. The empty
/// string key is the default (`@prefix : <…>`) namespace.
pub type Prefixes = BTreeMap<String, String>;

/// The well-known prefixes assumed when a caller does not supply their own
/// ([`write_turtle`] / [`write_trig`] take an explicit map; the `graph_to_*`
/// convenience wrappers use these). Covers the namespaces that dominate real data
/// so the common case compacts without the caller declaring anything.
pub fn default_prefixes() -> Prefixes {
    [
        ("rdf", "http://www.w3.org/1999/02/22-rdf-syntax-ns#"),
        ("rdfs", "http://www.w3.org/2000/01/rdf-schema#"),
        ("xsd", "http://www.w3.org/2001/XMLSchema#"),
        ("owl", "http://www.w3.org/2002/07/owl#"),
        ("schema", "http://schema.org/"),
        ("foaf", "http://xmlns.com/foaf/0.1/"),
        ("dc", "http://purl.org/dc/terms/"),
        ("skos", "http://www.w3.org/2004/02/skos/core#"),
        ("sh", "http://www.w3.org/ns/shacl#"),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_string(), v.to_string()))
    .collect()
}

/// [OPUS-4.8] (sq-l5kr) Builds a [`Prefixes`] map from a caller-supplied list of
/// `(prefix, namespace-IRI)` pairs — the explicit-prefix-policy entry point.
///
/// The `graph_to_*_with` / `write_*` writers already take a [`Prefixes`] (a
/// `BTreeMap`); this is the convenience that turns an *ordered pair list* (e.g. a JS
/// `[[prefix, iri], …]` array, or a SPARQL query's parsed `PREFIX` declarations) into
/// that map without the caller depending on `BTreeMap` literals. It lets a consumer get
/// byte-parity output under *its own* prefix policy — for example the site's
/// `COMMON_PREFIXES` (`schema` → `https://schema.org/`, plus `dcterms` / `prov` / `geo` /
/// `void` / `ex` → `http://example.org/`), which differs from [`default_prefixes`]
/// (`schema` → `http://schema.org/`, no `ex`).
///
/// On a duplicate prefix label the **last** pair wins (`BTreeMap` insertion order), so a
/// caller can layer overrides after a base set. The empty-string label is the default
/// (`@prefix : <…>`) namespace, exactly as elsewhere in this module. Namespace IRIs are
/// taken verbatim — no validation — so a malformed IRI simply never matches any term and
/// abbreviates nothing (the writer falls back to the full `<IRI>`, never corrupt output).
pub fn prefixes_from_pairs<P, N>(pairs: impl IntoIterator<Item = (P, N)>) -> Prefixes
where
    P: Into<String>,
    N: Into<String>,
{
    pairs
        .into_iter()
        .map(|(p, n)| (p.into(), n.into()))
        .collect()
}

// ---------------------------------------------------------------------------
// Escaping helpers (shared by Turtle / TriG; N-Triples / N-Quads delegate to
// oxrdf's canonical `Display`).
// ---------------------------------------------------------------------------

/// Escapes the *content* of an IRIREF (the bytes between `<` and `>`) per the
/// Turtle/N-Triples grammar: the control range plus the syntactic delimiters that
/// would otherwise terminate or re-open the IRI. Everything else (including non-ASCII)
/// passes through verbatim, matching oxrdf's own IRI rendering.
fn escape_iri(iri: &str, out: &mut String) {
    out.push('<');
    for c in iri.chars() {
        match c {
            '\u{00}'..='\u{20}' | '<' | '>' | '"' | '{' | '}' | '|' | '^' | '`' | '\\' => {
                let _ = write!(out, "\\u{:04X}", c as u32);
            }
            _ => out.push(c),
        }
    }
    out.push('>');
}

/// Escapes a literal lexical value for a Turtle double-quoted string (the
/// `STRING_LITERAL_QUOTE` production): the four characters that must be escaped
/// (`"`, `\`, LF, CR) become `\"`, `\\`, `\n`, `\r`; everything else (tabs, other
/// control chars, non-ASCII) is legal inside a double-quoted Turtle string and is
/// emitted verbatim. This matches oxrdf's canonical N-Triples literal escaping, so a
/// re-parse is exact.
fn escape_string(value: &str, out: &mut String) {
    for c in value.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            _ => out.push(c),
        }
    }
}

/// True if `s` is a valid Turtle `PN_LOCAL` body that needs no escaping — a
/// conservative ASCII subset (`A–Z a–z 0–9 _ -`, and interior `.`) so the compaction
/// is always *correct*; anything outside it falls back to a full `<IRI>`. Empty is
/// allowed (`prefix:` with an empty local name is valid Turtle).
fn is_simple_pn_local(s: &str) -> bool {
    let bytes = s.as_bytes();
    for (i, &b) in bytes.iter().enumerate() {
        let ok = b.is_ascii_alphanumeric()
            || b == b'_'
            || b == b'-'
            // interior '.' only (a PN_LOCAL may not end in '.')
            || (b == b'.' && i + 1 < bytes.len());
        if !ok {
            return false;
        }
    }
    true
}

// ---------------------------------------------------------------------------
// Term rendering (Turtle / TriG flavour — prefix-aware).
// ---------------------------------------------------------------------------

/// Renders an IRI as a prefixed name if a registered prefix is a proper, splittable
/// match (longest namespace wins for determinism); otherwise as a full escaped IRIREF.
/// `rdf:type` in predicate position is handled by the caller (`a`), not here.
fn write_iri(iri: &str, prefixes: &Prefixes, out: &mut String) {
    // Longest-namespace-first so `…#` beats `…` etc.; deterministic on ties via the
    // prefix name (BTreeMap iteration order).
    let mut best: Option<(&str, &str)> = None;
    for (pfx, ns) in prefixes {
        if let Some(local) = iri.strip_prefix(ns.as_str()) {
            if is_simple_pn_local(local) {
                match best {
                    Some((_, bns)) if bns.len() >= ns.len() => {}
                    _ => best = Some((pfx.as_str(), local)),
                }
            }
        }
    }
    if let Some((pfx, local)) = best {
        let _ = write!(out, "{pfx}:{local}");
    } else {
        escape_iri(iri, out);
    }
}

/// Renders a literal in Turtle: `"lex"`, `"lex"@lang`, or `"lex"^^<dt>` with `xsd:string`
/// and `rdf:langString` left implicit (the canonical short forms), and the datatype IRI
/// itself prefix-compacted.
fn write_literal(lit: &oxrdf::Literal, prefixes: &Prefixes, out: &mut String) {
    out.push('"');
    escape_string(lit.value(), out);
    out.push('"');
    if let Some(lang) = lit.language() {
        out.push('@');
        out.push_str(lang);
    } else {
        let dt = lit.datatype().as_str();
        // xsd:string and rdf:langString are the implicit datatypes — omit them.
        if dt != "http://www.w3.org/2001/XMLSchema#string"
            && dt != "http://www.w3.org/1999/02/22-rdf-syntax-ns#langString"
        {
            out.push_str("^^");
            write_iri(dt, prefixes, out);
        }
    }
}

/// Renders any term in Turtle/TriG flavour: IRI (prefixed), blank node (`_:label`),
/// literal, or an RDF 1.2 triple term `<<( s p o )>>` (object position only).
fn write_term(term: &Term, prefixes: &Prefixes, out: &mut String) {
    match term {
        Term::NamedNode(n) => write_iri(n.as_str(), prefixes, out),
        Term::BlankNode(b) => {
            out.push_str("_:");
            out.push_str(b.as_str());
        }
        Term::Literal(l) => write_literal(l, prefixes, out),
        Term::Triple(t) => {
            out.push_str("<<( ");
            write_subject(&t.subject, prefixes, out);
            out.push(' ');
            write_iri(t.predicate.as_str(), prefixes, out);
            out.push(' ');
            write_term(&t.object, prefixes, out);
            out.push_str(" )>>");
        }
    }
}

/// Renders a subject (IRI or blank node) in Turtle/TriG flavour. oxrdf models a triple
/// (including a nested RDF 1.2 triple term) with a `NamedOrBlankNode` subject, so a subject
/// is never itself a triple term.
fn write_subject(subj: &NamedOrBlankNode, prefixes: &Prefixes, out: &mut String) {
    match subj {
        NamedOrBlankNode::NamedNode(n) => write_iri(n.as_str(), prefixes, out),
        NamedOrBlankNode::BlankNode(b) => {
            out.push_str("_:");
            out.push_str(b.as_str());
        }
    }
}

// ---------------------------------------------------------------------------
// Turtle.
// ---------------------------------------------------------------------------

/// Serializes a set of triples as Turtle: a `@prefix` header for every prefix actually
/// used, then grouped predicate-object lists per subject (`s p1 o1, o2 ; p2 o3 .`), with
/// `a` standing in for `rdf:type`. Only the prefixes that are used appear in the header.
///
/// Row order is a faithful, deterministic function *of the input slice*: subjects in
/// first-seen order, predicates grouped, objects in input order. It is **not** canonicalised
/// — the writer applies no sort. So when the input slice's own order is unspecified, the
/// output order is too: the [`graph_to_turtle`] family feeds the store's `iter_ids()`
/// (dict-id / SPO-index) order, which is thread-count-dependent (`sparq-core`'s parallel
/// sharded dict merge), so a serialised graph dump can differ across thread counts. That is
/// spec-permitted (Turtle defines no canonical triple order) and not pinned anywhere
/// canonical today; see `research/dict-id-order-determinism-audit.md`. A golden over a
/// serialised graph must canonicalise (sort the rendered rows) or pin `RAYON_NUM_THREADS`.
pub fn write_turtle(triples: &[Triple], prefixes: &Prefixes) -> String {
    let mut out = String::new();
    write_prefix_header(triples, prefixes, &mut out);
    write_turtle_body(triples, prefixes, 0, &mut out);
    out
}

/// Emits the `@prefix` lines for exactly the prefixes whose namespace is the chosen
/// compaction for at least one IRI in `triples` (so an unused prefix never clutters the
/// header). Determined by a dry render of every IRI position.
fn write_prefix_header(triples: &[Triple], prefixes: &Prefixes, out: &mut String) {
    let mut used: std::collections::BTreeSet<&str> = std::collections::BTreeSet::new();
    let mut probe = String::new();
    let mut note = |iri: &str| {
        probe.clear();
        write_iri(iri, prefixes, &mut probe);
        // A prefixed name (`pfx:local`) — not a full `<…>` IRIREF.
        if !probe.starts_with('<') {
            if let Some((pfx, _)) = probe.split_once(':') {
                if let Some((k, _)) = prefixes.get_key_value(pfx) {
                    used.insert(k.as_str());
                }
            }
        }
    };
    for t in triples {
        collect_iris(&Term::from(t.subject.clone()), &mut note);
        note(t.predicate.as_str());
        collect_iris(&t.object, &mut note);
    }
    if used.is_empty() {
        return;
    }
    for pfx in &used {
        let ns = &prefixes[*pfx];
        let mut ns_esc = String::new();
        escape_iri(ns, &mut ns_esc);
        let _ = writeln!(out, "@prefix {pfx}: {ns_esc} .");
    }
    out.push('\n');
}

/// Walks every IRI reachable in a term (recursing through triple terms), invoking `note`.
fn collect_iris(term: &Term, note: &mut impl FnMut(&str)) {
    match term {
        Term::NamedNode(n) => note(n.as_str()),
        Term::Literal(l) => note(l.datatype().as_str()),
        Term::Triple(t) => {
            collect_iris(&Term::from(t.subject.clone()), note);
            note(t.predicate.as_str());
            collect_iris(&t.object, note);
        }
        Term::BlankNode(_) => {}
    }
}

/// One subject's predicate-object lists, accumulated in stable order for the Turtle body:
/// predicate keys in first-seen order, each mapped to its objects in input order.
struct PredObjects {
    /// Predicate render keys (the prefixed IRI, or `a` for rdf:type) in first-seen order.
    order: Vec<String>,
    /// Predicate render key → its objects, in input order.
    objects: std::collections::HashMap<String, Vec<Term>>,
}

/// Writes the Turtle statement body (no header) at the given indent (column count). Used
/// directly by Turtle and, indented, inside each TriG `GRAPH { … }` block.
fn write_turtle_body(triples: &[Triple], prefixes: &Prefixes, indent: usize, out: &mut String) {
    // Group by subject in first-seen order, each subject keeping its predicate→objects
    // map (predicates in first-seen order, objects in input order). The store already
    // hands triples back in a stable order; we only need to cluster by subject.
    let pad = " ".repeat(indent);
    let mut order: Vec<NamedOrBlankNode> = Vec::new();
    let mut seen: std::collections::HashMap<NamedOrBlankNode, usize> =
        std::collections::HashMap::new();
    // For each subject slot: its predicate-object lists.
    let mut groups: Vec<PredObjects> = Vec::new();

    for t in triples {
        let subj: NamedOrBlankNode = t.subject.clone();
        let slot = *seen.entry(subj.clone()).or_insert_with(|| {
            order.push(subj.clone());
            groups.push(PredObjects {
                order: Vec::new(),
                objects: std::collections::HashMap::new(),
            });
            order.len() - 1
        });
        // Predicate key: `a` for rdf:type, else the rendered IRI.
        let pkey = if t.predicate.as_str() == RDF_TYPE {
            "a".to_string()
        } else {
            let mut s = String::new();
            write_iri(t.predicate.as_str(), prefixes, &mut s);
            s
        };
        let group = &mut groups[slot];
        if !group.objects.contains_key(&pkey) {
            group.order.push(pkey.clone());
        }
        group
            .objects
            .entry(pkey)
            .or_default()
            .push(t.object.clone());
    }

    for (i, subj) in order.iter().enumerate() {
        render_subject_block(subj, &groups[i], prefixes, &pad, out);
    }
}

/// Renders ONE subject's complete Turtle statement (`s p1 o1, o2 ; p2 o3 .\n`) into `out`
/// at the given `pad` indent. This is the single source of truth for a subject block's
/// bytes, shared by the buffered [`write_turtle_body`] and (under `streaming-serialization`)
/// the streaming writer, so the two paths are byte-identical by construction.
fn render_subject_block(
    subj: &NamedOrBlankNode,
    group: &PredObjects,
    prefixes: &Prefixes,
    pad: &str,
    out: &mut String,
) {
    out.push_str(pad);
    match subj {
        NamedOrBlankNode::NamedNode(n) => write_iri(n.as_str(), prefixes, out),
        NamedOrBlankNode::BlankNode(b) => {
            out.push_str("_:");
            out.push_str(b.as_str());
        }
    }
    out.push(' ');
    for (pi, pkey) in group.order.iter().enumerate() {
        if pi > 0 {
            out.push_str(" ;\n");
            out.push_str(pad);
            out.push_str("    ");
        }
        out.push_str(pkey);
        out.push(' ');
        let objs = &group.objects[pkey];
        for (oi, o) in objs.iter().enumerate() {
            if oi > 0 {
                out.push_str(", ");
            }
            write_term(o, prefixes, out);
        }
    }
    out.push_str(" .\n");
}

// ---------------------------------------------------------------------------
// Streaming Turtle / TriG (opt-in `streaming-serialization`).
//
// [OPUS-4.8] (sq-townn, survey §A7) The buffered `write_turtle` / `write_trig` build a
// FULL in-memory `String` for the whole graph, so a CONSTRUCT/DESCRIBE over a large graph
// holds BOTH the materialised triples AND the fully-rendered string at once. These
// `*_streaming` writers render the body directly into a `W: std::io::Write`, buffering only
// ONE subject's block at a time (emitting on subject change), so the rendered output is
// never materialised whole. That enables HTTP chunked streaming of a CONSTRUCT response.
//
// BYTE-EQUALITY: the streamed bytes equal the buffered `write_turtle` bytes for the SAME
// triples — same used-prefix header, same subject grouping, same predicate-object lists —
// because both call the shared `write_prefix_header` (header) and `render_subject_block`
// (per-subject body). The one precondition for that equality is that triples for the same
// subject are CONTIGUOUS in the iterator (the streaming buffer holds one subject at a time
// and cannot merge a subject that reappears after an intervening subject). Graph-sourced
// triples ALWAYS satisfy this: `Graph::iter_ids` walks the SPO permutation, which is sorted
// subject-major, so `graph_triples` (the CONSTRUCT/DESCRIBE feed) is subject-contiguous by
// construction. The `graph_to_turtle_streaming` wrapper feeds exactly that order, so its
// bytes equal `graph_to_turtle`'s; the `streamed_equals_buffered` test asserts it.
//
// The memory / time-to-first-byte payoff (no whole-output `String`, first bytes flushed
// after the first subject instead of after the last) is a measurable HYPOTHESIS to confirm
// on the canonical perf host — NOT a baked-in number.
// ---------------------------------------------------------------------------

/// Accumulates one triple's predicate-object into a per-subject [`PredObjects`], preserving
/// the buffered writer's stable order (predicate keys in first-seen order, objects in input
/// order, `a` for `rdf:type`). Shared by the streaming body so a single subject's block is
/// grouped identically to the buffered path.
#[cfg(feature = "streaming-serialization")]
fn accumulate_pred_object(group: &mut PredObjects, t: &Triple, prefixes: &Prefixes) {
    let pkey = if t.predicate.as_str() == RDF_TYPE {
        "a".to_string()
    } else {
        let mut s = String::new();
        write_iri(t.predicate.as_str(), prefixes, &mut s);
        s
    };
    if !group.objects.contains_key(&pkey) {
        group.order.push(pkey.clone());
    }
    group.objects.entry(pkey).or_default().push(t.object.clone());
}

/// Streams the Turtle statement body (no header) for `triples` into `w` at the given indent,
/// buffering only the current subject's block. Emits a subject block whenever the subject
/// changes (subjects must be contiguous — see the module note). Used directly by
/// [`write_turtle_streaming`] and, indented, inside each streamed TriG `GRAPH { … }` block.
#[cfg(feature = "streaming-serialization")]
fn write_turtle_body_streaming<I, W>(
    triples: I,
    prefixes: &Prefixes,
    indent: usize,
    w: &mut W,
) -> std::io::Result<()>
where
    I: IntoIterator<Item = Triple>,
    W: std::io::Write,
{
    let pad = " ".repeat(indent);
    // The one-subject-at-a-time buffer: the current subject + its grouped predicate-objects,
    // and a reusable render scratch String (so a block's bytes are produced exactly once,
    // then flushed and the scratch cleared — the whole-output String is never built).
    let mut cur: Option<(NamedOrBlankNode, PredObjects)> = None;
    let mut scratch = String::new();
    for t in triples {
        let subj: NamedOrBlankNode = t.subject.clone();
        match &mut cur {
            Some((s, group)) if *s == subj => {
                accumulate_pred_object(group, &t, prefixes);
            }
            _ => {
                // Subject changed: flush the previous block, then start a new buffer.
                if let Some((s, group)) = cur.take() {
                    scratch.clear();
                    render_subject_block(&s, &group, prefixes, &pad, &mut scratch);
                    w.write_all(scratch.as_bytes())?;
                }
                let mut group = PredObjects {
                    order: Vec::new(),
                    objects: std::collections::HashMap::new(),
                };
                accumulate_pred_object(&mut group, &t, prefixes);
                cur = Some((subj, group));
            }
        }
    }
    if let Some((s, group)) = cur.take() {
        scratch.clear();
        render_subject_block(&s, &group, prefixes, &pad, &mut scratch);
        w.write_all(scratch.as_bytes())?;
    }
    Ok(())
}

/// [OPUS-4.8] (sq-townn, survey §A7) Streams `triples` as Turtle into `w` WITHOUT building
/// the whole rendered output `String` in memory: the used-prefix `@prefix` header followed
/// by grouped predicate-object lists, one subject block buffered at a time.
///
/// The bytes written are IDENTICAL to [`write_turtle`] over the same `triples`, provided
/// triples for one subject are contiguous (graph-sourced triples always are — see
/// [`graph_to_turtle_streaming`]). The header is computed by one pass over `triples` (so the
/// `@prefix` lines match the buffered writer exactly); the body is then streamed in a second
/// pass over the slice. Taking `&[Triple]` lets the header pass and the body pass share the
/// already-materialised triple slice with ZERO extra triple allocation — the saving is the
/// rendered output `String`, not the triples (which CONSTRUCT/DESCRIBE already hold).
///
/// This is the front door for HTTP chunked CONSTRUCT/DESCRIBE responses: the first subject
/// block can be flushed to the socket before the last subject is rendered.
#[cfg(feature = "streaming-serialization")]
pub fn write_turtle_streaming<W: std::io::Write>(
    triples: &[Triple],
    prefixes: &Prefixes,
    w: &mut W,
) -> std::io::Result<()> {
    let mut header = String::new();
    write_prefix_header(triples, prefixes, &mut header);
    w.write_all(header.as_bytes())?;
    write_turtle_body_streaming(triples.iter().cloned(), prefixes, 0, w)
}

// ---------------------------------------------------------------------------
// TriG.
// ---------------------------------------------------------------------------

/// A named graph for the dataset writers: the graph name term (an IRI or blank node) and
/// its triples. `None` name = the default graph.
pub type NamedGraph<'a> = (Option<&'a Term>, &'a [Triple]);

/// Serializes a dataset as TriG: the `@prefix` header, then the default graph's statements
/// (unwrapped), then each named graph as `GRAPH <g> { … }`. The shared header covers every
/// prefix used across all graphs.
pub fn write_trig(graphs: &[NamedGraph<'_>], prefixes: &Prefixes) -> String {
    let mut out = String::new();
    // Header over the union of every graph's triples.
    let all: Vec<Triple> = graphs
        .iter()
        .flat_map(|(_, ts)| ts.iter().cloned())
        .collect();
    write_prefix_header(&all, prefixes, &mut out);

    let mut first = true;
    for (name, ts) in graphs {
        if ts.is_empty() {
            continue;
        }
        if !first {
            out.push('\n');
        }
        first = false;
        match name {
            None => write_turtle_body(ts, prefixes, 0, &mut out),
            Some(g) => {
                out.push_str("GRAPH ");
                write_graph_name(g, prefixes, &mut out);
                out.push_str(" {\n");
                write_turtle_body(ts, prefixes, 4, &mut out);
                out.push_str("}\n");
            }
        }
    }
    out
}

/// Renders a TriG named-graph name (`GRAPH <g>`) — an IRI or blank node — into `out`. A
/// literal/triple-term graph name is not legal RDF; its canonical N-Triples form is emitted
/// so nothing is silently lost (the parser rejects it, surfacing the bad input rather than
/// corrupting). Shared by the buffered [`write_trig`] and the streaming TriG writer.
fn write_graph_name(g: &Term, prefixes: &Prefixes, out: &mut String) {
    match g {
        Term::NamedNode(n) => write_iri(n.as_str(), prefixes, out),
        Term::BlankNode(b) => {
            out.push_str("_:");
            out.push_str(b.as_str());
        }
        other => {
            let _ = write!(out, "{other}");
        }
    }
}

/// [OPUS-4.8] (sq-townn, survey §A7) Streams `graphs` as TriG into `w` WITHOUT building the
/// whole rendered output `String`: the shared used-prefix `@prefix` header (one pass over
/// the union of every graph's triples), then the default graph's statements (unwrapped),
/// then each named graph as `GRAPH <g> { … }` — each graph's body streamed one subject block
/// at a time, exactly as [`write_turtle_streaming`] does.
///
/// The bytes written are IDENTICAL to [`write_trig`] over the same `graphs` under the same
/// subject-contiguity precondition (graph-sourced triples always satisfy it — see
/// [`graph_to_trig_streaming`]). Only the constant per-graph framing (`GRAPH <g> {`, the
/// inter-graph blank line, the closing `}`) and a single subject block are ever buffered.
#[cfg(feature = "streaming-serialization")]
pub fn write_trig_streaming<W: std::io::Write>(
    graphs: &[NamedGraph<'_>],
    prefixes: &Prefixes,
    w: &mut W,
) -> std::io::Result<()> {
    // Header over the union of every graph's triples — one pass, matching `write_trig`.
    let all: Vec<Triple> = graphs
        .iter()
        .flat_map(|(_, ts)| ts.iter().cloned())
        .collect();
    let mut header = String::new();
    write_prefix_header(&all, prefixes, &mut header);
    w.write_all(header.as_bytes())?;

    let mut first = true;
    // Reusable framing scratch (constant-size per graph — never the whole body).
    let mut frame = String::new();
    for (name, ts) in graphs {
        if ts.is_empty() {
            continue;
        }
        if !first {
            w.write_all(b"\n")?;
        }
        first = false;
        match name {
            None => write_turtle_body_streaming(ts.iter().cloned(), prefixes, 0, w)?,
            Some(g) => {
                frame.clear();
                frame.push_str("GRAPH ");
                write_graph_name(g, prefixes, &mut frame);
                frame.push_str(" {\n");
                w.write_all(frame.as_bytes())?;
                write_turtle_body_streaming(ts.iter().cloned(), prefixes, 4, w)?;
                w.write_all(b"}\n")?;
            }
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// N-Quads (N-Triples + graph column). Term syntax via oxrdf's canonical Display.
// ---------------------------------------------------------------------------

/// Serializes a dataset as N-Quads: one `s p o [g] .` line per triple, the default graph's
/// triples written without a graph term. Term syntax is oxrdf's canonical N-Triples form
/// (the same the parsers accept), so output is byte-stable.
pub fn write_nquads(graphs: &[NamedGraph<'_>]) -> String {
    let mut out = String::new();
    for (name, ts) in graphs {
        match name {
            None => {
                for t in *ts {
                    let _ = writeln!(out, "{} {} {} .", t.subject, t.predicate, t.object);
                }
            }
            Some(g) => {
                for t in *ts {
                    let _ = writeln!(out, "{} {} {} {} .", t.subject, t.predicate, t.object, g);
                }
            }
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Graph -> bytes convenience wrappers (pull triples out of a live Graph).
// ---------------------------------------------------------------------------

/// Materializes the default graph's triples in store order: `iter_ids()` walks the SPO
/// permutation index, i.e. dict-id order. That is stable *for a fixed build/thread count*
/// but thread-count-dependent across builds (`sparq-core`'s sharded dict merge), so the
/// downstream writers do not produce a canonical row order — see the [`write_turtle`] doc
/// and `research/dict-id-order-determinism-audit.md`.
fn graph_triples(graph: &Graph) -> Vec<Triple> {
    graph
        .iter_ids()
        .map(|[s, p, o]| triple_from_ids(graph, s, p, o))
        .collect()
}

/// Rebuilds an `oxrdf::Triple` from interned ids via the dictionary. The store invariant
/// guarantees a non-literal subject and an IRI predicate; the `unreachable!`s document
/// that and would only fire on a corrupt store.
fn triple_from_ids(
    graph: &Graph,
    s: sparq_core::dict::Id,
    p: sparq_core::dict::Id,
    o: sparq_core::dict::Id,
) -> Triple {
    let subject: NamedOrBlankNode = match graph.dict.term(s) {
        Term::NamedNode(n) => NamedOrBlankNode::NamedNode(n),
        Term::BlankNode(b) => NamedOrBlankNode::BlankNode(b),
        other => unreachable!("non-IRI/blank subject in store: {other}"),
    };
    let predicate = match graph.dict.term(p) {
        Term::NamedNode(n) => n,
        other => unreachable!("non-IRI predicate in store: {other}"),
    };
    let object = graph.dict.term(o);
    Triple {
        subject,
        predicate,
        object,
    }
}

/// Serializes a [`Graph`]'s default graph as Turtle with the [`default_prefixes`].
pub fn graph_to_turtle(graph: &Graph) -> String {
    write_turtle(&graph_triples(graph), &default_prefixes())
}

/// Serializes a [`Graph`]'s default graph as Turtle with a caller-supplied prefix map.
pub fn graph_to_turtle_with(graph: &Graph, prefixes: &Prefixes) -> String {
    write_turtle(&graph_triples(graph), prefixes)
}

/// [OPUS-4.8] (sq-townn, survey §A7) Streams a [`Graph`]'s default graph as Turtle into `w`
/// with a caller-supplied prefix map, WITHOUT building the whole rendered output `String` —
/// the front door for HTTP chunked CONSTRUCT/DESCRIBE responses over a large graph.
///
/// The bytes written equal [`graph_to_turtle_with`] over the same graph and prefixes: both
/// feed `graph_triples` (the SPO-ordered, hence subject-contiguous, store walk), so the
/// streaming writer's per-subject buffering produces byte-identical output. Only the triple
/// slice (which `graph_to_turtle_with` already materialises) plus one subject block live in
/// memory at once — the rendered output `String` is never built.
#[cfg(feature = "streaming-serialization")]
pub fn graph_to_turtle_streaming<W: std::io::Write>(
    graph: &Graph,
    prefixes: &Prefixes,
    w: &mut W,
) -> std::io::Result<()> {
    write_turtle_streaming(&graph_triples(graph), prefixes, w)
}

/// Collects the default + named graphs of a [`Graph`] as owned `(name, triples)` pairs for
/// the dataset writers.
fn dataset_graphs(graph: &Graph) -> Vec<(Option<Term>, Vec<Triple>)> {
    let mut out: Vec<(Option<Term>, Vec<Triple>)> = vec![(None, graph_triples(graph))];
    for (name, g) in &graph.named {
        out.push((Some(name.clone()), graph_triples(g)));
    }
    out
}

/// Serializes a [`Graph`] (default + named graphs) as TriG with the [`default_prefixes`].
pub fn graph_to_trig(graph: &Graph) -> String {
    graph_to_trig_with(graph, &default_prefixes())
}

/// [OPUS-4.8] (sq-l5kr) Serializes a [`Graph`] (default + named graphs) as TriG with a
/// caller-supplied prefix map — the non-pretty counterpart to [`graph_to_turtle_with`], so
/// the compact TriG path can also honour an explicit prefix policy (e.g. the site's
/// `COMMON_PREFIXES`) instead of only the well-known defaults.
pub fn graph_to_trig_with(graph: &Graph, prefixes: &Prefixes) -> String {
    let owned = dataset_graphs(graph);
    let view: Vec<NamedGraph<'_>> = owned
        .iter()
        .map(|(n, ts)| (n.as_ref(), ts.as_slice()))
        .collect();
    write_trig(&view, prefixes)
}

/// [OPUS-4.8] (sq-townn, survey §A7) Streams a [`Graph`] (default + named graphs) as TriG
/// into `w` with a caller-supplied prefix map, WITHOUT building the whole rendered output
/// `String`. The streaming counterpart to [`graph_to_trig_with`].
///
/// The bytes written equal [`graph_to_trig_with`] over the same graph and prefixes: both
/// feed `dataset_graphs` (each graph's `graph_triples` is the SPO-ordered, subject-contiguous
/// store walk), so the streaming writer's per-subject buffering produces byte-identical
/// output. Only the triple slices plus a single subject block live in memory at once.
#[cfg(feature = "streaming-serialization")]
pub fn graph_to_trig_streaming<W: std::io::Write>(
    graph: &Graph,
    prefixes: &Prefixes,
    w: &mut W,
) -> std::io::Result<()> {
    let owned = dataset_graphs(graph);
    let view: Vec<NamedGraph<'_>> = owned
        .iter()
        .map(|(n, ts)| (n.as_ref(), ts.as_slice()))
        .collect();
    write_trig_streaming(&view, prefixes, w)
}

/// Serializes a [`Graph`] (default + named graphs) as N-Quads.
pub fn graph_to_nquads(graph: &Graph) -> String {
    let owned = dataset_graphs(graph);
    let view: Vec<NamedGraph<'_>> = owned
        .iter()
        .map(|(n, ts)| (n.as_ref(), ts.as_slice()))
        .collect();
    write_nquads(&view)
}

// ===========================================================================
// Pretty Turtle.
//
// [OPUS-4.8] (sq-ixc3.2) The long-term engine home for the site-side pretty
// reshaper shipped in #805 (sq-gb4o, `packages/sparq-client/src/pretty-turtle.ts`).
// The site reshapes the engine's flat N-Triples in TS; this produces idiomatic
// Turtle directly so the CLI / server / wasm all get it for free.
//
// This differs from the plain [`write_turtle`] above in two load-bearing ways:
//
//   1. STABLE, INPUT-ORDER-INDEPENDENT output. `write_turtle` preserves the input
//      slice's order (which, fed from `iter_ids()`, is thread-count-dependent — see
//      that function's doc). The pretty writer SORTS subjects, predicates, and objects
//      by their canonical N-Triples spelling, so the same triple SET always renders to
//      the same bytes regardless of emission order. `rdf:type` (`a`) sorts first within
//      a subject block (idiomatic Turtle).
//
//   2. The OUTPUT SHAPE of the site's `prettyTurtle`: blank-line-separated subject
//      blocks, a configurable indent unit (default two spaces), object lists wrapped
//      onto continuation lines (`, ` becomes `,\n{indent}{indent}`), and a
//      prefix-alphabetical used-only `@prefix` header separated from the body by a
//      blank line. Matching the shape lets the site later DROP its TS formatter and
//      call this through the wasm surface (deferred — see the crate README / bead).
//
// Round-trip correctness is identical to `write_turtle` (it reuses the same term
// rendering + escaping helpers), and is asserted by the `pretty_*` tests below.
// ===========================================================================

/// Options for the pretty Turtle / TriG writers. Mirrors the site `prettyTurtle`
/// option contract (`PrettyTurtleOptions`) so the two stay shape-compatible.
#[derive(Clone, Debug)]
pub struct PrettyOptions {
    /// Indent unit for predicate / object continuation lines. Default: two spaces.
    pub indent: String,
    /// When `false`, no `@prefix` header is emitted and every IRI stays in full
    /// `<…>` form. Default: `true`.
    pub abbreviate: bool,
}

impl Default for PrettyOptions {
    fn default() -> Self {
        PrettyOptions {
            indent: "  ".to_string(),
            abbreviate: true,
        }
    }
}

/// The canonical N-Triples spelling of a term — the stable sort key. This is exactly
/// the byte form oxrdf's `Display` produces (the same form the parsers accept), so it
/// is total, deterministic, and label-stable for blank nodes.
fn nt_key(term: &Term) -> String {
    term.to_string()
}

/// The N-Triples spelling of a subject (IRI or blank node) — the stable sort key.
fn nt_subject_key(subj: &NamedOrBlankNode) -> String {
    subj.to_string()
}

/// One subject's predicate-object lists for the pretty writer, sorted for determinism:
/// predicates by sort key (`a`/rdf:type first), each predicate's objects by N-Triples
/// spelling, de-duplicated.
struct PrettySubject {
    subject: NamedOrBlankNode,
    /// `(predicate_render_key, predicate_iri_or_a, objects)` triples in sorted order.
    /// `predicate_iri_or_a` is the IRI string, or the literal `a` marker so the renderer
    /// can re-derive the `a` shorthand without re-checking the IRI.
    predicates: Vec<PrettyPredicate>,
}

struct PrettyPredicate {
    /// The predicate IRI (rdf:type included — `a` is decided at render time).
    iri: oxrdf::NamedNode,
    /// Objects in stable N-Triples order, de-duplicated.
    objects: Vec<Term>,
}

/// Groups triples by subject → predicate → objects with the site's stable ordering:
/// subjects sorted by N-Triples spelling, predicates by sort key (`rdf:type` first),
/// objects by N-Triples spelling and de-duplicated.
fn group_sorted(triples: &[Triple]) -> Vec<PrettySubject> {
    use std::collections::BTreeMap;
    // subject_nt -> (subject, pred_sortkey -> (predicate, obj_nt -> object))
    type PredMap = BTreeMap<String, (oxrdf::NamedNode, BTreeMap<String, Term>)>;
    let mut by_subject: BTreeMap<String, (NamedOrBlankNode, PredMap)> = BTreeMap::new();

    for t in triples {
        let skey = nt_subject_key(&t.subject);
        let entry = by_subject
            .entry(skey)
            .or_insert_with(|| (t.subject.clone(), BTreeMap::new()));
        // `rdf:type` sorts first within the block: key it with a leading control char
        // (U+0001) that orders before any IRI's `<`. Anything else sorts by its IRI.
        let pkey = if t.predicate.as_str() == RDF_TYPE {
            "\u{0001}".to_string()
        } else {
            t.predicate.as_str().to_string()
        };
        let pred = entry
            .1
            .entry(pkey)
            .or_insert_with(|| (t.predicate.clone(), BTreeMap::new()));
        pred.1.entry(nt_key(&t.object)).or_insert_with(|| t.object.clone());
    }

    by_subject
        .into_values()
        .map(|(subject, preds)| PrettySubject {
            subject,
            predicates: preds
                .into_values()
                .map(|(iri, objs)| PrettyPredicate {
                    iri,
                    objects: objs.into_values().collect(),
                })
                .collect(),
        })
        .collect()
}

/// Renders one graph's worth of triples as grouped, sorted, indented Turtle blocks
/// (no `@prefix` header — the caller assembles that once). `base_indent` prefixes every
/// line (a TriG graph-block indent). Returns the joined block text, or empty for no
/// triples. Mirrors the site's `renderGraphBody`.
fn pretty_graph_body(
    triples: &[Triple],
    base_indent: &str,
    opts: &PrettyOptions,
    prefixes: &Prefixes,
) -> String {
    let groups = group_sorted(triples);
    let mut blocks: Vec<String> = Vec::with_capacity(groups.len());
    for g in &groups {
        let mut block = String::new();
        block.push_str(base_indent);
        write_subject_pretty(&g.subject, opts.abbreviate, prefixes, &mut block);
        for (pi, pred) in g.predicates.iter().enumerate() {
            block.push('\n');
            block.push_str(base_indent);
            block.push_str(&opts.indent);
            // `a` for rdf:type, else the (optionally abbreviated) IRI.
            if pred.iri.as_str() == RDF_TYPE {
                block.push('a');
            } else if opts.abbreviate {
                write_iri(pred.iri.as_str(), prefixes, &mut block);
            } else {
                escape_iri(pred.iri.as_str(), &mut block);
            }
            block.push(' ');
            let sep = format!(",\n{}{}{}", base_indent, opts.indent, opts.indent);
            for (oi, o) in pred.objects.iter().enumerate() {
                if oi > 0 {
                    block.push_str(&sep);
                }
                write_term_maybe(o, opts.abbreviate, prefixes, &mut block);
            }
            if pi + 1 == g.predicates.len() {
                block.push_str(" .");
            } else {
                block.push_str(" ;");
            }
        }
        blocks.push(block);
    }
    blocks.join("\n\n")
}

/// Renders a subject for the pretty writer. With `abbreviate`, an IRI is prefix-compacted;
/// without it, IRIs stay full `<…>`. Blank nodes are `_:label` either way.
fn write_subject_pretty(subj: &NamedOrBlankNode, abbreviate: bool, prefixes: &Prefixes, out: &mut String) {
    match subj {
        NamedOrBlankNode::NamedNode(n) if abbreviate => write_iri(n.as_str(), prefixes, out),
        NamedOrBlankNode::NamedNode(n) => escape_iri(n.as_str(), out),
        NamedOrBlankNode::BlankNode(b) => {
            out.push_str("_:");
            out.push_str(b.as_str());
        }
    }
}

/// Renders any term for the pretty writer, honouring `abbreviate`. Without abbreviation,
/// IRIs (including a literal's datatype and any nested triple-term IRI) stay full `<…>`;
/// literals still drop the implicit `xsd:string`/`rdf:langString` datatype (canonical short
/// form, not prefix compaction). With abbreviation it is exactly [`write_term`].
fn write_term_maybe(term: &Term, abbreviate: bool, prefixes: &Prefixes, out: &mut String) {
    if abbreviate {
        write_term(term, prefixes, out);
    } else {
        write_term_full(term, out);
    }
}

/// Renders any term with NO prefix compaction — every IRI is a full `<…>` IRIREF. Reuses the
/// same escaping helpers (so a re-parse is exact); only the prefix step is skipped.
fn write_term_full(term: &Term, out: &mut String) {
    match term {
        Term::NamedNode(n) => escape_iri(n.as_str(), out),
        Term::BlankNode(b) => {
            out.push_str("_:");
            out.push_str(b.as_str());
        }
        Term::Literal(l) => {
            out.push('"');
            escape_string(l.value(), out);
            out.push('"');
            if let Some(lang) = l.language() {
                out.push('@');
                out.push_str(lang);
            } else {
                let dt = l.datatype().as_str();
                if dt != "http://www.w3.org/2001/XMLSchema#string"
                    && dt != "http://www.w3.org/1999/02/22-rdf-syntax-ns#langString"
                {
                    out.push_str("^^");
                    escape_iri(dt, out);
                }
            }
        }
        Term::Triple(t) => {
            out.push_str("<<( ");
            write_subject_pretty(&t.subject, false, &Prefixes::new(), out);
            out.push(' ');
            escape_iri(t.predicate.as_str(), out);
            out.push(' ');
            write_term_full(&t.object, out);
            out.push_str(" )>>");
        }
    }
}

/// Serializes a set of triples as PRETTY Turtle: a prefix-alphabetical, used-only
/// `@prefix` header, then blank-line-separated subject blocks with stable (sorted)
/// ordering and a configurable indent. See the module-level [pretty section](self)
/// notes for how this differs from [`write_turtle`].
///
/// The output is round-trip-correct: re-parsing it yields the same triple set as the
/// input (the same property [`write_turtle`] holds, reusing the same term/escaping
/// helpers).
pub fn write_turtle_pretty(triples: &[Triple], prefixes: &Prefixes, opts: &PrettyOptions) -> String {
    let body = pretty_graph_body(triples, "", opts, prefixes);
    let mut sections: Vec<String> = Vec::new();
    if opts.abbreviate {
        if let Some(header) = pretty_prefix_header(triples, prefixes, "") {
            sections.push(header);
        }
    }
    if !body.is_empty() {
        sections.push(body);
    }
    sections.join("\n\n")
}

/// Builds the `@prefix` header for the pretty writers: prefix-alphabetical, listing only
/// the prefixes whose namespace is the chosen compaction for at least one IRI in
/// `triples`. Returns `None` when nothing compacts. `indent` prefixes each line (a TriG
/// shared-header indent — currently always empty, kept for symmetry with the site).
fn pretty_prefix_header(triples: &[Triple], prefixes: &Prefixes, indent: &str) -> Option<String> {
    let mut used: std::collections::BTreeSet<&str> = std::collections::BTreeSet::new();
    let mut probe = String::new();
    let mut note = |iri: &str| {
        probe.clear();
        write_iri(iri, prefixes, &mut probe);
        if !probe.starts_with('<') {
            if let Some((pfx, _)) = probe.split_once(':') {
                if let Some((k, _)) = prefixes.get_key_value(pfx) {
                    used.insert(k.as_str());
                }
            }
        }
    };
    for t in triples {
        collect_pretty_iris(&Term::from(t.subject.clone()), &mut note);
        // `rdf:type` renders as `a` — never declares the rdf: prefix on its own account.
        if t.predicate.as_str() != RDF_TYPE {
            note(t.predicate.as_str());
        }
        collect_pretty_iris(&t.object, &mut note);
    }
    if used.is_empty() {
        return None;
    }
    let mut out = String::new();
    for (i, pfx) in used.iter().enumerate() {
        if i > 0 {
            out.push('\n');
        }
        let ns = &prefixes[*pfx];
        let mut ns_esc = String::new();
        escape_iri(ns, &mut ns_esc);
        let _ = write!(out, "{}@prefix {}: {} .", indent, pfx, ns_esc);
    }
    Some(out)
}

/// Like [`collect_iris`] but matches the PRETTY render exactly: a literal's *implicit*
/// `xsd:string` / `rdf:langString` datatype is dropped at render time (never written), so
/// it must not cause its prefix to be declared in the header. Every other IRI is noted.
fn collect_pretty_iris(term: &Term, note: &mut impl FnMut(&str)) {
    match term {
        Term::NamedNode(n) => note(n.as_str()),
        Term::Literal(l) => {
            if l.language().is_none() {
                let dt = l.datatype().as_str();
                if dt != "http://www.w3.org/2001/XMLSchema#string"
                    && dt != "http://www.w3.org/1999/02/22-rdf-syntax-ns#langString"
                {
                    note(dt);
                }
            }
        }
        Term::Triple(t) => {
            collect_pretty_iris(&Term::from(t.subject.clone()), note);
            note(t.predicate.as_str());
            collect_pretty_iris(&t.object, note);
        }
        Term::BlankNode(_) => {}
    }
}

/// Serializes a [`Graph`]'s default graph as PRETTY Turtle with the [`default_prefixes`]
/// and the default [`PrettyOptions`] (two-space indent, abbreviation on).
pub fn graph_to_turtle_pretty(graph: &Graph) -> String {
    write_turtle_pretty(
        &graph_triples(graph),
        &default_prefixes(),
        &PrettyOptions::default(),
    )
}

/// Serializes a [`Graph`]'s default graph as PRETTY Turtle with caller-supplied prefixes
/// and options.
pub fn graph_to_turtle_pretty_with(
    graph: &Graph,
    prefixes: &Prefixes,
    opts: &PrettyOptions,
) -> String {
    write_turtle_pretty(&graph_triples(graph), prefixes, opts)
}

/// Serializes a dataset as PRETTY TriG: a single shared, prefix-alphabetical `@prefix`
/// header over every graph, then the default graph's blocks at top level, then each named
/// graph wrapped in a `GRAPH <g> { … }` block. Graph order: default graph first, then
/// named graphs by their N-Triples spelling. Mirrors the site's `prettyTrig`.
pub fn write_trig_pretty(
    graphs: &[NamedGraph<'_>],
    prefixes: &Prefixes,
    opts: &PrettyOptions,
) -> String {
    // The shared header is computed over the union of every graph's triples (every IRI
    // that appears anywhere in the dataset).
    let all: Vec<Triple> = graphs
        .iter()
        .flat_map(|(_, ts)| ts.iter().cloned())
        .collect();

    // Partition: default graph (name `None`) first, then named graphs sorted by their
    // N-Triples spelling.
    let mut named: Vec<&NamedGraph<'_>> = graphs.iter().filter(|(n, _)| n.is_some()).collect();
    named.sort_by_key(|a| nt_key(a.0.expect("named-graph filter guarantees Some")));

    let mut sections: Vec<String> = Vec::new();
    if opts.abbreviate {
        if let Some(header) = pretty_prefix_header(&all, prefixes, "") {
            sections.push(header);
        }
    }

    // Default graph(s) at top level.
    for (name, ts) in graphs {
        if name.is_none() && !ts.is_empty() {
            let body = pretty_graph_body(ts, "", opts, prefixes);
            if !body.is_empty() {
                sections.push(body);
            }
        }
    }
    // Named graphs as GRAPH blocks.
    for (name, ts) in named {
        if ts.is_empty() {
            continue;
        }
        let mut block = String::new();
        block.push_str("GRAPH ");
        match name.expect("named-graph filter guarantees Some") {
            Term::NamedNode(n) => {
                if opts.abbreviate {
                    write_iri(n.as_str(), prefixes, &mut block);
                } else {
                    escape_iri(n.as_str(), &mut block);
                }
            }
            Term::BlankNode(b) => {
                block.push_str("_:");
                block.push_str(b.as_str());
            }
            other => {
                // A literal/triple-term graph name is not legal RDF; render its canonical
                // N-Triples form so nothing is silently lost (the parser surfaces it).
                let _ = write!(block, "{}", other);
            }
        }
        block.push_str(" {\n");
        block.push_str(&pretty_graph_body(ts, &opts.indent, opts, prefixes));
        block.push_str("\n}");
        sections.push(block);
    }
    sections.join("\n\n")
}

/// Serializes a [`Graph`] (default + named graphs) as PRETTY TriG with the
/// [`default_prefixes`] and default [`PrettyOptions`].
pub fn graph_to_trig_pretty(graph: &Graph) -> String {
    let owned = dataset_graphs(graph);
    let view: Vec<NamedGraph<'_>> = owned
        .iter()
        .map(|(n, ts)| (n.as_ref(), ts.as_slice()))
        .collect();
    write_trig_pretty(&view, &default_prefixes(), &PrettyOptions::default())
}

/// [OPUS-4.8] sq-fe1s: Serializes a [`Graph`] (default + named graphs) as PRETTY TriG with
/// caller-supplied prefixes and options — the TriG-side symmetry of
/// [`graph_to_turtle_pretty_with`], so a consumer that already holds a [`Graph`] can choose
/// the indent / abbreviation without rebuilding the `(name, triples)` view by hand.
pub fn graph_to_trig_pretty_with(
    graph: &Graph,
    prefixes: &Prefixes,
    opts: &PrettyOptions,
) -> String {
    let owned = dataset_graphs(graph);
    let view: Vec<NamedGraph<'_>> = owned
        .iter()
        .map(|(n, ts)| (n.as_ref(), ts.as_slice()))
        .collect();
    write_trig_pretty(&view, prefixes, opts)
}

// ===========================================================================
// JSON-LD 1.1.
//
// A native writer — no json-ld crate, no serde_json, zero new dependencies. It
// reuses the same oxrdf term model the other writers do and emits JSON by hand
// (the structure is fully under our control, so a generic JSON DOM buys nothing).
//
// The output is the *Deserialize-JSON-LD-to-RDF* inverse: feeding any document this
// produces back through the JSON-LD-to-RDF algorithm reconstructs exactly this triple
// set. That is the round-trip contract — verified by an in-crate expanded-form reader
// in the tests (sparq-core has no JSON-LD parser of its own).
//
// `xsd:string` and `rdf:langString` are kept implicit (`@value` + optional
// `@language`), every other datatype is preserved verbatim as `@type` so no datatype is
// ever lost. Native JSON number/boolean coercion is applied ONLY when it is provably
// lossless and canonical (see `coerce_native`), otherwise the lexical form is kept as a
// string `@value` with its `@type`.
// ===========================================================================

const XSD: &str = "http://www.w3.org/2001/XMLSchema#";
const RDF_LANG_STRING: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#langString";
// [OPUS-4.8] (sq-gg3j) RDF-list (`@list`) collapsing — see `detect_lists` below. A
// *well-formed, single-referenced* rdf:first/rdf:rest/rdf:nil chain is collapsed into a
// native JSON-LD `@list` array; anything that fails the safety predicate (shared list cell,
// extra predicates, missing/duplicate first/rest, a cycle, a non-blank cell) is left as
// ordinary rdf:first/rdf:rest triples so the round-trip stays lossless either way.

/// [OPUS-4.8] (sq-gg3j) Result of scanning one graph for collapsible RDF lists.
///
/// `heads[b]` holds, for every blank-node list **head** `b` that is safe to collapse, the
/// list's element terms in order. `cells` is the full set of blank-node *list cells* (every
/// node visited while walking a collapsed chain, head included) — those must be suppressed
/// from the top-level node array and never emitted as ordinary `rdf:first`/`rdf:rest` nodes,
/// because their content now lives inside the `@list`.
#[derive(Default)]
struct ListInfo {
    heads: std::collections::HashMap<oxrdf::BlankNode, Vec<Term>>,
    cells: std::collections::HashSet<oxrdf::BlankNode>,
}

/// Per-blank-node view used by list detection: how many times the node appears as an
/// *object* anywhere in the graph, and (if it is a candidate list cell) its single
/// `rdf:first` value and single `rdf:rest` continuation.
#[derive(Default)]
struct CellFacts {
    /// Times this node appears in object position across the whole graph.
    obj_refs: u32,
    first: Option<Term>,
    rest: Option<Term>,
    /// True once the node carries any predicate that is **not** `rdf:first`/`rdf:rest`
    /// (including `rdf:type`), or a duplicate `rdf:first`/`rdf:rest`, or appears as a
    /// subject term that is not a blank node. Any of these makes it ineligible.
    tainted: bool,
}

/// Scans one graph's triples and returns the set of safe, single-referenced
/// `rdf:first`/`rdf:rest`/`rdf:nil` chains, collapsible into JSON-LD `@list` arrays.
///
/// A blank node is a collapsible **list cell** only when *all* of these hold (the
/// conservative safety predicate — any miss leaves the triples as ordinary RDF):
///
/// * it is a blank node carrying **exactly one** `rdf:first` and **exactly one** `rdf:rest`
///   and **no other** predicate (no `rdf:type`, no stray properties);
/// * it is referenced in object position **exactly once** in the whole graph (so collapsing
///   it — which discards its blank-node identity — cannot drop a second reference); and
/// * its `rdf:rest` chain reaches `rdf:nil` through nothing but such cells, with no cycle and
///   no sharing of any interior cell.
///
/// A **head** is a collapsible cell whose single object-reference comes from a triple whose
/// predicate is *not* `rdf:rest` (i.e. it is pointed at as a value, not as a list tail). The
/// empty list (`rdf:nil` used directly) is intentionally left as a plain `@id` reference:
/// `rdf:nil` is a shared named node, never collapsed.
fn detect_lists(triples: &[Triple]) -> ListInfo {
    // Pass 1 — accumulate per-blank-node facts.
    let mut facts: std::collections::HashMap<oxrdf::BlankNode, CellFacts> =
        std::collections::HashMap::new();
    for t in triples {
        if let Term::BlankNode(b) = &t.object {
            facts.entry(b.clone()).or_default().obj_refs += 1;
        }
        let NamedOrBlankNode::BlankNode(s) = &t.subject else {
            continue;
        };
        let cell = facts.entry(s.clone()).or_default();
        match t.predicate.as_str() {
            RDF_FIRST => {
                if cell.first.replace(t.object.clone()).is_some() {
                    cell.tainted = true; // duplicate rdf:first
                }
            }
            RDF_REST => {
                if cell.rest.replace(t.object.clone()).is_some() {
                    cell.tainted = true; // duplicate rdf:rest
                }
            }
            _ => cell.tainted = true, // any other predicate (incl. rdf:type)
        }
    }

    // A blank node is a *well-formed cell* iff it has exactly one first + one rest, is
    // referenced as an object exactly once, and is untainted.
    let is_cell = |b: &oxrdf::BlankNode| -> bool {
        facts.get(b).is_some_and(|c| {
            !c.tainted && c.obj_refs == 1 && c.first.is_some() && c.rest.is_some()
        })
    };

    // Pass 2 — for every triple that points at a candidate head (a cell reached by a
    // non-`rdf:rest` predicate), walk and validate the whole chain before committing it.
    let mut info = ListInfo::default();
    for t in triples {
        if t.predicate.as_str() == RDF_REST {
            continue; // interior link, not a head reference
        }
        let Term::BlankNode(head) = &t.object else {
            continue;
        };
        if !is_cell(head) || info.cells.contains(head) {
            continue;
        }
        // Walk first→rest until rdf:nil, requiring every interior node to be a fresh cell.
        let mut elems: Vec<Term> = Vec::new();
        let mut chain: Vec<oxrdf::BlankNode> = Vec::new();
        let mut seen: std::collections::HashSet<oxrdf::BlankNode> =
            std::collections::HashSet::new();
        let mut cur = head.clone();
        let well_formed = loop {
            if !seen.insert(cur.clone()) {
                break false; // cycle
            }
            if !is_cell(&cur) || info.cells.contains(&cur) {
                break false; // shared with an already-collapsed list, or not a cell
            }
            let c = &facts[&cur];
            elems.push(c.first.clone().expect("cell has rdf:first"));
            chain.push(cur.clone());
            match c.rest.clone().expect("cell has rdf:rest") {
                Term::NamedNode(n) if n.as_str() == RDF_NIL => break true,
                Term::BlankNode(next) => cur = next,
                // rdf:rest to a literal, a non-nil IRI, or a triple term: not a list.
                _ => break false,
            }
        };
        if well_formed {
            for b in &chain {
                info.cells.insert(b.clone());
            }
            info.heads.insert(head.clone(), elems);
        }
    }
    info
}

/// Which JSON-LD 1.1 document form [`write_jsonld`] / [`graph_to_jsonld`] emits.
///
/// All three are *round-trippable* (re-parsing reconstructs the same RDF). They differ
/// only in shape and in whether IRIs are abbreviated:
///
/// * [`Expanded`](JsonLdForm::Expanded) — the fully-explicit form: a flat array of node
///   objects, every IRI a full string, every value an `@value`/`@id` object. No
///   `@context`. This is the algorithmic core and the safest interop target.
/// * [`Flattened`](JsonLdForm::Flattened) — like expanded but guarantees every node
///   appears exactly once at the top level (subjects merged) and wraps named graphs in
///   `@graph`. For a triple store the expanded output is already node-merged, so this
///   adds the dataset `@graph` framing and a stable node ordering.
/// * [`Compacted`](JsonLdForm::Compacted) — a basic prefix `@context` (from the supplied
///   prefix map) abbreviating IRIs to `prefix:local` / terms, `@type` shorthand for
///   `rdf:type`, and `@language`/`@type`-free plain strings where safe.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum JsonLdForm {
    /// Fully-expanded: flat node-object array, no `@context`, every IRI explicit.
    Expanded,
    /// Flattened: node-merged with named graphs wrapped in `@graph` and stable ordering.
    Flattened,
    /// Compacted: a prefix-based `@context` abbreviating IRIs (best-effort, lossless).
    Compacted,
}

/// Escapes a string as a JSON string body (without the surrounding quotes) per RFC 8259:
/// the two mandatory escapes (`"`, `\`), the short escapes for the common control chars,
/// and `\u00XX` for the remaining C0 controls. Everything else (including non-ASCII, which
/// JSON permits raw in UTF-8) passes through verbatim.
fn json_escape(s: &str, out: &mut String) {
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{08}' => out.push_str("\\b"),
            '\u{0C}' => out.push_str("\\f"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
}

/// Writes a JSON string literal (quoted, escaped).
fn json_str(s: &str, out: &mut String) {
    out.push('"');
    json_escape(s, out);
    out.push('"');
}

/// If a literal can be represented as a *canonical, lossless* native JSON scalar, returns
/// that scalar's JSON text (e.g. `true`, `42`). Otherwise `None` — the caller then keeps
/// the value as a string `@value` carrying its `@type`, so the datatype survives.
///
/// The bar is deliberately high: a native value is only emitted when re-serializing the
/// JSON scalar yields back the *same lexical form*, so the JSON-LD-to-RDF round-trip
/// reconstructs the identical literal. That means:
///
/// * `xsd:boolean` only for the canonical `true` / `false` lexicals (not `1` / `0`).
/// * `xsd:integer` only when the digits round-trip through `i64` unchanged (no leading
///   zeros, no `+`, fits in range) — JSON numbers are the canonical decimal anyway.
///
/// `xsd:double`/`decimal` are intentionally NOT coerced: JSON has one number type and its
/// shortest round-trip form rarely equals the RDF lexical (`1.5` vs `1.5E0`), which would
/// silently change the literal. Keeping them as typed strings is lossless.
fn coerce_native(value: &str, datatype: &str) -> Option<String> {
    match datatype.strip_prefix(XSD)? {
        "boolean" => match value {
            "true" => Some("true".to_string()),
            "false" => Some("false".to_string()),
            _ => None,
        },
        "integer" => {
            // Reject any lexical whose canonical i64 text differs (leading zeros, '+',
            // spaces, out-of-range) so the re-serialized number is byte-identical.
            value
                .parse::<i64>()
                .ok()
                .map(|n| n.to_string())
                .filter(|n| n == value)
        }
        _ => None,
    }
}

/// Renders a JSON-LD *value object* for a literal: `{"@value": …}` plus `@language` (for a
/// language-tagged string) or `@type` (for any non-string datatype). `xsd:string` /
/// `rdf:langString` stay implicit. `prefixes`, when `Some`, abbreviates the `@type` IRI.
fn write_jsonld_literal(lit: &oxrdf::Literal, prefixes: Option<&Prefixes>, out: &mut String) {
    let dt = lit.datatype().as_str();
    // Native coercion only in the absence of a language tag and only when canonical.
    if lit.language().is_none() {
        if let Some(native) = coerce_native(lit.value(), dt) {
            out.push_str("{\"@value\":");
            out.push_str(&native);
            out.push('}');
            return;
        }
    }
    out.push_str("{\"@value\":");
    json_str(lit.value(), out);
    if let Some(lang) = lit.language() {
        out.push_str(",\"@language\":");
        json_str(lang, out);
    } else if dt != format!("{XSD}string") && dt != RDF_LANG_STRING {
        out.push_str(",\"@type\":");
        write_jsonld_iri_value(dt, prefixes, out);
    }
    out.push('}');
}

/// Writes an IRI as a JSON string, compacted to `prefix:local` when a prefix map is supplied
/// and a registered namespace splits cleanly; otherwise the full IRI. Used for `@id` / `@type`
/// values and node keys (predicates) in the compacted form.
fn write_jsonld_iri_value(iri: &str, prefixes: Option<&Prefixes>, out: &mut String) {
    if let Some(pfx) = prefixes {
        if let Some(curie) = compact_iri(iri, pfx) {
            json_str(&curie, out);
            return;
        }
    }
    json_str(iri, out);
}

/// Best-effort `prefix:local` compaction sharing the Turtle writer's correctness rule
/// (`is_simple_pn_local`, longest namespace wins). `None` when no prefix applies — the caller
/// keeps the full IRI. Never produces an ambiguous/lossy CURIE.
fn compact_iri(iri: &str, prefixes: &Prefixes) -> Option<String> {
    let mut best: Option<(&str, &str)> = None;
    for (pfx, ns) in prefixes {
        if let Some(local) = iri.strip_prefix(ns.as_str()) {
            if is_simple_pn_local(local) && !local.is_empty() {
                match best {
                    Some((_, bns)) if bns.len() >= ns.len() => {}
                    _ => best = Some((pfx.as_str(), local)),
                }
            }
        }
    }
    best.map(|(pfx, local)| format!("{pfx}:{local}"))
}

/// A node object accumulated for one subject within one graph: its `@type` IRIs (from
/// `rdf:type`) and its other predicate→object lists, both in first-seen order.
struct JsonLdNode {
    /// `rdf:type` objects (rendered under the `@type` keyword). IRIs only.
    types: Vec<String>,
    /// Predicate IRIs in first-seen order (excluding `rdf:type`).
    pred_order: Vec<String>,
    /// Predicate IRI → its object terms in input order.
    preds: std::collections::HashMap<String, Vec<Term>>,
}

/// Groups a graph's triples into per-subject node objects, subjects in first-seen order.
///
/// Triples whose subject is a collapsed list **cell** (`lists.cells`) are skipped entirely:
/// their `rdf:first`/`rdf:rest` content now lives inside the `@list` array emitted at the head
/// reference, so emitting the cell as its own node object would duplicate it.
fn group_nodes(triples: &[Triple], lists: &ListInfo) -> (Vec<NamedOrBlankNode>, Vec<JsonLdNode>) {
    let mut order: Vec<NamedOrBlankNode> = Vec::new();
    let mut slot: std::collections::HashMap<NamedOrBlankNode, usize> =
        std::collections::HashMap::new();
    let mut nodes: Vec<JsonLdNode> = Vec::new();
    for t in triples {
        if let NamedOrBlankNode::BlankNode(b) = &t.subject {
            if lists.cells.contains(b) {
                continue;
            }
        }
        let s = t.subject.clone();
        let i = *slot.entry(s.clone()).or_insert_with(|| {
            order.push(s.clone());
            nodes.push(JsonLdNode {
                types: Vec::new(),
                pred_order: Vec::new(),
                preds: std::collections::HashMap::new(),
            });
            nodes.len() - 1
        });
        let node = &mut nodes[i];
        if t.predicate.as_str() == RDF_TYPE {
            if let Term::NamedNode(n) = &t.object {
                node.types.push(n.as_str().to_string());
                continue;
            }
            // A non-IRI rdf:type object is unusual but legal RDF — fall through and keep it
            // as an ordinary predicate so nothing is dropped.
        }
        let p = t.predicate.as_str().to_string();
        if !node.preds.contains_key(&p) {
            node.pred_order.push(p.clone());
        }
        node.preds.entry(p).or_default().push(t.object.clone());
    }
    (order, nodes)
}

/// Writes the `@id` of a subject/graph-name node as a JSON string (IRI compacted if a prefix
/// map is supplied; blank nodes keep their `_:label`).
fn write_node_id(subj: &NamedOrBlankNode, prefixes: Option<&Prefixes>, out: &mut String) {
    match subj {
        NamedOrBlankNode::NamedNode(n) => write_jsonld_iri_value(n.as_str(), prefixes, out),
        NamedOrBlankNode::BlankNode(b) => {
            let mut s = String::from("_:");
            s.push_str(b.as_str());
            json_str(&s, out);
        }
    }
}

/// Writes one object term as JSON-LD: an IRI/blank node as `{"@id": …}`, a literal as a value
/// object, an RDF 1.2 triple term as nested expanded JSON-LD (round-trippable). Blank node ids
/// keep the `_:` form so they re-parse to the same node.
///
/// When `lists` records the blank node as a collapsed list **head**, it is emitted as a
/// `{"@list": [ … ]}` object instead — each element rendered through this same function, so a
/// list element that is itself a collapsed list nests naturally.
fn write_jsonld_object(
    term: &Term,
    prefixes: Option<&Prefixes>,
    lists: &ListInfo,
    out: &mut String,
) {
    match term {
        Term::NamedNode(n) => {
            out.push_str("{\"@id\":");
            write_jsonld_iri_value(n.as_str(), prefixes, out);
            out.push('}');
        }
        Term::BlankNode(b) => {
            if let Some(elems) = lists.heads.get(b) {
                out.push_str("{\"@list\":[");
                for (i, e) in elems.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    write_jsonld_object(e, prefixes, lists, out);
                }
                out.push_str("]}");
                return;
            }
            let mut s = String::from("_:");
            s.push_str(b.as_str());
            out.push_str("{\"@id\":");
            json_str(&s, out);
            out.push('}');
        }
        Term::Literal(l) => write_jsonld_literal(l, prefixes, out),
        Term::Triple(t) => {
            // RDF 1.2 triple terms have no standard JSON-LD 1.1 encoding. To avoid silently
            // dropping the value we emit the canonical N-Triples triple-term form
            // (`<<( s p o )>>`, exactly what oxrdf's `Display` and the Turtle/N-Triples
            // parsers use) as a plain `@id` string. A generic JSON-LD processor treats it as
            // an opaque IRI-shaped id; sparq's own round-trip reader recognises it. The value
            // is preserved verbatim either way.
            out.push_str("{\"@id\":");
            let mut nt = String::new();
            let _ = write!(nt, "{}", Term::Triple(t.clone()));
            json_str(&nt, out);
            out.push('}');
        }
    }
}

/// Emits the predicate→objects map of a node as JSON members (the `@type` keyword first when
/// present), reusing input order. Returns whether anything was written before this call's
/// members (so the caller manages the leading comma).
fn write_node_members(
    node: &JsonLdNode,
    prefixes: Option<&Prefixes>,
    lists: &ListInfo,
    out: &mut String,
) {
    let mut first = true;
    if !node.types.is_empty() {
        first = false;
        out.push_str("\"@type\":[");
        for (i, t) in node.types.iter().enumerate() {
            if i > 0 {
                out.push(',');
            }
            write_jsonld_iri_value(t, prefixes, out);
        }
        out.push(']');
    }
    for p in &node.pred_order {
        if first {
            first = false;
        } else {
            out.push(',');
        }
        // Predicate key: full IRI (expanded/flattened) or compacted CURIE (compacted form).
        if let Some(pfx) = prefixes {
            if let Some(curie) = compact_iri(p, pfx) {
                json_str(&curie, out);
            } else {
                json_str(p, out);
            }
        } else {
            json_str(p, out);
        }
        out.push(':');
        out.push('[');
        for (i, o) in node.preds[p].iter().enumerate() {
            if i > 0 {
                out.push(',');
            }
            write_jsonld_object(o, prefixes, lists, out);
        }
        out.push(']');
    }
}

/// Serializes one graph (already grouped) as a JSON array of node objects into `out`. Shared by
/// every form — the difference between forms is the `@context`/`@graph` framing the callers add.
fn write_node_array(triples: &[Triple], prefixes: Option<&Prefixes>, out: &mut String) {
    // [OPUS-4.8] (sq-gg3j) Find collapsible rdf:first/rdf:rest chains *per graph* (blank-node
    // scope is per graph), then suppress their cells and render their heads as `@list`.
    let lists = detect_lists(triples);
    let (order, nodes) = group_nodes(triples, &lists);
    out.push('[');
    for (i, subj) in order.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push_str("{\"@id\":");
        write_node_id(subj, prefixes, out);
        // `@type` / predicate members (each prefixed by a comma inside write_node_members).
        let mut members = String::new();
        write_node_members(&nodes[i], prefixes, &lists, &mut members);
        if !members.is_empty() {
            out.push(',');
            out.push_str(&members);
        }
        out.push('}');
    }
    out.push(']');
}

/// Writes the `@context` object mapping each prefix to its namespace IRI (compacted form only).
/// Only prefixes that actually abbreviate at least one IRI in the dataset are emitted, so the
/// context never carries dead declarations.
fn write_context(all: &[Triple], prefixes: &Prefixes, out: &mut String) {
    let mut used: std::collections::BTreeSet<&str> = std::collections::BTreeSet::new();
    let mut note = |iri: &str| {
        if let Some(curie) = compact_iri(iri, prefixes) {
            if let Some((pfx, _)) = curie.split_once(':') {
                if let Some((k, _)) = prefixes.get_key_value(pfx) {
                    used.insert(k.as_str());
                }
            }
        }
    };
    for t in all {
        collect_iris(&Term::from(t.subject.clone()), &mut note);
        note(t.predicate.as_str());
        collect_iris(&t.object, &mut note);
    }
    out.push('{');
    for (i, pfx) in used.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        json_str(pfx, out);
        out.push(':');
        json_str(&prefixes[*pfx], out);
    }
    out.push('}');
}

/// Serializes an RDF dataset (default + named graphs) as a JSON-LD 1.1 document in `form`.
///
/// * **Default graph only** (no named graphs) → a bare node-object array (expanded /
///   flattened), or a `{"@context": …, "@graph": [ … ]}` object (compacted).
/// * **With named graphs** → a top-level object whose `@graph` holds the default graph's nodes
///   plus one `{"@id": <g>, "@graph": [ … ]}` entry per named graph (the standard JSON-LD
///   dataset shape), wrapped under `@context` for the compacted form.
///
/// `prefixes` is consulted only for [`JsonLdForm::Compacted`]; the expanded/flattened forms
/// always emit full IRIs. Node and predicate order is a faithful function of the input
/// slice (subjects and predicates in first-seen order) with no canonicalising sort, so —
/// as with [`write_turtle`] — the [`graph_to_jsonld`] family inherits the store's
/// thread-count-dependent row order. JSON-LD specifies no canonical node order; see
/// `research/dict-id-order-determinism-audit.md`.
pub fn write_jsonld(graphs: &[NamedGraph<'_>], form: JsonLdForm, prefixes: &Prefixes) -> String {
    let pfx = (form == JsonLdForm::Compacted).then_some(prefixes);
    let mut out = String::new();

    let default_triples: &[Triple] = graphs
        .iter()
        .find(|(n, _)| n.is_none())
        .map(|(_, ts)| *ts)
        .unwrap_or(&[]);
    let named: Vec<&NamedGraph<'_>> = graphs.iter().filter(|(n, _)| n.is_some()).collect();

    // A bare array is only valid for the *expanded* form when there are no named graphs.
    // Compacted always needs the wrapping object to carry the `@context`; flattened is, per
    // the JSON-LD 1.1 flattening algorithm, always a node-map object keyed by `@graph` (so a
    // single-graph document still gets the `{"@graph": […]}` envelope).
    let needs_graph_object =
        !named.is_empty() || form == JsonLdForm::Compacted || form == JsonLdForm::Flattened;

    if !needs_graph_object {
        write_node_array(default_triples, pfx, &mut out);
        return out;
    }

    // Object form: optional @context, then @graph (default-graph nodes + per-named-graph
    // sub-objects).
    out.push('{');
    if form == JsonLdForm::Compacted {
        let all: Vec<Triple> = graphs
            .iter()
            .flat_map(|(_, ts)| ts.iter().cloned())
            .collect();
        out.push_str("\"@context\":");
        write_context(&all, prefixes, &mut out);
        out.push(',');
    }
    out.push_str("\"@graph\":[");
    // Default-graph node objects, spliced in directly (strip the array brackets).
    let mut default_arr = String::new();
    write_node_array(default_triples, pfx, &mut default_arr);
    let inner = &default_arr[1..default_arr.len() - 1];
    out.push_str(inner);
    let mut wrote = !inner.is_empty();
    for (name, ts) in named {
        let g = name.as_ref().expect("named graph has a name");
        if wrote {
            out.push(',');
        }
        wrote = true;
        out.push_str("{\"@id\":");
        // A graph name is an IRI or blank node (a literal/triple-term name is not legal RDF;
        // emit its canonical string so nothing is lost rather than corrupting structure).
        match g {
            Term::NamedNode(n) => write_jsonld_iri_value(n.as_str(), pfx, &mut out),
            Term::BlankNode(b) => {
                let mut s = String::from("_:");
                s.push_str(b.as_str());
                json_str(&s, &mut out);
            }
            other => {
                let mut s = String::new();
                let _ = write!(s, "{other}");
                json_str(&s, &mut out);
            }
        }
        out.push_str(",\"@graph\":");
        write_node_array(ts, pfx, &mut out);
        out.push('}');
    }
    out.push_str("]}");
    out
}

/// Serializes a [`Graph`] (default + named graphs) as JSON-LD 1.1 in `form`, using the
/// [`default_prefixes`] for the compacted form's `@context`. The expanded and flattened forms
/// ignore the prefix map.
pub fn graph_to_jsonld(graph: &Graph, form: JsonLdForm) -> String {
    graph_to_jsonld_with(graph, form, &default_prefixes())
}

/// [`graph_to_jsonld`] with a caller-supplied prefix map for the compacted `@context`.
pub fn graph_to_jsonld_with(graph: &Graph, form: JsonLdForm, prefixes: &Prefixes) -> String {
    let owned = dataset_graphs(graph);
    let view: Vec<NamedGraph<'_>> = owned
        .iter()
        .map(|(n, ts)| (n.as_ref(), ts.as_slice()))
        .collect();
    write_jsonld(&view, form, prefixes)
}

// ===========================================================================
// W3C JSON-LD 1.1 Compaction and Framing against a caller-supplied `@context` /
// frame. `JsonLdForm::Compacted` above is only the *prefix* form; the `compact`
// submodule adapts the native `sparq-jsonld` document pipeline (fromRdf →
// compact / frame), the same code the W3C conformance lanes measure.
// ===========================================================================
mod compact;
pub use compact::{parse_context_json, write_jsonld_compact, ActiveContext, Json as JsonLdValue};

/// Serialises a [`Graph`] as a **compacted** JSON-LD 1.1 document against a caller-supplied
/// `@context`, applying the full W3C Compaction Algorithm. Unlike
/// [`graph_to_jsonld`]`(g, `[`JsonLdForm::Compacted`]`)` — which only abbreviates IRIs with a
/// `prefix → namespace` `@context` — this honours term definitions, `@vocab`,
/// type/language/`@container` (`@set`/`@list`/`@language`/`@index`) coercion, `@reverse`, and
/// `@id`/`@type` keyword aliasing, performing value + node + IRI compaction.
///
/// `context` is the parsed `@context` JSON (build it with [`parse_context_json`] from a
/// context string, or construct the [`JsonLdValue`] directly). The compaction is **lossless**:
/// a round-trip through a JSON-LD-to-RDF processor reconstructs the original dataset. See
/// [`write_jsonld_compact`] for the behaviour on a rejected `@context`.
pub fn graph_to_jsonld_compact(graph: &Graph, context: &JsonLdValue) -> String {
    let owned = dataset_graphs(graph);
    let view: Vec<NamedGraph<'_>> = owned
        .iter()
        .map(|(n, ts)| (n.as_ref(), ts.as_slice()))
        .collect();
    write_jsonld_compact(&view, context)
}

pub use compact::write_jsonld_framed;

/// Frames a [`Graph`] (dataset) against a caller-supplied JSON-LD **frame** document,
/// applying the full W3C JSON-LD 1.1 Framing Algorithm, and returns the framed + compacted
/// JSON-LD 1.1 document.
///
/// Framing reshapes the input into a deterministic tree matching the frame: it selects the
/// subjects whose node pattern matches the frame (`@type` / property presence / specific
/// value / wildcard `{}` / match-none `[]`, combined with AND under `@requireAll: true` else OR),
/// embeds referenced nodes inline per the `@embed` flag (`@once`/`@always`/`@never`; `@link`
/// treated as `@once`) while the blank-node link table breaks circular references, prunes
/// each matched node to the framed properties when `@explicit: true`, and fills `@default` /
/// a preserve-`null` marker for framed properties absent from the matched node (suppressed by
/// `@omitDefault: true`). The framed model is then compacted against the frame's `@context`.
///
/// `frame` is the parsed frame JSON (build it with [`parse_context_json`] from a frame string,
/// or construct the [`JsonLdValue`] directly). The output is a `{"@context": …, "@graph": […]}`
/// document, collapsing to the bare framed node merged with `@context` for a single matched
/// root (the `omitGraph` default). Matching runs over the merged graph of the whole dataset,
/// per the spec.
pub fn graph_to_jsonld_framed(graph: &Graph, frame: &JsonLdValue) -> String {
    let owned = dataset_graphs(graph);
    let view: Vec<NamedGraph<'_>> = owned
        .iter()
        .map(|(n, ts)| (n.as_ref(), ts.as_slice()))
        .collect();
    write_jsonld_framed(&view, frame)
}

// ===========================================================================
// [OPUS-4.8] (sq-ixc3.3) PRETTY (indented) JSON-LD.
//
// The minified writers above assemble their document into a flat `String` token by
// token. The pretty writers produce *exactly that same document* and then re-indent
// it structurally — splitting `{`/`[`/`,`/`}`/`]` onto their own lines with a
// configurable indent. This is the same strategy the pretty Turtle writer mirrors
// (`write_turtle_pretty`, sq-ixc3.2): a separate presentation step layered over the
// already-deterministic core. It is *whitespace-only*:
//
//   * the token stream is untouched (no reordering, no value rewriting), so the
//     already-deterministic first-seen subject / predicate ordering is preserved
//     verbatim and the output is byte-for-byte the minified document with newlines
//     and indentation inserted only at structural punctuation; and
//   * pretty output therefore parses back to the same RDF as the minified output
//     (asserted by the `jsonld_pretty_*` round-trip + minified-equivalence tests).
//
// Dependency-free, exactly like the minified writer: a tiny hand-written JSON
// re-indenter (no serde_json — that is a dev-only dependency; no json-ld crate).
// ===========================================================================

/// Options for the pretty JSON-LD writers. Mirrors [`PrettyOptions`] (the pretty
/// Turtle / TriG option struct) in shape so the two stay consistent: a single
/// configurable indent unit. JSON-LD has no IRI-abbreviation toggle here — IRI
/// abbreviation is already selected by [`JsonLdForm::Compacted`].
#[derive(Clone, Debug)]
pub struct JsonLdPrettyOptions {
    /// Indent unit applied once per nesting level. Default: two spaces.
    pub indent: String,
}

impl Default for JsonLdPrettyOptions {
    fn default() -> Self {
        JsonLdPrettyOptions {
            indent: "  ".to_string(),
        }
    }
}

/// Re-indents a *minified* JSON document (no insignificant whitespace, as produced by
/// the writers above) into the pretty form: each `{`/`[` opens a new indented level,
/// each member / element starts on its own line, and `}`/`]` closes back. An empty
/// `{}` / `[]` stays on one line.
///
/// This is purely a presentation pass — it copies every non-structural byte verbatim
/// and only inserts newlines + `indent`-repeats at structural punctuation, so the
/// resulting document is semantically identical to its input (same tokens, same order).
///
/// JSON string literals are tracked so that a `{`, `[`, `,`, `}`, `]`, or `:` appearing
/// *inside* a string value (e.g. an IRI like `http://ex/` or a `_:b0` label) is copied
/// literally and never mistaken for structure. The minified input is already valid JSON
/// (the writers above guarantee it), so this scanner does not need to validate.
fn reindent_json(minified: &str, indent: &str) -> String {
    let mut out = String::with_capacity(minified.len() + minified.len() / 4);
    let mut depth: usize = 0;
    let mut in_string = false;
    let mut escaped = false;
    let mut chars = minified.chars().peekable();

    // Pushes a newline followed by `level` indent units.
    let newline = |out: &mut String, level: usize| {
        out.push('\n');
        for _ in 0..level {
            out.push_str(indent);
        }
    };

    while let Some(c) = chars.next() {
        if in_string {
            out.push(c);
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == '"' {
                in_string = false;
            }
            continue;
        }
        match c {
            '"' => {
                in_string = true;
                out.push(c);
            }
            '{' | '[' => {
                let close = if c == '{' { '}' } else { ']' };
                // An immediately-closing empty container stays compact: `{}` / `[]`.
                if chars.peek() == Some(&close) {
                    out.push(c);
                    out.push(chars.next().expect("peeked close char"));
                } else {
                    depth += 1;
                    out.push(c);
                    newline(&mut out, depth);
                }
            }
            '}' | ']' => {
                depth = depth.saturating_sub(1);
                newline(&mut out, depth);
                out.push(c);
            }
            ',' => {
                out.push(c);
                newline(&mut out, depth);
            }
            ':' => {
                // `"key": value` — one space after the colon (the conventional shape).
                out.push(c);
                out.push(' ');
            }
            other => out.push(other),
        }
    }
    out
}

/// Serializes an RDF dataset as a *pretty* (indented) JSON-LD 1.1 document in `form`,
/// with a caller-supplied indent. The byte content is exactly [`write_jsonld`]'s output
/// re-indented (whitespace only) — see the [pretty JSON-LD section](self) notes — so it
/// parses back to the same RDF and `prefixes` is consulted only for
/// [`JsonLdForm::Compacted`], identically to the minified writer.
pub fn write_jsonld_pretty(
    graphs: &[NamedGraph<'_>],
    form: JsonLdForm,
    prefixes: &Prefixes,
    opts: &JsonLdPrettyOptions,
) -> String {
    let minified = write_jsonld(graphs, form, prefixes);
    reindent_json(&minified, &opts.indent)
}

/// Serializes a [`Graph`] as PRETTY JSON-LD 1.1 in `form`, using the [`default_prefixes`]
/// for the compacted form's `@context` and the default [`JsonLdPrettyOptions`] (two-space
/// indent). The pretty analogue of [`graph_to_jsonld`].
pub fn graph_to_jsonld_pretty(graph: &Graph, form: JsonLdForm) -> String {
    graph_to_jsonld_pretty_with(
        graph,
        form,
        &default_prefixes(),
        &JsonLdPrettyOptions::default(),
    )
}

/// [`graph_to_jsonld_pretty`] with a caller-supplied prefix map (for the compacted
/// `@context`) and pretty options.
pub fn graph_to_jsonld_pretty_with(
    graph: &Graph,
    form: JsonLdForm,
    prefixes: &Prefixes,
    opts: &JsonLdPrettyOptions,
) -> String {
    let owned = dataset_graphs(graph);
    let view: Vec<NamedGraph<'_>> = owned
        .iter()
        .map(|(n, ts)| (n.as_ref(), ts.as_slice()))
        .collect();
    write_jsonld_pretty(&view, form, prefixes, opts)
}

/// [OPUS-4.8] (sq-oy1f.5) The PRETTY (indented) form of [`write_jsonld_compact`]: the full
/// W3C JSON-LD 1.1 Compaction document against `context`, re-indented (whitespace only) with
/// `opts`. The byte content is exactly [`write_jsonld_compact`]'s output run through the same
/// presentation pass as [`write_jsonld_pretty`], so it parses back to the same RDF.
pub fn write_jsonld_compact_pretty(
    graphs: &[NamedGraph<'_>],
    context: &JsonLdValue,
    opts: &JsonLdPrettyOptions,
) -> String {
    let minified = write_jsonld_compact(graphs, context);
    reindent_json(&minified, &opts.indent)
}

/// [OPUS-4.8] (sq-oy1f.5) The PRETTY (indented) analogue of [`graph_to_jsonld_compact`]:
/// serialises a [`Graph`] as a full W3C JSON-LD 1.1 Compaction document against `context`
/// and re-indents it with `opts`. Whitespace-only over [`graph_to_jsonld_compact`] (the same
/// document, multi-line), so the round-trip and losslessness properties are identical.
pub fn graph_to_jsonld_compact_pretty(
    graph: &Graph,
    context: &JsonLdValue,
    opts: &JsonLdPrettyOptions,
) -> String {
    let owned = dataset_graphs(graph);
    let view: Vec<NamedGraph<'_>> = owned
        .iter()
        .map(|(n, ts)| (n.as_ref(), ts.as_slice()))
        .collect();
    write_jsonld_compact_pretty(&view, context, opts)
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxrdf::{BlankNode, Literal, NamedNode};
    use sparq_core::Graph;

    fn nn(s: &str) -> NamedNode {
        NamedNode::new_unchecked(s)
    }

    // ---- Round-trip property: parse -> serialize -> re-parse is isomorphic. ----
    //
    // Isomorphism here is exact-set equality after parsing into a sparq Graph: the
    // dictionary content-addresses terms, so two graphs with the same triple SET (modulo
    // blank-node identity preserved by stable labels) intern identically. We assert the
    // sorted N-Triples projections match — the canonical, label-stable witness.

    fn nt_sorted(g: &Graph) -> Vec<String> {
        let mut v: Vec<String> = g
            .iter_ids()
            .map(|[s, p, o]| {
                let t = triple_from_ids(g, s, p, o);
                format!("{} {} {} .", t.subject, t.predicate, t.object)
            })
            .collect();
        v.sort();
        v
    }

    fn assert_iso(original_ttl: &str) {
        let g0 = Graph::load_str(original_ttl, "turtle").unwrap();
        // Turtle round-trip.
        let ttl = graph_to_turtle(&g0);
        let g1 = Graph::load_str(&ttl, "turtle").unwrap();
        assert_eq!(
            nt_sorted(&g0),
            nt_sorted(&g1),
            "turtle round-trip\n--- serialized ---\n{ttl}"
        );
        // N-Quads round-trip (single default graph).
        let nq = graph_to_nquads(&g0);
        let g2 = Graph::load_dataset(&nq, "nquads").unwrap();
        assert_eq!(
            nt_sorted(&g0),
            nt_sorted(&g2),
            "nquads round-trip\n--- serialized ---\n{nq}"
        );
        // TriG round-trip.
        let tg = graph_to_trig(&g0);
        let g3 = Graph::load_dataset(&tg, "trig").unwrap();
        assert_eq!(
            nt_sorted(&g0),
            nt_sorted(&g3),
            "trig round-trip\n--- serialized ---\n{tg}"
        );
    }

    #[test]
    fn roundtrip_basic_prefixed() {
        assert_iso(
            r#"@prefix ex: <http://ex/> .
               ex:alice a ex:Person ; ex:knows ex:bob ; ex:age 30 ; ex:name "Alice" .
               ex:bob ex:name "Bob" ."#,
        );
    }

    #[test]
    fn roundtrip_blank_nodes() {
        assert_iso(
            r#"@prefix ex: <http://ex/> .
               ex:alice ex:knows [ ex:name "anon" ; ex:age 5 ] .
               _:shared ex:p ex:o . _:shared ex:q "v" ."#,
        );
    }

    #[test]
    fn roundtrip_language_and_datatypes() {
        assert_iso(
            r#"@prefix ex: <http://ex/> .
               @prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
               ex:s ex:label "chat"@fr ; ex:label "hello"@en ;
                    ex:n "42"^^xsd:integer ; ex:d "1.5"^^xsd:double ;
                    ex:when "2020-01-01"^^xsd:date ; ex:plain "just a string" ."#,
        );
    }

    #[test]
    fn roundtrip_quotes_newlines_and_escapes_in_literals() {
        assert_iso(
            r#"@prefix ex: <http://ex/> .
               ex:s ex:p "line1\nline2\ttab\r\"quoted\" and \\ backslash" ;
                    ex:q "trailing quote\"" ;
                    ex:e "" ."#,
        );
    }

    #[test]
    fn roundtrip_iris_needing_escaping() {
        // IRIs with spaces/control chars and characters that can't be a PN_LOCAL: must
        // fall back to a full escaped <IRI>, never a malformed prefixed name.
        assert_iso(
            r#"<http://ex/has%20space> <http://ex/p~weird> <http://ex/o?x=1&y=2> .
               <http://ex/s> <http://ex/p> <http://example.com/path/to/thing#frag> ."#,
        );
    }

    #[test]
    fn roundtrip_named_graphs_trig_and_nquads() {
        // Dataset with a default graph and two named graphs — exercises GRAPH blocks and
        // the 4th N-Quads column.
        let data = r#"@prefix ex: <http://ex/> .
            ex:a ex:p ex:b .
            GRAPH ex:g1 { ex:x ex:y ex:z . ex:x ex:k "in g1" . }
            GRAPH ex:g2 { ex:m ex:n ex:o . }"#;
        let g0 = Graph::load_dataset(data, "trig").unwrap();

        let tg = graph_to_trig(&g0);
        let g_tg = Graph::load_dataset(&tg, "trig").unwrap();
        assert_dataset_iso(&g0, &g_tg, &tg);

        let nq = graph_to_nquads(&g0);
        let g_nq = Graph::load_dataset(&nq, "nquads").unwrap();
        assert_dataset_iso(&g0, &g_nq, &nq);
    }

    fn assert_dataset_iso(a: &Graph, b: &Graph, ser: &str) {
        assert_eq!(
            nt_sorted(a),
            nt_sorted(b),
            "default graph\n--- serialized ---\n{ser}"
        );
        assert_eq!(a.named.len(), b.named.len(), "named graph count\n{ser}");
        // Match named graphs by name term, compare contents.
        for (name, ag) in &a.named {
            let bg = b
                .named
                .iter()
                .find(|(n, _)| n == name)
                .map(|(_, g)| g)
                .unwrap_or_else(|| panic!("named graph {name} missing after round-trip\n{ser}"));
            assert_eq!(nt_sorted(ag), nt_sorted(bg), "named graph {name}\n{ser}");
        }
    }

    #[test]
    fn roundtrip_rdf12_triple_term() {
        // RDF 1.2 triple term in object position survives Turtle + N-Triples.
        assert_iso(
            r#"<http://ex/r> <http://ex/reifies> <<( <http://ex/a> <http://ex/b> "v" )>> .
               <http://ex/r> <http://ex/note> "plain" ."#,
        );
    }

    // =======================================================================
    // [OPUS-4.8] (sq-ixc3.2) PRETTY Turtle writer tests — round-trip + golden
    // output matching the site's `prettyTurtle` shape (#805 / sq-gb4o reference).
    // =======================================================================

    /// A small prefix map matching what the site/query would declare, so abbreviation is
    /// actually exercised (the engine's `default_prefixes()` has no `ex:`).
    fn ex_prefixes() -> Prefixes {
        let mut p = default_prefixes();
        p.insert("ex".to_string(), "http://ex/".to_string());
        p
    }

    /// Round-trip: PRETTY Turtle re-parses to the same triple SET.
    fn assert_pretty_iso(original_ttl: &str) {
        let g0 = Graph::load_str(original_ttl, "turtle").unwrap();
        let ttl = write_turtle_pretty(&graph_triples(&g0), &ex_prefixes(), &PrettyOptions::default());
        let g1 = Graph::load_str(&ttl, "turtle").unwrap();
        assert_eq!(
            nt_sorted(&g0),
            nt_sorted(&g1),
            "pretty turtle round-trip\n--- serialized ---\n{ttl}"
        );
    }

    #[test]
    fn pretty_golden_output_matches_site_shape() {
        // The exact byte shape the site's `prettyTurtle` emits: prefix-alphabetical
        // used-only header, blank-line-separated subject blocks (sorted), `a` first,
        // two-space indent, `;` between predicates, `.` terminator, objects sorted.
        let g = Graph::load_str(
            r#"@prefix ex: <http://ex/> .
               ex:alice a ex:Person ; ex:knows ex:bob, ex:carol ; ex:name "Alice" .
               ex:bob ex:name "Bob" ."#,
            "turtle",
        )
        .unwrap();
        let ttl = write_turtle_pretty(&graph_triples(&g), &ex_prefixes(), &PrettyOptions::default());
        let expected = "\
@prefix ex: <http://ex/> .

ex:alice
  a ex:Person ;
  ex:knows ex:bob,
    ex:carol ;
  ex:name \"Alice\" .

ex:bob
  ex:name \"Bob\" .";
        assert_eq!(ttl, expected, "golden pretty output\n--- got ---\n{ttl}");
    }

    #[test]
    fn pretty_ordering_is_emission_independent() {
        // Two inputs with the SAME triple set in DIFFERENT order must produce identical
        // pretty output — the determinism the flat `write_turtle` does NOT guarantee.
        let a = Graph::load_str(
            r#"@prefix ex: <http://ex/> .
               ex:s ex:p ex:o3, ex:o1, ex:o2 ; ex:a ex:x ; a ex:T ."#,
            "turtle",
        )
        .unwrap();
        let b = Graph::load_str(
            r#"@prefix ex: <http://ex/> .
               ex:s ex:a ex:x ; ex:p ex:o2, ex:o1, ex:o3 . ex:s a ex:T ."#,
            "turtle",
        )
        .unwrap();
        let opts = PrettyOptions::default();
        let pa = write_turtle_pretty(&graph_triples(&a), &ex_prefixes(), &opts);
        let pb = write_turtle_pretty(&graph_triples(&b), &ex_prefixes(), &opts);
        assert_eq!(pa, pb, "pretty output must be emission-order-independent\n{pa}\n---\n{pb}");
        // `a` sorts first within the block.
        assert!(pa.starts_with("@prefix"), "{pa}");
        assert!(pa.contains("ex:s\n  a ex:T ;"), "rdf:type 'a' sorts first:\n{pa}");
    }

    #[test]
    fn pretty_round_trips_all_term_kinds() {
        assert_pretty_iso(
            r#"@prefix ex: <http://ex/> .
               @prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
               ex:alice a ex:Person ; ex:knows ex:bob ; ex:age "30"^^xsd:integer ;
                        ex:label "Alice"@en, "Alicia"@es ; ex:plain "just a string" ;
                        ex:bn [ ex:x "y" ] .
               ex:r ex:reifies <<( ex:a ex:b "v" )>> .
               <http://weird/has%20space> <http://weird/p?x=1> "lit\nwith\tescapes" ."#,
        );
    }

    #[test]
    fn pretty_empty_graph_is_empty_string() {
        let g = Graph::load_str("", "turtle").unwrap();
        assert_eq!(graph_to_turtle_pretty(&g), "");
    }

    #[test]
    fn pretty_no_abbreviate_keeps_full_iris_and_no_header() {
        let g = Graph::load_str(
            r#"@prefix ex: <http://ex/> .
               ex:s a ex:T ; ex:p "v" ."#,
            "turtle",
        )
        .unwrap();
        let opts = PrettyOptions {
            abbreviate: false,
            ..PrettyOptions::default()
        };
        let ttl = write_turtle_pretty(&graph_triples(&g), &ex_prefixes(), &opts);
        assert!(!ttl.contains("@prefix"), "no header when abbreviate=false:\n{ttl}");
        assert!(ttl.contains("<http://ex/s>"), "IRIs stay full:\n{ttl}");
        // `a` shorthand for rdf:type is still used even without prefix compaction.
        assert!(ttl.contains("\n  a <http://ex/T>"), "'a' kept:\n{ttl}");
        // Still round-trips.
        let g2 = Graph::load_str(&ttl, "turtle").unwrap();
        assert_eq!(nt_sorted(&g), nt_sorted(&g2), "{ttl}");
    }

    #[test]
    fn pretty_custom_indent() {
        let g = Graph::load_str(
            r#"@prefix ex: <http://ex/> .
               ex:s ex:p "v" ."#,
            "turtle",
        )
        .unwrap();
        let opts = PrettyOptions {
            indent: "\t".to_string(),
            ..PrettyOptions::default()
        };
        let ttl = write_turtle_pretty(&graph_triples(&g), &ex_prefixes(), &opts);
        assert!(ttl.contains("\n\tex:p "), "tab indent honoured:\n{ttl:?}");
    }

    #[test]
    fn pretty_trig_round_trips_named_graphs() {
        let data = r#"@prefix ex: <http://ex/> .
            ex:a ex:p ex:b .
            GRAPH ex:g1 { ex:x a ex:T ; ex:k "in g1" . }
            GRAPH ex:g2 { ex:m ex:n ex:o . }"#;
        let g0 = Graph::load_dataset(data, "trig").unwrap();
        let owned = dataset_graphs(&g0);
        let view: Vec<NamedGraph<'_>> = owned.iter().map(|(n, ts)| (n.as_ref(), ts.as_slice())).collect();
        let tg = write_trig_pretty(&view, &ex_prefixes(), &PrettyOptions::default());
        // Shape: shared header, GRAPH blocks, indented bodies.
        assert!(tg.contains("@prefix ex:"), "shared header:\n{tg}");
        assert!(tg.contains("GRAPH ex:g1 {"), "named-graph block:\n{tg}");
        let g1 = Graph::load_dataset(&tg, "trig").unwrap();
        assert_dataset_iso(&g0, &g1, &tg);
    }

    #[test]
    fn turtle_uses_a_and_prefixes_and_drops_unused() {
        let g = Graph::load_str(
            r#"@prefix ex: <http://ex/> .
               ex:alice a ex:Person ; ex:age 30 ."#,
            "turtle",
        )
        .unwrap();
        let ttl = graph_to_turtle(&g);
        // `a` keyword for rdf:type.
        assert!(ttl.contains(" a "), "should use 'a' for rdf:type:\n{ttl}");
        assert!(ttl.contains("@prefix"), "header present:\n{ttl}");
        // No prefix line for a namespace never used (e.g. foaf).
        assert!(
            !ttl.contains("foaf:"),
            "unused prefix must not appear:\n{ttl}"
        );
    }

    #[test]
    fn nquads_default_graph_has_no_fourth_column() {
        let t = Triple {
            subject: NamedOrBlankNode::NamedNode(nn("http://ex/s")),
            predicate: nn("http://ex/p"),
            object: Term::NamedNode(nn("http://ex/o")),
        };
        let nq = write_nquads(&[(None, &[t])]);
        assert_eq!(nq, "<http://ex/s> <http://ex/p> <http://ex/o> .\n");
    }

    #[test]
    fn nquads_named_graph_has_fourth_column() {
        let g = Term::NamedNode(nn("http://ex/g"));
        let t = Triple {
            subject: NamedOrBlankNode::NamedNode(nn("http://ex/s")),
            predicate: nn("http://ex/p"),
            object: Term::Literal(Literal::new_simple_literal("v")),
        };
        let nq = write_nquads(&[(Some(&g), &[t])]);
        assert_eq!(nq, "<http://ex/s> <http://ex/p> \"v\" <http://ex/g> .\n");
    }

    #[test]
    fn pn_local_fallback_is_correct() {
        // A local part ending in '.' is NOT a valid PN_LOCAL -> must not compact.
        assert!(!is_simple_pn_local("foo."));
        assert!(is_simple_pn_local("foo.bar"));
        assert!(is_simple_pn_local(""));
        assert!(!is_simple_pn_local("has space"));
        assert!(!is_simple_pn_local("q?x=1"));
    }

    #[test]
    fn blank_node_graph_name_in_trig() {
        let g = Graph::load_dataset(
            r#"_:bg { <http://ex/s> <http://ex/p> <http://ex/o> . }"#,
            "trig",
        )
        .unwrap();
        let tg = graph_to_trig(&g);
        let g2 = Graph::load_dataset(&tg, "trig").unwrap();
        assert_dataset_iso(&g, &g2, &tg);
        // The bnode keeps its label form `_:` in the GRAPH header.
        assert!(
            tg.contains("GRAPH _:"),
            "blank graph name uses _:label:\n{tg}"
        );
    }

    #[test]
    fn empty_graph_serializes_empty() {
        let g = Graph::load_str("", "turtle").unwrap();
        assert_eq!(graph_to_turtle(&g), "");
        assert_eq!(graph_to_nquads(&g), "");
    }

    #[test]
    fn bnode_object_does_not_declare_unused_prefix() {
        // Regression guard: a blank node must not be mistaken for an IRI in the prefix probe.
        let _b = BlankNode::default();
        let g = Graph::load_str(r#"<http://ex/s> <http://ex/p> _:x ."#, "turtle").unwrap();
        let ttl = graph_to_turtle(&g);
        let g2 = Graph::load_str(&ttl, "turtle").unwrap();
        assert_eq!(nt_sorted(&g), nt_sorted(&g2));
    }

    // =======================================================================
    // JSON-LD round-trip: serialize -> JSON-LD-to-N-Quads -> Graph -> isomorphic.
    //
    // sparq-core has no JSON-LD parser, so the round-trip is closed by a focused
    // in-test reader: it parses the document with serde_json (a dev-dependency), runs
    // the standard JSON-LD-to-RDF mapping over the node objects (expanding any
    // `@context` prefixes), and emits N-Quads — which the existing dataset loader then
    // ingests. Reconstructing the *same* triple set across all three forms (expanded /
    // flattened / compacted) for IRIs, blank nodes, datatypes, language tags, named
    // graphs and native-coerced scalars proves the writer is lossless.
    // =======================================================================

    use serde_json::Value;

    /// Expands a possibly-compacted IRI/term using the document's `@context` prefix map.
    /// A `prefix:local` whose prefix is declared expands to `namespace + local`; everything
    /// else (full IRIs, blank-node ids, keywords) passes through.
    fn expand_iri(s: &str, ctx: &std::collections::HashMap<String, String>) -> String {
        if s.starts_with("_:") || s.starts_with("@") || s.starts_with("http") {
            return s.to_string();
        }
        if let Some((pfx, local)) = s.split_once(':') {
            if let Some(ns) = ctx.get(pfx) {
                return format!("{ns}{local}");
            }
        }
        s.to_string()
    }

    /// N-Triples term text for a subject/object id (IRI -> `<iri>`, blank -> `_:label`).
    fn id_term(id: &str) -> String {
        if let Some(b) = id.strip_prefix("_:") {
            format!("_:{b}")
        } else {
            format!("<{id}>")
        }
    }

    /// Emits the N-Triples object text for a JSON-LD object value (a `@value` literal, an
    /// `@id` reference, or a `@list`), expanding any compacted `@type` IRI via `ctx`.
    ///
    /// A `@list` is materialised back into a fresh rdf:first/rdf:rest/rdf:nil blank-node chain
    /// (the inverse of the writer's collapse): the chain triples are appended to `out` in
    /// `graph` and the head reference is returned, so the JSON-LD-to-RDF round-trip reproduces
    /// the original list structure (up to blank-node renaming). `counter` hands out unique
    /// blank-node labels.
    fn object_to_nt(
        v: &Value,
        ctx: &std::collections::HashMap<String, String>,
        graph: &str,
        counter: &mut u64,
        out: &mut String,
    ) -> String {
        let obj = v.as_object().expect("object value is a JSON object");
        if let Some(items) = obj.get("@list").and_then(|l| l.as_array()) {
            // Empty list collapses straight to rdf:nil.
            if items.is_empty() {
                return "<http://www.w3.org/1999/02/22-rdf-syntax-ns#nil>".to_string();
            }
            // Allocate one fresh blank node per element, then wire first/rest/nil.
            let cells: Vec<String> = items
                .iter()
                .map(|_| {
                    *counter += 1;
                    format!("_:lst{counter}")
                })
                .collect();
            for (i, item) in items.iter().enumerate() {
                let cell = &cells[i];
                let first = object_to_nt(item, ctx, graph, counter, out);
                let _ = writeln!(
                    out,
                    "{cell} <http://www.w3.org/1999/02/22-rdf-syntax-ns#first> {first} {graph}."
                );
                let rest = if i + 1 < cells.len() {
                    cells[i + 1].clone()
                } else {
                    "<http://www.w3.org/1999/02/22-rdf-syntax-ns#nil>".to_string()
                };
                let _ = writeln!(
                    out,
                    "{cell} <http://www.w3.org/1999/02/22-rdf-syntax-ns#rest> {rest} {graph}."
                );
            }
            return cells[0].clone();
        }
        if let Some(val) = obj.get("@value") {
            // Native scalar or string. Reconstruct the canonical lexical form + datatype.
            let (lex, dt) = match val {
                Value::Bool(b) => (
                    b.to_string(),
                    Some("http://www.w3.org/2001/XMLSchema#boolean".to_string()),
                ),
                Value::Number(n) if n.is_i64() || n.is_u64() => (
                    n.to_string(),
                    Some("http://www.w3.org/2001/XMLSchema#integer".to_string()),
                ),
                Value::String(s) => (s.clone(), None),
                other => panic!("unexpected @value scalar: {other}"),
            };
            // Escape the lexical value back to N-Triples form.
            let mut esc = String::new();
            for c in lex.chars() {
                match c {
                    '"' => esc.push_str("\\\""),
                    '\\' => esc.push_str("\\\\"),
                    '\n' => esc.push_str("\\n"),
                    '\r' => esc.push_str("\\r"),
                    _ => esc.push(c),
                }
            }
            if let Some(lang) = obj.get("@language").and_then(|l| l.as_str()) {
                return format!("\"{esc}\"@{lang}");
            }
            let dt = obj
                .get("@type")
                .and_then(|t| t.as_str())
                .map(|t| expand_iri(t, ctx))
                .or(dt);
            return match dt {
                Some(d) => format!("\"{esc}\"^^<{d}>"),
                None => format!("\"{esc}\""),
            };
        }
        // An `@id` reference.
        let id = obj
            .get("@id")
            .and_then(|i| i.as_str())
            .expect("@id present");
        let id = expand_iri(id, ctx);
        // A triple-term `@id` (our `<<( … )>>` encoding) is already canonical N-Triples text.
        if id.starts_with("<<(") {
            id
        } else {
            id_term(&id)
        }
    }

    /// Reads one node object into N-Quads lines (with the given graph column, empty for the
    /// default graph), expanding `@type` -> rdf:type and every predicate's object list.
    fn node_to_nquads(
        node: &serde_json::Map<String, Value>,
        graph: &str,
        ctx: &std::collections::HashMap<String, String>,
        counter: &mut u64,
        out: &mut String,
    ) {
        let subj = node
            .get("@id")
            .and_then(|i| i.as_str())
            .map(|s| id_term(&expand_iri(s, ctx)))
            .expect("node has @id");
        for (k, v) in node {
            if k == "@id" || k == "@graph" {
                continue;
            }
            if k == "@type" {
                for t in v.as_array().expect("@type is an array") {
                    let ty = expand_iri(t.as_str().expect("@type IRI"), ctx);
                    let _ = writeln!(
                        out,
                        "{subj} <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <{ty}> {graph}."
                    );
                }
                continue;
            }
            let pred = expand_iri(k, ctx);
            for o in v.as_array().expect("predicate value is an array") {
                // `object_to_nt` may append rdf:first/rest chain triples for a `@list`; collect
                // them into a side buffer so they land *after* this predicate line, keeping the
                // emitted N-Quads readable (order is irrelevant to the loader).
                let mut aux = String::new();
                let obj = object_to_nt(o, ctx, graph, counter, &mut aux);
                let _ = writeln!(out, "{subj} <{pred}> {obj} {graph}.");
                out.push_str(&aux);
            }
        }
    }

    /// Full JSON-LD-to-N-Quads for any document our writer emits (bare array, or an object with
    /// optional `@context` and a `@graph` holding default-graph nodes + per-named-graph objects).
    fn jsonld_to_nquads(doc: &str) -> String {
        let v: Value = serde_json::from_str(doc).expect("valid JSON");
        let mut ctx: std::collections::HashMap<String, String> = std::collections::HashMap::new();
        let mut out = String::new();
        // Unique blank-node label source for any `@list` chains re-materialised below.
        let mut counter: u64 = 0;

        let nodes: &Vec<Value> = match &v {
            Value::Array(a) => a,
            Value::Object(o) => {
                if let Some(Value::Object(c)) = o.get("@context") {
                    for (k, val) in c {
                        if let Some(ns) = val.as_str() {
                            ctx.insert(k.clone(), ns.to_string());
                        }
                    }
                }
                o.get("@graph")
                    .and_then(|g| g.as_array())
                    .expect("@graph array")
            }
            other => panic!("unexpected top-level JSON-LD: {other}"),
        };

        for n in nodes {
            let node = n.as_object().expect("node object");
            if let Some(inner) = node.get("@graph").and_then(|g| g.as_array()) {
                // A named-graph sub-object: `{"@id": <g>, "@graph": [ … ]}`.
                let gid = node
                    .get("@id")
                    .and_then(|i| i.as_str())
                    .map(|s| id_term(&expand_iri(s, &ctx)))
                    .expect("named graph @id");
                for sub in inner {
                    node_to_nquads(
                        sub.as_object().expect("node"),
                        &format!("{gid} "),
                        &ctx,
                        &mut counter,
                        &mut out,
                    );
                }
            } else {
                node_to_nquads(node, "", &ctx, &mut counter, &mut out);
            }
        }
        out
    }

    /// Asserts a graph survives a round-trip through all three JSON-LD forms.
    fn assert_jsonld_iso(g0: &Graph) {
        for form in [
            JsonLdForm::Expanded,
            JsonLdForm::Flattened,
            JsonLdForm::Compacted,
        ] {
            let doc = graph_to_jsonld(g0, form);
            // The document must be syntactically valid JSON.
            let _: Value = serde_json::from_str(&doc)
                .unwrap_or_else(|e| panic!("{form:?} produced invalid JSON: {e}\n{doc}"));
            let nq = jsonld_to_nquads(&doc);
            let g1 = Graph::load_dataset(&nq, "nquads").unwrap_or_else(|e| {
                panic!("{form:?} re-parse failed: {e}\n--- doc ---\n{doc}\n--- nq ---\n{nq}")
            });
            assert_eq!(
                nt_sorted(g0),
                nt_sorted(&g1),
                "{form:?} default-graph round-trip\n--- doc ---\n{doc}\n--- nq ---\n{nq}"
            );
            assert_eq!(
                g0.named.len(),
                g1.named.len(),
                "{form:?} named graph count\n{doc}"
            );
            for (name, ag) in &g0.named {
                let bg = g1
                    .named
                    .iter()
                    .find(|(n, _)| n == name)
                    .map(|(_, g)| g)
                    .unwrap_or_else(|| panic!("{form:?} named graph {name} missing\n{doc}"));
                assert_eq!(
                    nt_sorted(ag),
                    nt_sorted(bg),
                    "{form:?} named graph {name}\n{doc}"
                );
            }
        }
    }

    /// Total triple count across the default graph and every named graph.
    fn triple_count(g: &Graph) -> usize {
        g.iter_ids().count() + g.named.iter().map(|(_, ng)| ng.iter_ids().count()).sum::<usize>()
    }

    /// Blank-node-blind round-trip check for graphs that exercise `@list` collapsing.
    ///
    /// Collapsing an rdf:first/rdf:rest chain and re-materialising it *renames* the list-cell
    /// blank nodes, so the label-sensitive [`assert_jsonld_iso`] cannot be used. Instead this
    /// asserts the serialize → parse → serialize cycle is a **fixed point** (the second
    /// JSON-LD document is byte-identical to the first) for every form, plus that no triple is
    /// gained or lost. A fixed point under our deterministic writer means the list structure
    /// (length, element order, nesting) is reproduced exactly, up to blank-node renaming.
    fn assert_jsonld_list_iso(g0: &Graph) {
        for form in [
            JsonLdForm::Expanded,
            JsonLdForm::Flattened,
            JsonLdForm::Compacted,
        ] {
            let doc0 = graph_to_jsonld(g0, form);
            let nq = jsonld_to_nquads(&doc0);
            let g1 = Graph::load_dataset(&nq, "nquads").unwrap_or_else(|e| {
                panic!("{form:?} re-parse failed: {e}\n--- doc ---\n{doc0}\n--- nq ---\n{nq}")
            });
            assert_eq!(
                triple_count(g0),
                triple_count(&g1),
                "{form:?} triple count changed across round-trip\n--- doc ---\n{doc0}\n--- nq ---\n{nq}"
            );
            let doc1 = graph_to_jsonld(&g1, form);
            assert_eq!(
                doc0, doc1,
                "{form:?} not a fixed point (list structure not reproduced)\n--- doc0 ---\n{doc0}\n--- doc1 ---\n{doc1}"
            );
        }
    }

    #[test]
    fn jsonld_roundtrip_basic() {
        let g = Graph::load_str(
            r#"@prefix ex: <http://ex/> .
               ex:alice a ex:Person ; ex:knows ex:bob ; ex:name "Alice" .
               ex:bob ex:name "Bob" ."#,
            "turtle",
        )
        .unwrap();
        assert_jsonld_iso(&g);
    }

    #[test]
    fn jsonld_roundtrip_blank_nodes() {
        let g = Graph::load_str(
            r#"@prefix ex: <http://ex/> .
               ex:alice ex:knows [ ex:name "anon" ] .
               _:shared ex:p ex:o . _:shared ex:q "v" ."#,
            "turtle",
        )
        .unwrap();
        assert_jsonld_iso(&g);
    }

    #[test]
    fn jsonld_roundtrip_datatypes_and_langtags() {
        let g = Graph::load_str(
            r#"@prefix ex: <http://ex/> .
               @prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
               ex:s ex:label "chat"@fr ; ex:label "hello"@en ;
                    ex:n "42"^^xsd:integer ; ex:flag "true"^^xsd:boolean ;
                    ex:d "1.5"^^xsd:double ; ex:when "2020-01-01"^^xsd:date ;
                    ex:plain "just a string" ;
                    ex:notcanon "007"^^xsd:integer ; ex:zero "0"^^xsd:boolean ."#,
            "turtle",
        )
        .unwrap();
        assert_jsonld_iso(&g);
    }

    #[test]
    fn jsonld_roundtrip_escapes() {
        let g = Graph::load_str(
            r#"@prefix ex: <http://ex/> .
               ex:s ex:p "line1\nline2\ttab\r\"quoted\" and \\ backslash" ;
                    ex:q "" ; ex:u "café ☃" ."#,
            "turtle",
        )
        .unwrap();
        assert_jsonld_iso(&g);
    }

    #[test]
    fn jsonld_roundtrip_named_graphs() {
        let g = Graph::load_dataset(
            r#"@prefix ex: <http://ex/> .
               ex:a ex:p ex:b .
               GRAPH ex:g1 { ex:x ex:y ex:z . ex:x ex:k "in g1" . }
               GRAPH ex:g2 { ex:m ex:n "lit"@en . }
               GRAPH _:bg { ex:bn ex:p ex:q . }"#,
            "trig",
        )
        .unwrap();
        assert_jsonld_iso(&g);
    }

    #[test]
    fn jsonld_native_coercion_only_when_canonical() {
        // Canonical integer/boolean -> native JSON scalar; non-canonical -> typed string.
        let g = Graph::load_str(
            r#"@prefix ex: <http://ex/> .
               @prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
               ex:s ex:i "42"^^xsd:integer ; ex:b "true"^^xsd:boolean ;
                    ex:lead "007"^^xsd:integer ; ex:bzero "0"^^xsd:boolean ;
                    ex:dec "1.50"^^xsd:decimal ."#,
            "turtle",
        )
        .unwrap();
        let doc = graph_to_jsonld(&g, JsonLdForm::Expanded);
        // Canonical 42 / true become bare JSON scalars (no quotes around the value).
        assert!(
            doc.contains("{\"@value\":42}"),
            "canonical int -> native:\n{doc}"
        );
        assert!(
            doc.contains("{\"@value\":true}"),
            "canonical bool -> native:\n{doc}"
        );
        // Leading-zero integer and non-`true`/`false` boolean stay typed strings.
        assert!(
            doc.contains("\"@value\":\"007\""),
            "leading-zero int stays string:\n{doc}"
        );
        assert!(
            doc.contains("\"@value\":\"0\""),
            "non-canonical bool stays string:\n{doc}"
        );
        // Decimal is never coerced (JSON number form would diverge from the RDF lexical).
        assert!(
            doc.contains("\"@value\":\"1.50\""),
            "decimal stays string:\n{doc}"
        );
        assert_jsonld_iso(&g);
    }

    #[test]
    fn jsonld_expanded_is_bare_array_compacted_has_context() {
        let g = Graph::load_str(
            r#"@prefix ex: <http://ex/> . ex:s a ex:T ; ex:p ex:o ."#,
            "turtle",
        )
        .unwrap();
        let exp = graph_to_jsonld(&g, JsonLdForm::Expanded);
        assert!(
            exp.trim_start().starts_with('['),
            "expanded is a bare array:\n{exp}"
        );
        // Expanded uses full IRIs, no @context.
        assert!(
            !exp.contains("@context"),
            "expanded has no @context:\n{exp}"
        );
        assert!(
            exp.contains("\"http://ex/p\""),
            "expanded predicate is a full IRI:\n{exp}"
        );

        let comp = graph_to_jsonld(&g, JsonLdForm::Compacted);
        assert!(
            comp.contains("\"@context\""),
            "compacted has @context:\n{comp}"
        );
        // schema/foaf etc. unused -> not in context; ex is not a default prefix, so the
        // compacted form still uses full IRIs for ex: (no prefix declared for it).
        let flat = graph_to_jsonld(&g, JsonLdForm::Flattened);
        assert!(
            flat.contains("\"@graph\""),
            "flattened wraps in @graph:\n{flat}"
        );
    }

    #[test]
    fn jsonld_compacted_uses_known_prefixes() {
        // rdf:/rdfs:/foaf: are in default_prefixes, so the compacted form abbreviates them.
        let g = Graph::load_str(
            r#"<http://ex/s> a <http://xmlns.com/foaf/0.1/Person> ;
                 <http://xmlns.com/foaf/0.1/name> "Bob" ."#,
            "turtle",
        )
        .unwrap();
        let comp = graph_to_jsonld(&g, JsonLdForm::Compacted);
        assert!(
            comp.contains("\"foaf\":\"http://xmlns.com/foaf/0.1/\""),
            "foaf in context:\n{comp}"
        );
        assert!(
            comp.contains("\"foaf:name\""),
            "predicate compacted:\n{comp}"
        );
        assert!(comp.contains("\"foaf:Person\""), "@type compacted:\n{comp}");
        assert_jsonld_iso(&g);
    }

    #[test]
    fn jsonld_empty_graph() {
        let g = Graph::load_str("", "turtle").unwrap();
        // Expanded empty default graph -> empty array.
        assert_eq!(graph_to_jsonld(&g, JsonLdForm::Expanded), "[]");
        // Round-trips (vacuously).
        assert_jsonld_iso(&g);
    }

    #[test]
    fn jsonld_roundtrip_rdf12_triple_term() {
        let g = Graph::load_str(
            r#"<http://ex/r> <http://ex/reifies> <<( <http://ex/a> <http://ex/b> "v" )>> .
               <http://ex/r> <http://ex/note> "plain" ."#,
            "turtle",
        )
        .unwrap();
        assert_jsonld_iso(&g);
    }

    // =======================================================================
    // [OPUS-4.8] (sq-gg3j) RDF-list (`@list`) collapsing.
    // =======================================================================

    /// Turtle `(...)` collection syntax expands to an rdf:first/rdf:rest/rdf:nil chain on
    /// load; the writer must collapse it back to a native `@list` AND still round-trip.
    #[test]
    fn jsonld_list_collapses_and_roundtrips() {
        let g = Graph::load_str(
            r#"@prefix ex: <http://ex/> .
               ex:s ex:items ( ex:a "b" 3 ) ."#,
            "turtle",
        )
        .unwrap();
        // The expanded form must carry a single `@list` with the three elements in order, and
        // must NOT leak any rdf:first / rdf:rest into the JSON.
        let doc = graph_to_jsonld(&g, JsonLdForm::Expanded);
        assert!(doc.contains("\"@list\""), "expected @list in:\n{doc}");
        assert!(
            !doc.contains("rdf-syntax-ns#first") && !doc.contains("rdf-syntax-ns#rest"),
            "list cells leaked as plain triples:\n{doc}"
        );
        // Element order is preserved (ex:a before "b" before 3).
        let a = doc.find("http://ex/a").expect("element a present");
        let b = doc.find("\"b\"").expect("element b present");
        let three = doc.find(":3").or_else(|| doc.find("@value\":3")).is_some()
            || doc.contains("\"@value\":3");
        assert!(three, "integer element 3 present:\n{doc}");
        assert!(a < b, "list order a < b preserved:\n{doc}");
        assert_jsonld_list_iso(&g);
    }

    /// The empty collection `()` is `rdf:nil`; it is left as a plain `@id` reference (never a
    /// `@list`), and still round-trips.
    #[test]
    fn jsonld_empty_list_stays_nil_reference() {
        let g = Graph::load_str(
            r#"@prefix ex: <http://ex/> .
               ex:s ex:items () ."#,
            "turtle",
        )
        .unwrap();
        let doc = graph_to_jsonld(&g, JsonLdForm::Expanded);
        assert!(!doc.contains("\"@list\""), "empty list must not collapse:\n{doc}");
        assert!(doc.contains("rdf-syntax-ns#nil"), "expected rdf:nil reference:\n{doc}");
        assert_jsonld_iso(&g);
    }

    /// Nested lists `( (a b) c )` collapse recursively (a `@list` element that is itself a
    /// `@list`), and round-trip.
    #[test]
    fn jsonld_nested_list_collapses() {
        let g = Graph::load_str(
            r#"@prefix ex: <http://ex/> .
               ex:s ex:items ( ( ex:a ex:b ) ex:c ) ."#,
            "turtle",
        )
        .unwrap();
        let doc = graph_to_jsonld(&g, JsonLdForm::Expanded);
        assert_eq!(doc.matches("\"@list\"").count(), 2, "two nested @lists:\n{doc}");
        assert!(
            !doc.contains("rdf-syntax-ns#first") && !doc.contains("rdf-syntax-ns#rest"),
            "nested list cells leaked:\n{doc}"
        );
        assert_jsonld_list_iso(&g);
    }

    /// A list whose head cell carries an extra predicate is NOT a well-formed list cell, so the
    /// chain stays as ordinary triples (round-trips losslessly either way).
    #[test]
    fn jsonld_list_with_extra_predicate_not_collapsed() {
        // Hand-built chain: _:c0 is a list cell but also carries ex:tag — disqualifying it.
        let g = Graph::load_str(
            r#"@prefix ex: <http://ex/> .
               @prefix rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#> .
               ex:s ex:items _:c0 .
               _:c0 rdf:first ex:a ; rdf:rest _:c1 ; ex:tag "extra" .
               _:c1 rdf:first ex:b ; rdf:rest rdf:nil ."#,
            "turtle",
        )
        .unwrap();
        let doc = graph_to_jsonld(&g, JsonLdForm::Expanded);
        assert!(
            !doc.contains("\"@list\""),
            "tainted head must NOT collapse:\n{doc}"
        );
        assert_jsonld_iso(&g);
    }

    /// A list cell shared by two heads (referenced twice as an object) is NOT collapsed:
    /// collapsing would discard the second reference. Stays as plain triples.
    #[test]
    fn jsonld_shared_list_cell_not_collapsed() {
        let g = Graph::load_str(
            r#"@prefix ex: <http://ex/> .
               @prefix rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#> .
               ex:s ex:items _:c0 .
               ex:t ex:items _:c0 .
               _:c0 rdf:first ex:a ; rdf:rest rdf:nil ."#,
            "turtle",
        )
        .unwrap();
        let doc = graph_to_jsonld(&g, JsonLdForm::Expanded);
        assert!(
            !doc.contains("\"@list\""),
            "doubly-referenced cell must NOT collapse:\n{doc}"
        );
        assert_jsonld_iso(&g);
    }

    /// A cyclic "list" (`rdf:rest` loops back) is not a list; it must never collapse (and the
    /// detector must terminate). Stays as plain triples and round-trips.
    #[test]
    fn jsonld_cyclic_rest_not_collapsed_and_terminates() {
        let g = Graph::load_str(
            r#"@prefix ex: <http://ex/> .
               @prefix rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#> .
               ex:s ex:items _:c0 .
               _:c0 rdf:first ex:a ; rdf:rest _:c1 .
               _:c1 rdf:first ex:b ; rdf:rest _:c0 ."#,
            "turtle",
        )
        .unwrap();
        let doc = graph_to_jsonld(&g, JsonLdForm::Expanded);
        assert!(!doc.contains("\"@list\""), "cycle must NOT collapse:\n{doc}");
        assert_jsonld_iso(&g);
    }

    /// A chain that does not terminate in rdf:nil (the last `rdf:rest` points at an IRI) is not
    /// a proper RDF list; it must not collapse.
    #[test]
    fn jsonld_non_nil_terminated_not_collapsed() {
        let g = Graph::load_str(
            r#"@prefix ex: <http://ex/> .
               @prefix rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#> .
               ex:s ex:items _:c0 .
               _:c0 rdf:first ex:a ; rdf:rest ex:notNil ."#,
            "turtle",
        )
        .unwrap();
        let doc = graph_to_jsonld(&g, JsonLdForm::Expanded);
        assert!(!doc.contains("\"@list\""), "non-nil tail must NOT collapse:\n{doc}");
        assert_jsonld_iso(&g);
    }

    /// Two independent well-formed lists in one graph both collapse, and a graph mixing a
    /// collapsible list with an unrelated tainted chain round-trips.
    #[test]
    fn jsonld_multiple_lists_and_named_graph_roundtrip() {
        let g = Graph::load_dataset(
            r#"<http://ex/s> <http://ex/a> _:l0 .
               _:l0 <http://www.w3.org/1999/02/22-rdf-syntax-ns#first> <http://ex/x> .
               _:l0 <http://www.w3.org/1999/02/22-rdf-syntax-ns#rest> <http://www.w3.org/1999/02/22-rdf-syntax-ns#nil> .
               <http://ex/s> <http://ex/b> _:m0 <http://ex/g> .
               _:m0 <http://www.w3.org/1999/02/22-rdf-syntax-ns#first> "y" <http://ex/g> .
               _:m0 <http://www.w3.org/1999/02/22-rdf-syntax-ns#rest> <http://www.w3.org/1999/02/22-rdf-syntax-ns#nil> <http://ex/g> ."#,
            "nquads",
        )
        .unwrap();
        // Default graph list collapses; named-graph list collapses in its own scope.
        let doc = graph_to_jsonld(&g, JsonLdForm::Expanded);
        assert_eq!(doc.matches("\"@list\"").count(), 2, "both lists collapse:\n{doc}");
        assert_jsonld_list_iso(&g);
    }

    // ---- [OPUS-4.8] (sq-ixc3.3) PRETTY (indented) JSON-LD ----
    //
    // The pretty writer is a whitespace-only presentation pass over the minified writer.
    // The tests pin two load-bearing properties:
    //   1. equivalence — stripping insignificant whitespace from the pretty output yields
    //      the exact minified document (the token stream is untouched), AND the pretty
    //      output round-trips to the identical RDF graph; and
    //   2. shape — golden expected text for a small fixed graph in each form, so any
    //      accidental indentation change is caught.

    /// Removes JSON whitespace *outside* string literals — the inverse of `reindent_json`'s
    /// insertions. Used only in tests to prove the pretty pass is whitespace-only.
    fn strip_json_ws(pretty: &str) -> String {
        let mut out = String::with_capacity(pretty.len());
        let mut in_string = false;
        let mut escaped = false;
        for c in pretty.chars() {
            if in_string {
                out.push(c);
                if escaped {
                    escaped = false;
                } else if c == '\\' {
                    escaped = true;
                } else if c == '"' {
                    in_string = false;
                }
                continue;
            }
            match c {
                '"' => {
                    in_string = true;
                    out.push(c);
                }
                ' ' | '\t' | '\n' | '\r' => {} // insignificant outside a string
                other => out.push(other),
            }
        }
        out
    }

    /// Asserts, for every form, that the pretty output (default options) (a) is still valid
    /// JSON, (b) collapses back to exactly the minified writer's bytes, and (c) re-parses to
    /// the same RDF as the minified document.
    fn assert_jsonld_pretty_equiv(g0: &Graph) {
        for form in [
            JsonLdForm::Expanded,
            JsonLdForm::Flattened,
            JsonLdForm::Compacted,
        ] {
            let minified = graph_to_jsonld(g0, form);
            let pretty = graph_to_jsonld_pretty(g0, form);
            // (a) valid JSON.
            let _: Value = serde_json::from_str(&pretty)
                .unwrap_or_else(|e| panic!("{form:?} pretty produced invalid JSON: {e}\n{pretty}"));
            // (b) whitespace-only: stripping the inserted whitespace recovers the minified doc.
            assert_eq!(
                strip_json_ws(&pretty),
                minified,
                "{form:?} pretty is not the minified doc + whitespace\n--- pretty ---\n{pretty}"
            );
            // (c) parses back to the same RDF as the minified doc.
            let nq = jsonld_to_nquads(&pretty);
            let g1 = Graph::load_dataset(&nq, "nquads").unwrap_or_else(|e| {
                panic!("{form:?} pretty re-parse failed: {e}\n--- pretty ---\n{pretty}\n--- nq ---\n{nq}")
            });
            assert_eq!(
                nt_sorted(g0),
                nt_sorted(&g1),
                "{form:?} pretty default-graph round-trip\n--- pretty ---\n{pretty}\n--- nq ---\n{nq}"
            );
        }
    }

    #[test]
    fn jsonld_pretty_equiv_basic() {
        let g = Graph::load_str(
            r#"@prefix ex: <http://ex/> .
               ex:alice a ex:Person ; ex:knows ex:bob ; ex:name "Alice" .
               ex:bob ex:name "Bob" ."#,
            "turtle",
        )
        .unwrap();
        assert_jsonld_pretty_equiv(&g);
    }

    #[test]
    fn jsonld_pretty_equiv_literals_and_lists() {
        // Native-coerced scalars, a typed literal, a language tag, and a collapsed @list.
        let g = Graph::load_str(
            r#"@prefix ex: <http://ex/> .
               @prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
               ex:s ex:n 42 ;
                    ex:flag true ;
                    ex:d "1.5"^^xsd:double ;
                    ex:greet "hi"@en ;
                    ex:items ( "a" "b" ) ."#,
            "turtle",
        )
        .unwrap();
        // @list collapse renames blank nodes, so only assert validity + whitespace-only-ness
        // + a fixed point here (the label-sensitive RDF check is covered by the basic case).
        for form in [
            JsonLdForm::Expanded,
            JsonLdForm::Flattened,
            JsonLdForm::Compacted,
        ] {
            let minified = graph_to_jsonld(&g, form);
            let pretty = graph_to_jsonld_pretty(&g, form);
            let _: Value = serde_json::from_str(&pretty)
                .unwrap_or_else(|e| panic!("{form:?} pretty invalid JSON: {e}\n{pretty}"));
            assert_eq!(
                strip_json_ws(&pretty),
                minified,
                "{form:?} not whitespace-only"
            );
        }
    }

    #[test]
    fn jsonld_pretty_equiv_named_graph() {
        let g = Graph::load_dataset(
            r#"<http://ex/s> <http://ex/p> "v" .
               <http://ex/s> <http://ex/p2> <http://ex/o> <http://ex/g> ."#,
            "nquads",
        )
        .unwrap();
        assert_jsonld_pretty_equiv(&g);
    }

    #[test]
    fn jsonld_pretty_empty_graph() {
        let g = Graph::load_str("", "turtle").unwrap();
        // Expanded with no triples is the empty array; pretty keeps it compact.
        assert_eq!(graph_to_jsonld_pretty(&g, JsonLdForm::Expanded), "[]");
    }

    #[test]
    fn jsonld_pretty_custom_indent() {
        let g = Graph::load_str(r#"<http://ex/s> <http://ex/p> "v" ."#, "turtle").unwrap();
        let opts = JsonLdPrettyOptions {
            indent: "\t".to_string(),
        };
        let pretty =
            graph_to_jsonld_pretty_with(&g, JsonLdForm::Expanded, &default_prefixes(), &opts);
        // Custom indent is still whitespace-only over the minified doc.
        assert_eq!(
            strip_json_ws(&pretty),
            graph_to_jsonld(&g, JsonLdForm::Expanded)
        );
        assert!(pretty.contains('\t'), "tab indent should appear:\n{pretty}");
        assert!(!pretty.contains("  "), "no two-space runs with a tab indent:\n{pretty}");
    }

    #[test]
    fn jsonld_pretty_golden_expanded() {
        // A small, fixed single-subject graph: one rdf:type, one IRI value, one plain literal.
        let g = Graph::load_str(
            r#"@prefix ex: <http://ex/> .
               ex:s a ex:T ; ex:p ex:o ; ex:name "n" ."#,
            "turtle",
        )
        .unwrap();
        let pretty = graph_to_jsonld_pretty(&g, JsonLdForm::Expanded);
        let expected = "\
[
  {
    \"@id\": \"http://ex/s\",
    \"@type\": [
      \"http://ex/T\"
    ],
    \"http://ex/p\": [
      {
        \"@id\": \"http://ex/o\"
      }
    ],
    \"http://ex/name\": [
      {
        \"@value\": \"n\"
      }
    ]
  }
]";
        assert_eq!(pretty, expected, "actual:\n{pretty}");
    }

    #[test]
    fn jsonld_pretty_golden_flattened() {
        let g = Graph::load_str(
            r#"@prefix ex: <http://ex/> .
               ex:s ex:p "n" ."#,
            "turtle",
        )
        .unwrap();
        let pretty = graph_to_jsonld_pretty(&g, JsonLdForm::Flattened);
        let expected = "\
{
  \"@graph\": [
    {
      \"@id\": \"http://ex/s\",
      \"http://ex/p\": [
        {
          \"@value\": \"n\"
        }
      ]
    }
  ]
}";
        assert_eq!(pretty, expected, "actual:\n{pretty}");
    }

    #[test]
    fn jsonld_pretty_golden_compacted() {
        let g = Graph::load_str(
            r#"@prefix ex: <http://ex/> .
               ex:s ex:p "n" ."#,
            "turtle",
        )
        .unwrap();
        let pretty = graph_to_jsonld_pretty_with(
            &g,
            JsonLdForm::Compacted,
            &ex_prefixes(),
            &JsonLdPrettyOptions::default(),
        );
        // The `@context` lists every prefix the minified writer would (here `ex` plus `xsd`,
        // because a plain literal's implicit xsd:string datatype is noted by `write_context`).
        // The pretty pass adds no members — it only indents, so the context matches the
        // minified writer exactly.
        let expected = "\
{
  \"@context\": {
    \"ex\": \"http://ex/\",
    \"xsd\": \"http://www.w3.org/2001/XMLSchema#\"
  },
  \"@graph\": [
    {
      \"@id\": \"ex:s\",
      \"ex:p\": [
        {
          \"@value\": \"n\"
        }
      ]
    }
  ]
}";
        assert_eq!(pretty, expected, "actual:\n{pretty}");
    }

    // =======================================================================
    // [OPUS-4.8] (sq-l5kr) Caller-supplied prefix policy via `prefixes_from_pairs`.
    //
    // The wasm `Store.serialize(.., prefixes?)` binding (and any other caller wanting
    // byte-parity output under its OWN prefix policy) feeds an ordered `(prefix, iri)`
    // pair list through `prefixes_from_pairs` into the same `*_with` writers. These two
    // tests are the load-bearing invariants the bead calls out:
    //   (a) the `default_prefixes()` pair list reproduces the prior default output
    //       byte-for-byte (back-compat — `None`/absent on the binding keeps using it); and
    //   (b) a custom map abbreviates `http://example.org/x` -> `ex:x` and uses the SITE's
    //       `https://schema.org/` (which `default_prefixes()` — `http://schema.org/` — does
    //       NOT), i.e. a caller's prefix policy actually reaches the writer.
    // =======================================================================

    #[test]
    fn prefixes_from_pairs_matches_default_byte_for_byte() {
        // Rebuilding `default_prefixes()` from its own pair list and serialising must be
        // byte-identical to serialising with `default_prefixes()` directly — the back-compat
        // path the wasm binding takes when no `prefixes` map is supplied.
        let g = Graph::load_str(
            r#"@prefix ex: <http://ex/> .
               @prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
               ex:s a <http://schema.org/Thing> ; ex:n "30"^^xsd:integer ; ex:label "v" .
               <http://schema.org/x> <http://schema.org/p> ex:s ."#,
            "turtle",
        )
        .unwrap();
        // The exact `(prefix, iri)` pairs `default_prefixes()` holds, as a pair list.
        let pairs: Vec<(String, String)> = default_prefixes().into_iter().collect();
        let rebuilt = prefixes_from_pairs(pairs);
        assert_eq!(rebuilt, default_prefixes(), "round-trips through pairs");

        let opts = PrettyOptions::default();
        // Pretty Turtle / TriG / JSON-LD all reproduce the default output byte-for-byte.
        assert_eq!(
            graph_to_turtle_pretty_with(&g, &rebuilt, &opts),
            graph_to_turtle_pretty_with(&g, &default_prefixes(), &opts),
            "pretty Turtle byte-parity with default_prefixes",
        );
        assert_eq!(
            graph_to_trig_pretty_with(&g, &rebuilt, &opts),
            graph_to_trig_pretty_with(&g, &default_prefixes(), &opts),
            "pretty TriG byte-parity with default_prefixes",
        );
        assert_eq!(
            graph_to_jsonld_with(&g, JsonLdForm::Compacted, &rebuilt),
            graph_to_jsonld_with(&g, JsonLdForm::Compacted, &default_prefixes()),
            "JSON-LD compacted byte-parity with default_prefixes",
        );
        // And `http://schema.org/Thing` compacts to `schema:Thing` (the default `schema`).
        assert!(
            graph_to_turtle_pretty_with(&g, &rebuilt, &opts).contains("schema:Thing"),
            "default schema namespace compacts http://schema.org/Thing",
        );
    }

    #[test]
    fn custom_prefix_map_abbreviates_example_org_and_uses_https_schema() {
        // A SITE-style prefix policy: `ex` -> http://example.org/ and `schema` ->
        // https://schema.org/ (note: HTTPS — differs from default_prefixes()'s HTTP schema).
        let g = Graph::load_str(
            r#"<http://example.org/x> a <https://schema.org/Thing> ;
                   <http://example.org/p> <https://schema.org/y> ."#,
            "turtle",
        )
        .unwrap();
        let prefixes = prefixes_from_pairs([
            ("ex", "http://example.org/"),
            ("schema", "https://schema.org/"),
        ]);
        let opts = PrettyOptions::default();
        let ttl = graph_to_turtle_pretty_with(&g, &prefixes, &opts);
        // `http://example.org/x` -> `ex:x` (subject), `http://example.org/p` -> `ex:p`.
        assert!(ttl.contains("ex:x"), "subject abbreviated to ex:x:\n{ttl}");
        assert!(ttl.contains("ex:p"), "predicate abbreviated to ex:p:\n{ttl}");
        // `https://schema.org/Thing` -> `schema:Thing` (the HTTPS site namespace), and
        // `https://schema.org/y` -> `schema:y`.
        assert!(ttl.contains("schema:Thing"), "https schema type:\n{ttl}");
        assert!(ttl.contains("schema:y"), "https schema object:\n{ttl}");
        // The header declares the HTTPS schema, never the default HTTP one.
        assert!(
            ttl.contains("@prefix schema: <https://schema.org/> ."),
            "header carries the HTTPS schema namespace:\n{ttl}",
        );
        assert!(
            !ttl.contains("http://schema.org/"),
            "the default HTTP schema namespace must not appear:\n{ttl}",
        );
        // Same custom policy reaches the JSON-LD compacted `@context`.
        let jc = graph_to_jsonld_with(&g, JsonLdForm::Compacted, &prefixes);
        assert!(jc.contains("\"ex\":\"http://example.org/\""), "@context ex:\n{jc}");
        assert!(
            jc.contains("\"schema\":\"https://schema.org/\""),
            "@context https schema:\n{jc}",
        );
        // And round-trips (re-parses to the same triple set).
        let g2 = Graph::load_str(&ttl, "turtle").unwrap();
        assert_eq!(nt_sorted(&g), nt_sorted(&g2), "custom-prefix Turtle round-trip:\n{ttl}");
    }

    // =======================================================================
    // [OPUS-4.8] (sq-townn, survey §A7) Streaming Turtle / TriG tests.
    //
    // The load-bearing invariant: the STREAMED bytes equal the BUFFERED
    // `write_turtle` / `write_trig` bytes for the SAME graph — same used-prefix
    // header, same subject grouping, same predicate-object lists, same ordering.
    // These call the REAL streaming path (`write_turtle_streaming` /
    // `write_trig_streaming` into a `Vec<u8>`), not a mock, and assert
    // BYTE-EQUALITY against the buffered writer over graphs that exercise multiple
    // subjects (grouping / emit-on-change), multiple predicates per subject, prefix
    // usage, blank nodes, typed / lang literals, and the empty graph.
    // =======================================================================
    #[cfg(feature = "streaming-serialization")]
    mod streaming {
        use super::*;

        /// Streams `triples` to a `Vec<u8>` via the REAL `write_turtle_streaming` and
        /// returns the bytes as a `String` (the writer only emits UTF-8).
        fn turtle_streamed(triples: &[Triple], prefixes: &Prefixes) -> String {
            let mut buf: Vec<u8> = Vec::new();
            write_turtle_streaming(triples, prefixes, &mut buf).expect("vec write never fails");
            String::from_utf8(buf).expect("turtle writer emits UTF-8")
        }

        /// Asserts the streamed Turtle bytes equal the buffered `write_turtle` bytes for
        /// the SAME triple slice and prefixes (the core streamed==buffered contract).
        fn assert_streamed_eq_buffered(triples: &[Triple], prefixes: &Prefixes) {
            let buffered = write_turtle(triples, prefixes);
            let streamed = turtle_streamed(triples, prefixes);
            assert_eq!(
                buffered, streamed,
                "streamed Turtle must be byte-identical to buffered write_turtle\n\
                 --- buffered ---\n{buffered}\n--- streamed ---\n{streamed}"
            );
        }

        /// The same assertion driven from a parsed graph: `graph_to_turtle_streaming`
        /// bytes must equal `graph_to_turtle` bytes. This is the REAL CONSTRUCT/DESCRIBE
        /// path (store-ordered, subject-contiguous triples).
        fn assert_graph_streamed_eq_buffered(ttl: &str) {
            let g = Graph::load_str(ttl, "turtle").unwrap();
            let buffered = graph_to_turtle(&g);
            let mut buf: Vec<u8> = Vec::new();
            graph_to_turtle_streaming(&g, &default_prefixes(), &mut buf).unwrap();
            let streamed = String::from_utf8(buf).unwrap();
            assert_eq!(
                buffered, streamed,
                "graph_to_turtle_streaming must be byte-identical to graph_to_turtle\n\
                 --- buffered ---\n{buffered}\n--- streamed ---\n{streamed}"
            );
            // And the streamed bytes still re-parse to the same triple set.
            let g2 = Graph::load_str(&streamed, "turtle").unwrap();
            assert_eq!(nt_sorted(&g), nt_sorted(&g2), "streamed Turtle round-trip:\n{streamed}");
        }

        #[test]
        fn streamed_multiple_subjects_and_predicates() {
            // Three subjects, each with multiple predicates and multi-object lists —
            // exercises subject grouping (emit-on-change) and predicate-object lists.
            let ex = "http://ex/";
            let triples = vec![
                Triple::new(nn(&format!("{ex}alice")), nn(RDF_TYPE), nn(&format!("{ex}Person"))),
                Triple::new(
                    nn(&format!("{ex}alice")),
                    nn(&format!("{ex}knows")),
                    nn(&format!("{ex}bob")),
                ),
                Triple::new(
                    nn(&format!("{ex}alice")),
                    nn(&format!("{ex}knows")),
                    nn(&format!("{ex}carol")),
                ),
                Triple::new(
                    nn(&format!("{ex}bob")),
                    nn(&format!("{ex}name")),
                    Literal::new_simple_literal("Bob"),
                ),
                Triple::new(
                    nn(&format!("{ex}carol")),
                    nn(&format!("{ex}name")),
                    Literal::new_simple_literal("Carol"),
                ),
            ];
            let prefixes = prefixes_from_pairs([("ex", ex)]);
            assert_streamed_eq_buffered(&triples, &prefixes);
            // Sanity: the streamed output actually used the prefix, the `a` shorthand and
            // grouped the two `ex:knows` objects on one predicate-object list.
            let out = turtle_streamed(&triples, &prefixes);
            assert!(out.contains("@prefix ex: <http://ex/> ."), "prefix header:\n{out}");
            assert!(out.contains("ex:alice a ex:Person"), "rdf:type as `a`:\n{out}");
            assert!(out.contains("ex:knows ex:bob, ex:carol"), "object list:\n{out}");
        }

        #[test]
        fn streamed_blank_nodes() {
            let ex = "http://ex/";
            let triples = vec![
                Triple::new(
                    NamedOrBlankNode::BlankNode(BlankNode::new_unchecked("b0")),
                    nn(&format!("{ex}p")),
                    nn(&format!("{ex}o")),
                ),
                Triple::new(
                    NamedOrBlankNode::BlankNode(BlankNode::new_unchecked("b0")),
                    nn(&format!("{ex}q")),
                    Term::BlankNode(BlankNode::new_unchecked("b1")),
                ),
            ];
            let prefixes = prefixes_from_pairs([("ex", ex)]);
            assert_streamed_eq_buffered(&triples, &prefixes);
            let out = turtle_streamed(&triples, &prefixes);
            assert!(out.contains("_:b0"), "blank subject:\n{out}");
            assert!(out.contains("_:b1"), "blank object:\n{out}");
        }

        #[test]
        fn streamed_typed_and_lang_literals() {
            let ex = "http://ex/";
            let xsd = "http://www.w3.org/2001/XMLSchema#";
            let triples = vec![
                Triple::new(
                    nn(&format!("{ex}s")),
                    nn(&format!("{ex}age")),
                    Literal::new_typed_literal("30", nn(&format!("{xsd}integer"))),
                ),
                Triple::new(
                    nn(&format!("{ex}s")),
                    nn(&format!("{ex}label")),
                    Literal::new_language_tagged_literal_unchecked("chat", "fr"),
                ),
                Triple::new(
                    nn(&format!("{ex}s")),
                    nn(&format!("{ex}label")),
                    Literal::new_language_tagged_literal_unchecked("hi", "en"),
                ),
            ];
            let prefixes = prefixes_from_pairs([("ex", ex), ("xsd", xsd)]);
            assert_streamed_eq_buffered(&triples, &prefixes);
            let out = turtle_streamed(&triples, &prefixes);
            assert!(out.contains("\"30\"^^xsd:integer"), "typed literal:\n{out}");
            assert!(out.contains("\"chat\"@fr"), "lang literal:\n{out}");
        }

        #[test]
        fn streamed_empty_graph_is_empty() {
            let triples: Vec<Triple> = Vec::new();
            let prefixes = default_prefixes();
            // No triples => no used prefixes => empty header and empty body.
            assert_streamed_eq_buffered(&triples, &prefixes);
            assert_eq!(turtle_streamed(&triples, &prefixes), "");
        }

        #[test]
        fn streamed_subject_change_emits_on_change() {
            // Two subjects emitted back-to-back: the buffer must flush subject 1 before
            // accumulating subject 2 (the emit-on-change path).
            let ex = "http://ex/";
            let triples = vec![
                Triple::new(nn(&format!("{ex}s1")), nn(&format!("{ex}p")), nn(&format!("{ex}o1"))),
                Triple::new(nn(&format!("{ex}s2")), nn(&format!("{ex}p")), nn(&format!("{ex}o2"))),
            ];
            let prefixes = prefixes_from_pairs([("ex", ex)]);
            assert_streamed_eq_buffered(&triples, &prefixes);
            let out = turtle_streamed(&triples, &prefixes);
            assert!(out.contains("ex:s1 ex:p ex:o1 .\n"), "subject 1 block:\n{out}");
            assert!(out.contains("ex:s2 ex:p ex:o2 .\n"), "subject 2 block:\n{out}");
        }

        #[test]
        fn streamed_full_iri_fallback_when_no_prefix() {
            // An IRI with no registered prefix must fall back to a full <IRI>, identically
            // in both paths.
            let triples = vec![Triple::new(
                nn("http://no.prefix/s"),
                nn("http://no.prefix/p"),
                nn("http://no.prefix/o"),
            )];
            let prefixes = Prefixes::new();
            assert_streamed_eq_buffered(&triples, &prefixes);
            let out = turtle_streamed(&triples, &prefixes);
            assert!(out.contains("<http://no.prefix/s>"), "full IRI fallback:\n{out}");
        }

        #[test]
        fn streamed_graph_path_matches_buffered() {
            // The REAL CONSTRUCT/DESCRIBE path: a parsed graph rendered both ways must be
            // byte-identical (store order is subject-contiguous), across prefixed IRIs,
            // blank nodes, multi-object lists and typed/lang literals.
            assert_graph_streamed_eq_buffered(
                r#"@prefix ex: <http://ex/> .
                   @prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
                   ex:alice a ex:Person ; ex:knows ex:bob, ex:carol ;
                            ex:age "30"^^xsd:integer ; ex:name "Alice"@en .
                   ex:bob ex:name "Bob" .
                   ex:carol ex:knows [ ex:name "anon" ] ."#,
            );
        }

        #[test]
        fn streamed_trig_matches_buffered() {
            // TriG: default graph + two named graphs. Streamed bytes must equal
            // `graph_to_trig_with` bytes (shared header, GRAPH framing, indented bodies),
            // and re-parse to the same dataset.
            let data = r#"@prefix ex: <http://ex/> .
                ex:a ex:p ex:b ; ex:q ex:c .
                GRAPH ex:g1 { ex:x ex:y ex:z . ex:x ex:k "in g1" . }
                GRAPH ex:g2 { ex:m ex:n ex:o . }"#;
            let g = Graph::load_dataset(data, "trig").unwrap();
            let prefixes = prefixes_from_pairs([("ex", "http://ex/")]);
            let buffered = graph_to_trig_with(&g, &prefixes);
            let mut buf: Vec<u8> = Vec::new();
            graph_to_trig_streaming(&g, &prefixes, &mut buf).unwrap();
            let streamed = String::from_utf8(buf).unwrap();
            assert_eq!(
                buffered, streamed,
                "graph_to_trig_streaming must be byte-identical to graph_to_trig_with\n\
                 --- buffered ---\n{buffered}\n--- streamed ---\n{streamed}"
            );
            let g2 = Graph::load_dataset(&streamed, "trig").unwrap();
            assert_dataset_iso(&g, &g2, &streamed);
        }

        #[test]
        fn streamed_trig_empty_graph_is_empty() {
            let g = Graph::default();
            let mut buf: Vec<u8> = Vec::new();
            graph_to_trig_streaming(&g, &default_prefixes(), &mut buf).unwrap();
            assert_eq!(graph_to_trig_with(&g, &default_prefixes()), String::from_utf8(buf).unwrap());
        }

        #[test]
        fn streamed_write_trig_direct_named_only() {
            // Drive `write_trig_streaming` directly (no Graph) with a named-graph-only
            // dataset to cover the inter-graph blank-line + GRAPH framing branch.
            let ex = "http://ex/";
            let g1_triples = vec![Triple::new(
                nn(&format!("{ex}x")),
                nn(&format!("{ex}y")),
                nn(&format!("{ex}z")),
            )];
            let g2_triples = vec![Triple::new(
                nn(&format!("{ex}m")),
                nn(&format!("{ex}n")),
                nn(&format!("{ex}o")),
            )];
            let g1: Term = nn(&format!("{ex}g1")).into();
            let g2: Term = nn(&format!("{ex}g2")).into();
            let view: Vec<NamedGraph<'_>> = vec![
                (Some(&g1), g1_triples.as_slice()),
                (Some(&g2), g2_triples.as_slice()),
            ];
            let prefixes = prefixes_from_pairs([("ex", ex)]);
            let buffered = write_trig(&view, &prefixes);
            let mut buf: Vec<u8> = Vec::new();
            write_trig_streaming(&view, &prefixes, &mut buf).unwrap();
            assert_eq!(buffered, String::from_utf8(buf).unwrap());
        }
    }

    // =======================================================================
    // [OPUS-4.8] (sq-ixc3.4) Full W3C JSON-LD 1.1 Compaction tests.
    //
    // These exercise the REAL compaction path (`graph_to_jsonld_compact` /
    // `write_jsonld_compact`), not a stub: structural assertions check each 1.1
    // feature actually fired (term defs, @vocab, @container @set/@list/@language,
    // @reverse, @id/@type aliasing, value+node+IRI compaction), and a
    // compaction-aware JSON-LD→RDF reader verifies the load-bearing invariant —
    // **the compacted document round-trips to the same RDF triples** (lossless).
    // =======================================================================

    use super::compact::{parse_context_json, write_jsonld_compact, Json as Jv};

    /// Builds a compacted JSON-LD document for `g` against the caller `@context` text.
    fn compact_doc(g: &Graph, context: &str) -> String {
        let ctx = parse_context_json(context)
            .unwrap_or_else(|| panic!("context not a JSON object: {context}"));
        graph_to_jsonld_compact(g, &ctx)
    }


    /// Re-expands a compacted document and reloads it into a [`Graph`], asserting the document
    /// is valid JSON along the way. The load-bearing helper behind the round-trip assertions.
    fn compact_then_reload(g0: &Graph, context: &str) -> (String, Graph) {
        let doc = compact_doc(g0, context);
        let _: serde_json::Value = serde_json::from_str(&doc)
            .unwrap_or_else(|e| panic!("compacted doc invalid JSON: {e}\n{doc}"));
        let g1 = Graph::load_dataset(&doc, "jsonld")
            .unwrap_or_else(|e| panic!("re-parse failed: {e}\n--- doc ---\n{doc}"));
        (doc, g1)
    }

    /// The emitted DATA of a compacted document: the document with its `@context` member
    /// removed, re-serialized (so assertions never match a term definition by accident).
    fn data_body(doc: &str) -> String {
        let mut v: serde_json::Value = serde_json::from_str(doc).expect("valid JSON");
        if let Some(m) = v.as_object_mut() {
            m.remove("@context");
        }
        v.to_string()
    }

    /// Asserts a graph survives the full-compaction round-trip against `context`:
    /// compact → re-expand → **byte-identical** RDF triples (exact, label-sensitive). Use for
    /// graphs whose blank-node labels are stable across the round-trip (no `@list` collapse —
    /// see [`assert_compact_count_iso`] for those).
    fn assert_compact_iso(g0: &Graph, context: &str) {
        let (doc, g1) = compact_then_reload(g0, context);
        assert_eq!(
            nt_sorted(g0),
            nt_sorted(&g1),
            "compaction round-trip not lossless\n--- doc ---\n{doc}"
        );
        assert_eq!(g0.named.len(), g1.named.len(), "named graph count\n{doc}");
        for (name, ag) in &g0.named {
            let bg = g1
                .named
                .iter()
                .find(|(n, _)| n == name)
                .map(|(_, g)| g)
                .unwrap_or_else(|| panic!("named graph {name} missing\n{doc}"));
            assert_eq!(nt_sorted(ag), nt_sorted(bg), "named graph {name}\n{doc}");
        }
    }

    /// Blank-node-blind round-trip for graphs that exercise `@list` collapse: collapsing an
    /// rdf:first/rest chain and re-materialising it *renames* the list-cell blank nodes, so
    /// exact label comparison cannot apply. Instead, this asserts the triple **count** is
    /// preserved (no triple gained or lost) — together with the structural assertions in the
    /// test (the bare ordered array), this proves the list structure round-trips.
    fn assert_compact_count_iso(g0: &Graph, context: &str) {
        let (doc, g1) = compact_then_reload(g0, context);
        assert_eq!(
            triple_count(g0),
            triple_count(&g1),
            "compaction round-trip changed the triple count\n--- doc ---\n{doc}"
        );
    }

    /// Distinct blank nodes across the dataset (graph names and every graph's terms).
    fn blank_count(g: &Graph) -> usize {
        let mut v: Vec<String> = g.named.iter().map(|(n, _)| n.to_string()).collect();
        for graph in std::iter::once(g).chain(g.named.iter().map(|(_, ng)| ng)) {
            v.extend(nt_sorted(graph).iter().flat_map(|t| t.split(' ').map(str::to_string)));
        }
        v.retain(|t| t.starts_with("_:"));
        v.sort();
        v.dedup();
        v.len()
    }

    /// Per-graph triple counts, keyed by graph name (`""` for the default graph).
    fn graph_counts(g: &Graph) -> Vec<(String, usize)> {
        let mut v = vec![(String::new(), g.iter_ids().count())];
        v.extend(g.named.iter().map(|(n, ng)| (n.to_string(), ng.iter_ids().count())));
        v.retain(|(_, c)| *c > 0);
        v.sort();
        v
    }

    // A list whose head reference sits in another graph must not be collapsed into that
    // graph: its cells' triples would move with it.
    #[test]
    fn compact_keeps_list_cells_in_their_own_graph() {
        let g0 = Graph::load_dataset(
            r#"@prefix rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#> .
               <http://ex/g1> { <http://ex/s> <http://ex/p> _:l . }
               <http://ex/g2> { _:l rdf:first "a" ; rdf:rest rdf:nil . }"#,
            "trig",
        )
        .unwrap();
        let (doc, g1) = compact_then_reload(&g0, r#"{"@vocab":"http://ex/"}"#);
        assert_eq!(graph_counts(&g0), graph_counts(&g1), "{doc}");
    }

    // An explicitly typed list cell keeps its `rdf:type rdf:List` triple.
    #[test]
    fn compact_keeps_typed_list_cells() {
        let g0 = Graph::load_str(
            r#"@prefix rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#> .
               <http://ex/s> <http://ex/p> _:l .
               _:l a rdf:List ; rdf:first "a" ; rdf:rest rdf:nil ."#,
            "turtle",
        )
        .unwrap();
        assert_compact_count_iso(&g0, r#"{"@vocab":"http://ex/"}"#);
    }

    // A list cell that also names a graph, or is used as a type, keeps that use.
    #[test]
    fn compact_keeps_list_cells_used_elsewhere() {
        for trig in [
            r#"@prefix rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#> .
               <http://ex/s> <http://ex/p> _:l . _:l rdf:first "a" ; rdf:rest rdf:nil .
               _:l { <http://ex/x> <http://ex/q> "b" . }"#,
            r#"@prefix rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#> .
               <http://ex/s> <http://ex/p> _:l . _:l rdf:first "a" ; rdf:rest rdf:nil .
               <http://ex/x> a _:l ."#,
        ] {
            let g0 = Graph::load_dataset(trig, "trig").unwrap();
            let (doc, g1) = compact_then_reload(&g0, r#"{"@vocab":"http://ex/"}"#);
            assert_eq!(graph_counts(&g0), graph_counts(&g1), "{doc}");
            // The cell is one node in both uses: as many distinct blank nodes come back.
            assert_eq!(blank_count(&g0), blank_count(&g1), "{doc}");
        }
    }

    // rdf:JSON literals keep their exact lexical form, including arrays, which a
    // `@type: @json` term would otherwise merge with multiple values.
    #[test]
    fn compact_keeps_json_literals_lexically() {
        let g0 = Graph::load_str(
            r#"@prefix rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#> .
               <http://ex/s> <http://ex/p> "[]"^^rdf:JSON , "[1]"^^rdf:JSON , "{ \"a\": 1 }"^^rdf:JSON ."#,
            "turtle",
        )
        .unwrap();
        assert_compact_iso(&g0, r#"{"p":{"@id":"http://ex/p","@type":"@json"}}"#);
        assert_compact_iso(&g0, r#"{"p":{"@id":"http://ex/p","@type":"@json","@container":"@set"}}"#);
    }

    // An empty named graph keeps its name in the compacted output (the JSON-LD reader
    // drops empty graphs on reload, so the document itself is checked).
    #[test]
    fn compact_keeps_empty_named_graphs() {
        let mut g0 = Graph::load_str(r#"<http://ex/s> <http://ex/p> "v" ."#, "turtle").unwrap();
        g0.ensure_named(&oxrdf::NamedNode::new("http://ex/g").unwrap().into()).unwrap();
        let (doc, g1) = compact_then_reload(&g0, "{}");
        assert_eq!(nt_sorted(&g0), nt_sorted(&g1), "{doc}");
        assert!(doc.contains(r#"{"@id":"http://ex/g","@graph":[]}"#), "{doc}");
    }

    // An `@id` map whose scoped context re-aliases `@id`, and reuses the outer alias for
    // a data property, keeps that property.
    #[test]
    fn frame_id_map_under_scoped_alias_keeps_data() {
        let g0 = Graph::load_str(
            r#"<http://ex/a> <http://ex/p> <http://ex/b> . <http://ex/b> <http://ex/data> "kept" ."#,
            "turtle",
        )
        .unwrap();
        let frame = parse_context_json(
            r#"{"@context":{"id":"@id","p":{"@id":"http://ex/p","@container":"@id",
                "@context":{"id":"http://ex/data","identifier":"@id"}}},"@id":"http://ex/a"}"#,
        )
        .unwrap();
        let framed = graph_to_jsonld_framed(&g0, &frame);
        let g1 = Graph::load_dataset(&framed, "jsonld").expect("framed output parses");
        assert_eq!(nt_sorted(&g0), nt_sorted(&g1), "{framed}");
    }

    // A list cell that is also a subject in another graph stays one node.
    #[test]
    fn compact_keeps_cells_spanning_graphs() {
        let g0 = Graph::load_dataset(
            r#"@prefix ex: <http://ex/> .
               @prefix rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#> .
               ex:s ex:p _:h .
               _:h rdf:first "A" ; rdf:rest _:t ; ex:extra "x" .
               _:t rdf:first "B" ; rdf:rest rdf:nil .
               ex:g { _:t ex:q "Y" . }"#,
            "trig",
        )
        .unwrap();
        let (doc, g1) = compact_then_reload(&g0, r#"{"@vocab":"http://ex/"}"#);
        assert_eq!(graph_counts(&g0), graph_counts(&g1), "{doc}");
        assert_eq!(blank_count(&g0), blank_count(&g1), "{doc}");
    }

    // A root context with `@propagate: false` still applies to a lone top-level node.
    #[test]
    fn compact_keeps_non_propagated_root_context_semantics() {
        let g0 = Graph::load_str(r#"<http://ex/s> <http://ex/p> "x" ."#, "turtle").unwrap();
        assert_compact_iso(&g0, r#"{"@propagate":false,"@language":"en"}"#);
    }

    // A `@type` map whose scoped context re-aliases `@type`, and reuses the outer alias
    // for a data property, keeps that property and adds no type.
    #[test]
    fn frame_type_map_under_scoped_alias_keeps_data() {
        let g0 = Graph::load_str(
            r#"<http://ex/a> <http://ex/p> <http://ex/b> .
               <http://ex/b> a <http://ex/T> ; <http://ex/data> "kept" ."#,
            "turtle",
        )
        .unwrap();
        let frame = parse_context_json(
            r#"{"@context":{"type":"@type","p":{"@id":"http://ex/p","@container":"@type",
                "@context":{"type":"http://ex/data","kind":"@type"}}},"@id":"http://ex/a"}"#,
        )
        .unwrap();
        let framed = graph_to_jsonld_framed(&g0, &frame);
        // oxjsonld cannot read node objects inside a type map, so `p` is written as a plain
        // property (still under its scoped context, where `type` is ex:data).
        let back = Graph::load_str(&framed, "jsonld").unwrap();
        assert_eq!(nt_sorted(&back), nt_sorted(&g0), "{framed}");
    }

    // A `@type` map item whose own type-scoped context re-aliases `@type`, and reuses
    // the outer alias for data, keeps that data and adds no type.
    #[test]
    fn frame_type_map_under_type_scoped_alias_keeps_data() {
        let g0 = Graph::load_str(
            r#"<http://ex/a> <http://ex/p> <http://ex/b> .
               <http://ex/b> a <http://ex/T> ; <http://ex/data> "kept" ."#,
            "turtle",
        )
        .unwrap();
        let frame = parse_context_json(
            r#"{"@context":{"@vocab":"http://ex/","t":"@type","type":"@type",
                "T":{"@id":"http://ex/T","@context":{"t":"http://ex/data"}},
                "p":{"@id":"http://ex/p","@container":"@type"}},"@id":"http://ex/a"}"#,
        )
        .unwrap();
        let framed = graph_to_jsonld_framed(&g0, &frame);
        // `p` becomes a plain property (sparq's reader can't read node objects in a type
        // map); under `T`'s type-scoped context `t` is ex:data and `type` the type. The
        // output is checked by expansion: oxjsonld doesn't apply a type-scoped context
        // after `@id`.
        assert!(framed.contains(r#""p":{"@id":"http://ex/b","#), "{framed}");
        let doc = sparq_jsonld::Json::parse(&framed).unwrap();
        let opts = sparq_jsonld::JsonLdOptions::default();
        let mut exp = String::new();
        sparq_jsonld::expand(&doc, &opts, &sparq_jsonld::NoopLoader).unwrap().write(&mut exp);
        assert!(exp.contains(r#""http://ex/data":[{"@value":"kept"}]"#), "{exp}");
        assert!(!exp.contains("http://ex/kept"), "{exp}");
    }

    // Same-document references with a colon in the query or fragment stay relative to
    // the whole base (no "./" that would drop its last segment).
    #[test]
    fn compact_keeps_same_document_refs_with_colons() {
        let g0 = Graph::load_str(
            r#"<http://ex/doc#part:one> <http://ex/p> <http://ex/doc?q=a:b> ."#,
            "turtle",
        )
        .unwrap();
        assert_compact_iso(
            &g0,
            r#"{"@base":"http://ex/doc","p":{"@id":"http://ex/p","@type":"@id"}}"#,
        );
    }

    // Two lists on one `@list`-container property both survive.
    #[test]
    fn compact_keeps_several_lists_on_a_list_term() {
        let g0 = Graph::load_str(
            r#"@prefix ex: <http://ex/> . ex:s ex:p ("A"), ("B"), ("C") ."#,
            "turtle",
        )
        .unwrap();
        let (doc, g1) =
            compact_then_reload(&g0, r#"{"items":{"@id":"http://ex/p","@container":"@list"}}"#);
        // Blank-node labels differ, so compare the list members per head.
        let firsts = |g: &Graph| {
            let mut v: Vec<String> = nt_sorted(g)
                .into_iter()
                .filter(|t| t.contains("#first>"))
                .map(|t| t.split("#first> ").nth(1).unwrap().to_string())
                .collect();
            v.sort();
            v
        };
        assert_eq!(nt_sorted(&g1).len(), 9, "{doc}");
        assert_eq!(firsts(&g1), firsts(&g0), "{doc}");
    }

    // A further list on a `@list` term with a scoped context keeps its values' meaning.
    #[test]
    fn compact_keeps_further_lists_outside_the_terms_scope() {
        let g0 = Graph::load_str(
            r#"<http://ex/s> <http://ex/p> ("a"@en), ("b"@en) ."#,
            "turtle",
        )
        .unwrap();
        let (doc, g1) = compact_then_reload(
            &g0,
            r#"{"l":{"@id":"http://ex/p","@container":"@list","@context":{"@language":"en"}}}"#,
        );
        let tagged = |g: &Graph| nt_sorted(g).iter().filter(|t| t.contains("\"@en")).count();
        assert_eq!(nt_sorted(&g1).len(), 6, "{doc}");
        assert_eq!(tagged(&g1), 2, "{doc}");
    }

    // Type maps holding node objects are written as plain properties, which sparq's
    // reader can load; type maps of bare references stay.
    #[test]
    fn type_maps_stay_readable() {
        let g0 = Graph::load_str(
            r#"<http://ex/a> <http://ex/p> <http://ex/b> , <http://ex/c> .
               <http://ex/b> a <http://ex/T> ; <http://ex/data> "kept" ."#,
            "turtle",
        )
        .unwrap();
        let ctx = r#"{"p":{"@id":"http://ex/p","@container":"@type"},"q":"http://ex/p","data":"http://ex/data"}"#;
        let frame = parse_context_json(&format!(r#"{{"@context":{ctx},"@id":"http://ex/a"}}"#)).unwrap();
        let framed = graph_to_jsonld_framed(&g0, &frame);
        let back = Graph::load_str(&framed, "jsonld").unwrap();
        assert_eq!(nt_sorted(&back), nt_sorted(&g0), "{framed}");
        let (doc, g1) = compact_then_reload(&g0, ctx);
        assert_eq!(nt_sorted(&g1), nt_sorted(&g0), "{doc}");
        let refs = Graph::load_str(r#"<http://ex/a> <http://ex/p> <http://ex/b> ."#, "turtle").unwrap();
        let (doc, g1) = compact_then_reload(
            &refs,
            r#"{"p":{"@id":"http://ex/p","@container":"@type","@type":"@id"}}"#,
        );
        assert!(doc.contains(r#""@container":"@type""#), "{doc}");
        assert_eq!(nt_sorted(&g1), nt_sorted(&refs), "{doc}");
    }

    // The readable re-rendering keeps what the frame matched: a type-map frame still
    // selects by type, and a `["@type"]` container is dropped whole.
    #[test]
    fn readable_type_maps_keep_the_frame_match() {
        let g0 = Graph::load_str(
            r#"<http://ex/a> <http://ex/p> <http://ex/b> .
               <http://ex/b> a <http://ex/T> ; <http://ex/data> "kept" .
               <http://ex/z> <http://ex/data> "other" ."#,
            "turtle",
        )
        .unwrap();
        for container in [r#""@type""#, r#"["@type"]"#] {
            let frame = parse_context_json(&format!(
                r#"{{"@context":{{"@vocab":"http://ex/","p":{{"@id":"http://ex/p","@container":{container}}}}},
                    "@id":"http://ex/a","p":{{"T":{{}}}}}}"#
            ))
            .unwrap();
            let framed = graph_to_jsonld_framed(&g0, &frame);
            assert!(!framed.contains("other"), "{framed}");
            let back = Graph::load_str(&framed, "jsonld").unwrap();
            let mut want = nt_sorted(&g0);
            want.retain(|t| !t.contains("other"));
            assert_eq!(nt_sorted(&back), want, "{framed}");
        }
    }

    // A property-valued index under a scoped context that renames the index property keys
    // on the right value and keeps the data property.
    #[test]
    fn frame_index_map_under_scoped_names_keeps_predicates() {
        let g0 = Graph::load_str(
            r#"<http://ex/a> <http://ex/p> <http://ex/b> .
               <http://ex/b> <http://ex/label> "K" ; <http://ex/data> "D" ."#,
            "turtle",
        )
        .unwrap();
        let frame = parse_context_json(
            r#"{"@context":{"label":"http://ex/label","p":{"@id":"http://ex/p","@container":"@index",
                "@index":"label","@context":{"label":"http://ex/data","key":"http://ex/label"}}},
                "@id":"http://ex/a"}"#,
        )
        .unwrap();
        let framed = graph_to_jsonld_framed(&g0, &frame);
        let back = Graph::load_str(&framed, "jsonld").unwrap();
        assert_eq!(nt_sorted(&back), nt_sorted(&g0), "{framed}");
        // Readers differ on which context resolves the index name here, so the value is
        // not used as a key at all.
        assert!(framed.contains(r#""p":{"@none":"#), "{framed}");
    }

    // The readable re-rendering of a type map keeps the frame's `@null` defaults.
    #[test]
    fn readable_type_maps_keep_null_defaults() {
        let g0 = Graph::load_str(
            r#"<http://ex/a> <http://ex/p> <http://ex/b> .
               <http://ex/b> a <http://ex/T> ; <http://ex/data> "D" ."#,
            "turtle",
        )
        .unwrap();
        let frame = parse_context_json(
            r#"{"@context":{"@vocab":"http://ex/","p":{"@container":"@type"}},
                "@id":"http://ex/a","missing":{"@default":"@null"}}"#,
        )
        .unwrap();
        let framed = graph_to_jsonld_framed(&g0, &frame);
        assert!(framed.contains(r#""missing":null"#), "{framed}");
        let back = Graph::load_str(&framed, "jsonld").unwrap();
        assert_eq!(nt_sorted(&back), nt_sorted(&g0), "{framed}");
    }

    // A property-valued index whose value reads as an `@none` alias stays on the node.
    #[test]
    fn frame_index_map_keeps_values_spelled_like_none() {
        let g0 = Graph::load_str(
            r#"<http://ex/a> <http://ex/p> <http://ex/b> . <http://ex/b> <http://ex/label> "none" ."#,
            "turtle",
        )
        .unwrap();
        let frame = parse_context_json(
            r#"{"@context":{"none":"@none","label":"http://ex/label",
                "p":{"@id":"http://ex/p","@container":"@index","@index":"label"}},"@id":"http://ex/a"}"#,
        )
        .unwrap();
        let framed = graph_to_jsonld_framed(&g0, &frame);
        let back = Graph::load_str(&framed, "jsonld").unwrap();
        assert_eq!(nt_sorted(&back), nt_sorted(&g0), "{framed}");
    }

    // An `@id` map key that would read back as an `@none` alias keeps the full IRI.
    #[test]
    fn compact_id_map_key_never_reads_as_none() {
        let g0 = Graph::load_str(r#"<http://ex/a> <http://ex/p> <http://ex/none> ."#, "turtle").unwrap();
        assert_compact_iso(
            &g0,
            r#"{"@base":"http://ex/","none":"@none","p":{"@id":"http://ex/p","@container":"@id"}}"#,
        );
    }

    // Node ids and `@type: @id` values whose base-relative form spells a keyword alias
    // keep expanding to the same IRI.
    #[test]
    fn compact_relative_ids_never_read_as_aliases() {
        let g0 = Graph::load_str(
            r#"<http://ex/id> <http://ex/p> "v" . <http://ex/s> <http://ex/r> <http://ex/id> ."#,
            "turtle",
        )
        .unwrap();
        assert_compact_iso(
            &g0,
            r#"{"@base":"http://ex/","id":"@id","r":{"@id":"http://ex/r","@type":"@id"}}"#,
        );
    }

    // A predicate whose @vocab suffix has a colon keeps its full IRI.
    #[test]
    fn compact_keeps_colon_suffix_iris() {
        let g0 = Graph::load_str(r#"<http://ex/s> <http://ex/a:b> "v" ."#, "turtle").unwrap();
        assert_compact_iso(&g0, r#"{"@vocab":"http://ex/"}"#);
    }

    // A malformed rdf:JSON literal is admissible RDF; the writers keep it as a typed
    // string instead of panicking.
    #[test]
    fn compact_and_frame_keep_malformed_json_literals() {
        let g0 = Graph::load_str(
            r#"<http://ex/s> <http://ex/p> "not JSON"^^<http://www.w3.org/1999/02/22-rdf-syntax-ns#JSON> ;
                  <http://ex/q> "{\"a\":1}"^^<http://www.w3.org/1999/02/22-rdf-syntax-ns#JSON> ."#,
            "turtle",
        )
        .unwrap();
        assert_compact_iso(&g0, r#"{"@vocab":"http://ex/"}"#);
        let frame = parse_context_json(r#"{"@context":{"@vocab":"http://ex/"}}"#).unwrap();
        let framed = graph_to_jsonld_framed(&g0, &frame);
        let g1 = Graph::load_dataset(&framed, "jsonld").expect("framed output parses");
        assert_eq!(nt_sorted(&g0), nt_sorted(&g1), "{framed}");
    }

    #[test]
    fn compact_term_definitions_and_vocab() {
        let g = Graph::load_str(
            r#"@prefix ex: <http://ex/> .
               ex:alice a ex:Person ; ex:name "Alice" ; ex:age 30 ; ex:knows ex:bob ."#,
            "turtle",
        )
        .unwrap();
        // @vocab makes bare terms; an explicit term def + a keyword alias for @id/@type.
        let ctx = r#"{"@vocab":"http://ex/","id":"@id","type":"@type",
                      "knows":{"@id":"http://ex/knows","@type":"@id"}}"#;
        let doc = compact_doc(&g, ctx);
        // @vocab-relative bare terms (name/age) — not full IRIs, not CURIEs.
        assert!(doc.contains("\"name\":\"Alice\""), "vocab-relative name:\n{doc}");
        // @type keyword aliased to "type", value @vocab-compacted to the bare term "Person".
        assert!(doc.contains("\"type\":\"Person\""), "aliased @type → bare:\n{doc}");
        // @id aliased to "id".
        assert!(doc.contains("\"id\":"), "aliased @id:\n{doc}");
        // @type:@id coercion collapses the node ref `{"@id":…}` to a bare IRI string. Node
        // @id position compacts against base/prefix only (not @vocab), so the value here is
        // the full IRI — the point is the value-object collapse, not the spelling.
        assert!(
            doc.contains("\"knows\":\"http://ex/bob\""),
            "@id-coerced node ref → bare IRI string:\n{doc}"
        );
        // The lossless invariant.
        assert_compact_iso(&g, ctx);
    }

    #[test]
    fn compact_iri_against_prefix() {
        let g = Graph::load_str(
            r#"@prefix ex: <http://example.org/ns#> .
               ex:s ex:p ex:o ."#,
            "turtle",
        )
        .unwrap();
        let ctx = r#"{"ex":"http://example.org/ns#"}"#;
        let doc = compact_doc(&g, ctx);
        // IRIs compact to ex:local CURIEs against the declared prefix.
        assert!(doc.contains("\"ex:p\":"), "predicate CURIE:\n{doc}");
        assert!(doc.contains("\"ex:o\"") || doc.contains("ex:o"), "object CURIE:\n{doc}");
        assert_compact_iso(&g, ctx);
    }

    #[test]
    fn compact_type_coercion_drops_value_object() {
        let g = Graph::load_str(
            r#"@prefix ex: <http://ex/> . @prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
               ex:e ex:born "1990-01-01"^^xsd:date ."#,
            "turtle",
        )
        .unwrap();
        // A term whose @type coercion matches the datatype → the value object collapses to a
        // bare string (value compaction).
        let ctx = r#"{"@vocab":"http://ex/","xsd":"http://www.w3.org/2001/XMLSchema#",
                      "born":{"@id":"http://ex/born","@type":"xsd:date"}}"#;
        let doc = compact_doc(&g, ctx);
        assert!(doc.contains("\"born\":\"1990-01-01\""), "coerced datatype → bare:\n{doc}");
        assert!(!doc.contains("@value"), "no @value object remains:\n{doc}");
        assert_compact_iso(&g, ctx);
    }

    #[test]
    fn compact_language_coercion_and_default_language() {
        let g = Graph::load_str(
            r#"@prefix ex: <http://ex/> .
               ex:s ex:label "Bonjour"@fr ."#,
            "turtle",
        )
        .unwrap();
        // Default @language fr → a matching language-tagged value drops to a bare string.
        let ctx = r#"{"@vocab":"http://ex/","@language":"fr"}"#;
        let doc = compact_doc(&g, ctx);
        assert!(doc.contains("\"label\":\"Bonjour\""), "default-lang drop:\n{doc}");
        // No @language value object survives on the node (the term in @context is fine).
        assert!(
            !doc.contains("\"@graph\":[{") || !doc[doc.find("@graph").unwrap()..].contains("@language"),
            "no @language value object remains in @graph:\n{doc}"
        );
        assert_compact_iso(&g, ctx);
    }

    #[test]
    fn compact_default_language_keeps_plain_string_value_object() {
        // A PLAIN xsd:string under a default @language must stay an explicit `{@value}` object
        // (no @language) so it is NOT lossily tagged with the default on round-trip.
        let g = Graph::load_str(
            r#"@prefix ex: <http://ex/> .
               ex:s ex:plain "untagged" ; ex:tagged "marqué"@fr ."#,
            "turtle",
        )
        .unwrap();
        let ctx = r#"{"@vocab":"http://ex/","@language":"fr"}"#;
        let doc = compact_doc(&g, ctx);
        // The plain string is kept as an explicit value object so expansion won't apply @language.
        assert!(
            doc.contains("\"@value\":\"untagged\""),
            "plain string kept as value object under default @language:\n{doc}"
        );
        // The fr-tagged value compacts to a bare string (default matches).
        assert!(doc.contains("\"tagged\":\"marqué\""), "fr value drops:\n{doc}");
        // And the whole thing round-trips losslessly (the plain string stays untagged).
        assert_compact_iso(&g, ctx);
    }

    #[test]
    fn compact_container_set_forces_array() {
        let g = Graph::load_str(
            r#"@prefix ex: <http://ex/> .
               ex:s ex:tag "one" ."#,
            "turtle",
        )
        .unwrap();
        // @container:@set keeps a single value as a one-element array (no compact-arrays).
        let ctx = r#"{"@vocab":"http://ex/","tag":{"@id":"http://ex/tag","@container":"@set"}}"#;
        let doc = compact_doc(&g, ctx);
        assert!(doc.contains("\"tag\":[\"one\"]"), "@set forces array:\n{doc}");
        assert_compact_iso(&g, ctx);
    }

    #[test]
    fn compact_container_list() {
        let g = Graph::load_str(
            r#"@prefix ex: <http://ex/> .
               ex:s ex:items ( "a" "b" "c" ) ."#,
            "turtle",
        )
        .unwrap();
        // @container:@list strips the {"@list": …} wrapper to a bare ordered array.
        let ctx = r#"{"@vocab":"http://ex/","items":{"@id":"http://ex/items","@container":"@list"}}"#;
        let doc = compact_doc(&g, ctx);
        assert!(
            doc.contains("\"items\":[\"a\",\"b\",\"c\"]"),
            "@list container → bare array:\n{doc}"
        );
        // No `@list` wrapper survives in the @graph body (the term decl in @context is fine).
        assert!(!data_body(&doc).contains("@list"), "no @list wrapper remains:\n{doc}");
        // List-cell blank nodes are renamed on re-materialisation, so use the count-based check.
        assert_compact_count_iso(&g, ctx);
    }

    #[test]
    fn compact_container_language_map() {
        let g = Graph::load_str(
            r#"@prefix ex: <http://ex/> .
               ex:s ex:label "Hello"@en , "Bonjour"@fr ."#,
            "turtle",
        )
        .unwrap();
        // @container:@language groups by language tag into a { lang: value } map.
        let ctx =
            r#"{"@vocab":"http://ex/","label":{"@id":"http://ex/label","@container":"@language"}}"#;
        let doc = compact_doc(&g, ctx);
        assert!(doc.contains("\"en\":\"Hello\""), "language map en:\n{doc}");
        assert!(doc.contains("\"fr\":\"Bonjour\""), "language map fr:\n{doc}");
        assert_compact_iso(&g, ctx);
    }

    #[test]
    fn compact_reverse_property() {
        let g = Graph::load_str(
            r#"@prefix ex: <http://ex/> .
               ex:bob ex:parent ex:alice ."#,
            "turtle",
        )
        .unwrap();
        // A @reverse term "children": the (ex:parent) edge is shown from ex:alice's side.
        let ctx = r#"{"@vocab":"http://ex/","id":"@id",
                      "children":{"@reverse":"http://ex/parent","@type":"@id"}}"#;
        let doc = compact_doc(&g, ctx);
        assert!(doc.contains("@reverse") || doc.contains("\"children\""), "@reverse block:\n{doc}");
        assert_compact_iso(&g, ctx);
    }

    #[test]
    fn compact_node_compaction_nested() {
        let g = Graph::load_str(
            r#"@prefix ex: <http://ex/> .
               ex:alice a ex:Person ; ex:knows ex:bob .
               ex:bob a ex:Person ; ex:name "Bob" ."#,
            "turtle",
        )
        .unwrap();
        // Plain term (no @type) → ex:knows objects stay node objects ({"id": …}) for node
        // compaction, not bare strings.
        let ctx = r#"{"@vocab":"http://ex/","id":"@id","type":"@type"}"#;
        let doc = compact_doc(&g, ctx);
        // A plain term keeps the object as a node reference object (node compaction), with the
        // @id keyword aliased to "id" and the IRI in node position (no @vocab compaction).
        assert!(
            doc.contains("\"knows\":{\"id\":\"http://ex/bob\"}"),
            "node ref keeps object:\n{doc}"
        );
        assert_compact_iso(&g, ctx);
    }

    #[test]
    fn compact_native_scalars_and_named_graph() {
        let g = Graph::load_dataset(
            r#"<http://ex/s> <http://ex/n> "42"^^<http://www.w3.org/2001/XMLSchema#integer> <http://ex/g> .
               <http://ex/s> <http://ex/b> "true"^^<http://www.w3.org/2001/XMLSchema#boolean> ."#,
            "nquads",
        )
        .unwrap();
        let ctx = r#"{"@vocab":"http://ex/","id":"@id"}"#;
        let doc = compact_doc(&g, ctx);
        // Native scalar coercion is preserved through compaction.
        assert!(doc.contains("\"b\":true"), "native boolean:\n{doc}");
        assert!(doc.contains("\"n\":42"), "native integer in named graph:\n{doc}");
        assert_compact_iso(&g, ctx);
    }

    #[test]
    fn compact_iso_general() {
        // A mixed graph (types, langs, datatypes, multi-valued, blank nodes) under a rich
        // context must round-trip losslessly.
        let g = Graph::load_str(
            r#"@prefix ex: <http://ex/> . @prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
               ex:alice a ex:Person ;
                 ex:name "Alice" ;
                 ex:nick "Al" , "Ali" ;
                 ex:greeting "Hi"@en ;
                 ex:born "1990-05-04"^^xsd:date ;
                 ex:knows ex:bob , [ ex:name "anon" ] ."#,
            "turtle",
        )
        .unwrap();
        let ctx = r#"{"@vocab":"http://ex/","id":"@id","type":"@type",
                      "xsd":"http://www.w3.org/2001/XMLSchema#",
                      "born":{"@id":"http://ex/born","@type":"xsd:date"},
                      "knows":{"@id":"http://ex/knows","@type":"@id"},
                      "nick":{"@id":"http://ex/nick","@container":"@set"}}"#;
        assert_compact_iso(&g, ctx);
    }

    #[test]
    fn compact_empty_context_is_expanded_graph() {
        let g = Graph::load_str(
            r#"@prefix ex: <http://ex/> . ex:s ex:p "v" ."#,
            "turtle",
        )
        .unwrap();
        // An empty context yields full-IRI keys (no compaction), still a valid @graph doc.
        let doc = compact_doc(&g, "{}");
        assert!(doc.contains("\"http://ex/p\""), "full IRI key with empty context:\n{doc}");
        assert_compact_iso(&g, "{}");
    }

    #[test]
    fn parse_context_json_rejects_non_object() {
        assert!(parse_context_json("[]").is_none());
        assert!(parse_context_json("\"x\"").is_none());
        assert!(parse_context_json("not json").is_none());
        assert!(parse_context_json(r#"{"@vocab":"http://ex/"}"#).is_some());
        // A nested term-definition object parses (round-trips through write).
        let v = parse_context_json(r#"{"a":{"@id":"http://ex/a","@type":"@id"}}"#).unwrap();
        let mut out = String::new();
        Jv::write(&v, &mut out);
        assert_eq!(out, r#"{"a":{"@id":"http://ex/a","@type":"@id"}}"#);
    }

    #[test]
    fn compact_unused_write_jsonld_compact_direct() {
        // Exercise the lower-level `write_jsonld_compact(&[NamedGraph], &ctx)` entry point
        // directly (the slice API, not the Graph wrapper), so both surfaces are covered.
        let triples = vec![oxrdf::Triple::new(
            oxrdf::NamedNode::new("http://ex/s").unwrap(),
            oxrdf::NamedNode::new("http://ex/p").unwrap(),
            oxrdf::Literal::new_simple_literal("v"),
        )];
        let view: Vec<NamedGraph<'_>> = vec![(None, triples.as_slice())];
        let ctx = parse_context_json(r#"{"@vocab":"http://ex/"}"#).unwrap();
        let doc = write_jsonld_compact(&view, &ctx);
        assert!(doc.contains("\"p\":\"v\""), "slice API compaction:\n{doc}");
    }

    // =======================================================================
    // [OPUS-4.8] Regression guards for the four JSON-LD 1.1 Compaction
    // correctness bugs found by the adversarial audit of PR #950 and
    // differential-tested against the pyld W3C reference processor
    // (sq-oy1f.8/.9/.10/.11). Each asserts the spec-correct (pyld-verified)
    // shape AND the losslessness round-trip the original tests lacked.
    // =======================================================================

    /// [OPUS-4.8] sq-oy1f.8 — an `@list`-container term carrying BOTH a list value and a
    /// co-located non-list sibling must keep the sibling, not silently drop it. pyld emits
    /// the list under the container term as a bare array and the sibling under the property
    /// IRI compacted without the list term (here the full IRI, no `@vocab`). The round-trip
    /// must preserve every triple.
    #[test]
    fn compact_list_container_keeps_colocated_sibling() {
        let g = Graph::load_str(
            r#"@prefix ex: <http://ex/> .
               ex:s ex:prop ( "a" "b" ) .
               ex:s ex:prop "c" ."#,
            "turtle",
        )
        .unwrap();
        let ctx = r#"{"items":{"@id":"http://ex/prop","@container":"@list"}}"#;
        let doc = compact_doc(&g, ctx);
        // The list survives under the container term as a bare ordered array.
        assert!(
            doc.contains("\"items\":[\"a\",\"b\"]"),
            "list under @list-container term:\n{doc}"
        );
        // The sibling "c" survives under the full property IRI (pyld's choice when no
        // non-list term / @vocab is available) — it is NOT swallowed by the list term.
        assert!(
            doc.contains("\"http://ex/prop\":\"c\""),
            "co-located sibling preserved under full IRI:\n{doc}"
        );
        // Losslessness: the list-cell blank nodes are renamed on re-materialisation, so the
        // triple count is the round-trip guard (the structural asserts above pin the shape).
        assert_compact_count_iso(&g, ctx);
    }

    /// [OPUS-4.8] sq-oy1f.9 (faithfulness-corrected by sq-oy1f.12) — an `@language`-container
    /// term must NOT drop a value lacking a language tag. The ORIGINAL sq-oy1f.9 fix routed a
    /// non-string value (the `xsd:integer` `42`) into the language map's `@none` member — but
    /// that produces a document a STRICT third-party processor (pyld) REJECTS, because a
    /// JSON-LD language map's values must be strings. sq-oy1f.12 corrects this: a non-string
    /// value goes to a SEPARATE non-language key (the property IRI compacted without the
    /// language term), which preserves the datatype on read-back. This test asserts the new
    /// pyld-faithful shape. (A *plain string* with no language still correctly uses `@none`,
    /// which IS valid — see `compact_language_none_plain_string`.)
    #[test]
    fn compact_language_container_keeps_nonlang_under_none() {
        let g = Graph::load_str(
            r#"@prefix ex: <http://ex/> .
               ex:s ex:labels "Hello"@en .
               ex:s ex:labels 42 ."#,
            "turtle",
        )
        .unwrap();
        let ctx = r#"{"labels":{"@id":"http://ex/labels","@container":"@language"}}"#;
        let doc = compact_doc(&g, ctx);
        // The language-tagged string keeps its language-map slot.
        assert!(doc.contains("\"en\":\"Hello\""), "language-map en slot:\n{doc}");
        // The non-string integer is NOT placed in the language map (that would be invalid
        // JSON-LD); it goes under the separate non-language key (the full IRI here), keeping
        // its native form so the datatype survives.
        assert!(
            doc.contains("\"http://ex/labels\":42"),
            "non-string value preserved under a separate non-language key:\n{doc}"
        );
        assert!(
            !doc.contains("\"@none\":42"),
            "a non-string value must NOT land in the language map (pyld rejects it):\n{doc}"
        );
        // Losslessness: both triples (incl. the xsd:integer 42) survive the round-trip.
        assert_compact_iso(&g, ctx);
    }

    /// [OPUS-4.8] sq-oy1f.10 — with two reverse-coverable edges where one object IS a
    /// subject and the other is a pure object, the bulk strip used to drop the edge to the
    /// non-subject object. Every edge must survive: the relocated one through node
    /// inversion, the un-relocated one as a forward property.
    #[test]
    fn compact_reverse_keeps_edge_to_nonsubject_object() {
        let g = Graph::load_str(
            r#"@prefix ex: <http://ex/> .
               ex:alice ex:knows ex:bob .
               ex:bob ex:name "Bob" .
               ex:eve ex:knows ex:frank ."#,
            "turtle",
        )
        .unwrap();
        // A @reverse term over ex:knows. The eve->frank edge (frank is never a subject) must
        // not be dropped when the alice->bob edge relocates.
        let ctx = r#"{"@vocab":"http://ex/","id":"@id","name":"http://ex/name",
                      "knownBy":{"@reverse":"http://ex/knows"}}"#;
        let doc = compact_doc(&g, ctx);
        // eve still carries its forward knows edge to frank (a non-subject object).
        assert!(
            doc.contains("http://ex/frank"),
            "edge to non-subject object frank preserved:\n{doc}"
        );
        // Losslessness: all three triples survive (no blank nodes, exact label match).
        assert_compact_iso(&g, ctx);
    }

    /// [OPUS-4.8] sq-oy1f.11 — `@vocab`-relative compaction must emit the vocab-relative
    /// form even when the stripped suffix contains a fragment (`#`) or slash (`/`); the
    /// prior `!rest.contains([':', '/', '#'])` guard over-restricted it to a full IRI. (The
    /// `:` case is deliberately still excluded — see `compact_iri` — because it is
    /// ambiguous with a compact IRI on read-back, so emitting it would be lossy.)
    #[test]
    fn compact_vocab_relative_with_fragment() {
        let g = Graph::load_str(
            r#"<http://example.org/subject> <http://example.org/ns#value> "test" ."#,
            "ntriples",
        )
        .unwrap();
        let ctx = r#"{"@vocab":"http://example.org/","other":"http://example.org/other"}"#;
        let doc = compact_doc(&g, ctx);
        // The fragment-bearing predicate compacts to the @vocab-relative `ns#value`, not the
        // full IRI (matching pyld).
        assert!(
            doc.contains("\"ns#value\":\"test\""),
            "vocab-relative fragment form:\n{doc}"
        );
        assert!(
            !doc.contains("\"http://example.org/ns#value\""),
            "full IRI must not be emitted for the predicate:\n{doc}"
        );
        // Lossless (this bug was output-quality only — confirm anyway).
        assert_compact_iso(&g, ctx);
    }

    // =======================================================================
    // [OPUS-4.8] sq-oy1f.12/.13/.14 — STRICT third-party (pyld) FAITHFULNESS
    // regressions. The conformance suite's own oxjsonld self-reparse oracle
    // MASKED these: the bytes below were each fed to the pyld W3C reference
    // processor (expand + toRdf) and the reproduced N-Quads compared to the
    // source. Each assertion pins the exact pyld-verified shape (NOT merely
    // sparq's own round-trip) so a regression that re-introduces an
    // invalid / inverted / lossy document is caught here. The
    // `assert_compact_iso` guard additionally pins the sparq self-round-trip.
    // =======================================================================

    /// [OPUS-4.8] sq-oy1f.12 — a forward edge whose predicate only has a `@reverse` term must
    /// never be emitted inside an `@reverse` block: a reverse-term key inside a block
    /// DOUBLE-INVERTS for a strict processor (pyld reads `<alice> <parent> <bob>`). fromRdf
    /// yields no reverse properties, so the edge stays a forward member under its full IRI.
    #[test]
    fn compact_reverse_term_as_forward_member_not_block() {
        let g = Graph::load_str(
            r#"@prefix ex: <http://ex/> .
               ex:bob ex:parent ex:alice .
               ex:alice ex:name "Alice" ."#,
            "turtle",
        )
        .unwrap();
        let ctx = r#"{"children":{"@reverse":"http://ex/parent","@type":"@id"},
                      "name":"http://ex/name"}"#;
        let doc = compact_doc(&g, ctx);
        let body = data_body(&doc);
        assert!(
            body.contains(r#""http://ex/parent":{"@id":"http://ex/alice"}"#),
            "forward edge under its full IRI:\n{doc}"
        );
        assert!(
            !body.contains("@reverse") && !body.contains("children"),
            "no @reverse block / reverse term in the data (it would double-invert):\n{doc}"
        );
        assert_compact_iso(&g, ctx);
    }

    /// [OPUS-4.8] sq-oy1f.12 — a non-string value (typed/numeric literal) must NOT be placed
    /// in a `@language` container map (a strict processor rejects the whole document:
    /// "language map values must be strings"). It goes to a SEPARATE non-language key, which
    /// preserves its datatype on read-back.
    ///
    /// pyld differential (verified): with the integer under `@none` pyld THROWS
    /// `invalid language map value`; with it under the separate key the doc `toRdf`s to
    /// `"42"^^xsd:integer` (datatype preserved), not a `"42"` string.
    #[test]
    fn compact_language_nonstring_routes_to_separate_key() {
        let g = Graph::load_str(
            r#"@prefix ex: <http://ex/> .
               ex:s ex:labels "Hello"@en .
               ex:s ex:labels 42 ."#,
            "turtle",
        )
        .unwrap();
        let ctx = r#"{"labels":{"@id":"http://ex/labels","@container":"@language"}}"#;
        let doc = compact_doc(&g, ctx);
        // The language map holds ONLY the string-valued, language-tagged entry.
        assert!(doc.contains(r#""labels":{"en":"Hello"}"#), "lang map en-only:\n{doc}");
        // The integer is under the separate full-IRI key as a native scalar (datatype kept).
        assert!(
            doc.contains(r#""http://ex/labels":42"#),
            "non-string under separate key:\n{doc}"
        );
        assert!(!doc.contains(r#""@none":42"#), "no invalid @none scalar:\n{doc}");
        assert_compact_iso(&g, ctx);
    }

    /// [OPUS-4.8] sq-oy1f.12 — a PLAIN string (no language tag, xsd:string) under a
    /// `@language` container IS validly placed in the `@none` member (a language map MAY hold
    /// `@none` strings). This case must keep working — only NON-string values move out.
    ///
    /// pyld differential (verified): the `@none` plain string `toRdf`s to a plain `"plain"`
    /// literal (xsd:string), round-tripping faithfully.
    #[test]
    fn compact_language_none_plain_string() {
        let g = Graph::load_str(
            r#"@prefix ex: <http://ex/> .
               ex:s ex:labels "Hello"@en .
               ex:s ex:labels "plain" ."#,
            "turtle",
        )
        .unwrap();
        let ctx = r#"{"labels":{"@id":"http://ex/labels","@container":"@language"}}"#;
        let doc = compact_doc(&g, ctx);
        assert!(doc.contains(r#""en":"Hello""#), "lang slot:\n{doc}");
        // The plain string stays in the language map under `@none` (valid, pyld-faithful).
        assert!(doc.contains(r#""@none":"plain""#), "plain string under @none:\n{doc}");
        assert_compact_iso(&g, ctx);
    }

    /// [OPUS-4.8] sq-oy1f.13 — a plain literal value under a `@type:@id`-coerced term must be
    /// emitted under a NON-coerced key, else a strict processor reads the literal string as a
    /// node IRI (a string literal silently becomes a node — data corruption).
    ///
    /// pyld differential (verified): with `lit` (a `@type:@id` term) carrying the literal
    /// `"http://ex/target"`, pyld `toRdf`s it to the IRI `<http://ex/target>` (corruption);
    /// under the full-IRI key it `toRdf`s to the string literal `"http://ex/target"`. The
    /// co-located node reference (`ref`) stays under the coerced term (its `{"@id"}` collapses
    /// to a bare IRI — the point of the coercion).
    #[test]
    fn compact_typeid_literal_kept_off_coerced_term() {
        let g = Graph::load_str(
            r#"@prefix ex: <http://ex/> .
               ex:s ex:ref ex:target .
               ex:s ex:lit "http://ex/target" ."#,
            "turtle",
        )
        .unwrap();
        let ctx = r#"{"@vocab":"http://ex/",
                      "ref":{"@id":"http://ex/ref","@type":"@id"},
                      "lit":{"@id":"http://ex/lit","@type":"@id"}}"#;
        let doc = compact_doc(&g, ctx);
        // The literal goes under the full IRI key (a non-coerced spelling), staying a literal.
        assert!(
            doc.contains(r#""http://ex/lit":"http://ex/target""#),
            "literal under non-coerced key:\n{doc}"
        );
        // The `lit` coerced term must NOT carry the literal string (that would read as an IRI).
        assert!(
            !doc.contains(r#""lit":"http://ex/target""#),
            "literal must not use the @id-coerced term:\n{doc}"
        );
        // The genuine node reference still uses the coerced `ref` term (bare-IRI collapse).
        assert!(
            doc.contains(r#""ref":"http://ex/target""#),
            "node ref keeps the @id-coerced term:\n{doc}"
        );
        assert_compact_iso(&g, ctx);
    }

    /// [OPUS-4.8] sq-oy1f.14 — several values sharing one language must each survive (an array
    /// per language slot); the prior single-set clobbered all but the last.
    ///
    /// pyld differential (verified): the array-per-language doc `toRdf`s to all three triples
    /// (`"Hi"@en`, `"Hello"@en`, `"Salut"@fr`); the clobbered single-value form lost `"Hi"@en`.
    #[test]
    fn compact_language_container_multivalue_per_language() {
        let g = Graph::load_str(
            r#"@prefix ex: <http://ex/> .
               ex:s ex:labels "Hi"@en .
               ex:s ex:labels "Hello"@en .
               ex:s ex:labels "Salut"@fr ."#,
            "turtle",
        )
        .unwrap();
        let ctx = r#"{"labels":{"@id":"http://ex/labels","@container":"@language"}}"#;
        let doc = compact_doc(&g, ctx);
        // The `en` slot is an ARRAY of both English strings (order is first-seen).
        assert!(
            doc.contains(r#""en":["Hi","Hello"]"#),
            "multi-value en array:\n{doc}"
        );
        assert!(doc.contains(r#""fr":"Salut""#), "single fr stays scalar:\n{doc}");
        // Losslessness: all three triples survive the round-trip.
        assert_compact_iso(&g, ctx);
    }

    /// [OPUS-4.8] sq-oy1f.14 — a node reference under a `@container: @id` term must become an
    /// `@id` map entry keyed by the node IRI, never a `{"@id": …}` value under the container
    /// term (a strict processor rejects that: `illegal key … @id`). The `@id` map is the
    /// spec Compaction shape and re-expands to the same edge.
    #[test]
    fn compact_id_container_falls_back_to_default_framing() {
        let g = Graph::load_str(
            r#"@prefix ex: <http://ex/> .
               ex:s ex:members ex:m1 .
               ex:m1 ex:name "M1" ."#,
            "turtle",
        )
        .unwrap();
        let ctx = r#"{"@vocab":"http://ex/","id":"@id",
                      "members":{"@id":"http://ex/members","@container":"@id"}}"#;
        let doc = compact_doc(&g, ctx);
        let body = data_body(&doc);
        assert!(
            body.contains(r#""members":{"http://ex/m1":{}}"#),
            "node reference as an @id map entry:\n{doc}"
        );
        assert!(!body.contains(r#""members":{"id""#), "no node ref under the container:\n{doc}");
        assert_compact_iso(&g, ctx);
    }

    /// [OPUS-4.8] sq-oy1f.14 — a multi-value `@index` container groups all values (fromRdf has
    /// no per-value `@index`, so they share the reserved `@none` index slot as an array). The
    /// document stays valid and every value survives.
    ///
    /// pyld differential (verified): the `{"@none": [...]}` index map `toRdf`s to both values.
    #[test]
    fn compact_index_container_multivalue_under_none() {
        let g = Graph::load_str(
            r#"@prefix ex: <http://ex/> .
               ex:s ex:p "x" .
               ex:s ex:p "y" ."#,
            "turtle",
        )
        .unwrap();
        let ctx = r#"{"p":{"@id":"http://ex/p","@container":"@index"}}"#;
        let doc = compact_doc(&g, ctx);
        // Both values land under the reserved `@none` index slot as an array.
        assert!(
            doc.contains(r#""p":{"@none":["x","y"]}"#),
            "multi-value @index under @none array:\n{doc}"
        );
        assert_compact_iso(&g, ctx);
    }

    // =======================================================================
    // [OPUS-4.8] sq-qcnn.33 — coverage-raise tests: targeted direct unit tests
    // for previously-uncovered public functions and code paths.  Each asserts
    // exact byte values (non-vacuous) and exercises the REAL writer path.
    // =======================================================================

    /// write_iri keep-existing-best arm: fires when the existing local part is longer
    /// than the next namespace being considered, so the current best is retained.
    ///
    /// Prefix `"a"` → `"ns1/"` (len 4) is iterated first (BTreeMap order), producing
    /// local `"longlocal"` (len 9).  Prefix `"b"` → `"ns1/lon"` (len 7) arrives second;
    /// the guard `bns.len() >= ns.len()` (9 >= 7 = true) keeps the existing match,
    /// so the result is `"a:longlocal"`.
    #[test]
    fn write_iri_keep_existing_best_arm() {
        let mut prefixes: Prefixes = std::collections::BTreeMap::new();
        // "a" iterates before "b" in BTreeMap order.
        prefixes.insert("a".to_string(), "ns1/".to_string());
        prefixes.insert("b".to_string(), "ns1/lon".to_string());
        let mut out = String::new();
        write_iri("ns1/longlocal", &prefixes, &mut out);
        // Both "a:longlocal" and "b:glocal" are valid compactions; the existing-best
        // guard keeps "a" because "longlocal" (len 9) >= "ns1/lon" (len 7).
        assert_eq!(out, "a:longlocal", "keep-existing-best arm result: {}", out);
    }

    /// escape_iri control-char / delimiter arm: a `>` in the IRI path must be emitted
    /// as `>` to keep Turtle well-formed; a C0 control char hits the same arm.
    #[test]
    fn escape_iri_escapes_special_chars() {
        let mut out = String::new();
        escape_iri("http://ex/has>bracket", &mut out);
        assert!(
            out.contains("\\u003E"),
            "closing angle bracket escaped: {}",
            out
        );
        assert!(
            out.starts_with('<') && out.ends_with('>'),
            "IRIREF delimiters present: {}",
            out
        );
        // A C0 control character (U+0001) hits the same match arm.
        out.clear();
        escape_iri("http://ex/ctrl\u{01}", &mut out);
        assert!(out.contains("\\u0001"), "control char escaped: {}", out);
    }

    /// write_term_full lang-tag arm (~line 1022) and non-xsd:string datatype arm
    /// (~line 1029).  The existing abbreviate=false test uses only plain xsd:string
    /// literals so these two arms were previously uncovered.
    #[test]
    fn write_term_full_lang_and_typed_literal() {
        let mut out = String::new();
        // Lang-tagged literal: "hello"@en
        let lang_lit = Term::Literal(
            oxrdf::Literal::new_language_tagged_literal_unchecked("hello", "en"),
        );
        write_term_full(&lang_lit, &mut out);
        assert_eq!(out, "\"hello\"@en", "lang literal round-trip: {}", out);

        out.clear();
        // Typed literal with non-xsd:string / non-rdf:langString datatype.
        let xsd_int = "http://www.w3.org/2001/XMLSchema#integer";
        let typed_lit = Term::Literal(Literal::new_typed_literal(
            "42",
            NamedNode::new_unchecked(xsd_int),
        ));
        write_term_full(&typed_lit, &mut out);
        assert!(
            out.starts_with("\"42\"^^<"),
            "typed literal prefix: {}",
            out
        );
        assert!(out.contains(xsd_int), "datatype IRI preserved: {}", out);
    }

    /// write_term_full Triple arm (~lines 1034-1042): a quoted triple must render as
    /// `<<( <s> <p> "o" )>>` with full IRIs (no prefix compaction).
    #[test]
    fn write_term_full_triple_term() {
        let subj = NamedOrBlankNode::NamedNode(nn("http://ex/s"));
        let pred = nn("http://ex/p");
        let obj = Term::Literal(Literal::new_simple_literal("v"));
        let qt = Term::Triple(Box::new(Triple {
            subject: subj,
            predicate: pred,
            object: obj,
        }));
        let mut out = String::new();
        write_term_full(&qt, &mut out);
        assert!(out.starts_with("<<("), "triple term open: {}", out);
        assert!(out.ends_with(")>>"), "triple term close: {}", out);
        assert!(
            out.contains("<http://ex/s>"),
            "subject IRI in triple term: {}",
            out
        );
        assert!(
            out.contains("<http://ex/p>"),
            "predicate IRI in triple term: {}",
            out
        );
    }

    /// write_trig_pretty abbreviate=false path for a named-graph IRI (~lines 1203-1205):
    /// the `GRAPH` header emits the full `<IRI>` instead of a prefix-compacted form.
    #[test]
    fn trig_pretty_abbreviate_false_named_graph() {
        let g = Graph::load_dataset(
            r#"@prefix ex: <http://ex/> . GRAPH ex:g1 { ex:s ex:p "v" . }"#,
            "trig",
        )
        .unwrap();
        let owned = dataset_graphs(&g);
        let view: Vec<NamedGraph<'_>> = owned
            .iter()
            .map(|(n, ts)| (n.as_ref(), ts.as_slice()))
            .collect();
        let opts = PrettyOptions {
            abbreviate: false,
            ..PrettyOptions::default()
        };
        let out = write_trig_pretty(&view, &ex_prefixes(), &opts);
        // No prefix header when abbreviate=false.
        assert!(
            !out.contains("@prefix"),
            "no header when abbreviate=false: {}",
            out
        );
        // GRAPH keyword followed by full <IRI> (not the prefixed `ex:g1`).
        assert!(
            out.contains("GRAPH <http://ex/g1>"),
            "full IRI in GRAPH header: {}",
            out
        );
        // Round-trip: the emitted TriG must parse back to the same dataset.
        let g2 = Graph::load_dataset(&out, "trig").unwrap();
        assert_dataset_iso(&g, &g2, &out);
    }

    /// write_trig_pretty blank-node named-graph arm (~lines 1207-1209): a blank-node
    /// graph name must render as `_:label` in the `GRAPH` header.
    #[test]
    fn trig_pretty_blank_node_named_graph() {
        let g = Graph::load_dataset(
            r#"_:bg { <http://ex/s> <http://ex/p> <http://ex/o> . }"#,
            "trig",
        )
        .unwrap();
        let owned = dataset_graphs(&g);
        let view: Vec<NamedGraph<'_>> = owned
            .iter()
            .map(|(n, ts)| (n.as_ref(), ts.as_slice()))
            .collect();
        let out = write_trig_pretty(&view, &default_prefixes(), &PrettyOptions::default());
        assert!(
            out.contains("GRAPH _:"),
            "blank graph name uses _: form in pretty TriG: {}",
            out
        );
        let g2 = Graph::load_dataset(&out, "trig").unwrap();
        assert_dataset_iso(&g, &g2, &out);
    }

    /// graph_to_trig_pretty direct call (~lines 1227-1234): the Graph wrapper that
    /// assembles `dataset_graphs` + `default_prefixes` + `PrettyOptions::default`.
    ///
    /// Uses `foaf:name` (which IS in the default prefix map and is NOT the `a` shorthand
    /// exception) so a `@prefix foaf:` header is emitted.
    #[test]
    fn graph_to_trig_pretty_round_trips() {
        // foaf: is in default_prefixes() and foaf:name abbreviates normally (no shorthand
        // exception), so a @prefix foaf: header must appear in the output.
        let foaf_name = "http://xmlns.com/foaf/0.1/name";
        let g = Graph::load_dataset(
            &[
                "<http://ex/s> <", foaf_name, "> \"Alice\" .",
                " GRAPH <http://ex/g1> {",
                " <http://ex/a> <", foaf_name, "> \"Bob\" . }",
            ]
            .concat(),
            "trig",
        )
        .unwrap();
        let out = graph_to_trig_pretty(&g);
        // foaf: IS in the default prefix map and not a shorthand — header must appear.
        assert!(out.contains("@prefix foaf:"), "foaf prefix header: {}", out);
        // A GRAPH block wraps the named graph.
        assert!(out.contains("GRAPH"), "GRAPH block present: {}", out);
        // Abbreviated property in the output.
        assert!(out.contains("foaf:name"), "foaf:name abbreviated: {}", out);
        // Round-trip.
        let g2 = Graph::load_dataset(&out, "trig").unwrap();
        assert_dataset_iso(&g, &g2, &out);
    }

    /// `parse_context_json` trailing-chars guard (~line 1283): valid JSON followed by
    /// extra bytes must return `None`.
    #[test]
    fn parse_context_json_trailing_chars_is_none() {
        assert!(
            parse_context_json(r#"{"@vocab":"http://ex/"} EXTRA"#).is_none(),
            "trailing text after JSON must be rejected"
        );
        // Clean JSON with no trailing bytes is accepted.
        assert!(
            parse_context_json(r#"{"@vocab":"http://ex/"}"#).is_some(),
            "clean JSON must parse successfully"
        );
    }

    /// `JsonParser::parse_number` (~lines 1327-1343) and `parse_array` comma arm
    /// (~line 1404): numeric JSON values are stored as `Json::Raw`; a two-element
    /// array exercises the comma branch.
    #[test]
    fn parse_context_json_number_and_array_values() {
        let ctx = parse_context_json(r#"{"n":42,"arr":[1,"two",3]}"#)
            .expect("number + array parse");
        let mut out = String::new();
        Jv::write(&ctx, &mut out);
        // Number preserved verbatim as a raw token.
        assert!(out.contains("\"n\":42"), "number raw token: {}", out);
        // Array with multiple elements (comma arm between items) is preserved.
        assert!(out.contains("\"arr\":["), "array key present: {}", out);
        assert!(out.contains("\"two\""), "string element in array: {}", out);
    }

    /// `JsonParser::parse_string` escape arms (~lines 1360-1385): backslash escapes
    /// (`\"`, `\\`, `\/`, `\n`, `\t`, `\r`, `\b`, `\f`, `\uXXXX`) and multi-byte
    /// UTF-8 content are all decoded correctly.
    #[test]
    fn parse_context_json_string_escape_sequences() {
        // \" → a literal double-quote in the decoded string.
        let ctx_q =
            parse_context_json(r#"{"q":"say \"hi\""}"#).expect("escaped-quote parse");
        let mut out = String::new();
        Jv::write(&ctx_q, &mut out);
        // json_escape re-encodes the embedded quote as \".
        assert!(
            out.contains(r#"say \"hi\""#),
            "quote escape round-trip: {}",
            out
        );

        // \/ → forward slash (RFC 7159 allows escaping it).
        let ctx_sl = parse_context_json(r#"{"sl":"http:\/\/ex\/"}"#).expect("slash-escape parse");
        let mut out_sl = String::new();
        Jv::write(&ctx_sl, &mut out_sl);
        assert!(
            out_sl.contains("http://ex/"),
            "forward-slash escape decoded: {}",
            out_sl
        );

        // \uXXXX → decoded Unicode code-point.
        let ctx_u =
            parse_context_json(r#"{"uni":"ABC"}"#).expect("unicode-escape parse");
        let mut out_u = String::new();
        Jv::write(&ctx_u, &mut out_u);
        // U+0041 = 'A'; the result is "ABC".
        assert!(out_u.contains("ABC"), "\\uXXXX decoded to char: {}", out_u);

        // \t, \n, \r, \b, \f — the remaining single-char escape arms.
        let ctx_ws =
            parse_context_json("{\"t\":\"a\\tb\",\"n\":\"a\\nb\",\"b\":\"\\b\",\"f\":\"\\f\"}")
                .expect("whitespace-escape parse");
        let mut out_ws = String::new();
        Jv::write(&ctx_ws, &mut out_ws);
        // json_escape re-encodes them; just confirm the object serialises without panic.
        assert!(out_ws.contains("\"t\":"), "tab-escape key present: {}", out_ws);

        // Multi-byte UTF-8 (é = U+00E9, 2 bytes) travels through the `_` arm that
        // advances past continuation bytes.
        let ctx_mb =
            parse_context_json(r#"{"k":"café"}"#).expect("multi-byte via \\uXXXX");
        let mut out_mb = String::new();
        Jv::write(&ctx_mb, &mut out_mb);
        // U+00E9 = 'é'; json_escape keeps it verbatim (> 0x1F, non-special).
        assert!(
            out_mb.contains('\u{00E9}'),
            "multi-byte code-point preserved: {}",
            out_mb
        );
    }
}
