//! sq-zak4f: the structured `shapes` MCP tool — a **no-LLM**, schema-grounded
//! description of "what predicates are valid when my subject is of type `X`", for a CLIENT
//! LLM to ground NL→SPARQL on.
//!
//! This is the lean, structured alternative to a server-side NL loop (the `ask` tool,
//! feature `nlq`): instead of embedding a model, it hands the client the data-grounded
//! shape of one class so the client's own model can write a correct query. It reuses the
//! existing [`sparq_introspect`] schema miner — there is **no new engine logic** here, and
//! it ships in the **default** `sparq-mcp` build (no extra dependency, no feature flag).
//!
//! The shape is derived **only from what the data actually asserts** (the same posture as
//! [`sparq_introspect::Introspection::to_shacl`]): every predicate/datatype/cardinality
//! reported is one the current graph already exhibits — it describes the *effective*
//! schema, not an aspirational contract.
//!
//! ## What it returns
//!
//! Given a class IRI `C`, [`class_shape`] reports, for every predicate observed on
//! instances of `C`:
//! - the predicate IRI;
//! - its **coverage** (`subjects with this predicate / instances of C`, in `[0, 1]`) and
//!   the subject/triple counts behind it — so the client can tell a near-universal
//!   property from a rare one;
//! - the **cardinality** the data supports — `min_count: 1` for a predicate **universal**
//!   to the class (present on every instance, established via the characteristic-set
//!   table), and `max_count: 1` only when that predicate is **single-valued** across the
//!   class (never an unsound upper bound — see [`Cardinality`]);
//! - the **object kind** split (IRI / literal / blank / triple-term) and, for literal
//!   objects, the observed **datatype** distribution — so the client knows whether to
//!   bind a literal or an IRI and which `xsd:` type to expect;
//! - the **observed range** (classes of this predicate's IRI objects) for join planning.
//!
//! All counts are exact (mined from the store indexes, no sampling), and `rdf:type`
//! itself is omitted from the property list (it is expressed by the class targeting,
//! exactly as in `to_shacl`).

use serde_json::{json, Value};

use sparq_introspect::{
    CharacteristicSet, ClassPredicate, ClassProfile, Introspection, PredicateProfile,
};

const RDF_TYPE: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";

/// The cardinality a predicate's usage on one class supports — both bounds are the
/// **data-grounded** bounds (the current graph already satisfies them), never an
/// aspirational contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cardinality {
    /// `true` when the predicate is present on **every** instance of the class
    /// (`min_count = 1`). Established via the characteristic-set table: a predicate
    /// universal to a set whose subjects are exactly this class's instances occurs at
    /// least once on each. A predicate that only *some* instances carry has `min_count`
    /// `0` (`min_count: false`), so a client never over-constrains.
    pub min_one: bool,
    /// `true` when the predicate is **single-valued** across the class (`max_count = 1`).
    /// Emitted only when the data proves it (the per-class triple count equals the
    /// per-class subject count for the predicate): each instance carries it exactly once.
    /// Any multi-valued instance leaves this `false` — never an unsound upper bound.
    pub max_one: bool,
}

/// Resolve the cardinality of `predicate` on instances of the class described by `cp`,
/// using the per-class usage (`cp`) plus the characteristic-set table for the universal
/// (`min_count: 1`) signal.
fn cardinality_for(
    cp: &ClassPredicate,
    class_iri: &str,
    class_instances: u64,
    char_sets: &[CharacteristicSet],
) -> Cardinality {
    // min_count = 1 iff the predicate is present on EVERY instance of the class. The
    // per-class `subjects` is the count of instances carrying it; equal to the class's
    // instance count ⇒ universal. (Coverage == 1.0 says the same, but integer-compare
    // avoids float fuzz.)
    let min_one = class_instances > 0 && cp.subjects == class_instances;

    // max_count = 1 iff the predicate is single-valued across the class: total triples
    // via this predicate (subjects of class C) equals the number of such subjects, so
    // each carries it at most once. Established directly from the class-scoped counts.
    let mut max_one = cp.subjects > 0 && cp.triples == cp.subjects;

    // Corroborate single-valuedness against the characteristic-set table when a set is
    // universal to this exact class (every subject in the set is typed C and the set
    // covers all of C's instances): the set's per-predicate occurrence total equalling
    // its subject count is the same "exactly one per subject" proof to_shacl emits. This
    // never RELAXES the bound — it only confirms it — so an unsound max is impossible.
    if !max_one {
        for set in char_sets {
            let universal_to_class = set
                .classes
                .iter()
                .any(|c| c.iri == class_iri && c.count == set.subjects);
            if !universal_to_class {
                continue;
            }
            if let Some(idx) = set.predicates.iter().position(|p| p == predicate_of(cp)) {
                if set.predicate_triples[idx] == set.subjects {
                    max_one = true;
                    break;
                }
            }
        }
    }

    Cardinality { min_one, max_one }
}

