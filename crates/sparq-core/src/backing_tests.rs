use super::*;
use crate::Graph;
use std::collections::BTreeMap;
use std::path::PathBuf;

struct Directory(PathBuf);
impl Directory {
    fn new() -> Self {
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        Self::create(|| {
            std::env::temp_dir().join(format!(
                "sparq-backing-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            ))
        })
    }

    fn create(mut candidate: impl FnMut() -> PathBuf) -> Self {
        loop {
            let path = candidate();
            match std::fs::create_dir(&path) {
                Ok(()) => return Self(path),
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(error) => panic!("cannot create {}: {error}", path.display()),
            }
        }
    }
}
impl Drop for Directory {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).unwrap();
    }
}

#[test]
fn scratch_paths_do_not_reuse_existing_directories() {
    let root = Directory::new();
    let existing = root.0.join("existing");
    std::fs::create_dir(&existing).unwrap();
    std::fs::write(existing.join("sentinel"), "keep").unwrap();
    let fresh = root.0.join("fresh");
    let mut candidates = [existing.clone(), fresh.clone()].into_iter();
    let directory = Directory::create(|| candidates.next().expect("no fresh path was used"));
    assert_eq!(directory.0, fresh);
    assert_eq!(
        std::fs::read_to_string(existing.join("sentinel")).unwrap(),
        "keep"
    );
    drop(directory);
    assert!(existing.is_dir(), "existing directory was removed");
}

#[test]
#[cfg(not(miri))]
fn threshold_boundaries_and_empty() {
    let dir = Directory::new();
    let path = dir.0.join("bytes");
    for len in [
        0,
        SMALL_FILE_BYTES - 1,
        SMALL_FILE_BYTES,
        SMALL_FILE_BYTES + 1,
    ] {
        std::fs::write(&path, vec![42; len]).unwrap();
        let backing = with_threshold(SMALL_FILE_BYTES, || Backing::open(&path).unwrap());
        assert_eq!(
            matches!(backing, Backing::Owned(_)),
            len <= SMALL_FILE_BYTES,
            "wrong backing at boundary {len}"
        );
        assert_eq!(&*backing, vec![42; len]);
    }
}

#[test]
fn changed_size_is_bounded_and_refused() {
    use std::io::Seek;
    let dir = Directory::new();
    let path = dir.0.join("bytes");
    for actual in [0, 8, 17, SMALL_FILE_BYTES, SMALL_FILE_BYTES * 16] {
        std::fs::write(&path, [1; 16]).unwrap();
        let file = File::open(&path).unwrap();
        let len = file.metadata().unwrap().len();
        std::fs::OpenOptions::new()
            .write(true)
            .open(&path)
            .unwrap()
            .set_len(actual as u64)
            .unwrap();
        let mut observer = file.try_clone().unwrap();
        let error = Backing::from_file(file, len, SMALL_FILE_BYTES)
            .err()
            .expect("changed-size file was accepted");
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert_eq!(
            observer.stream_position().unwrap(),
            actual.min(SMALL_FILE_BYTES + 1) as u64,
            "read exceeded its bound"
        );
    }
}

#[test]
fn mapping_failure_names_limit() {
    let error = mapping_error(io::Error::from(io::ErrorKind::OutOfMemory));
    assert_eq!(error.kind(), io::ErrorKind::Other);
    assert!(error.to_string().contains("vm.max_map_count"));
    assert!(error.get_ref().unwrap().is::<MappingLimit>());
    let other = mapping_error(io::Error::from(io::ErrorKind::PermissionDenied));
    assert_eq!(other.kind(), io::ErrorKind::PermissionDenied);
}

fn sample() -> Graph {
    Graph::load_str(
        r#"<urn:s> <urn:p> "42.25"^^<http://www.w3.org/2001/XMLSchema#double>;
        <urn:date> "2026-11-13T08:00:00Z"^^<http://www.w3.org/2001/XMLSchema#dateTime>;
        <urn:text> "label"@en ."#,
        "ttl",
    )
    .unwrap()
}

fn tree(path: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    fn walk(root: &Path, path: &Path, result: &mut BTreeMap<PathBuf, Vec<u8>>) {
        for entry in std::fs::read_dir(path).unwrap() {
            let entry = entry.unwrap();
            if entry.file_type().unwrap().is_dir() {
                walk(root, &entry.path(), result);
            } else {
                result.insert(
                    entry.path().strip_prefix(root).unwrap().to_owned(),
                    std::fs::read(entry.path()).unwrap(),
                );
            }
        }
    }
    let mut result = BTreeMap::new();
    walk(path, path, &mut result);
    result
}

