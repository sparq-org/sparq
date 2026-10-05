// [GPT-6] Manual dialect-discriminating results through the actual suite runner.
use sparq_conformance::manifest::{EntryKind, QueryAction, TestEntry, UpdateState};
use sparq_conformance::run::{run_query_test, Status};

#[test]
fn suite_dialect_reaches_select_ask_and_construct_without_golden_relaxation() {
    let root = std::env::temp_dir().join(format!("sparq-ebv-suite-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let xml = |body: &str| {
        format!(
            "<?xml version=\"1.0\"?><sparql xmlns=\"http://www.w3.org/2005/sparql-results#\">{body}</sparql>"
        )
    };
    let binding = "<binding name=\"v\"><literal datatype=\"http://www.w3.org/2001/XMLSchema#boolean\">false</literal></binding>";
    let invalid = "\"z\"^^<http://www.w3.org/2001/XMLSchema#boolean>";
    for (form, query, extension, rec, draft) in [
        (
            "select",
            format!("SELECT (!!{invalid} AS ?v) {{}}"),
            "srx",
            xml(&format!(
                "<head><variable name=\"v\"/></head><results><result>{binding}</result></results>"
            )),
            xml("<head><variable name=\"v\"/></head><results><result></result></results>"),
        ),
        (
            "ask",
            format!("ASK {{FILTER(!{invalid})}}"),
            "srx",
            xml("<head/><boolean>true</boolean>"),
            xml("<head/><boolean>false</boolean>"),
        ),
        (
            "construct",
            format!(
                "CONSTRUCT {{<http://ex/s> <http://ex/p> <http://ex/o>}} WHERE {{FILTER(!{invalid})}}"
            ),
            "ttl",
            "<http://ex/s> <http://ex/p> <http://ex/o> .".into(),
            String::new(),
        ),
    ] {
        let query_path = root.join(format!("{form}.rq"));
        let result_path = root.join(format!("{form}.{extension}"));
        std::fs::write(&query_path, &query).unwrap();
        let mut entry = TestEntry {
            id: format!("urn:manual:{form}"),
            name: form.into(),
            suite: "sparql11/manual-ebv".into(),
            kind: EntryKind::QueryEval,
            withdrawn: false,
            action: QueryAction {
                query: Some(query_path.clone()),
                ..Default::default()
            },
            result_file: Some(result_path.clone()),
            update_request: None,
            update_pre: UpdateState::default(),
            update_post: UpdateState::default(),
        };
        std::fs::write(&result_path, &rec).unwrap();
        assert!(matches!(run_query_test(&entry), Status::Pass), "REC {form}");
        entry.suite = "sparql12/manual-ebv".into();
        assert!(
            matches!(run_query_test(&entry), Status::Fail(_)),
            "WD must reject the REC golden for {form}"
        );
        std::fs::write(&result_path, &draft).unwrap();
        let outcome = run_query_test(&entry);
        assert!(matches!(outcome, Status::Pass), "WD {form}: {outcome:?}");
        std::fs::write(&query_path, format!("VERSION '1.1' {query}")).unwrap();
        assert!(
            matches!(run_query_test(&entry), Status::Fail(message) if message.contains("contradicts")),
            "suite/declaration conflict {form}"
        );
    }
    std::fs::remove_dir_all(root).unwrap();
}
