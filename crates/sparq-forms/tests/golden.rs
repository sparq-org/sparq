//! Golden-file FormDescription tests: each fixture pair
//! `tests/fixtures/<name>.{data,shapes}.ttl` derives a form whose pretty JSON
//! must equal `tests/fixtures/<name>.golden.json` byte-for-byte. Regenerate
//! intentionally with `UPDATE_GOLDENS=1 cargo test -p sparq-forms --test golden`
//! and REVIEW the diff — the goldens are the crate's behavioural contract.
//! [FABLE-5] sq-lsp7k.1.1

use oxrdf::{NamedNode, Term};
use sparq_core::Graph;
use sparq_forms::{derive_form, FormOptions};
use std::path::PathBuf;

fn check(name: &str, focus: &str, opts: &FormOptions) {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let load = |suffix: &str| {
        let path = dir.join(format!("{name}.{suffix}.ttl"));
        let ttl = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path:?}: {e}"));
        Graph::load_str(&ttl, "turtle").unwrap_or_else(|e| panic!("{path:?}: {e}"))
    };
    let data = load("data");
    let shapes = load("shapes");
    let focus = Term::from(NamedNode::new_unchecked(format!("http://example.org/{focus}")));
    let form = derive_form(&data, &shapes, &focus, opts);
    let mut json = serde_json::to_string_pretty(&form).expect("FormDescription serializes");
    json.push('\n');

    let golden_path = dir.join(format!("{name}.golden.json"));
    if std::env::var_os("UPDATE_GOLDENS").is_some() {
        std::fs::write(&golden_path, &json).unwrap();
        return;
    }
    let golden = std::fs::read_to_string(&golden_path)
        .unwrap_or_else(|e| panic!("{golden_path:?}: {e} (run with UPDATE_GOLDENS=1 to seed)"));
    assert_eq!(
        json, golden,
        "{name}: derived FormDescription drifted from {golden_path:?} — if intentional, \
         regenerate with UPDATE_GOLDENS=1 and review the diff"
    );
}

/// Property groups + fractional sh:order + names/descriptions + cardinalities
/// + boolean/date/langString widgets + dash:singleLine + sh:deactivated.
#[test]
fn golden_person_groups() {
    check("person_groups", "eve", &FormOptions::default());
}

/// sh:in enum + sh:class + nested sh:node sub-form + dash:rootClass +
/// rdf:HTML + explicit dash:viewer.
#[test]
fn golden_enum_nested() {
    check("enum_nested", "order42", &FormOptions::default());
}

/// Multi-shape switcher (sh:targetClass + dash:applicableToClass) +
/// sh:inversePath incoming references + off-shape read-only Other group.
#[test]
fn golden_inverse_multi() {
    check("inverse_multi", "apollo", &FormOptions::default());
}

/// [OPUS-4.8] sq-vfcxv: predicate-target applicability — sh:targetSubjectsOf
/// and sh:targetObjectsOf join the switcher, ranked below
/// dash:applicableToClass (which therefore stays the selected shape), and a
/// predicate target whose predicate the focus node does not carry stays out.
#[test]
fn golden_predicate_targets() {
    check("predicate_targets", "alice", &FormOptions::default());
}

/// #6284: every fixture source names its generated golden companion and the
/// regeneration command, so a change scoped to one source file still sees the
/// two-file rule (editing a source's content moves the golden JSON).
#[test]
fn every_fixture_source_names_its_golden() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let mut checked = 0;
    for entry in std::fs::read_dir(&dir).unwrap() {
        let path = entry.unwrap().path();
        let file = path.file_name().unwrap().to_str().unwrap().to_string();
        let Some(name) = file
            .strip_suffix(".data.ttl")
            .or_else(|| file.strip_suffix(".shapes.ttl"))
        else {
            continue;
        };
        let golden = format!("{name}.golden.json");
        assert!(
            dir.join(&golden).exists(),
            "{file}: missing companion {golden}"
        );
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(
            text.contains(&format!("fixtures/{golden}")),
            "{file} must name its golden companion fixtures/{golden}"
        );
        assert!(
            text.contains("UPDATE_GOLDENS=1 cargo test -p sparq-forms --test golden"),
            "{file} must state the golden regeneration command"
        );
        checked += 1;
    }
    assert!(
        checked >= 8,
        "expected the 8 fixture sources, checked {checked}"
    );
}