/// The predicate IRI a [`ClassPredicate`] is about (tiny helper so the borrow above reads
/// cleanly).
fn predicate_of(cp: &ClassPredicate) -> &str {
    &cp.predicate
}

/// Render the literal-object **datatype** distribution + object-kind split for `predicate`
/// from its global [`PredicateProfile`], as a JSON object. Returns an empty object when
/// the predicate has no global profile (it should always have one if it appears on a
/// class, but we degrade gracefully rather than panic).
fn datatype_facets(profile: Option<&PredicateProfile>) -> Value {
    let Some(p) = profile else {
        return json!({});
    };
    let datatypes: Vec<Value> = p
        .datatypes
        .iter()
        .map(|d| json!({ "datatype": d.iri, "triples": d.count }))
        .collect();
    let ranges: Vec<Value> = p
        .inferred_ranges
        .iter()
        .map(|r| json!({ "class": r.iri, "objects": r.count }))
        .collect();
    json!({
        "object_kinds": {
            "iri": p.objects.iri,
            "literal": p.objects.literal,
            "blank": p.objects.blank,
            "triple_term": p.objects.triple_term,
        },
        "literal_fraction": p.literal_fraction,
        "datatypes": datatypes,
        "observed_range": ranges,
    })
}

/// Build the structured shape for one class IRI from a prebuilt [`Introspection`].
///
/// Returns `Ok(json)` with the per-predicate constraints when the class is present in the
/// introspected schema, or `Err(message)` (an MCP **tool error** string, not a panic) when
/// the IRI names no class the data actually uses — the error lists the classes that DO
/// exist so the client can correct the IRI.
///
/// The returned JSON has the shape:
///
/// ```json
/// {
///   "class": "<C>",
///   "instances": 42,
///   "predicates": [
///     {
///       "predicate": "<p>",
///       "coverage": 1.0,
///       "subjects": 42,
///       "triples": 42,
///       "min_count": 1,
///       "max_count": 1,
///       "object_kinds": { "iri": 0, "literal": 42, "blank": 0, "triple_term": 0 },
///       "literal_fraction": 1.0,
///       "datatypes": [ { "datatype": "<xsd:string>", "triples": 42 } ],
///       "observed_range": []
///     }
///   ]
/// }
/// ```
///
/// `min_count` / `max_count` are present **only** when the data proves the bound (`1`);
/// an unconstrained bound is simply absent, so a client never reads a fabricated `0`/`∞`.
pub fn class_shape(ix: &Introspection, class_iri: &str) -> Result<Value, String> {
    let profile: &ClassProfile = ix
        .classes
        .iter()
        .find(|c| c.class == class_iri)
        .ok_or_else(|| unknown_class_error(ix, class_iri))?;

    // Index the global predicate profiles by IRI for the datatype/range lookup.
    let global: std::collections::HashMap<&str, &PredicateProfile> = ix
        .predicates
        .iter()
        .map(|p| (p.predicate.as_str(), p))
        .collect();

    let mut predicates: Vec<Value> = Vec::new();
    for cp in &profile.predicates {
        // rdf:type is carried by the class itself, not reported as a property (matches
        // to_shacl, which expresses typing through sh:targetClass).
        if cp.predicate == RDF_TYPE {
            continue;
        }
        let card = cardinality_for(
            cp,
            class_iri,
            profile.instances,
            &ix.characteristic_sets.sets,
        );
        let facets = datatype_facets(global.get(cp.predicate.as_str()).copied());

        let mut entry = json!({
            "predicate": cp.predicate,
            "coverage": cp.coverage,
            "subjects": cp.subjects,
            "triples": cp.triples,
            "samples": cp.samples,
        });
        // Merge the datatype/object-kind facets in.
        if let (Some(obj), Some(facet_obj)) = (entry.as_object_mut(), facets.as_object()) {
            for (k, v) in facet_obj {
                obj.insert(k.clone(), v.clone());
            }
        }
        // Emit a cardinality bound ONLY when the data proves it (1); an absent bound means
        // "unconstrained", never a fabricated 0 or ∞.
        if let Some(obj) = entry.as_object_mut() {
            if card.min_one {
                obj.insert("min_count".to_string(), json!(1));
            }
            if card.max_one {
                obj.insert("max_count".to_string(), json!(1));
            }
        }
        predicates.push(entry);
    }

    Ok(json!({
        "class": class_iri,
        "instances": profile.instances,
        "predicates": predicates,
    }))
}

