//! **SPARQL UPDATE lineage** (sq-xwdd): capture W3C PROV-O provenance for
//! the data a SPARQL `INSERT … WHERE` / `INSERT DATA` / `DELETE …` operation **changes**
//! in a store.
//!
//! # UPDATE *is* derivation (for the inserts) + invalidation (for the deletes)
//!
//! A SPARQL UPDATE mutates a graph in place. The PROV-O reading of that mutation is
//! two-sided and standard:
//!
//! * the triples it **inserts** are *newly generated* data — a [`prov:Entity`] that
//!   was [`prov:wasGeneratedBy`] the update [`prov:Activity`] and, for an
//!   `INSERT … WHERE`, [`prov:wasDerivedFrom`] the matched source(s);
//! * the triples it **deletes** are *retracted* data — a [`prov:Entity`] the activity
//!   [`prov:wasInvalidatedBy`] (PROV-O's term for "ceased to exist / be valid"). We do
//!   **not** claim deletes are "derived"; retraction is invalidation, not generation.
//!
//! This is the same opt-in, lean-core shape as the CONSTRUCT path (`derive_construct`):
//! nothing in sparq's default build depends on it, and capture is exact — we read the
//! engine's **resolved** [`sparq_engine::UpdateEffect`] log (the concrete per-graph
//! triple delta the update actually committed), so the lineage reflects what happened,
//! not a re-evaluation of the (possibly non-deterministic) update text.
//!
//! # What is and is NOT covered
//!
//! Covered: the data-bearing operations — `INSERT DATA`, `DELETE DATA`,
//! `DELETE/INSERT … WHERE` (incl. `USING`), and `LOAD` (the loaded triples are the
//! resolved inserts). Structural operations — `CLEAR` / `DROP` / `CREATE` — change a
//! graph's *existence/emptiness*, not its triples-as-data, and the engine records them
//! as opaque `UpdateEffect::Clear/Drop/Create` markers without a resolved triple set; we
//! count them as activity kinds but emit **no** per-triple entity for them (there is no
//! sound per-triple derivation to assert). A `CLEAR`/`DROP` that retracts existing data
//! is therefore not enumerated triple-by-triple here — only the explicit `DELETE`
//! retractions are. (This is a deliberate honesty boundary, not an oversight.)
//!
//! [`prov:Entity`]: https://www.w3.org/TR/prov-o/#Entity
//! [`prov:Activity`]: https://www.w3.org/TR/prov-o/#Activity
//! [`prov:wasGeneratedBy`]: https://www.w3.org/TR/prov-o/#wasGeneratedBy
//! [`prov:wasDerivedFrom`]: https://www.w3.org/TR/prov-o/#wasDerivedFrom
//! [`prov:wasInvalidatedBy`]: https://www.w3.org/TR/prov-o/#wasInvalidatedBy

use std::time::SystemTime;

use oxrdf::vocab::rdf;
use oxrdf::{Literal, NamedNode, NamedOrBlankNode, Term, Triple};
use sparq_core::Graph;
use sparq_engine::{QueryBudget, UpdateEffect};

use crate::{datetime_literal, mint, prov, ProvConfig};

/// A completed SPARQL UPDATE, with its W3C PROV-O lineage.
///
/// Produced by [`derive_update`]. The update has **already been applied** to the graph
/// (it is captured from the resolved effect log of the in-place application). The record
/// carries the concrete triples the update **inserted** ([`inserted`](Self::inserted) —
/// the *derived* data) and **deleted** ([`deleted`](Self::deleted) — the *retracted*
/// data); [`prov_graph`](Self::prov_graph) materialises the lineage as PROV-O RDF.
#[derive(Clone, Debug)]
pub struct UpdateDerivation {
    inserted: Vec<Triple>,
    deleted: Vec<Triple>,
    activity: NamedNode,
    /// The generated-result entity (the inserts). Always minted/configured even when the
    /// update inserts nothing, so the activity is still a nameable PROV node.
    entity: NamedNode,
    used: Vec<NamedNode>,
    agent: Option<NamedNode>,
    started: SystemTime,
    ended: SystemTime,
    query: String,
    /// Operation-kind label surfaced on the activity (e.g. `"INSERT DATA"`,
    /// `"DELETE/INSERT WHERE"`, `"SPARQL UPDATE"` for a mixed/multi-op body).
    kind: &'static str,
}

impl UpdateDerivation {
    /// The triples the update **inserted** — the newly *generated* (derived) data.
    pub fn inserted(&self) -> &[Triple] {
        &self.inserted
    }

    /// The triples the update **deleted** — the *retracted* (invalidated) data.
    pub fn deleted(&self) -> &[Triple] {
        &self.deleted
    }

    /// The activity IRI ([`prov:Activity`](https://www.w3.org/TR/prov-o/#Activity)).
    pub fn activity(&self) -> &NamedNode {
        &self.activity
    }

    /// The generated-result entity IRI ([`prov:Entity`](https://www.w3.org/TR/prov-o/#Entity)),
    /// naming the set of inserted triples.
    pub fn entity(&self) -> &NamedNode {
        &self.entity
    }

    /// The configured input-source IRIs, in their original order.
    // sq-cg237
    pub fn used_inputs(&self) -> &[NamedNode] {
        &self.used
    }

    /// Start/end wall-clock instants of the update activity.
    pub fn timing(&self) -> (SystemTime, SystemTime) {
        (self.started, self.ended)
    }

    /// The PROV-O lineage of this update as an RDF graph (a `Vec<Triple>`).
    ///
    /// Emits, for the update activity `A`, the generated entity `E` (the inserts), and
    /// each configured input `Iᵢ`:
    ///
    /// - `A a prov:Activity`, `A rdfs:label "<kind>"`, `A prov:value "<sparql>"`
    /// - `A prov:startedAtTime "…"^^xsd:dateTime`, `A prov:endedAtTime "…"^^xsd:dateTime`
    /// - `E a prov:Entity`, `E prov:wasGeneratedBy A` — **only when the update inserted
    ///   data** (no inserts ⇒ no generated entity, which is the correct PROV reading: a
    ///   pure-delete update generated nothing)
    /// - `A prov:used Iᵢ`, `E prov:wasDerivedFrom Iᵢ` (for each input; the
    ///   `wasDerivedFrom` only when there is an `E`)
    /// - `A prov:wasAssociatedWith G` (if an agent is configured)
    /// - for **deletes**: one fresh blank-node `prov:Entity` per retracted triple,
    ///   `… prov:wasInvalidatedBy A`. Deletes are invalidations, not derivations, so they
    ///   are never `wasGeneratedBy`/`wasDerivedFrom`.
    ///
    /// All IRIs are absolute (deleted-entity nodes are blank), so the graph is valid
    /// PROV-O and round-trips through any RDF parser.
    pub fn prov_graph(&self) -> Vec<Triple> {
        let a = NamedOrBlankNode::NamedNode(self.activity.clone());
        let e = NamedOrBlankNode::NamedNode(self.entity.clone());
        let mut out: Vec<Triple> = Vec::new();
        let mut push = |s: NamedOrBlankNode, p: NamedNode, o: Term| {
            out.push(Triple {
                subject: s,
                predicate: p,
                object: o,
            });
        };

        // Activity: type, kind label, the update recipe, timing.
        push(
            a.clone(),
            NamedNode::from(rdf::TYPE),
            Term::NamedNode(prov("Activity")),
        );
        push(
            a.clone(),
            NamedNode::new_unchecked("http://www.w3.org/2000/01/rdf-schema#label"),
            Term::Literal(Literal::new_simple_literal(self.kind)),
        );
        push(
            a.clone(),
            prov("value"),
            Term::Literal(Literal::new_simple_literal(&self.query)),
        );
        push(
            a.clone(),
            prov("startedAtTime"),
            Term::Literal(datetime_literal(self.started)),
        );
        push(
            a.clone(),
            prov("endedAtTime"),
            Term::Literal(datetime_literal(self.ended)),
        );

        if let Some(agent) = &self.agent {
            push(
                a.clone(),
                prov("wasAssociatedWith"),
                Term::NamedNode(agent.clone()),
            );
        }

        // Generated entity (the inserts). A pure-delete update generates nothing, so we
        // only assert the result entity + generation/derivation edges when it inserted.
        if !self.inserted.is_empty() {
            push(
                e.clone(),
                NamedNode::from(rdf::TYPE),
                Term::NamedNode(prov("Entity")),
            );
            push(
                e.clone(),
                prov("wasGeneratedBy"),
                Term::NamedNode(self.activity.clone()),
            );
            for input in &self.used {
                push(
                    e.clone(),
                    prov("wasDerivedFrom"),
                    Term::NamedNode(input.clone()),
                );
            }
        }
        // `prov:used` is on the activity regardless of whether anything was inserted — the
        // activity consulted those inputs (the WHERE/USING dataset) even if no row matched.
        for input in &self.used {
            push(a.clone(), prov("used"), Term::NamedNode(input.clone()));
        }

        // Deleted triples: each retracted triple is an entity the activity invalidated.
        // (The retracted triple's terms are not re-stated — a deleted triple no longer
        // exists in the store, so we name it by a fresh blank-node entity, not by value.)
        for _ in &self.deleted {
            let de = NamedOrBlankNode::BlankNode(oxrdf::BlankNode::default());
            push(
                de.clone(),
                NamedNode::from(rdf::TYPE),
                Term::NamedNode(prov("Entity")),
            );
            push(
                de,
                prov("wasInvalidatedBy"),
                Term::NamedNode(self.activity.clone()),
            );
        }
        out
    }

