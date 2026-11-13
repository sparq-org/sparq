use super::*;

// Frozen pre-bounding query: keep this independent of the production record builder.
fn legacy_query(target: &str, candidates: &[String]) -> String {
    let mut names = vec![target.to_string()];
    for candidate in candidates {
        if !names.contains(candidate) {
            names.push(candidate.clone());
        }
    }
    format!(
        "SELECT ?g ?ct ?bk ?etag ?mod ?appendSeq WHERE {{ VALUES ?g {{ {} }}
         GRAPH ?g {{ <urn:pss:index#record> <urn:pss:index#contentType> ?ct ;
           <urn:pss:index#blobKey> ?bk ; <urn:pss:index#etag> ?etag .
           OPTIONAL {{ <urn:pss:index#record> <urn:pss:index#modified> ?mod }}
           OPTIONAL {{ <urn:pss:index#record> <urn:pss:index#appendSeq> ?appendSeq }} }} }}",
        names
            .iter()
            .map(|name| sparql::iri(name).unwrap())
            .collect::<Vec<_>>()
            .join(" "),
    )
}

fn legacy_plan(graph: &Graph, target: &str, candidates: &[String]) -> Result<ReadPlan, String> {
    let result = sparq_engine::query(graph, &legacy_query(target, candidates)).unwrap();
    let mut records = std::collections::HashMap::new();
    for row in &result.rows {
        // Freeze the old cell extraction too: sharing the production extractor would
        // let a required-field decoding regression change both sides of the oracle.
        let value = |name| {
            let column = result.vars.iter().position(|v| v.as_str() == name).unwrap();
            match row[column].as_ref()? {
                oxrdf::Term::Literal(l) => Some(l.value().to_string()),
                oxrdf::Term::NamedNode(n) => Some(n.as_str().to_string()),
                oxrdf::Term::BlankNode(b) => Some(b.as_str().to_string()),
                _ => None,
            }
        };
        let required = |name, field| {
            value(name)
                .ok_or_else(|| format!("sparq backend error: fatal: read-plan row missing {field}"))
        };
        let name = required("g", "the ?g binding")?;
        let metadata = ResourceMeta {
            content_type: required("ct", "contentType")?,
            blob_key: required("bk", "blobKey")?,
            etag: required("etag", "etag")?,
            last_modified: value("mod").and_then(|s| timestamp::from_xsd_datetime(&s)),
            append_seq: value("appendSeq")
                .map(|s| s.parse())
                .transpose()
                .map_err(|_| "sparq backend error: invalid append sequence".to_string())?
                .unwrap_or(0),
        };
        records.entry(name).or_insert(metadata);
    }
    Ok(ReadPlan {
        target: records.get(target).cloned(),
        acls: candidates
            .iter()
            .map(|c| (c.clone(), records.get(c).map(|m| m.etag.clone())))
            .collect(),
    })
}

fn both_query_forms(candidates: Vec<String>) -> [Vec<String>; 2] {
    let mut wide = candidates.clone();
    wide.extend(
        (0..=sparql::READ_PLAN_CONSTANT_CANDIDATES)
            .map(|i| format!("https://pod.example/unindexed/{i}.acl")),
    );
    [candidates, wide]
}

fn fixture() -> (Graph, Vec<String>) {
    let mut graph = Graph::new();
    let mut names = vec!["https://pod.example/absent".to_string()];
    let mut insert = String::from("INSERT DATA {");
    // Every subset of the required fields, including a present graph with no record.
    for mask in 0..8 {
        let name = format!("https://pod.example/fields-{mask}");
        names.push(name.clone());
        insert.push_str(&format!("GRAPH <{name}> {{ <urn:other> <urn:p> <urn:o> . "));
        for (bit, predicate, value) in [
            (1, "contentType", "text/turtle"),
            (2, "blobKey", "blob"),
            (4, "etag", "tag"),
        ] {
            if mask & bit != 0 {
                insert.push_str(&format!(
                    "<urn:pss:index#record> <urn:pss:index#{predicate}> \"{value}\" . "
                ));
            }
        }
        insert.push_str("} ");
    }
    let variants = [
        "<urn:pss:index#modified> \"2026-01-01T00:00:00Z\" ; <urn:pss:index#appendSeq> 7",
        "<urn:pss:index#modified> \"invalid-date\"",
        "<urn:pss:index#appendSeq> \"invalid-sequence\"",
        "<urn:pss:index#appendSeq> -1",
        "<urn:pss:index#appendSeq> 0, \"invalid-sequence\"",
        "<urn:pss:index#etag> \"another-tag\"",
        "<urn:pss:index#contentType> <<( <urn:s> <urn:p> <urn:o> )>>",
        "<urn:pss:index#blobKey> <<( <urn:s> <urn:p> <urn:o> )>>",
        "<urn:pss:index#etag> <<( <urn:s> <urn:p> <urn:o> )>>",
    ];
    for (i, extra) in variants.iter().enumerate() {
        let name = format!("https://pod.example/variant-{i}");
        names.push(name.clone());
        insert.push_str(&format!(
            "GRAPH <{name}> {{ <urn:pss:index#record> <urn:pss:index#contentType> \"text/plain\" ;
             <urn:pss:index#blobKey> \"variant-blob\" ; <urn:pss:index#etag> \"variant-tag\" ; {extra} . }} "
        ));
    }
    insert.push('}');
    sparq_engine::update_in_place(&mut graph, &insert).unwrap();
    (graph, names)
}

#[tokio::test]
async fn read_plan_matches_legacy_query_for_partial_and_malformed_records() {
    let (graph, names) = fixture();
    let client = EmbeddedSparqClient::over(graph).unwrap();
    // Keep a single malformed candidate per walk so exact error diagnostics can be
    // compared: SPARQL leaves the row order between competing malformed graphs unspecified.
    for (i, target) in names.iter().enumerate() {
        // Each malformed record is exercised as a target and as an ACL after a valid ACL.
        // The rotated walk preserves nonlexical nearest-first order, duplicate candidates,
        // self-candidates, absent records, and an empty candidate list.
        for candidates in [
            Vec::new(),
            vec![
                names[8].clone(),
                target.clone(),
                names[0].clone(),
                target.clone(),
            ],
            std::iter::once(target.clone())
                .chain((0..5).map(|j| names[(i + j * 3) % 11].clone()))
                .collect(),
        ]
        .into_iter()
        .flat_map(both_query_forms)
        {
            let old_target = target.clone();
            let old_candidates = candidates.clone();
            let expected = client
                .actor
                .call(move |g| Ok(legacy_plan(g, &old_target, &old_candidates)))
                .await
                .unwrap();
            let before = client.engine_round_trips();
            let actual = client
                .read_plan(target, &candidates)
                .await
                .map_err(|e| e.to_string());
            assert_eq!(
                actual, expected,
                "legacy parity for {target}, {candidates:?}"
            );
            assert_eq!(client.engine_round_trips() - before, 1, "one snapshot job");
        }
    }

    // Vary graph insertion, value insertion and walk order. Repeated
    // field values exercise first-row retention after the legacy VALUES join.
    for seed in 0usize..12 {
        let mut graph = Graph::new();
        let mut names = Vec::new();
        for j in 0..6 {
            let i = (j + seed) % 6;
            let name = format!("https://pod.example/generated-{i}");
            names.push(name.clone());
            let values = if (seed + j).is_multiple_of(2) {
                "\"first\", \"second\", \"third\""
            } else {
                "\"third\", \"second\", \"first\""
            };
            sparq_engine::update_in_place(
                &mut graph,
                &format!(
                    "INSERT DATA {{ GRAPH <{name}> {{ <urn:pss:index#record>
                     <urn:pss:index#contentType> {values} ;
                     <urn:pss:index#blobKey> {values} ; <urn:pss:index#etag> {values} ;
                     <urn:pss:index#modified> \"invalid\", \"2026-01-01T00:00:00Z\" ;
                     <urn:pss:index#appendSeq> 0, 7 . }} }}"
                ),
            )
            .unwrap();
        }
        let target = names[seed % names.len()].clone();
        names.reverse();
        names.push(target.clone());
        names.push("https://pod.example/absent".to_string());
        let client = EmbeddedSparqClient::over(graph).unwrap();
        for candidates in both_query_forms(names) {
            let old_target = target.clone();
            let old_candidates = candidates.clone();
            let expected = client
                .actor
                .call(move |g| Ok(legacy_plan(g, &old_target, &old_candidates)))
                .await
                .unwrap();
            assert_eq!(
                client
                    .read_plan(&target, &candidates)
                    .await
                    .map_err(|e| e.to_string()),
                expected,
                "multi-valued legacy parity for seed {seed}"
            );
        }
    }
}

// Count actual metadata BGP evaluations from the existing engine execution trace. Query
// capture is test-only and records the exact query used by the public read_plan call.
async fn metadata_evaluations(client: &EmbeddedSparqClient) -> usize {
    client
        .actor
        .call(|g| {
            let trace =
                sparq_engine::explain_analyze(g, g.read_plan_query.as_ref().unwrap()).unwrap();
            Ok(trace
                .split("Execution trace (operator → output rows, wall time):")
                .nth(1)
                .unwrap()
                .lines()
                .filter(|line| {
                    line.trim_start().starts_with("BGP ") && !line.contains("(0 patterns")
                })
                .count())
        })
        .await
        .unwrap()
}

#[tokio::test]
async fn read_plan_evaluates_only_distinct_requested_graphs() {
    let (graph, names) = fixture();
    let client = EmbeddedSparqClient::over(graph).unwrap();
    let target = &names[8];
    let candidates = vec![
        target.clone(),
        names[9].clone(),
        names[0].clone(),
        names[9].clone(),
    ];
    let plan = client.read_plan(target, &candidates).await.unwrap();
    let evaluations = metadata_evaluations(&client).await;
    assert_eq!(
        evaluations, 9,
        "each distinct requested graph evaluates its required and optional metadata patterns once"
    );
    // Add unrelated records, including one that the read-plan parser would reject.
    client
        .actor
        .call(|g| {
            for i in 0..12 {
                sparq_engine::update_in_place(
                    g,
                    &format!(
                        "INSERT DATA {{ GRAPH <https://unrelated.example/{i}> {{
                 <urn:pss:index#record> <urn:pss:index#contentType> \"text/plain\" ;
                 <urn:pss:index#blobKey> \"unrelated\" ; <urn:pss:index#etag> \"unrelated\" ;
                 <urn:pss:index#appendSeq> \"invalid\" . }} }}"
                    ),
                )
                .unwrap();
            }
            Ok(())
        })
        .await
        .unwrap();
    assert_eq!(client.read_plan(target, &candidates).await.unwrap(), plan);
    assert_eq!(
        metadata_evaluations(&client).await,
        evaluations,
        "unrelated graphs must not add metadata evaluations"
    );
}

#[tokio::test]
async fn read_plan_long_paths_match_legacy_query() {
    let boundary = sparql::READ_PLAN_CONSTANT_CANDIDATES;
    for count in [boundary - 1, boundary, boundary + 1, 4096] {
        let mut path = String::from("https://pod.example/");
        let mut candidates = Vec::new();
        for _ in 0..count {
            path.push_str("a/");
            candidates.push(format!("{path}.acl"));
        }
        candidates.reverse();
        let target = format!("{path}doc");
        let client = EmbeddedSparqClient::in_memory().unwrap();
        if count != 4096 {
            let metadata = ResourceMeta {
                content_type: "text/turtle".into(),
                blob_key: "target-blob".into(),
                etag: "target-tag".into(),
                last_modified: None,
                append_seq: 0,
            };
            client.put_meta(&target, metadata.clone()).await.unwrap();
            client
                .put_meta(candidates.last().unwrap(), metadata)
                .await
                .unwrap();
        }
        let old_target = target.clone();
        let old_candidates = candidates.clone();
        let expected = client
            .actor
            .call(move |g| Ok(legacy_plan(g, &old_target, &old_candidates)))
            .await
            .unwrap();
        let before = client.engine_round_trips();
        assert_eq!(
            client
                .read_plan(&target, &candidates)
                .await
                .map_err(|e| e.to_string()),
            expected,
            "long-path legacy parity at candidate count {count}"
        );
        assert_eq!(client.engine_round_trips() - before, 1, "one snapshot job");
        let used_values = client
            .actor
            .call(|g| Ok(g.read_plan_query.as_ref().unwrap().contains("VALUES ?g {")))
            .await
            .unwrap();
        assert_eq!(
            used_values,
            count > boundary,
            "long-path query form at candidate count {count}"
        );
    }
}
