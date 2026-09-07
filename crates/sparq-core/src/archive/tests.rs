// [GPT-6] Native codec parity, shared lifetime, overlays and hostile-container tests.
use super::*;
use oxrdf::{BlankNode, Literal, NamedNode, Term};
use std::sync::atomic::{AtomicU64, Ordering};

struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "sparq-archive-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn path(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn sample() -> Graph {
    let mut nq = String::new();
    for i in 0..260 {
        nq.push_str(&format!("<https://e/s{i}> <https://e/p> \"value {i}\" .\n"));
        nq.push_str(&format!("<https://e/s{i}> <https://e/n> \"{i}\"^^<http://www.w3.org/2001/XMLSchema#integer> <https://e/g> .\n"));
    }
    nq.push_str("<https://e/s> <https://e/date> \"2026-01-01T12:00:00Z\"^^<http://www.w3.org/2001/XMLSchema#dateTime> _:b .\n");
    nq.push_str("<https://e/s> <https://e/label> \"bonjour\"@fr _:b .\n");
    let mut graph = Graph::load_dataset(&nq, "nquads").unwrap();
    graph.named.push((
        Term::NamedNode(NamedNode::new_unchecked("https://e/empty")),
        Graph::new(),
    ));
    graph
}

fn dump(graph: &Graph) -> Vec<String> {
    fn visit(graph: &Graph, prefix: &str, result: &mut Vec<String>) {
        for row in graph.iter_ids() {
            result.push(format!(
                "{prefix}|{} {} {}",
                graph.dict.term(row[0]),
                graph.dict.term(row[1]),
                graph.dict.term(row[2])
            ));
            for id in row {
                let term = graph.dict.term(id);
                assert_eq!(graph.dict.lookup(&term), id);
            }
        }
        for (name, child) in &graph.named {
            let prefix = format!("{prefix}/{name}");
            result.push(format!("graph:{prefix}"));
            visit(child, &prefix, result);
        }
    }
    let mut result = Vec::new();
    visit(graph, "", &mut result);
    result.sort();
    result
}

fn write(path: &Path, graph: &Graph, compressed: bool) {
    let mut writer = NativeArchiveWriter::create(path).unwrap();
    assert_eq!(writer.append(graph, compressed).unwrap(), 0);
    assert_eq!(writer.finish().unwrap().datasets, 1);
}

// Every test owns its temporary file and never changes it while mapped views live.
fn open(path: &Path) -> NativeArchive {
    // SAFETY: the test retains an immutable file until all views are dropped.
    unsafe { NativeArchive::open(path).unwrap() }
}

struct Component {
    name: String,
    name_position: usize,
    extent_position: usize,
    offset: usize,
    length: usize,
}

fn components(bytes: &[u8]) -> Vec<Component> {
    let index = u64::from_le_bytes(bytes[16..24].try_into().unwrap()) as usize;
    let table = u64::from_le_bytes(bytes[index + 8..index + 16].try_into().unwrap()) as usize;
    let mut input = Input::new(&bytes[table..]);
    let count = input.usize().unwrap();
    (0..count)
        .map(|_| {
            let n = input.u32().unwrap() as usize;
            let name_position = table + input.position;
            let name = std::str::from_utf8(input.take(n).unwrap())
                .unwrap()
                .to_owned();
            let extent_position = table + input.position;
            Component {
                name,
                name_position,
                extent_position,
                offset: input.usize().unwrap(),
                length: input.usize().unwrap(),
            }
        })
        .collect()
}

#[test]
fn raw_and_compressed_components_match_directory_codec_and_roundtrip() {
    let temp = Temp::new();
    let graph = sample();
    let expected = dump(&graph);
    for compressed in [false, true] {
        let path = temp.path(&format!("archive-{compressed}"));
        let directory = temp.path(&format!("directory-{compressed}"));
        write(&path, &graph, compressed);
        if compressed {
            graph.save_compressed(&directory).unwrap();
        } else {
            graph.save(&directory).unwrap();
        }
        let bytes = std::fs::read(&path).unwrap();
        for entry in components(&bytes) {
            assert_eq!(
                &bytes[entry.offset..entry.offset + entry.length],
                std::fs::read(directory.join(&entry.name)).unwrap(),
                "component {}",
                entry.name
            );
        }
        let archive = open(&path);
        assert_eq!(dump(&archive.load_dataset(0).unwrap()), expected);
        assert!(archive.load_dataset(1).is_err());
    }
}