    /// The PROV-O lineage serialised as N-Triples (also a valid Turtle document).
    pub fn prov_ntriples(&self) -> String {
        sparq_engine::triples_to_ntriples(&self.prov_graph())
    }

    /// The same PROV-O lineage as [`prov_graph`](Self::prov_graph), serialised as
    /// prefix-compacted Turtle.
    ///
    /// sq-ijw35
    pub fn prov_turtle(&self) -> String {
        crate::triples_to_prov_turtle(&self.prov_graph())
    }
}

/// Apply a SPARQL UPDATE to `graph` **in place**, capturing W3C PROV-O lineage for the
/// data it changes.
///
/// The update is evaluated through the engine's effect-capturing in-place path
/// ([`sparq_engine::update_in_place_capturing`]), so the returned [`UpdateDerivation`]
/// records the **resolved** triples that were actually inserted (the derived data, a
/// `prov:Entity` that `wasGeneratedBy` the activity) and deleted (retracted data, each a
/// `prov:Entity` the activity `wasInvalidatedBy`) — not a re-evaluation of the text.
///
/// On a parse error or a failing non-`SILENT` `LOAD`, `graph` is left in whatever
/// partially-applied state the update reached (identical to
/// [`sparq_engine::update_in_place`]) and no derivation is returned.
///
/// ```
/// use sparq_core::Graph;
/// use sparq_prov::{derive_update, ProvConfig};
/// use oxrdf::NamedNode;
///
/// let mut g = Graph::load_str("@prefix ex: <http://ex/> . ex:a ex:age 30 .", "turtle").unwrap();
/// let cfg = ProvConfig::with_inputs([NamedNode::new_unchecked("http://ex/src")]);
/// let d = derive_update(
///     &mut g,
///     "PREFIX ex: <http://ex/> INSERT { ?s ex:years ?a } WHERE { ?s ex:age ?a }",
///     cfg,
/// ).unwrap();
/// assert_eq!(d.inserted().len(), 1); // ex:a ex:years 30
/// let lineage = d.prov_graph();      // its PROV-O record
/// ```
pub fn derive_update(
    graph: &mut Graph,
    sparql: &str,
    config: ProvConfig,
) -> Result<UpdateDerivation, String> {
    derive_update_with_budget(graph, sparql, config, &QueryBudget::unlimited())
}

/// [`derive_update`] under a cooperative [`QueryBudget`] (bounds a `… WHERE` whose
/// pattern blows up, exactly as a budgeted `SELECT` is bounded).
pub fn derive_update_with_budget(
    graph: &mut Graph,
    sparql: &str,
    config: ProvConfig,
    budget: &QueryBudget,
) -> Result<UpdateDerivation, String> {
    let started = (config.clock)();
    let effects = sparq_engine::update_in_place_capturing(graph, sparql, budget)?;
    let ended = (config.clock)();

    let kind = kind_label(sparql);
    let (inserted, deleted) = partition_effects(&effects);

    let activity = config
        .activity
        .unwrap_or_else(|| mint("activity", started, ended, sparql));
    let entity = config
        .entity
        .unwrap_or_else(|| mint("entity", started, ended, sparql));

    Ok(UpdateDerivation {
        inserted,
        deleted,
        activity,
        entity,
        used: config.used,
        agent: config.agent,
        started,
        ended,
        query: sparql.to_string(),
        kind,
    })
}

/// Split the resolved effect log into the inserted (derived) and deleted (retracted)
/// triples, preserving capture order. Structural effects (CLEAR/DROP/CREATE) carry no
/// resolved triples, so they contribute nothing here (see the module docs).
fn partition_effects(effects: &[UpdateEffect]) -> (Vec<Triple>, Vec<Triple>) {
    let mut inserted = Vec::new();
    let mut deleted = Vec::new();
    for effect in effects {
        if let UpdateEffect::Delta {
            inserts, deletes, ..
        } = effect
        {
            inserted.extend(inserts.iter().filter_map(terms_to_triple));
            deleted.extend(deletes.iter().filter_map(terms_to_triple));
        }
    }
    (inserted, deleted)
}

/// A resolved `[subject, predicate, object]` term-triple → an `oxrdf::Triple`.
///
/// In well-formed RDF the subject is a NamedNode or BlankNode and the predicate a
/// NamedNode; the engine only ever produces such triples for the data operations (the
/// SPARQL grammar forbids a literal/triple-term subject or a non-IRI predicate). A
/// malformed shape is skipped rather than panicking — defensive, never expected.
fn terms_to_triple(terms: &[Term; 3]) -> Option<Triple> {
    let subject = match &terms[0] {
        Term::NamedNode(n) => NamedOrBlankNode::NamedNode(n.clone()),
        Term::BlankNode(b) => NamedOrBlankNode::BlankNode(b.clone()),
        _ => return None,
    };
    let predicate = match &terms[1] {
        Term::NamedNode(n) => n.clone(),
        _ => return None,
    };
    Some(Triple {
        subject,
        predicate,
        object: terms[2].clone(),
    })
}

/// A coarse, allocation-free operation-kind label for the activity's `rdfs:label`.
///
/// Looks at the leading keyword(s) of the (uppercased-insensitive) update text. A
/// multi-operation body (or one we don't special-case) is labelled `"SPARQL UPDATE"`.
/// This is a human-facing annotation only — lineage correctness rests on the resolved
/// effect log, not this string.
fn kind_label(sparql: &str) -> &'static str {
    // Strip leading PREFIX/BASE declarations + whitespace to reach the first operation.
    let body = strip_prologue(sparql);
    let upper = body.trim_start().to_ascii_uppercase();
    // Detect a multi-op body conservatively: more than one top-level operation keyword.
    if is_multi_op(&upper) {
        return "SPARQL UPDATE";
    }
    if upper.starts_with("INSERT DATA") {
        "INSERT DATA"
    } else if upper.starts_with("DELETE DATA") {
        "DELETE DATA"
    } else if upper.starts_with("INSERT")
        || upper.starts_with("DELETE")
        || upper.starts_with("WITH")
        || upper.starts_with("USING")
    {
        // INSERT { } WHERE, DELETE { } WHERE, DELETE WHERE, WITH … DELETE/INSERT, etc.
        "DELETE/INSERT WHERE"
    } else if upper.starts_with("LOAD") {
        "LOAD"
    } else {
        // CLEAR / DROP / CREATE / ADD / COPY / MOVE — or anything else.
        "SPARQL UPDATE"
    }
}

