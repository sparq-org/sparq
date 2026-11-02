/// Backend-taking entries rely on construction for the party ceiling. A repeated
/// check is unreachable at runtime, so pin this boundary in the entry sources.
#[test]
fn backend_entries_rely_on_constructor_party_cap() {
    for (module, source) in [
        ("oblivious_join", include_str!("../src/oblivious_join.rs")),
        ("join", include_str!("../src/join.rs")),
        ("hidden_path", include_str!("../src/hidden_path.rs")),
        ("hidden_distinct", include_str!("../src/hidden_distinct.rs")),
    ] {
        for (line, text) in source.lines().enumerate() {
            let code = text.split("//").next().unwrap();
            for cap in [
                "MAX_ROBUST_SHARES",
                "MAX_OUTPUT_PARTIES",
                "check_output_parties_cap",
            ] {
                assert!(
                    !code.contains(cap),
                    "{module}:{} repeats the constructor party cap: {code}",
                    line + 1
                );
            }
        }
    }
}