#[test]
fn all_dataset_views_share_mapping_and_outlive_archive_handle() {
    let temp = Temp::new();
    let path = temp.path("many");
    let graph = sample();
    let expected = dump(&graph);
    let mut writer = NativeArchiveWriter::create(&path).unwrap();
    for id in 0..12 {
        assert_eq!(writer.append(&graph, id % 2 == 0).unwrap(), id);
    }
    let stats = writer.finish().unwrap();
    assert_eq!(stats.bytes, std::fs::metadata(&path).unwrap().len());
    assert_eq!(std::fs::read_dir(&temp.0).unwrap().count(), 1);
    let archive = open(&path);
    let views: Vec<_> = (0..archive.len())
        .map(|id| archive.load_dataset(id).unwrap())
        .collect();
    for view in &views {
        let NumData::Mapped(MappedBytes::Region { mapping, .. }, _) = &view.numerics else {
            panic!("expected shared mapped numerics")
        };
        assert!(Arc::ptr_eq(mapping, &archive.mapping));
        for (_, child) in &view.named {
            let NumData::Mapped(MappedBytes::Region { mapping, .. }, _) = &child.numerics else {
                panic!("expected shared named view")
            };
            assert!(Arc::ptr_eq(mapping, &archive.mapping));
        }
    }
    drop(archive);
    for view in views {
        assert_eq!(dump(&view), expected);
    }
}

#[test]
fn overlays_fork_independently_and_persist_only_in_new_archive() {
    let temp = Temp::new();
    let path = temp.path("base");
    write(&path, &sample(), true);
    let archive = open(&path);
    let mut view = archive.load_dataset(0).unwrap();
    let original = dump(&view);
    let fork = view.fork();
    let s = NamedNode::new_unchecked("https://e/s0");
    let p = NamedNode::new_unchecked("https://e/p");
    view.remove_triple(s.clone(), p.clone(), Literal::new_simple_literal("value 0"))
        .unwrap();
    view.insert_triple(s, p, Literal::new_simple_literal("replacement"))
        .unwrap();
    view.named[0]
        .1
        .insert_triple(
            BlankNode::new_unchecked("added"),
            NamedNode::new_unchecked("https://e/new"),
            Literal::from(42),
        )
        .unwrap();
    let changed = dump(&view);
    assert_ne!(changed, original);
    assert_eq!(dump(&fork), original);
    assert_eq!(dump(&archive.load_dataset(0).unwrap()), original);
    let updated = temp.path("updated");
    write(&updated, &view, true);
    assert_eq!(dump(&open(&updated).load_dataset(0).unwrap()), changed);
    assert_eq!(std::fs::read_dir(&temp.0).unwrap().count(), 2);
}

fn rejected(path: &Path, bytes: &[u8]) {
    std::fs::write(path, bytes).unwrap();
    // SAFETY: this owned test file is unchanged until this expression drops all views.
    let result = unsafe { NativeArchive::open(path) }.and_then(|archive| archive.load_dataset(0));
    assert!(result.is_err(), "malformed archive was accepted");
}

#[test]
fn incomplete_and_overflowing_container_metadata_is_rejected() {
    let temp = Temp::new();
    let path = temp.path("good");
    write(&path, &sample(), false);
    let bytes = std::fs::read(path).unwrap();
    let bad = temp.path("bad");
    for length in [0, 1, 7, 8, 16, 39, 40, bytes.len() - 1] {
        rejected(&bad, &bytes[..length]);
    }
    for field in [8, 12, 16, 24, 32] {
        let mut changed = bytes.clone();
        changed[field..field + 4].fill(0xff);
        rejected(&bad, &changed);
    }
}