/// Drop leading `PREFIX …` / `BASE …` declarations so [`kind_label`] sees the first
/// operation keyword. Operates line-by-line on a best-effort basis (kind is cosmetic).
fn strip_prologue(sparql: &str) -> &str {
    let mut rest = sparql.trim_start();
    loop {
        let up = rest.to_ascii_uppercase();
        let decl = if up.starts_with("PREFIX") {
            // PREFIX ns: <iri> — skip to past the closing '>'.
            rest.find('>').map(|i| i + 1)
        } else if up.starts_with("BASE") {
            rest.find('>').map(|i| i + 1)
        } else {
            None
        };
        match decl {
            Some(i) if i <= rest.len() => rest = rest[i..].trim_start(),
            _ => return rest,
        }
    }
}

/// True if the update body contains more than one top-level operation, separated by `;`.
/// A conservative scan that ignores `;` inside `<…>` IRIs and `"…"`/`'…'` string
/// literals (where a semicolon is data, or a Turtle predicate-list separator inside a
/// template — which is also not an operation boundary). Cosmetic-only.
fn is_multi_op(upper: &str) -> bool {
    // We only need "is there a top-level `;` followed by a non-empty operation?".
    let bytes = upper.as_bytes();
    let mut depth_iri = false;
    let mut in_dq = false;
    let mut in_sq = false;
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i];
        match c {
            b'<' if !in_dq && !in_sq => depth_iri = true,
            b'>' if depth_iri => depth_iri = false,
            b'"' if !in_sq && !depth_iri => in_dq = !in_dq,
            b'\'' if !in_dq && !depth_iri => in_sq = !in_sq,
            b';' if !depth_iri && !in_dq && !in_sq => {
                // A top-level ';' — is anything non-blank after it?
                if upper[i + 1..].trim().is_empty() {
                    return false; // trailing separator only
                }
                // A ';' at the top level separates operations ONLY outside a template
                // `{ … }`. Templates use ';' as Turtle predicate-list shorthand, so a
                // top-level ';' inside braces is NOT an operation boundary. Track brace
                // depth to tell them apart.
                return has_top_level_semicolon(upper);
            }
            _ => {}
        }
        i += 1;
    }
    false
}