fn exercise(graph: &Graph) -> Vec<[String; 3]> {
    let rows: Vec<_> = graph.iter_ids().collect();
    for row in &rows {
        assert!(graph.store.contains(*row));
        for mask in 0..8 {
            let pattern = std::array::from_fn(|i| (mask & (1 << i) != 0).then_some(row[i]));
            let mut expected: Vec<_> = rows
                .iter()
                .copied()
                .filter(|candidate| {
                    pattern
                        .iter()
                        .zip(candidate)
                        .all(|(bound, id)| bound.is_none_or(|b| b == *id))
                })
                .collect();
            let scan = graph.store.scan(&pattern);
            let mut observed: Vec<_> = scan.rows.iter().map(|row| scan.to_spo(row)).collect();
            expected.sort_unstable();
            observed.sort_unstable();
            assert_eq!(observed, expected, "scan changed for {pattern:?}");
        }
        for &id in row {
            assert_eq!(graph.dict.lookup(&graph.dict.term(id)), id);
            let number = crate::numeric_of(&graph.dict.term(id));
            assert_eq!(
                graph.numeric_value(id),
                (!number.is_nan()).then_some(number),
                "numeric cache changed"
            );
            assert_eq!(graph.numeric_value(id), graph.fork().numeric_value(id));
            let observed = graph
                .temporals
                .lookup(id)
                .map(|t| (t.instant.to_bits(), t.kind, t.has_tz));
            let expected = crate::temporal_of_id(&graph.dict, id)
                .map(|t| (t.instant.to_bits(), t.kind, t.has_tz));
            assert_eq!(observed, expected, "temporal cache changed");
        }
    }
    let mut terms: Vec<_> = rows
        .into_iter()
        .map(|row| row.map(|id| graph.dict.term(id).to_string()))
        .collect();
    terms.sort_unstable();
    terms
}

#[test]
#[cfg(not(miri))]
fn owned_and_mapped_graphs_match() {
    for compressed in [false, true] {
        let dir = Directory::new();
        let source = dir.0.join("source");
        if compressed {
            sample().save_compressed(&source).unwrap();
        } else {
            sample().save(&source).unwrap();
        }
        drop(Graph::open(&source).unwrap());
        let before = tree(&source);
        let expected = exercise(&sample());
        let deleted_graph = Graph::load_str("<urn:s> <urn:text> \"label\"@en .", "ttl").unwrap();
        let deleted: Vec<_> = deleted_graph
            .iter_ids()
            .map(|row| row.map(|id| deleted_graph.dict.term(id)))
            .collect();
        let inserted_graph = Graph::load_str(
            r#"<urn:new> <urn:p> "17.5"^^<http://www.w3.org/2001/XMLSchema#double>;
            <urn:date> "2027-01-02T03:04:05Z"^^<http://www.w3.org/2001/XMLSchema#dateTime> ."#,
            "ttl",
        )
        .unwrap();
        let inserted: Vec<_> = inserted_graph
            .iter_ids()
            .map(|row| row.map(|id| inserted_graph.dict.term(id)))
            .collect();
        let mut expected_recovery = expected.clone();
        expected_recovery.retain(|row| !exercise(&deleted_graph).contains(row));
        expected_recovery.extend(exercise(&inserted_graph));
        expected_recovery.sort_unstable();
        let run = |threshold, destination: &Path| {
            with_threshold(threshold, || {
                let mut graph = Graph::open(&source).unwrap();
                let rows = exercise(&graph);
                assert_eq!(rows, expected, "open changed source rows");
                for id in 1..=graph.dict.len() as crate::Id {
                    let t = graph.temporals.lookup(id);
                    let fork = graph.fork();
                    assert_eq!(
                        t.map(|t| (t.instant.to_bits(), t.kind, t.has_tz)),
                        fork.temporals
                            .lookup(id)
                            .map(|t| (t.instant.to_bits(), t.kind, t.has_tz))
                    );
                }
                let bytes = graph.heap_bytes();
                std::fs::create_dir(destination).unwrap();
                std::fs::copy(source.join("sparq-store"), destination.join("sparq-store")).unwrap();
                graph.save(destination).unwrap();
                graph.apply_delta(&inserted, &deleted).unwrap();
                assert_eq!(exercise(&graph), expected_recovery, "delta changed rows");
                drop(graph);
                let recovered = Graph::open(&source).unwrap();
                let recovered_rows = exercise(&recovered);
                assert_eq!(recovered_rows, expected_recovery, "replay changed rows");
                (rows, bytes, recovered_rows)
            })
        };
        let (mapped, mapped_heap, mapped_recovery) = run(0, &dir.0.join("mapped"));
        // Restore this fixture's saved image so both strategies replay the same WAL.
        for (path, bytes) in &before {
            std::fs::write(source.join(path), bytes).unwrap();
        }
        let (owned, owned_heap, owned_recovery) = run(65536, &dir.0.join("owned"));
        assert_eq!(mapped, owned);
        assert_eq!(mapped_recovery, owned_recovery);
        assert!(
            owned_heap > mapped_heap,
            "owned backing omitted from heap accounting"
        );
        assert_eq!(
            tree(&dir.0.join("mapped")),
            tree(&dir.0.join("owned")),
            "save bytes depend on read backing"
        );
        // A clean open without updates must leave every persisted byte alone.
        for (path, bytes) in &before {
            std::fs::write(source.join(path), bytes).unwrap();
        }
        with_threshold(65536, || {
            exercise(&Graph::open(&source).unwrap());
        });
        assert_eq!(before, tree(&source), "open changed the persisted tree");
    }
}