#[test]
fn missing_tables_unaligned_overlapping_and_duplicate_components_are_rejected() {
    let temp = Temp::new();
    let path = temp.path("good");
    write(&path, &sample(), false);
    let bytes = std::fs::read(path).unwrap();
    let entries = components(&bytes);
    let bad = temp.path("bad");
    for name in [
        "numerics.bin",
        "temporals.bin",
        "predstats.bin",
        "dict-meta.bin",
        "named.bin",
    ] {
        let entry = entries.iter().find(|entry| entry.name == name).unwrap();
        let mut changed = bytes.clone();
        changed[entry.name_position] = b'X';
        rejected(&bad, &changed);
    }
    let first = &entries[0];
    for offset in [first.offset + 1, 0, bytes.len()] {
        let mut changed = bytes.clone();
        changed[first.extent_position..first.extent_position + 8]
            .copy_from_slice(&(offset as u64).to_le_bytes());
        rejected(&bad, &changed);
    }
    let mut changed = bytes.clone();
    let second = &entries[1];
    changed[second.extent_position..second.extent_position + 8]
        .copy_from_slice(&(first.offset as u64).to_le_bytes());
    rejected(&bad, &changed);
    let mut changed = bytes.clone();
    changed[second.name_position..second.name_position + first.name.len()]
        .copy_from_slice(first.name.as_bytes());
    rejected(&bad, &changed);
}

#[test]
fn native_dictionary_raw_rows_and_compressed_blocks_are_validated_before_use() {
    let temp = Temp::new();
    for compressed in [false, true] {
        let path = temp.path(&format!("good-{compressed}"));
        write(&path, &sample(), compressed);
        let bytes = std::fs::read(path).unwrap();
        let entries = components(&bytes);
        let bad = temp.path("bad");
        for name in ["dict-offs.bin", "dict-hid.bin"] {
            let entry = entries.iter().find(|entry| entry.name == name).unwrap();
            let mut changed = bytes.clone();
            changed[entry.offset..entry.offset + 4].fill(0xff);
            rejected(&bad, &changed);
        }
        let entry = entries
            .iter()
            .find(|entry| entry.name == "perm0.bin")
            .unwrap();
        let mut changed = bytes.clone();
        if compressed {
            // Invalid declared row count in the checked native compressed header.
            changed[entry.offset + 8..entry.offset + 16].fill(0xff);
        } else {
            // Zero is never a valid dictionary or inline ID.
            changed[entry.offset..entry.offset + 4].fill(0);
        }
        rejected(&bad, &changed);
    }
}

#[test]
fn empty_archive_and_exclusive_creation_and_unfinished_writer() {
    let temp = Temp::new();
    let path = temp.path("empty");
    let writer = NativeArchiveWriter::create(&path).unwrap();
    assert!(NativeArchiveWriter::create(&path).is_err());
    writer.finish().unwrap();
    let archive = open(&path);
    assert!(archive.is_empty());
    assert!(archive.load_dataset(0).is_err());
    let unfinished = temp.path("unfinished");
    let mut writer = NativeArchiveWriter::create(&unfinished).unwrap();
    writer.append(&sample(), false).unwrap();
    drop(writer);
    // SAFETY: writer is closed and the file is immutable for this call.
    assert!(unsafe { NativeArchive::open(&unfinished) }.is_err());
}

#[test]
fn too_deep_append_poisoning_cannot_publish_partial_dataset() {
    let temp = Temp::new();
    let mut nested = Graph::new();
    for _ in 0..MAX_DEPTH + 2 {
        let mut parent = Graph::new();
        parent.named.push((
            Term::NamedNode(NamedNode::new_unchecked("https://e/g")),
            nested,
        ));
        nested = parent;
    }
    let mut writer = NativeArchiveWriter::create(temp.path("deep")).unwrap();
    assert!(writer.append(&nested, false).is_err());
    assert!(writer.append(&Graph::new(), false).is_err());
    assert!(writer.finish().is_err());
}

#[test]
fn validate_row_rejects_zero_absent_duplicate_and_unsorted_ids() {
    let mut previous = None;
    assert!(validate_row([1, 2, 3], &mut previous, 3).is_ok());
    for row in [[1, 2, 3], [1, 2, 2], [0, 2, 3], [1, 2, 4]] {
        assert!(validate_row(row, &mut previous, 3).is_err());
    }
    assert!(validate_row([2, 2, 3], &mut previous, 3).is_ok());
}

#[test]
fn validate_archive_rows_rejects_dictionary_escape_and_duplicate_rows() {
    let good = crate::compress::CompressedPerm::encode(&[[1, 2, 3], [2, 2, 3]]);
    assert!(good.validate_archive_rows(3).is_ok());
    assert!(good.validate_archive_rows(2).is_err());
    let duplicate = crate::compress::CompressedPerm::encode(&[[1, 2, 3], [1, 2, 3]]);
    assert!(duplicate.validate_archive_rows(3).is_err());
}