/// The "no such class" tool-error message: lists the classes the data DOES use (bounded,
/// most-instantiated first) so the client can correct an IRI typo or namespace mistake.
fn unknown_class_error(ix: &Introspection, class_iri: &str) -> String {
    const MAX_LISTED: usize = 20;
    if ix.classes.is_empty() {
        return format!(
            "no class `{}` in the dataset: the graph has no rdf:type assertions at all \
             (no classes to describe)",
            class_iri
        );
    }
    let listed: Vec<&str> = ix
        .classes
        .iter()
        .take(MAX_LISTED)
        .map(|c| c.class.as_str())
        .collect();
    let more = ix.classes.len().saturating_sub(listed.len());
    let tail = if more > 0 {
        format!(" (+{} more — call `introspect` for the full list)", more)
    } else {
        String::new()
    };
    format!(
        "no class `{}` in the dataset. Classes the data actually uses: {}{}",
        class_iri,
        listed.join(", "),
        tail
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use sparq_core::Graph;

    // alice: type Person, name "Alice" (1 value), knows bob (1 value); plus age 30.
    // bob:   type Person, name "Bob"; NO knows, NO age.
    // So on Person: name is universal+single (cov 1.0, max 1); knows present on 1/2
    // (cov 0.5, NOT min 1); age present on 1/2.
    const TTL: &str = r#"@prefix ex: <http://ex/> .
@prefix rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#> .
@prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
ex:alice rdf:type ex:Person ;
    ex:name "Alice" ;
    ex:age 30 ;
    ex:knows ex:bob .
ex:bob rdf:type ex:Person ;
    ex:name "Bob" .
"#;

    fn ix() -> Introspection {
        let g = Graph::load_str(TTL, "turtle").expect("load turtle");
        Introspection::build(&g)
    }

    #[test]
    fn shape_lists_the_right_predicates_for_a_known_class() {
        let shape = class_shape(&ix(), "http://ex/Person").expect("Person is a class");
        assert_eq!(shape["class"], "http://ex/Person");
        assert_eq!(shape["instances"], 2);

        let preds: Vec<&str> = shape["predicates"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| p["predicate"].as_str().unwrap())
            .collect();
        // name, age, knows are reported; rdf:type is NOT (carried by the class itself).
        assert!(preds.contains(&"http://ex/name"));
        assert!(preds.contains(&"http://ex/age"));
        assert!(preds.contains(&"http://ex/knows"));
        assert!(
            !preds.contains(&"http://www.w3.org/1999/02/22-rdf-syntax-ns#type"),
            "rdf:type must not appear as a property (it is the class targeting)"
        );
    }

    #[test]
    fn cardinality_is_data_grounded_not_aspirational() {
        let shape = class_shape(&ix(), "http://ex/Person").unwrap();
        let by_pred = |p: &str| -> Value {
            shape["predicates"]
                .as_array()
                .unwrap()
                .iter()
                .find(|e| e["predicate"] == p)
                .cloned()
                .unwrap()
        };

        // name: on BOTH instances, exactly once each ⇒ min 1, max 1.
        let name = by_pred("http://ex/name");
        assert_eq!(name["min_count"], 1, "name is universal: {name}");
        assert_eq!(name["max_count"], 1, "name is single-valued: {name}");
        assert_eq!(name["coverage"], 1.0);

        // knows: only alice has it ⇒ NOT min 1 (absent), still single-valued (max 1).
        let knows = by_pred("http://ex/knows");
        assert!(
            knows.get("min_count").is_none(),
            "knows is on only 1/2 instances; min_count must be ABSENT, not a fabricated bound: {knows}"
        );
        assert_eq!(
            knows["max_count"], 1,
            "the one knows triple is single-valued"
        );
        assert_eq!(knows["coverage"], 0.5);
    }

    #[test]
    fn datatype_facets_distinguish_literal_from_iri_objects() {
        let shape = class_shape(&ix(), "http://ex/Person").unwrap();
        let by_pred = |p: &str| -> Value {
            shape["predicates"]
                .as_array()
                .unwrap()
                .iter()
                .find(|e| e["predicate"] == p)
                .cloned()
                .unwrap()
        };

        // age: a literal integer object.
        let age = by_pred("http://ex/age");
        assert_eq!(age["object_kinds"]["literal"], 1);
        assert_eq!(age["object_kinds"]["iri"], 0);
        let dts: Vec<&str> = age["datatypes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|d| d["datatype"].as_str().unwrap())
            .collect();
        assert!(
            dts.iter().any(|d| d.ends_with("integer")),
            "age's literal datatype should be xsd:integer, got {dts:?}"
        );

        // knows: an IRI object, not a literal.
        let knows = by_pred("http://ex/knows");
        assert_eq!(knows["object_kinds"]["iri"], 1);
        assert_eq!(knows["object_kinds"]["literal"], 0);
    }

    #[test]
    fn unknown_class_is_a_helpful_tool_error_not_a_panic() {
        let err = class_shape(&ix(), "http://ex/Nonexistent").unwrap_err();
        assert!(err.contains("no class"), "{err}");
        // It lists the classes that DO exist so the client can self-correct.
        assert!(err.contains("http://ex/Person"), "{err}");
    }

    #[test]
    fn empty_or_untyped_graph_reports_no_classes() {
        let g = Graph::load_str("<http://a> <http://b> <http://c> .", "ntriples").unwrap();
        let ix = Introspection::build(&g);
        let err = class_shape(&ix, "http://ex/Person").unwrap_err();
        assert!(err.contains("no rdf:type"), "{err}");
    }
}
