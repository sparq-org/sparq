//! sq-bif.17: fail-closed coverage for every strict prefix of a valid
//! in-memory HDT archive through both public reader entry points.

use std::io::{Cursor, ErrorKind};

const TRIPLE_COUNT: usize = 80;

fn valid_hdt_bytes() -> Vec<u8> {
    let mut nt = String::new();
    for index in 0..TRIPLE_COUNT {
        nt.push_str(&format!(
            "<http://example.org/subject/{index}> <http://example.org/predicate> <http://example.org/object/{index}> .\n"
        ));
    }
    let dir = tempfile::tempdir().expect("creating scratch directory");
    let nt_path = dir.path().join("source.nt");
    std::fs::write(&nt_path, nt).expect("writing N-Triples fixture");
    let hdt = hdt::Hdt::read_nt(&nt_path).expect("building HDT fixture");
    let mut bytes = Vec::new();
    hdt.write(&mut bytes).expect("serializing HDT fixture");
    bytes
}

fn is_bitmap_body_truncation(error: &sparq_hdt::Error) -> bool {
    matches!(
        error,
        sparq_hdt::Error::Io(io)
            if io.kind() == ErrorKind::UnexpectedEof
                && io.to_string().contains("HDT bitmap body truncated")
    )
}

#[test]
fn every_truncated_prefix_is_rejected_without_panicking() {
    let bytes = valid_hdt_bytes();
    assert_eq!(
        sparq_hdt::load_reader(Cursor::new(bytes.clone()))
            .expect("valid archive must load")
            .len(),
        TRIPLE_COUNT
    );
    assert_eq!(
        sparq_hdt::stats_reader(Cursor::new(bytes.clone()))
            .expect("valid archive stats must load")
            .triples,
        TRIPLE_COUNT
    );

    let mut load_bitmap_body_prefix = None;
    let mut stats_bitmap_body_prefix = None;

    for prefix_len in 0..bytes.len() {
        let prefix = bytes[..prefix_len].to_vec();
        let load = std::panic::catch_unwind(|| sparq_hdt::load_reader(Cursor::new(prefix)));
        match load {
            Ok(Err(error)) => {
                if is_bitmap_body_truncation(&error) {
                    load_bitmap_body_prefix.get_or_insert(prefix_len);
                }
            }
            Ok(Ok(graph)) => panic!(
                "load_reader accepted truncated prefix {prefix_len}/{} with {} triples",
                bytes.len(),
                graph.len()
            ),
            Err(_) => panic!(
                "load_reader panicked on truncated prefix {prefix_len}/{}",
                bytes.len()
            ),
        }

        let prefix = bytes[..prefix_len].to_vec();
        let stats = std::panic::catch_unwind(|| sparq_hdt::stats_reader(Cursor::new(prefix)));
        match stats {
            Ok(Err(error)) => {
                if is_bitmap_body_truncation(&error) {
                    stats_bitmap_body_prefix.get_or_insert(prefix_len);
                }
            }
            Ok(Ok(stats)) => panic!(
                "stats_reader accepted truncated prefix {prefix_len}/{} with {} triples",
                bytes.len(),
                stats.triples
            ),
            Err(_) => panic!(
                "stats_reader panicked on truncated prefix {prefix_len}/{}",
                bytes.len()
            ),
        }
    }

    assert!(
        load_bitmap_body_prefix.is_some(),
        "load_reader must expose the pinned UnexpectedEof bitmap-body error"
    );
    assert_eq!(
        stats_bitmap_body_prefix, load_bitmap_body_prefix,
        "load_reader and stats_reader must reject the same bitmap-body truncation"
    );
}

#[test]
fn empty_input_errors_cleanly_for_both_readers() {
    let load = std::panic::catch_unwind(|| sparq_hdt::load_reader(Cursor::new(Vec::<u8>::new())));
    assert!(matches!(load, Ok(Err(_))), "load_reader must return Err");

    let stats = std::panic::catch_unwind(|| sparq_hdt::stats_reader(Cursor::new(Vec::<u8>::new())));
    assert!(matches!(stats, Ok(Err(_))), "stats_reader must return Err");
}
