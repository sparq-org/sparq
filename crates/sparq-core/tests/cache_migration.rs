// [GPT-6] Legacy derived caches must not override current literal semantics.
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
            "sparq-cache-migration-{}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
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
fn legacy_caches_are_recomputed_without_modifying_the_archive() {
    let graph = Graph::load_str(
        "@prefix xsd:<http://www.w3.org/2001/XMLSchema#> .
         <http://ex/a> <http://ex/n> \"1200\"^^xsd:byte; <http://ex/t> \"2023-02-29T00:00:00Z\"^^xsd:dateTime .
         <http://ex/b> <http://ex/n> \"127\"^^xsd:byte; <http://ex/t> \"2024-02-29T00:00:00Z\"^^xsd:dateTime .",
        "turtle",
    ).unwrap();
    let bad_num = graph
        .id_of(&Term::Literal(Literal::new_typed_literal(
            "1200",
            xsd::BYTE,
        )))
        .unwrap();
    let bad_time = graph
        .id_of(&Term::Literal(Literal::new_typed_literal(
            "2023-02-29T00:00:00Z",
            xsd::DATE_TIME,
        )))
        .unwrap();
    let expected = dump(&graph);
    for compressed in [false, true] {
        // Independently present new files model interrupted/partial migration.
        for current_mask in 0..4 {
            let scratch = Scratch::new();
            let archive = scratch.0.join("old");
            if compressed {
                graph.save_compressed(&archive).unwrap();
            } else {
                graph.save(&archive).unwrap();
            }
            let mut numeric = std::fs::read(archive.join("numerics-v2.bin"))
                .or_else(|_| std::fs::read(archive.join("numerics.bin")))
                .unwrap();
            let mut temporal = std::fs::read(archive.join("temporals-v2.bin"))
                .or_else(|_| std::fs::read(archive.join("temporals.bin")))
                .unwrap();
            let valid_numeric = numeric.clone();
            let valid_temporal = temporal.clone();
            // Exact legacy layouts: n little-endian f64 cells; temporal flags
            // follow those cells. These reproduce pre-validation accepted values.
            let ni = (bad_num as usize - 1) * 8;
            numeric[ni..ni + 8].copy_from_slice(&1200_f64.to_le_bytes());
            let ti = (bad_time as usize - 1) * 8;
            let old_instant =
                sparq_core::temporal::Timeline::parse_datetime("2023-03-01T00:00:00Z")
                    .unwrap()
                    .instant();
            temporal[ti..ti + 8].copy_from_slice(&old_instant.to_le_bytes());
            temporal[graph.dict.len() * 8 + bad_time as usize - 1] = 2; // zoned dateTime
            std::fs::write(archive.join("numerics.bin"), numeric).unwrap();
            std::fs::write(archive.join("temporals.bin"), temporal).unwrap();
            for (bit, name, bytes) in [
                (1, "numerics-v2.bin", valid_numeric),
                (2, "temporals-v2.bin", valid_temporal),
            ] {
                if current_mask & bit == 0 {
                    let _ = std::fs::remove_file(archive.join(name));
                } else {
                    std::fs::write(archive.join(name), bytes).unwrap();
                }
            }
            let before = files(&archive);
            let opened = Graph::open(&archive).unwrap();
            assert_eq!(
                opened.numeric_value(bad_num),
                None,
                "compressed={compressed}, mask={current_mask}"
            );
            assert!(
                opened.temporal_value(bad_time).is_none(),
                "compressed={compressed}, mask={current_mask}"
            );
            assert_eq!(
                dump(&opened),
                expected,
                "ill-typed RDF literals must remain stored"
            );
            for id in 1..=graph.dict.len() as u32 {
                assert_eq!(opened.numeric_value(id), graph.numeric_value(id));
                assert_eq!(
                    opened
                        .temporal_value(id)
                        .map(|t| (t.kind, t.has_tz, t.instant.to_bits())),
                    graph
                        .temporal_value(id)
                        .map(|t| (t.kind, t.has_tz, t.instant.to_bits()))
                );
            }
            let after = files(&archive);
            for (name, bytes) in before {
                assert_eq!(
                    after.get(&name),
                    Some(&bytes),
                    "opening rewrote existing {name}"
                );
            }
            let saved = scratch.0.join("current");
            opened.save(&saved).unwrap();
            assert!(saved.join("numerics-v2.bin").is_file());
            assert!(saved.join("temporals-v2.bin").is_file());
            assert!(!saved.join("numerics.bin").exists());
            assert!(!saved.join("temporals.bin").exists());
            let reopened = Graph::open(&saved).unwrap();
            assert_eq!(dump(&reopened), expected);
            assert_eq!(reopened.numeric_value(bad_num), None);
            assert!(reopened.temporal_value(bad_time).is_none());
        }
    }
}
