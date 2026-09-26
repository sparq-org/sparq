// [GPT-6] Recompute stale temporal values without rewriting the RDF archive.
#![cfg(feature = "mmap")]

use oxrdf::{Literal, Term, vocab::xsd};
use sparq_core::Graph;
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

struct Scratch(PathBuf);
impl Scratch {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "sparq-temporal-lexical-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn files(dir: &Path) -> BTreeMap<String, Vec<u8>> {
    std::fs::read_dir(dir)
        .unwrap()
        .map(|entry| {
            let entry = entry.unwrap();
            (
                entry.file_name().into_string().unwrap(),
                std::fs::read(entry.path()).unwrap(),
            )
        })
        .collect()
}
fn dump(graph: &Graph) -> Vec<[String; 3]> {
    let mut rows: Vec<_> = graph
        .iter_ids()
        .map(|row| row.map(|id| graph.dict.term(id).to_string()))
        .collect();
    rows.sort();
    rows
}

#[test]
fn stale_temporal_v2_cells_never_override_strict_lexical_validity() {
    let graph = Graph::load_str(
        r#"@prefix xsd:<http://www.w3.org/2001/XMLSchema#> .
        <http://ex/a> <http://ex/p> " 2024-02-29T00:00:00Z "^^xsd:dateTime .
        <http://ex/b> <http://ex/p> "2024-02-29T00:00:00"^^xsd:dateTimeStamp .
        <http://ex/c> <http://ex/p> " 2024-02-29Z "^^xsd:date .
        <http://ex/d> <http://ex/p> "2024-02-29T00:00:00.000000001Z"^^xsd:dateTime ."#,
        "turtle",
    )
    .unwrap();
    let bad_ids = [
        (" 2024-02-29T00:00:00Z ", xsd::DATE_TIME),
        ("2024-02-29T00:00:00", xsd::DATE_TIME_STAMP),
        (" 2024-02-29Z ", xsd::DATE),
    ]
    .map(|(lex, datatype)| {
        graph
            .id_of(&Term::Literal(Literal::new_typed_literal(lex, datatype)))
            .unwrap()
    });
    let good_id = graph
        .id_of(&Term::Literal(Literal::new_typed_literal(
            "2024-02-29T00:00:00.000000001Z",
            xsd::DATE_TIME,
        )))
        .unwrap();
    let expected = dump(&graph);
    for compressed in [false, true] {
        for current_present in [false, true] {
            let scratch = Scratch::new();
            let archive = scratch.0.join("old");
            if compressed {
                graph.save_compressed(&archive).unwrap();
            } else {
                graph.save(&archive).unwrap();
            }
            // Build the exact former n*f64 + n*flag layout, including stale valid cells.
            let current = std::fs::read(archive.join("temporals-v3.bin"))
                .or_else(|_| std::fs::read(archive.join("temporals-v2.bin")))
                .unwrap();
            let mut stale = current.clone();
            for (index, id) in bad_ids.into_iter().enumerate() {
                let offset = (id as usize - 1) * 8;
                stale[offset..offset + 8].copy_from_slice(&1_709_164_800_f64.to_le_bytes());
                stale[graph.dict.len() * 8 + id as usize - 1] = if index == 2 {
                    4
                } else if index == 1 {
                    1
                } else {
                    2
                };
            }
            std::fs::write(archive.join("temporals-v2.bin"), &stale).unwrap();
            std::fs::write(archive.join("temporals.bin"), &stale).unwrap();
            if current_present {
                std::fs::write(archive.join("temporals-v3.bin"), &current).unwrap();
            } else {
                let _ = std::fs::remove_file(archive.join("temporals-v3.bin"));
            }
            let before = files(&archive);
            let opened = Graph::open(&archive).unwrap();
            for id in bad_ids {
                assert!(
                    opened.temporal_value(id).is_none(),
                    "stale approximate id={id}, compressed={compressed}, current={current_present}"
                );
                assert!(
                    opened.exact_temporal_value(id).is_none(),
                    "stale exact id={id}"
                );
            }
            assert!(opened.temporal_value(good_id).is_some());
            let exact = opened.exact_temporal_value(good_id).unwrap();
            let zero = sparq_core::temporal::ExactTemporal::of_lit(
                "2024-02-29T00:00:00Z",
                xsd::DATE_TIME.as_str(),
            )
            .unwrap();
            assert_eq!(exact.compare(zero), Some(std::cmp::Ordering::Greater));
            assert_eq!(dump(&opened), expected, "ill-typed RDF terms remain stored");
            let after = files(&archive);
            for (name, bytes) in before {
                assert_eq!(after.get(&name), Some(&bytes), "rewrote {name}");
            }
            let saved = scratch.0.join("new");
            opened.save(&saved).unwrap();
            assert!(saved.join("temporals-v3.bin").is_file());
            assert!(!saved.join("temporals-v2.bin").exists());
            assert!(!saved.join("temporals.bin").exists());
            let reopened = Graph::open(&saved).unwrap();
            assert_eq!(dump(&reopened), expected);
            for id in bad_ids {
                assert!(reopened.temporal_value(id).is_none());
            }
        }
    }
}