#[test]
fn owned_open_miri() {
    for compressed in [false, true] {
        let dir = Directory::new();
        let source = sample();
        // This fixture is private to the test. Exercise the real file opener without
        // asking Miri to emulate the platform directory-lock syscall.
        if compressed {
            source
                .save_compressed_unlocked(&dir.0, crate::Role::Root)
                .unwrap();
        } else {
            source.save_unlocked(&dir.0, crate::Role::Root).unwrap();
        }
        with_threshold(65536, || {
            let graph = Graph::open_dir(&dir.0, None, false, crate::Role::Root).unwrap();
            assert!(matches!(graph.numerics, crate::NumData::Owned(_)));
            assert!(matches!(graph.temporals, crate::TempData::Owned(_)));
            assert_eq!(exercise(&graph), exercise(&source));
            assert!(graph.dict.heap_bytes() > 0);
        });
    }
}

#[test]
#[cfg(all(target_os = "linux", not(miri)))]
fn tiny_slots_hold_no_file_mappings() {
    with_threshold(SMALL_FILE_BYTES, || {
        let dir = Directory::new();
        let source = dir.0.join("store");
        let mut seed = Graph::new();
        for i in 0..2048 {
            seed.add_named(
                oxrdf::NamedNode::new_unchecked(format!("urn:g:{i}")).into(),
                Graph::new(),
            )
            .unwrap();
        }
        seed.save(&source).unwrap();
        drop(seed);
        let triple = [
            oxrdf::NamedNode::new_unchecked("urn:s").into(),
            oxrdf::NamedNode::new_unchecked("urn:p").into(),
            oxrdf::NamedNode::new_unchecked("urn:o").into(),
        ];
        let mut graph = Graph::open(&source).unwrap();
        for i in 0..2048 {
            graph
                .apply_delta_to_named(i, std::slice::from_ref(&triple), &[])
                .unwrap();
        }
        drop(graph);
        for compacted in [false, true] {
            let mut graph = Graph::open(&source).unwrap();
            if compacted {
                graph.compact().unwrap();
            }
            drop(graph);
            let graph = Graph::open(&source).unwrap();
            let maps = std::fs::read_to_string("/proc/self/maps").unwrap();
            assert_eq!(
                maps.lines()
                    .filter(|l| l.contains(source.to_str().unwrap()))
                    .count(),
                0,
                "small slots retained file mappings (compacted={compacted})"
            );
            assert_eq!(graph.named().len(), 2048);
            assert!(graph.named().iter().all(|(_, g)| g.len() == 1));
        }
    });
}

#[test]
#[cfg(not(miri))]
fn small_corruption_matches_mapped_errors() {
    let dir = Directory::new();
    let source = dir.0.join("source");
    sample().save_compressed(&source).unwrap();
    for name in ["dict-offs.bin", "dict-terms.bin", "perm0.bin"] {
        let path = source.join(name);
        let original = std::fs::read(&path).unwrap();
        let mut corrupt = original.clone();
        if name == "dict-offs.bin" {
            corrupt.pop();
        } else if name == "dict-terms.bin" {
            corrupt[0] = 255;
        } else {
            corrupt[16..24].copy_from_slice(&u64::MAX.to_le_bytes());
        }
        std::fs::write(&path, corrupt).unwrap();
        let error = |threshold| {
            with_threshold(threshold, || {
                let error = Graph::open(&source).err().expect("corruption was accepted");
                (error.kind(), error.to_string())
            })
        };
        let mapped = error(0);
        let owned = error(65536);
        assert_eq!(owned, mapped, "validation depends on backing for {name}");
        assert_eq!(owned.0, io::ErrorKind::InvalidData);
        std::fs::write(&path, original).unwrap();
    }
}