/// Precise check: a `;` at brace-depth 0, outside IRIs/strings, with a following op.
fn has_top_level_semicolon(upper: &str) -> bool {
    let bytes = upper.as_bytes();
    let (mut brace, mut iri, mut dq, mut sq) = (0i32, false, false, false);
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'<' if !dq && !sq => iri = true,
            b'>' if iri => iri = false,
            b'"' if !sq && !iri => dq = !dq,
            b'\'' if !dq && !iri => sq = !sq,
            b'{' if !iri && !dq && !sq => brace += 1,
            b'}' if !iri && !dq && !sq => brace -= 1,
            b';' if brace == 0 && !iri && !dq && !sq && !upper[i + 1..].trim().is_empty() => {
                return true;
            }
            _ => {}
        }
        i += 1;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxrdf::vocab::xsd;
    use std::collections::HashSet;
    use std::time::{Duration, UNIX_EPOCH};

    const DATA: &str = r#"
        @prefix ex: <http://ex/> .
        ex:alice ex:age 30 ; ex:name "Alice" .
        ex:bob   ex:age 25 ; ex:name "Bob" .
    "#;

    fn g() -> Graph {
        Graph::load_str(DATA, "turtle").unwrap()
    }

    fn fixed_clock() -> SystemTime {
        UNIX_EPOCH + Duration::from_secs(1_700_000_000)
    }

    fn cfg() -> ProvConfig {
        ProvConfig {
            clock: fixed_clock,
            used: vec![NamedNode::new_unchecked("http://ex/source-graph")],
            ..ProvConfig::default()
        }
    }

    fn line(t: &Triple) -> String {
        format!("{} {} {} .", t.subject, t.predicate, t.object)
    }

    fn lines(d: &UpdateDerivation) -> HashSet<String> {
        d.prov_graph().iter().map(line).collect()
    }

    #[test]
    fn insert_where_records_generation_and_derivation() {
        let mut graph = g();
        let d = derive_update(
            &mut graph,
            "PREFIX ex: <http://ex/> INSERT { ?s ex:years ?a } WHERE { ?s ex:age ?a }",
            cfg(),
        )
        .unwrap();

        // The derived data: one renamed triple per matched subject; nothing retracted.
        assert_eq!(d.inserted().len(), 2);
        assert!(d.deleted().is_empty());
    }

    /// sq-cg237: the direct accessor exposes the configured input and agrees
    /// with the update activity's materialised `prov:used` edge.
    #[test]
    fn update_used_inputs_are_exposed_and_materialised() {
        let source = NamedNode::new_unchecked("http://ex/update-source");
        let mut graph = g();
        let derivation = derive_update(
            &mut graph,
            "PREFIX ex: <http://ex/> INSERT { ?s ex:years ?a } WHERE { ?s ex:age ?a }",
            ProvConfig::with_inputs([source.clone()]),
        )
        .unwrap();

        assert_eq!(derivation.used_inputs(), std::slice::from_ref(&source));
        assert_eq!(derivation.used_inputs().len(), 1);

        let activity = NamedOrBlankNode::NamedNode(derivation.activity().clone());
        assert!(derivation.prov_graph().iter().any(|triple| {
            triple.subject == activity
                && triple.predicate.as_str() == "http://www.w3.org/ns/prov#used"
                && triple.object == Term::NamedNode(source.clone())
        }));
    }

    #[test]
    fn insert_where_is_applied_in_place() {
        let mut graph = g();
        derive_update(
            &mut graph,
            "PREFIX ex: <http://ex/> INSERT { ?s ex:years ?a } WHERE { ?s ex:age ?a }",
            cfg(),
        )
        .unwrap();
        // The store now contains the inserted triples.
        let n = sparq_engine::construct(
            &graph,
            "PREFIX ex: <http://ex/> CONSTRUCT { ?s ex:years ?a } WHERE { ?s ex:years ?a }",
        )
        .unwrap();
        assert_eq!(n.len(), 2);
    }

    #[test]
    fn insert_where_emits_core_prov_shape() {
        let mut graph = g();
        let d = derive_update(
            &mut graph,
            "PREFIX ex: <http://ex/> INSERT { ?s ex:years ?a } WHERE { ?s ex:age ?a }",
            cfg(),
        )
        .unwrap();

        let ls = lines(&d);
        let a = d.activity().as_str();
        let e = d.entity().as_str();

        assert!(ls.contains(&format!(
            "<{a}> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://www.w3.org/ns/prov#Activity> ."
        )));
        assert!(ls.contains(&format!(
            "<{e}> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://www.w3.org/ns/prov#Entity> ."
        )));
        assert!(ls.contains(&format!(
            "<{e}> <http://www.w3.org/ns/prov#wasGeneratedBy> <{a}> ."
        )));
        assert!(ls.contains(&format!(
            "<{a}> <http://www.w3.org/ns/prov#used> <http://ex/source-graph> ."
        )));
        assert!(ls.contains(&format!(
            "<{e}> <http://www.w3.org/ns/prov#wasDerivedFrom> <http://ex/source-graph> ."
        )));
        // The activity is labelled with the operation kind.
        assert!(ls.contains(&format!(
            "<{a}> <http://www.w3.org/2000/01/rdf-schema#label> \"DELETE/INSERT WHERE\" ."
        )));
        // Timing: the fixed clock = 2023-11-14T22:13:20Z, typed xsd:dateTime.
        assert!(ls.contains(&format!(
            "<{a}> <http://www.w3.org/ns/prov#startedAtTime> \"2023-11-14T22:13:20Z\"^^<{}> .",
            xsd::DATE_TIME.as_str()
        )));
        assert!(ls.iter().any(|l| l.contains("#endedAtTime>")));
    }

    #[test]
    fn insert_data_is_a_generation_with_no_where_match() {
        let mut graph = g();
        let d = derive_update(
            &mut graph,
            "PREFIX ex: <http://ex/> INSERT DATA { ex:carol ex:age 40 }",
            cfg(),
        )
        .unwrap();
        assert_eq!(d.inserted().len(), 1);
        assert!(d.deleted().is_empty());
        let ls = lines(&d);
        let a = d.activity().as_str();
        // The kind label distinguishes INSERT DATA from INSERT...WHERE.
        assert!(ls.contains(&format!(
            "<{a}> <http://www.w3.org/2000/01/rdf-schema#label> \"INSERT DATA\" ."
        )));
        // Still has a generated entity.
        assert!(ls.iter().any(|l| l.contains("#wasGeneratedBy>")));
    }

    #[test]
    fn delete_insert_where_records_both_sides() {
        let mut graph = g();
        let d = derive_update(
            &mut graph,
            "PREFIX ex: <http://ex/> \
             DELETE { ?s ex:age ?a } INSERT { ?s ex:years ?a } WHERE { ?s ex:age ?a }",
            cfg(),
        )
        .unwrap();
        assert_eq!(d.inserted().len(), 2, "two ex:years inserts");
        assert_eq!(d.deleted().len(), 2, "two ex:age deletes");

        let ls = lines(&d);
        let a = d.activity().as_str();
        // Each delete is an entity the activity INVALIDATED — never generated/derived.
        let inval: Vec<_> = ls
            .iter()
            .filter(|l| l.contains("#wasInvalidatedBy>"))
            .collect();
        assert_eq!(inval.len(), 2, "one wasInvalidatedBy per deleted triple");
        assert!(inval.iter().all(|l| l.ends_with(&format!("<{a}> ."))));
        // No invalidated entity is also derived-from.
        assert!(ls
            .iter()
            .all(|l| { !(l.contains("#wasInvalidatedBy>") && l.contains("#wasDerivedFrom>")) }));
    }

    #[test]
    fn pure_delete_generates_nothing() {
        let mut graph = g();
        let d = derive_update(
            &mut graph,
            "PREFIX ex: <http://ex/> DELETE { ?s ex:name ?n } WHERE { ?s ex:name ?n }",
            cfg(),
        )
        .unwrap();
        assert!(d.inserted().is_empty());
        assert_eq!(d.deleted().len(), 2);

        let ls = lines(&d);
        // A pure-delete update GENERATED nothing: no result-entity type, no
        // wasGeneratedBy, no wasDerivedFrom.
        assert!(ls.iter().all(|l| !l.contains("#wasGeneratedBy>")));
        assert!(ls.iter().all(|l| !l.contains("#wasDerivedFrom>")));
        // ...but the activity still records `used` (it consulted the inputs).
        assert!(ls.iter().any(|l| l.contains("#used>")));
        // ...and each retracted triple is invalidated.
        assert_eq!(
            ls.iter()
                .filter(|l| l.contains("#wasInvalidatedBy>"))
                .count(),
            2
        );
    }

    #[test]
    fn prov_graph_is_valid_rdf_and_round_trips() {
        let mut graph = g();
        let d = derive_update(
            &mut graph,
            "PREFIX ex: <http://ex/> \
             DELETE { ?s ex:age ?a } INSERT { ?s ex:years ?a } WHERE { ?s ex:age ?a }",
            cfg(),
        )
        .unwrap();
        let nt = d.prov_ntriples();
        let reloaded = Graph::load_str(&nt, "turtle").expect("PROV-O output must be valid RDF");
        assert_eq!(reloaded.len(), d.prov_graph().len());

        use oxttl::NTriplesParser;
        let parsed: Vec<_> = NTriplesParser::new()
            .for_reader(nt.as_bytes())
            .collect::<Result<_, _>>()
            .expect("emitted lineage must be well-formed N-Triples");
        assert_eq!(parsed.len(), d.prov_graph().len());
    }

    /// sq-ijw35: the UpdateDerivation Turtle method preserves every
    /// provenance triple, and the inserted-result entity uses the registered prefix.
    #[test]
    fn prov_turtle_round_trips_the_exact_update_graph() {
        let mut graph = g();
        let d = derive_update(
            &mut graph,
            "PREFIX ex: <http://ex/> INSERT { ?s ex:years ?a } WHERE { ?s ex:age ?a }",
            cfg(),
        )
        .unwrap();

        let turtle = d.prov_turtle();
        assert!(
            turtle.contains("prov:Entity"),
            "expected the prefix-compacted result entity in {turtle}"
        );

        let parsed: Vec<_> = oxttl::TurtleParser::new()
            .for_slice(turtle.as_bytes())
            .collect::<Result<_, _>>()
            .expect("emitted lineage must be well-formed Turtle");
        let expected = d.prov_graph();
        assert_eq!(parsed.len(), expected.len());
        assert_eq!(
            parsed.into_iter().collect::<HashSet<_>>(),
            expected.into_iter().collect::<HashSet<_>>()
        );
    }

    #[test]
    fn explicit_iris_and_agent_are_honoured() {
        let mut graph = g();
        let activity = NamedNode::new_unchecked("http://ex/act/1");
        let entity = NamedNode::new_unchecked("http://ex/result/1");
        let agent = NamedNode::new_unchecked("http://ex/service");
        let config = ProvConfig {
            activity: Some(activity.clone()),
            entity: Some(entity.clone()),
            agent: Some(agent.clone()),
            used: vec![NamedNode::new_unchecked("http://ex/g")],
            clock: fixed_clock,
        };
        let d = derive_update(
            &mut graph,
            "PREFIX ex: <http://ex/> INSERT { ?s ex:years ?a } WHERE { ?s ex:age ?a }",
            config,
        )
        .unwrap();
        assert_eq!(d.activity(), &activity);
        assert_eq!(d.entity(), &entity);
        assert!(lines(&d).contains(
            "<http://ex/act/1> <http://www.w3.org/ns/prov#wasAssociatedWith> <http://ex/service> ."
        ));
    }

    #[test]
    fn minted_iris_are_stable_for_the_same_update() {
        let q = "PREFIX ex: <http://ex/> INSERT { ?s ex:years ?a } WHERE { ?s ex:age ?a }";
        let a = derive_update(&mut g(), q, cfg()).unwrap();
        let b = derive_update(&mut g(), q, cfg()).unwrap();
        assert_eq!(a.activity(), b.activity());
        assert_eq!(a.entity(), b.entity());
        let c = derive_update(
            &mut g(),
            "PREFIX ex: <http://ex/> INSERT DATA { <http://ex/x> <http://ex/p> 1 }",
            cfg(),
        )
        .unwrap();
        assert_ne!(a.activity(), c.activity());
    }

    #[test]
    fn rejects_query_text_not_an_update() {
        // A SELECT is not a valid UPDATE — the engine's parser rejects it.
        assert!(derive_update(&mut g(), "SELECT * WHERE { ?s ?p ?o }", cfg()).is_err());
    }

    #[test]
    fn kind_label_classifies_operations() {
        assert_eq!(kind_label("INSERT DATA { <a> <b> <c> }"), "INSERT DATA");
        assert_eq!(kind_label("DELETE DATA { <a> <b> <c> }"), "DELETE DATA");
        assert_eq!(
            kind_label("PREFIX ex: <http://ex/> INSERT { ?s ?p ?o } WHERE { ?s ?p ?o }"),
            "DELETE/INSERT WHERE"
        );
        assert_eq!(
            kind_label("DELETE WHERE { ?s ?p ?o }"),
            "DELETE/INSERT WHERE"
        );
        assert_eq!(
            kind_label("WITH <http://g/> DELETE { ?s ?p ?o } WHERE { ?s ?p ?o }"),
            "DELETE/INSERT WHERE"
        );
        assert_eq!(kind_label("LOAD <http://ex/doc>"), "LOAD");
        assert_eq!(kind_label("CLEAR DEFAULT"), "SPARQL UPDATE");
        // A multi-op body is labelled generically.
        assert_eq!(
            kind_label("INSERT DATA { <a> <b> <c> } ; DELETE DATA { <a> <b> <c> }"),
            "SPARQL UPDATE"
        );
    }

    #[test]
    fn semicolon_inside_template_is_not_a_multi_op() {
        // Turtle predicate-list ';' inside the INSERT template is NOT an op boundary.
        assert_eq!(
            kind_label(
                "PREFIX ex: <http://ex/> INSERT { ?s ex:a 1 ; ex:b 2 } WHERE { ?s ex:age ?o }"
            ),
            "DELETE/INSERT WHERE"
        );
        // A ';' inside a string literal is data, not an op boundary.
        assert_eq!(
            kind_label("INSERT DATA { <http://ex/s> <http://ex/p> \"a; b\" }"),
            "INSERT DATA"
        );
    }

    #[test]
    fn empty_inputs_omit_used_and_derived_edges() {
        let config = ProvConfig {
            clock: fixed_clock,
            ..ProvConfig::default()
        };
        let mut graph = g();
        let d = derive_update(
            &mut graph,
            "PREFIX ex: <http://ex/> INSERT { ?s ex:years ?a } WHERE { ?s ex:age ?a }",
            config,
        )
        .unwrap();
        let prov = d.prov_graph();
        assert!(prov
            .iter()
            .all(|t| t.predicate.as_str() != "http://www.w3.org/ns/prov#used"));
        assert!(prov
            .iter()
            .all(|t| t.predicate.as_str() != "http://www.w3.org/ns/prov#wasDerivedFrom"));
        // The generation edge survives (there were inserts).
        assert!(prov
            .iter()
            .any(|t| t.predicate.as_str() == "http://www.w3.org/ns/prov#wasGeneratedBy"));
    }

    // ── sq-bif.4: UPDATE-lineage correctness-suite additions — uncovered branches in
    // the effect partitioning, kind classification, and edge cases. ────────

    /// `UpdateDerivation::timing()` reports the captured activity window; under a fixed
    /// clock the start equals the end and both equal the configured instant.
    #[test]
    fn update_timing_reports_the_captured_instants() {
        let mut graph = g();
        let d = derive_update(
            &mut graph,
            "PREFIX ex: <http://ex/> INSERT { ?s ex:years ?a } WHERE { ?s ex:age ?a }",
            cfg(),
        )
        .unwrap();
        let (started, ended) = d.timing();
        assert_eq!(started, fixed_clock());
        assert_eq!(ended, fixed_clock());
        assert!(ended >= started);
    }

    /// `INSERT DATA` whose triple is *already present* records the operand batch as the
    /// generated entity: the engine's `Delta` effect for a ground DATA op carries the
    /// declared operand triples (it does not store-diff against pre-existing data), so the
    /// lineage attributes the asserted-into-existence triple to the activity. This is the
    /// correct reading for `INSERT DATA` — the operation *asserts* that data regardless of
    /// prior presence. The kind label distinguishes it as INSERT DATA.
    #[test]
    fn insert_data_records_operand_batch_as_generation() {
        let mut graph = g();
        let d = derive_update(
            &mut graph,
            // ex:alice ex:age 30 is already asserted in DATA — INSERT DATA re-asserts it.
            "PREFIX ex: <http://ex/> INSERT DATA { ex:alice ex:age 30 }",
            cfg(),
        )
        .unwrap();
        assert_eq!(d.inserted().len(), 1, "the operand triple is the generated data");
        assert!(d.deleted().is_empty());
        let ls = lines(&d);
        assert!(ls.iter().any(|l| l.contains("#wasGeneratedBy>")));
        assert!(ls.contains(&format!(
            "<{}> <http://www.w3.org/2000/01/rdf-schema#label> \"INSERT DATA\" .",
            d.activity().as_str()
        )));
    }

    /// `DELETE DATA` of a triple records the operand batch as a retraction (an
    /// invalidation entity) regardless of whether the triple was present — the operation
    /// *declares* that data removed, and the resolved effect log carries the operand. The
    /// kind is classified as DELETE DATA.
    #[test]
    fn delete_data_records_operand_batch_as_invalidation() {
        let mut graph = g();
        let d = derive_update(
            &mut graph,
            "PREFIX ex: <http://ex/> DELETE DATA { ex:nobody ex:age 99 }",
            cfg(),
        )
        .unwrap();
        assert_eq!(d.deleted().len(), 1, "the operand triple is the retracted data");
        assert!(d.inserted().is_empty());
        let ls = lines(&d);
        // One invalidation entity for the declared retraction; never generated/derived.
        assert_eq!(
            ls.iter().filter(|l| l.contains("#wasInvalidatedBy>")).count(),
            1
        );
        assert!(ls.iter().all(|l| !l.contains("#wasGeneratedBy>")));
        assert!(ls.contains(&format!(
            "<{}> <http://www.w3.org/2000/01/rdf-schema#label> \"DELETE DATA\" .",
            d.activity().as_str()
        )));
    }

    /// A `DELETE … WHERE` whose pattern matches nothing genuinely commits no delta, so the
    /// engine records no `Delta` effect: the derivation retracts nothing and emits no
    /// invalidation entity (a true no-op retraction — the correct PROV reading).
    #[test]
    fn delete_where_no_match_is_a_noop() {
        let mut graph = g();
        let d = derive_update(
            &mut graph,
            "PREFIX ex: <http://ex/> DELETE { ?s ex:age ?a } WHERE { ?s ex:nonexistent ?a }",
            cfg(),
        )
        .unwrap();
        assert!(d.inserted().is_empty());
        assert!(d.deleted().is_empty(), "no WHERE match ⇒ no retraction");
        let ls = lines(&d);
        assert!(ls.iter().all(|l| !l.contains("#wasInvalidatedBy>")));
        assert!(ls.iter().all(|l| !l.contains("#wasGeneratedBy>")));
        // The activity still ran and consulted its inputs.
        assert!(ls.iter().any(|l| l.contains("#used>")));
    }

    /// `DELETE DATA` of present triples is a pure invalidation — each retracted triple is
    /// a fresh blank-node `prov:Entity` `wasInvalidatedBy` the activity, never generated.
    #[test]
    fn delete_data_invalidates_each_present_triple() {
        let mut graph = g();
        let d = derive_update(
            &mut graph,
            "PREFIX ex: <http://ex/> DELETE DATA { ex:alice ex:age 30 . ex:bob ex:age 25 }",
            cfg(),
        )
        .unwrap();
        assert_eq!(d.deleted().len(), 2);
        assert!(d.inserted().is_empty());
        let ls = lines(&d);
        let inval = ls.iter().filter(|l| l.contains("#wasInvalidatedBy>")).count();
        assert_eq!(inval, 2, "one invalidation entity per deleted triple");
        // Each invalidated entity is a fresh blank node (no IRI subject for retracted
        // triples — they no longer exist to be named by value).
        assert!(ls
            .iter()
            .filter(|l| l.contains("#wasInvalidatedBy>"))
            .all(|l| l.starts_with("_:")));
        // A pure delete generates nothing.
        assert!(ls.iter().all(|l| !l.contains("#wasGeneratedBy>")));
    }

    /// Structural ops (`CLEAR`/`DROP`/`CREATE`) carry no resolved triple set, so the
    /// derivation enumerates no inserts/deletes and emits no per-triple entity — the
    /// deliberate honesty boundary documented in the module. They still parse + apply.
    #[test]
    fn structural_ops_emit_no_per_triple_entity() {
        for op in [
            "CLEAR DEFAULT",
            "DROP SILENT DEFAULT",
            "CREATE SILENT GRAPH <http://ex/g>",
        ] {
            let mut graph = g();
            let d = derive_update(&mut graph, op, cfg()).unwrap();
            assert!(d.inserted().is_empty(), "{} inserts nothing", op);
            assert!(d.deleted().is_empty(), "{} enumerates no deletes", op);
            let ls = lines(&d);
            // No generated entity, no per-triple invalidation entity.
            assert!(
                ls.iter().all(|l| !l.contains("#wasGeneratedBy>")),
                "{} must not generate an entity",
                op
            );
            assert!(
                ls.iter().all(|l| !l.contains("#wasInvalidatedBy>")),
                "{} must not enumerate invalidations",
                op
            );
            // …but it IS recorded as an activity (kind label "SPARQL UPDATE").
            assert!(ls.iter().any(|l| l.contains("#Activity>")));
        }
    }

    /// The emitted lineage of a pure-delete update is well-formed RDF with blank-node
    /// invalidation entities, and round-trips through both the loader and an independent
    /// N-Triples parser.
    #[test]
    fn pure_delete_lineage_round_trips() {
        let mut graph = g();
        let d = derive_update(
            &mut graph,
            "PREFIX ex: <http://ex/> DELETE { ?s ex:name ?n } WHERE { ?s ex:name ?n }",
            cfg(),
        )
        .unwrap();
        let nt = d.prov_ntriples();
        let reloaded = Graph::load_str(&nt, "turtle").expect("PROV-O must be valid RDF");
        assert_eq!(reloaded.len(), d.prov_graph().len());
        use oxttl::NTriplesParser;
        let parsed: Vec<_> = NTriplesParser::new()
            .for_reader(nt.as_bytes())
            .collect::<Result<_, _>>()
            .expect("well-formed N-Triples");
        assert_eq!(parsed.len(), d.prov_graph().len());
    }

    /// `terms_to_triple` defensively rejects a malformed resolved triple — a literal or
    /// blank-node *predicate*, or a literal *subject* — by skipping it (returning `None`)
    /// rather than panicking. The SPARQL grammar never produces these, but the converter
    /// is the boundary, so the guard is exercised directly.
    #[test]
    fn terms_to_triple_skips_malformed_shapes() {
        use oxrdf::{BlankNode, Literal, NamedNode as NN};
        let iri = || Term::NamedNode(NN::new_unchecked("http://ex/p"));
        let lit = || Term::Literal(Literal::new_simple_literal("x"));
        let blank = || Term::BlankNode(BlankNode::default());

        // Well-formed: IRI subject, IRI predicate, any object.
        assert!(terms_to_triple(&[iri(), iri(), lit()]).is_some());
        // Well-formed: blank-node subject.
        assert!(terms_to_triple(&[blank(), iri(), iri()]).is_some());
        // Malformed: literal subject ⇒ skipped.
        assert!(terms_to_triple(&[lit(), iri(), iri()]).is_none());
        // Malformed: literal predicate ⇒ skipped.
        assert!(terms_to_triple(&[iri(), lit(), iri()]).is_none());
        // Malformed: blank-node predicate ⇒ skipped.
        assert!(terms_to_triple(&[iri(), blank(), iri()]).is_none());
    }

    /// `partition_effects` splits a `Delta` effect log into inserts / deletes preserving
    /// capture order. Built from a `Delta` (the only externally-expressible variant — its
    /// fields erase to `Option<Term>` + `[Term; 3]`), so the converter is exercised on a
    /// hand-built log; the structural-marker skip path is covered by the real-engine
    /// `structural_ops_emit_no_per_triple_entity` test.
    #[test]
    fn partition_effects_splits_and_preserves_order() {
        use oxrdf::NamedNode as NN;
        let term = |s: &str| Term::NamedNode(NN::new_unchecked(s));
        let t = |o: &str| [term("http://ex/s"), term("http://ex/p"), term(o)];
        let effects = vec![UpdateEffect::Delta {
            slot: None, // default graph
            inserts: vec![t("http://ex/a"), t("http://ex/b")],
            deletes: vec![t("http://ex/c")],
        }];
        let (inserted, deleted) = partition_effects(&effects);
        assert_eq!(inserted.len(), 2, "two resolved inserts, order preserved");
        assert_eq!(deleted.len(), 1, "one resolved delete");
        assert_eq!(inserted[0].object, term("http://ex/a"));
        assert_eq!(inserted[1].object, term("http://ex/b"));
        assert_eq!(deleted[0].object, term("http://ex/c"));
    }

    /// `strip_prologue` skips a leading `BASE <…>` declaration (as well as `PREFIX`) so
    /// `kind_label` classifies the first real operation, not the prologue keyword.
    #[test]
    fn kind_label_strips_base_prologue() {
        assert_eq!(
            kind_label("BASE <http://ex/> INSERT DATA { <s> <p> <o> }"),
            "INSERT DATA"
        );
        // Mixed BASE + PREFIX prologue before the operation.
        assert_eq!(
            kind_label("BASE <http://ex/> PREFIX ex: <http://ex/> DELETE DATA { <s> <p> <o> }"),
            "DELETE DATA"
        );
    }

    /// A trailing top-level `;` with nothing after it is NOT a multi-op body — `is_multi_op`
    /// returns false for a single operation that merely ends in a separator.
    #[test]
    fn kind_label_trailing_semicolon_is_single_op() {
        assert_eq!(
            kind_label("INSERT DATA { <a> <b> <c> } ;"),
            "INSERT DATA",
            "a lone trailing ';' is not a second operation"
        );
        // Whitespace after the trailing ';' is also not an op.
        assert_eq!(kind_label("INSERT DATA { <a> <b> <c> } ;   "), "INSERT DATA");
    }

    /// A `;` inside a single-quoted string literal and inside an IRI is data / part of the
    /// term, never an operation boundary — `is_multi_op`/`has_top_level_semicolon` must
    /// ignore both. (Covers the single-quote and IRI-skip arms of both scanners.)
    #[test]
    fn semicolon_in_squote_and_iri_is_not_a_boundary() {
        // ';' inside a single-quoted literal in INSERT DATA: still one op.
        assert_eq!(
            kind_label("INSERT DATA { <http://ex/s> <http://ex/p> 'a; b' }"),
            "INSERT DATA"
        );
        // ';' that only ever appears inside an <IRI> is not a boundary.
        assert_eq!(
            kind_label("INSERT DATA { <http://ex/a;b> <http://ex/p> <http://ex/o> }"),
            "INSERT DATA"
        );
        // A genuine top-level ';' (outside braces/strings/IRIs) IS a multi-op boundary.
        assert_eq!(
            kind_label("DROP DEFAULT ; CREATE GRAPH <http://ex/g>"),
            "SPARQL UPDATE"
        );
    }

    /// A multi-op body whose FIRST operation contains a quoted `;` (inside a template
    /// literal) before the real top-level `;` op-boundary: the precise second-pass scanner
    /// (`has_top_level_semicolon`) must skip the in-string `;` (tracking both double- and
    /// single-quote state) and still find the genuine boundary ⇒ multi-op.
    #[test]
    fn quoted_semicolon_before_real_boundary_is_still_multi_op() {
        // Double-quoted ';' inside the first INSERT DATA template, then a real ';' boundary.
        assert_eq!(
            kind_label(
                "INSERT DATA { <http://ex/s> <http://ex/p> \"a; b\" } ; DROP DEFAULT"
            ),
            "SPARQL UPDATE"
        );
        // Single-quoted ';' inside the first template, then a real ';' boundary.
        assert_eq!(
            kind_label(
                "INSERT DATA { <http://ex/s> <http://ex/p> 'a; b' } ; CREATE GRAPH <http://ex/g>"
            ),
            "SPARQL UPDATE"
        );
    }

    // ── sq-qcnn.39: mutation-kill additions — exact prov_graph triple counts,
    // prov:value annotation check, and USING keyword coverage. ─────

    /// `UpdateDerivation::prov_graph()` for an INSERT…WHERE update (inserts non-empty,
    /// 1 used input, no agent) must emit exactly 9 triples (5 activity: type/label/value/
    /// startedAt/endedAt; 3 entity: type/wasGeneratedBy/wasDerivedFrom; 1 used).
    /// Pinning the count kills mutations that drop any single `push()` call.
    #[test]
    fn insert_where_prov_graph_exact_triple_count() {
        let mut graph = g();
        let d = derive_update(
            &mut graph,
            "PREFIX ex: <http://ex/> INSERT { ?s ex:years ?a } WHERE { ?s ex:age ?a }",
            cfg(),
        )
        .unwrap();
        // 5 activity + 3 entity (inserts non-empty) + 1 used = 9
        assert_eq!(
            d.prov_graph().len(),
            9,
            "INSERT WHERE with 1 input emits 9 prov triples"
        );
    }

    /// For a pure-DELETE update (2 deleted triples, 1 used input, no agent), the exact
    /// count is: 5 activity + 0 entity (inserts empty) + 1 used + 4 invalidation
    /// (2 × entity-type + wasInvalidatedBy) = 10. sq-qcnn.39
    #[test]
    fn pure_delete_prov_graph_exact_triple_count() {
        let mut graph = g();
        let d = derive_update(
            &mut graph,
            "PREFIX ex: <http://ex/> DELETE { ?s ex:name ?n } WHERE { ?s ex:name ?n }",
            cfg(),
        )
        .unwrap();
        assert_eq!(d.deleted().len(), 2, "two name triples deleted");
        // 5 activity + 1 used + 4 invalidation (2 × rdf:type + wasInvalidatedBy) = 10
        assert_eq!(
            d.prov_graph().len(),
            10,
            "pure DELETE with 2 deletes + 1 input emits 10 prov triples"
        );
    }

    /// For a DELETE/INSERT WHERE (2 inserts, 2 deletes, 1 used input), the exact count
    /// is: 5 activity + 3 entity + 1 used + 4 invalidation = 13. sq-qcnn.39
    #[test]
    fn delete_insert_prov_graph_exact_triple_count() {
        let mut graph = g();
        let d = derive_update(
            &mut graph,
            "PREFIX ex: <http://ex/> \
             DELETE { ?s ex:age ?a } INSERT { ?s ex:years ?a } WHERE { ?s ex:age ?a }",
            cfg(),
        )
        .unwrap();
        assert_eq!(d.inserted().len(), 2, "two inserts");
        assert_eq!(d.deleted().len(), 2, "two deletes");
        // 5 activity + 3 entity (wasGeneratedBy + type + wasDerivedFrom×1)
        // + 1 used + 4 invalidation = 13
        assert_eq!(
            d.prov_graph().len(),
            13,
            "DELETE/INSERT WHERE with 2 inserts + 2 deletes + 1 input emits 13 prov triples"
        );
    }

    /// The update activity records the verbatim update text as a `prov:value` recipe —
    /// the same annotation that `Derivation` (CONSTRUCT) emits for the query text.
    /// No existing test checks for `prov:value` in `UpdateDerivation::prov_graph()`,
    /// so this test kills the mutation that drops that push. sq-qcnn.39
    #[test]
    fn update_prov_graph_contains_query_recipe() {
        let q = "PREFIX ex: <http://ex/> INSERT { ?s ex:years ?a } WHERE { ?s ex:age ?a }";
        let mut graph = g();
        let d = derive_update(&mut graph, q, cfg()).unwrap();
        let ls = lines(&d);
        let a = d.activity().as_str();
        // The verbatim update text is recorded on the activity as prov:value.
        assert!(
            ls.contains(&format!(
                "<{a}> <http://www.w3.org/ns/prov#value> \"{q}\" ."
            )),
            "update prov_graph must carry the query text as prov:value; lines: {ls:?}"
        );
    }

    /// `kind_label` must classify a `USING`-prefixed update as `"DELETE/INSERT WHERE"`.
    /// The `USING` form is covered by the same branch as `WITH` / `INSERT` / `DELETE`
    /// (`starts_with("INSERT") || starts_with("DELETE") || starts_with("WITH") || starts_with("USING")`).
    /// Not having a direct test for `USING` leaves that branch arm unexercised by an
    /// exact-value assertion, allowing a mutation that removes the `starts_with("USING")`
    /// conjunct to survive. sq-qcnn.39
    #[test]
    fn kind_label_using_is_delete_insert_where() {
        assert_eq!(
            kind_label("USING <http://ex/g> DELETE { ?s ?p ?o } WHERE { ?s ?p ?o }"),
            "DELETE/INSERT WHERE"
        );
        // PREFIX-stripped form (strip_prologue removes the prologue, USING is then first).
        assert_eq!(
            kind_label(
                "PREFIX ex: <http://ex/> USING <http://ex/g> \
                 DELETE { ?s ex:p ?o } WHERE { ?s ex:p ?o }"
            ),
            "DELETE/INSERT WHERE"
        );
    }

    /// Pins the exact minted activity and entity IRIs for a known UPDATE derivation
    /// (fixed clock + fixed query text). The IRI is:
    ///   `urn:sparq:prov:{role}:{s_nanos:x}-{e_nanos:x}-{fnv1a(query):016x}`
    /// where s_nanos = e_nanos = 1_700_000_000 * 1_000_000_000 = 0x17979cfe362a0000
    /// and fnv1a("PREFIX ex: <http://ex/> INSERT { ?s ex:years ?a } WHERE { ?s ex:age ?a }")
    /// = 0x5125ddbf0b0c39fd.
    /// This kills any arithmetic mutation in `mint()` that alters the FNV-1a output,
    /// exercising the UPDATE call path specifically. sq-qcnn.39
    #[test]
    fn minted_iri_for_update_has_exact_known_value() {
        let q = "PREFIX ex: <http://ex/> INSERT { ?s ex:years ?a } WHERE { ?s ex:age ?a }";
        let d = derive_update(&mut g(), q, cfg()).unwrap();
        assert_eq!(
            d.activity().as_str(),
            "urn:sparq:prov:activity:17979cfe362a0000-17979cfe362a0000-5125ddbf0b0c39fd",
            "activity IRI must match FNV-1a XOR hash of the update text + fixed-clock nanos"
        );
        assert_eq!(
            d.entity().as_str(),
            "urn:sparq:prov:entity:17979cfe362a0000-17979cfe362a0000-5125ddbf0b0c39fd",
            "entity IRI must match FNV-1a XOR hash of the update text + fixed-clock nanos"
        );
    }

    // ── sq-d6hen: mutation hardening (from #1723) — direct truth-table tests over the
    // `is_multi_op` / `has_top_level_semicolon` boolean state machines. Testing only via
    // `kind_label` lets first-pass mutants survive, because `has_top_level_semicolon` is
    // a correcting second pass; these rows pin each scanner state (IRI / double-quote /
    // single-quote / brace-depth) and each cross-state guard directly.
    //
    // Triage note for future mutation runs: mutants that make `is_multi_op`'s FIRST pass
    // over-detect a `;` (e.g. `&&` → `||` on its guards) are semantically EQUIVALENT and
    // unkillable — a falsely-detected in-term `;` can never have a blank tail (the term's
    // closing delimiter follows it), so control always defers to the precise second pass,
    // which returns the correct global answer. Only under-detection, stuck-state, and
    // second-pass mutants are killable; the rows below cover those. ──────────

    /// Baseline rows: presence/absence of a top-level `;`, the non-blank-tail
    /// requirement, and degenerate inputs (a bare `;` also pins the `i + 1` tail-slice
    /// arithmetic: `i - 1` underflows, `i * 1` would report the `;` itself as a tail).
    #[test]
    fn has_top_level_semicolon_baseline_truth_table() {
        assert!(
            has_top_level_semicolon("A ; B"),
            "top-level ';' with a following op"
        );
        assert!(!has_top_level_semicolon("A B"), "no ';' at all");
        assert!(!has_top_level_semicolon("A ;"), "blank tail after ';'");
        assert!(
            !has_top_level_semicolon("A ;   "),
            "whitespace-only tail after ';'"
        );
        assert!(!has_top_level_semicolon(";"), "bare separator, no ops");
        assert!(!has_top_level_semicolon(""), "empty input");
    }

    /// Brace-depth rows: a `;` inside `{ … }` is Turtle predicate-list shorthand, never
    /// an op boundary; a `;` after the braces close is. The close-then-`;` row kills
    /// `+=`/`-=` swaps on the depth counter; the unbalanced-`}` row pins `brace == 0`
    /// exactly (a `<=`/`<` mutant would accept the negative depth).
    #[test]
    fn has_top_level_semicolon_brace_depth_truth_table() {
        assert!(
            !has_top_level_semicolon("{ A ; B }"),
            "';' inside braces is a predicate list, not a boundary"
        );
        assert!(
            has_top_level_semicolon("{ A } ; B"),
            "';' after braces close IS a boundary"
        );
        assert!(
            !has_top_level_semicolon("{ { A ; B } } C"),
            "';' at nested depth 2 is not a boundary"
        );
        assert!(
            has_top_level_semicolon("{ { A } } ; B"),
            "';' after nested braces close back to depth 0"
        );
        assert!(
            !has_top_level_semicolon("} ; A"),
            "';' at negative depth (unbalanced '}}') is not depth 0"
        );
    }

    /// Term-skip rows: a `;` inside an `<IRI>` / `"…"` / `'…'` is data; once the term
    /// closes, a following `;` is a real boundary (the true rows kill stuck-state
    /// mutants where a toggle becomes `= true`).
    #[test]
    fn has_top_level_semicolon_term_skip_truth_table() {
        assert!(!has_top_level_semicolon("<A;B> C"), "';' inside an IRI");
        assert!(
            has_top_level_semicolon("<A;B> ; C"),
            "IRI closes, then a real ';'"
        );
        assert!(
            !has_top_level_semicolon("\"A;B\" C"),
            "';' inside a double-quoted literal"
        );
        assert!(
            has_top_level_semicolon("\"A;B\" ; C"),
            "double-quoted literal closes, then a real ';'"
        );
        assert!(
            !has_top_level_semicolon("'A;B' C"),
            "';' inside a single-quoted literal"
        );
        assert!(
            has_top_level_semicolon("'A;B' ; C"),
            "single-quoted literal closes, then a real ';'"
        );
    }

    /// Cross-state guard rows: each row is true ONLY if the named state-opening byte is
    /// inert while a different state is active — dropping any one guard conjunct
    /// (or `&&` → `||`) leaves the scanner stuck in a phantom state so the trailing
    /// real `;` is missed.
    #[test]
    fn has_top_level_semicolon_cross_state_guard_truth_table() {
        assert!(
            has_top_level_semicolon("\"A'B\" ; C"),
            "'\\'' inside a double-quoted literal must not open single-quote state"
        );
        assert!(
            has_top_level_semicolon("'A\"B' ; C"),
            "'\"' inside a single-quoted literal must not open double-quote state"
        );
        assert!(
            has_top_level_semicolon("<A\"B> ; C"),
            "'\"' inside an IRI must not open double-quote state"
        );
        assert!(
            has_top_level_semicolon("<A'B> ; C"),
            "'\\'' inside an IRI must not open single-quote state"
        );
        assert!(
            has_top_level_semicolon("\"A<B\" ; C"),
            "'<' inside a double-quoted literal must not open IRI state"
        );
        assert!(
            has_top_level_semicolon("'A<B' ; C"),
            "'<' inside a single-quoted literal must not open IRI state"
        );
        assert!(
            has_top_level_semicolon("\"A{B\" ; C"),
            "'{{' inside a double-quoted literal must not bump brace depth"
        );
        assert!(
            has_top_level_semicolon("'A{B' ; C"),
            "'{{' inside a single-quoted literal must not bump brace depth"
        );
        assert!(
            has_top_level_semicolon("<A{B> ; C"),
            "'{{' inside an IRI must not bump brace depth"
        );
        assert!(
            has_top_level_semicolon("\"A}B\" ; C"),
            "'}}' inside a double-quoted literal must not drop brace depth below 0"
        );
        assert!(
            has_top_level_semicolon("'A}B' ; C"),
            "'}}' inside a single-quoted literal must not drop brace depth below 0"
        );
        assert!(
            has_top_level_semicolon("<A}B> ; C"),
            "'}}' inside an IRI must not drop brace depth below 0"
        );
        assert!(
            has_top_level_semicolon("A > B ; C"),
            "a stray '>' with no open IRI is inert"
        );
    }

    /// `is_multi_op` first-pass truth table: baseline rows plus the trailing-separator
    /// early return (a bare `;` also pins the first pass's `i + 1` tail slice).
    #[test]
    fn is_multi_op_baseline_truth_table() {
        assert!(
            is_multi_op("DROP DEFAULT ; CREATE GRAPH <HTTP://EX/G>"),
            "a genuine top-level op boundary"
        );
        assert!(!is_multi_op("DROP DEFAULT"), "single op, no ';'");
        assert!(!is_multi_op("DROP DEFAULT ;"), "lone trailing separator");
        assert!(
            !is_multi_op("DROP DEFAULT ;   "),
            "trailing separator + whitespace"
        );
        assert!(!is_multi_op(";"), "bare separator, no ops");
        assert!(!is_multi_op(""), "empty input");
        // First pass has no brace tracking, so it detects this ';' and must defer to
        // the precise pass, which rejects it (predicate-list shorthand).
        assert!(
            !is_multi_op("INSERT DATA { <S> <P> <O> ; <P2> <O2> }"),
            "';' inside a template is not a boundary"
        );
    }

    /// `is_multi_op` state rows: each true row requires the first pass to correctly
    /// LEAVE a term state again (a toggle mutated to `= true` sticks and hides the real
    /// boundary); the cross-guard rows mirror the precise pass's guard conjuncts.
    #[test]
    fn is_multi_op_state_tracking_truth_table() {
        assert!(!is_multi_op("CREATE GRAPH <HTTP://EX/A;B>"), "';' only inside an IRI");
        assert!(!is_multi_op("X \"A;B\""), "';' only inside a double-quoted literal");
        assert!(!is_multi_op("X 'A;B'"), "';' only inside a single-quoted literal");
        assert!(is_multi_op("<A> ; B"), "IRI closes, then a real boundary");
        assert!(is_multi_op("\"A\" ; B"), "double quote closes, then a real boundary");
        assert!(is_multi_op("'A' ; B"), "single quote closes, then a real boundary");
        assert!(
            is_multi_op("\"A<B\" ; C"),
            "'<' inside a double-quoted literal must not open IRI state"
        );
        assert!(
            is_multi_op("'A<B' ; C"),
            "'<' inside a single-quoted literal must not open IRI state"
        );
        assert!(
            is_multi_op("<A\"B> ; C"),
            "'\"' inside an IRI must not open double-quote state"
        );
        assert!(
            is_multi_op("<A'B> ; C"),
            "'\\'' inside an IRI must not open single-quote state"
        );
        assert!(
            is_multi_op("\"A'B\" ; C"),
            "'\\'' inside a double-quoted literal must not open single-quote state"
        );
        assert!(
            is_multi_op("'A\"B' ; C"),
            "'\"' inside a single-quoted literal must not open double-quote state"
        );
    }
}
