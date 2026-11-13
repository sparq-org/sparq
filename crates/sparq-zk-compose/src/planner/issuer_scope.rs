//! Syntax admission only. The adapter must resolve and enforce the returned issuer.
use super::*;

const VERIFIED_ISSUER: &str = "https://w3id.org/sparq/vcq#verifiedIssuer";

/// Admits a flat disclosure query or lowers a single-issuer GRAPH query.
///
/// Returns the original text for ordinary admitted queries. Scoped SELECT DISTINCT
/// queries may contain only issuer declarations and GRAPH blocks with positive
/// BGP/JOIN/FILTER bodies. Graph variables must be declared, used only as graph
/// names, and all name the same issuer IRI. This is syntax admission, not issuer
/// authentication: callers must resolve that IRI in their policy, restrict proving
/// credentials, and check every authenticated public issuer slot when verifying.
/// Native result verification does not call this helper or change its contract.
///
/// # Errors
/// Preserves ordinary admission errors; different issuer IRIs are refused with
/// `per-fact-issuer-binding-needs-query-method`.
pub fn lower_issuer_scope(sparql: &str) -> Result<(String, Option<String>), PlanError> {
    let original = match DisclosureQuery::parse(sparql) {
        Ok(_) => return Ok((sparql.to_owned(), None)),
        Err(e) => e,
    };
    // The first parse bounds raw text and punctuation before this second AST walk.
    if !matches!(&original, PlanError::Unsupported(s) if s == "operator outside positive BGP/JOIN/FILTER")
    {
        return Err(original);
    }
    let mut parsed = spargebra::SparqlParser::new()
        .parse_query(sparql)
        .map_err(|e| PlanError::Parse(e.to_string()))?;
    let spargebra::Query::Select { pattern, .. } = &mut parsed else {
        return Err(original);
    };
    let GraphPattern::Distinct { inner } = pattern else {
        return Err(original);
    };
    let GraphPattern::Project { inner, variables } = inner.as_mut() else {
        return Err(original);
    };
    let mut declarations = BTreeMap::new();
    let mut blocks = Vec::new();
    let mut pending = vec![inner.as_ref()];
    while let Some(node) = pending.pop() {
        match node {
            GraphPattern::Join { left, right } => {
                pending.push(right);
                pending.push(left);
            }
            GraphPattern::Bgp { patterns } => {
                for triple in patterns {
                    let (
                        TermPattern::Variable(graph),
                        NamedNodePattern::NamedNode(predicate),
                        TermPattern::NamedNode(issuer),
                    ) = (&triple.subject, &triple.predicate, &triple.object)
                    else {
                        return Err(original);
                    };
                    if predicate.as_str() != VERIFIED_ISSUER {
                        return Err(original);
                    }
                    if declarations.values().any(|iri| iri != issuer.as_str()) {
                        return Err(unsupported("per-fact-issuer-binding-needs-query-method"));
                    }
                    declarations.insert(graph.as_str().to_owned(), issuer.as_str().to_owned());
                }
            }
            GraphPattern::Graph {
                name: NamedNodePattern::Variable(graph),
                inner,
            } => {
                blocks.push((graph.as_str().to_owned(), inner.as_ref()));
            }
            _ => return Err(original),
        }
    }
    let names = declarations.keys().cloned().collect::<BTreeSet<_>>();
    let used = blocks
        .iter()
        .map(|(name, _)| name.clone())
        .collect::<BTreeSet<_>>();
    if names != used || variables.iter().any(|v| names.contains(v.as_str())) {
        return Err(original);
    }
    let mut body = GraphPattern::Bgp {
        patterns: Vec::new(),
    };
    for (_, block) in blocks {
        let mut patterns = Vec::new();
        let bound = collect_patterns(block, &mut patterns, NumericProfile::Unsigned)
            .map_err(|_| original.clone())?;
        if patterns.is_empty() || !bound.is_disjoint(&names) {
            return Err(original);
        }
        body = GraphPattern::Join {
            left: Box::new(body),
            right: Box::new(block.clone()),
        };
    }
    **inner = body;
    let lowered = parsed.to_string();
    DisclosureQuery::parse(&lowered).map_err(|_| original)?;
    Ok((lowered, declarations.into_values().next()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scoped(body: &str) -> String {
        format!("PREFIX vcq: <https://w3id.org/sparq/vcq#> SELECT DISTINCT ?x WHERE {{ {body} }}")
    }

    #[test]
    fn lowers_only_isolated_graph_variables_for_one_issuer() {
        let query = scoped(
            "?g vcq:verifiedIssuer <urn:issuer:a> . GRAPH ?g { ?s <urn:p> ?x . FILTER(?x >= 21) }",
        );
        let (flat, issuer) = lower_issuer_scope(&query).unwrap();
        assert_eq!(issuer.as_deref(), Some("urn:issuer:a"));
        assert_eq!(
            DisclosureQuery::parse(&flat).unwrap(),
            DisclosureQuery::parse("SELECT DISTINCT ?x WHERE { ?s <urn:p> ?x . FILTER(?x >= 21) }")
                .unwrap()
        );
        let two = scoped("?g vcq:verifiedIssuer <urn:issuer:a> . ?h vcq:verifiedIssuer <urn:issuer:a> . GRAPH ?g { ?s <urn:p> ?x } GRAPH ?h { ?s <urn:q> ?y . FILTER(?y >= 18) }");
        let (flat, issuer) = lower_issuer_scope(&two).unwrap();
        assert_eq!(issuer.as_deref(), Some("urn:issuer:a"));
        assert_eq!(DisclosureQuery::parse(&flat).unwrap().patterns.len(), 2);
        assert_eq!(
            lower_issuer_scope(&two.replace(
                "?h vcq:verifiedIssuer <urn:issuer:a>",
                "?h vcq:verifiedIssuer <urn:issuer:b>"
            ))
            .unwrap_err()
            .to_string(),
            unsupported("per-fact-issuer-binding-needs-query-method").to_string()
        );
    }

    #[test]
    fn rejects_scope_escape_and_preserves_existing_admission() {
        for body in [
            "GRAPH ?g { ?s <urn:p> ?x }",
            "?g vcq:verifiedIssuer <urn:issuer:a> . GRAPH ?g { ?s <urn:p> ?x } VALUES ?z { 1 }",
            "?g vcq:verifiedIssuer <urn:issuer:a> . GRAPH ?g {} GRAPH ?g { ?s <urn:p> ?x }",
            "?g vcq:verifiedIssuer <urn:issuer:a> . GRAPH ?h { ?s <urn:p> ?x }",
            "?g vcq:verifiedIssuer <urn:issuer:a> . ?h vcq:verifiedIssuer <urn:issuer:a> . GRAPH ?g { ?s <urn:p> ?x }",
            "?g vcq:verifiedIssuer <urn:issuer:a> . GRAPH ?g { ?g <urn:p> ?x }",
            "?g vcq:verifiedIssuer <urn:issuer:a> . GRAPH ?g { ?s ?g ?x }",
            "?g vcq:verifiedIssuer <urn:issuer:a> . GRAPH ?g { ?s <urn:p> ?g . ?s <urn:q> ?x }",
            "?g vcq:verifiedIssuer <urn:issuer:a> . GRAPH ?g { ?s <urn:p> ?x . FILTER(?g >= 21) }",
            "?g vcq:verifiedIssuer <urn:issuer:a> . GRAPH ?g { ?s <urn:p> ?x } FILTER(?x >= 21)",
            "?g vcq:verifiedIssuer <urn:issuer:a> . ?s <urn:q> ?x . GRAPH ?g { ?s <urn:p> ?x }",
            "?g vcq:verifiedIssuer <urn:issuer:a> . GRAPH <urn:fixed> { ?s <urn:p> ?x }",
            "?g vcq:verifiedIssuer ?issuer . GRAPH ?g { ?s <urn:p> ?x }",
            "?g <urn:wrong> <urn:issuer:a> . GRAPH ?g { ?s <urn:p> ?x }",
            "?g vcq:verifiedIssuer <urn:issuer:a> . GRAPH ?g { ?s <urn:p> ?x OPTIONAL { ?s <urn:q> ?y } }",
            "?g vcq:verifiedIssuer <urn:issuer:a> . GRAPH ?g { GRAPH ?g { ?s <urn:p> ?x } }",
            "?g vcq:verifiedIssuer <urn:issuer:a> . GRAPH ?g { ?s <urn:p> ?x . FILTER(?y >= 21) } GRAPH ?g { ?s <urn:q> ?y }",
        ] {
            assert!(lower_issuer_scope(&scoped(body)).is_err(), "{body}");
        }
        let query = scoped("?g vcq:verifiedIssuer <urn:issuer:a> . GRAPH ?g { ?s <urn:p> ?x }");
        assert!(lower_issuer_scope(&query.replace("DISTINCT ?x", "DISTINCT ?g ?x")).is_err());
        for query in [
            "SELECT DISTINCT ?x WHERE { ?s <urn:p> ?x }",
            "ASK { ?s <urn:p> ?x }",
        ] {
            assert_eq!(lower_issuer_scope(query).unwrap(), (query.to_owned(), None));
        }
        for query in [
            query.replace("SELECT DISTINCT", "SELECT"),
            format!("{query} LIMIT 1"),
            query.replace("WHERE", "FROM <urn:data> WHERE"),
        ] {
            assert_eq!(
                lower_issuer_scope(&query).unwrap_err().to_string(),
                DisclosureQuery::parse(&query).unwrap_err().to_string()
            );
        }
    }
}
