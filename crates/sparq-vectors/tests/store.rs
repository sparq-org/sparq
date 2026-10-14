//! `.spqv` store round-trip, sparse coverage and error-path tests.

use sparq_vectors::VectorStore;

fn tmp_path(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("sparq-vectors-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir.join(name)
}

#[test]
fn roundtrip_sparse_ids() {
    let path = tmp_path("roundtrip.spqv");
    let mut store = VectorStore::create(&path, 3).unwrap();
    // Sparse, unsorted ids — only some terms get embeddings.
    store.put(42, &[1.0, 2.0, 3.0]).unwrap();
    store.put(7, &[4.0, 5.0, 6.0]).unwrap();
    store.put(1_000_000, &[-1.5, 0.0, 2.5]).unwrap();

    // Reads work during the build phase…
    assert_eq!(store.get(7), Some(&[4.0f32, 5.0, 6.0][..]));
    assert_eq!(store.get(8), None);
    assert_eq!(store.len(), 3);

    store.finalize().unwrap();

    // …and from the finalized (mmap'd) handle…
    assert_eq!(store.get(42), Some(&[1.0f32, 2.0, 3.0][..]));
    assert_eq!(store.get(7), Some(&[4.0f32, 5.0, 6.0][..]));
    assert_eq!(store.get(1_000_000), Some(&[-1.5f32, 0.0, 2.5][..]));
    assert_eq!(store.get(8), None);
    assert_eq!(store.get(0), None);
    assert_eq!(store.get(u32::MAX), None);

    // …and from a fresh open.
    let reopened = VectorStore::open(&path).unwrap();
    assert_eq!(reopened.dim(), 3);
    assert_eq!(reopened.len(), 3);
    assert_eq!(reopened.get(42), Some(&[1.0f32, 2.0, 3.0][..]));
    assert_eq!(reopened.get(43), None);

    // iter() yields all pairs in insertion order.
    let pairs: Vec<(u32, Vec<f32>)> =
        reopened.iter().map(|(id, v)| (id, v.to_vec())).collect();
    assert_eq!(pairs[0], (42, vec![1.0, 2.0, 3.0]));
    assert_eq!(pairs[1], (7, vec![4.0, 5.0, 6.0]));
    assert_eq!(pairs[2], (1_000_000, vec![-1.5, 0.0, 2.5]));
}

#[test]
fn empty_store_roundtrips() {
    let path = tmp_path("empty.spqv");
    let mut store = VectorStore::create(&path, 8).unwrap();
    store.finalize().unwrap();
    let reopened = VectorStore::open(&path).unwrap();
    assert!(reopened.is_empty());
    assert_eq!(reopened.get(1), None);
    assert_eq!(reopened.iter().count(), 0);
}

#[test]
fn build_phase_errors() {
    let path = tmp_path("errors.spqv");
    assert!(VectorStore::create(&path, 0).is_err(), "dim 0 rejected");

    let mut store = VectorStore::create(&path, 2).unwrap();
    assert!(store.put(1, &[1.0]).is_err(), "dim mismatch rejected");
    assert!(store.put(1, &[1.0, f32::NAN]).is_err(), "non-finite rejected");
    assert!(store.put(1, &[0.0, 0.0]).is_err(), "all-zero rejected (cosine undefined)");
    store.put(1, &[1.0, 2.0]).unwrap();
    assert!(store.put(1, &[3.0, 4.0]).is_err(), "duplicate id rejected");
    store.finalize().unwrap();
    assert!(store.put(2, &[1.0, 2.0]).is_err(), "put after finalize rejected");
    store.finalize().unwrap(); // idempotent
}

#[test]
fn open_rejects_garbage() {
    let path = tmp_path("garbage.spqv");
    std::fs::write(&path, b"not a vector store at all").unwrap();
    assert!(VectorStore::open(&path).is_err());

    let short = tmp_path("short.spqv");
    std::fs::write(&short, b"SPQV").unwrap();
    assert!(VectorStore::open(&short).is_err());

    assert!(VectorStore::open(tmp_path("does-not-exist.spqv")).is_err());

    // Truncated body: valid header claiming more data than the file holds.
    let truncated = tmp_path("truncated.spqv");
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"SPQV");
    bytes.extend_from_slice(&1u32.to_le_bytes()); // version
    bytes.extend_from_slice(&4u32.to_le_bytes()); // dim
    bytes.extend_from_slice(&100u64.to_le_bytes()); // count
    bytes.extend_from_slice(&[0u8; 12]);
    std::fs::write(&truncated, &bytes).unwrap();
    assert!(VectorStore::open(&truncated).is_err());

    // Overflowing header: count×dim×4 wraps usize — must be rejected by checked
    // arithmetic, not pass a wrapped size check.
    let overflow = tmp_path("overflow.spqv");
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"SPQV");
    bytes.extend_from_slice(&1u32.to_le_bytes());
    bytes.extend_from_slice(&u32::MAX.to_le_bytes()); // dim
    bytes.extend_from_slice(&u64::MAX.to_le_bytes()); // count
    bytes.extend_from_slice(&[0u8; 12]);
    std::fs::write(&overflow, &bytes).unwrap();
    assert!(VectorStore::open(&overflow).is_err());
}

/// Builds a structurally well-sized 1-d, 2-vector file with a caller-chosen id→slot
/// index section, to exercise `open`'s index validation.
fn file_with_index(name: &str, index: &[(u32, u32)]) -> std::path::PathBuf {
    let path = tmp_path(name);
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"SPQV");
    bytes.extend_from_slice(&1u32.to_le_bytes()); // version
    bytes.extend_from_slice(&1u32.to_le_bytes()); // dim
    bytes.extend_from_slice(&2u64.to_le_bytes()); // count
    bytes.extend_from_slice(&[0u8; 12]);
    bytes.extend_from_slice(&1.0f32.to_le_bytes()); // slot 0
    bytes.extend_from_slice(&2.0f32.to_le_bytes()); // slot 1
    for &(id, slot) in index {
        bytes.extend_from_slice(&id.to_le_bytes());
        bytes.extend_from_slice(&slot.to_le_bytes());
    }
    std::fs::write(&path, &bytes).unwrap();
    path
}

#[test]
fn open_rejects_corrupt_index() {
    // A valid index round-trips…
    let good = file_with_index("idx-good.spqv", &[(5, 1), (9, 0)]);
    let store = VectorStore::open(&good).unwrap();
    assert_eq!(store.get(5), Some(&[2.0f32][..]));
    assert_eq!(store.get(9), Some(&[1.0f32][..]));

    // …but unsorted ids, duplicate ids, out-of-range slots and duplicate slots are
    // all rejected at open (they would otherwise corrupt get/iter).
    assert!(VectorStore::open(file_with_index("idx-unsorted.spqv", &[(9, 0), (5, 1)])).is_err());
    assert!(VectorStore::open(file_with_index("idx-dup-id.spqv", &[(5, 0), (5, 1)])).is_err());
    assert!(VectorStore::open(file_with_index("idx-bad-slot.spqv", &[(5, 0), (9, 7)])).is_err());
    assert!(VectorStore::open(file_with_index("idx-dup-slot.spqv", &[(5, 0), (9, 0)])).is_err());
}

// ------------------------------------------------------------- streaming writer

#[test]
fn streaming_writer_produces_byte_identical_store() {
    use sparq_vectors::StreamingWriter;
    // Same put sequence through both builders → byte-identical .spqv files.
    let a_path = tmp_path("stream-a.spqv");
    let b_path = tmp_path("stream-b.spqv");
    let puts: &[(u32, [f32; 3])] = &[
        (42, [1.0, 2.0, 3.0]),
        (7, [4.0, 5.0, 6.0]),
        (1_000_000, [-1.5, 0.0, 2.5]),
    ];
    let mut in_ram = VectorStore::create(&a_path, 3).unwrap();
    let mut streaming = StreamingWriter::create(&b_path, 3).unwrap();
    for (id, v) in puts {
        in_ram.put(*id, v).unwrap();
        streaming.put(*id, v).unwrap();
    }
    in_ram.finalize().unwrap();
    assert_eq!(streaming.len(), 3);
    let store = streaming.finalize().unwrap();
    assert_eq!(std::fs::read(&a_path).unwrap(), std::fs::read(&b_path).unwrap());

    // The returned handle is a normal opened store.
    assert_eq!(store.dim(), 3);
    assert_eq!(store.len(), 3);
    assert_eq!(store.get(7), Some(&[4.0f32, 5.0, 6.0][..]));
    assert_eq!(store.get(8), None);
    let pairs: Vec<u32> = store.iter().map(|(id, _)| id).collect();
    assert_eq!(pairs, [42, 7, 1_000_000], "insertion-slot order");

    // The sidecar id spill is gone.
    let mut sidecar = b_path.clone().into_os_string();
    sidecar.push(".ids-tmp");
    assert!(!std::path::PathBuf::from(sidecar).exists());
}

#[test]
fn streaming_writer_empty_and_validation() {
    use sparq_vectors::StreamingWriter;
    let path = tmp_path("stream-empty.spqv");
    let w = StreamingWriter::create(&path, 4).unwrap();
    assert!(w.is_empty());
    let store = w.finalize().unwrap();
    assert!(store.is_empty());
    assert!(VectorStore::open(&path).is_ok());

    let mut w = StreamingWriter::create(tmp_path("stream-bad.spqv"), 2).unwrap();
    assert!(w.put(1, &[1.0]).is_err(), "dim mismatch");
    assert!(w.put(1, &[f32::NAN, 1.0]).is_err(), "non-finite");
    assert!(w.put(1, &[0.0, 0.0]).is_err(), "all-zero");
    assert!(StreamingWriter::create(tmp_path("zero-dim.spqv"), 0).is_err());
}

#[test]
fn streaming_writer_detects_duplicates_at_finalize_and_cleans_up() {
    use sparq_vectors::StreamingWriter;
    let path = tmp_path("stream-dup.spqv");
    let mut w = StreamingWriter::create(&path, 2).unwrap();
    w.put(5, &[1.0, 0.0]).unwrap();
    w.put(5, &[0.0, 1.0]).unwrap(); // duplicate id: accepted here…
    let err = match w.finalize() {
        Ok(_) => panic!("duplicate id must fail finalize"),
        Err(e) => e, // …reported here
    };
    assert!(err.contains("duplicate"), "{err}");
    // On error both the partial store and the sidecar are removed.
    assert!(!path.exists());
    let mut sidecar = path.clone().into_os_string();
    sidecar.push(".ids-tmp");
    assert!(!std::path::PathBuf::from(sidecar).exists());
}

// ------------------------------------------------------------- bytes-backed open

#[test]
fn open_from_bytes_matches_mmap_open() {
    let path = tmp_path("bytes.spqv");
    let mut store = VectorStore::create(&path, 3).unwrap();
    store.put(42, &[1.0, 2.0, 3.0]).unwrap();
    store.put(7, &[4.0, 5.0, 6.0]).unwrap();
    store.finalize().unwrap();

    let bytes = std::fs::read(&path).unwrap();
    let mem = VectorStore::open_from_bytes(bytes).unwrap();
    assert_eq!(mem.dim(), 3);
    assert_eq!(mem.len(), 2);
    assert_eq!(mem.get(42), Some(&[1.0f32, 2.0, 3.0][..]));
    assert_eq!(mem.get(7), Some(&[4.0f32, 5.0, 6.0][..]));
    assert_eq!(mem.get(8), None);
    let pairs: Vec<u32> = mem.iter().map(|(id, _)| id).collect();
    assert_eq!(pairs, [42, 7]);
}

#[test]
fn open_from_bytes_validates_like_open() {
    assert!(VectorStore::open_from_bytes(b"garbage".to_vec()).is_err());
    let mut bad_magic = vec![0u8; 32];
    bad_magic[0..4].copy_from_slice(b"NOPE");
    assert!(VectorStore::open_from_bytes(bad_magic).is_err());
    // A truncated copy of a valid store must be rejected by the size check.
    let path = tmp_path("bytes-trunc.spqv");
    let mut store = VectorStore::create(&path, 2).unwrap();
    store.put(1, &[1.0, 2.0]).unwrap();
    store.finalize().unwrap();
    let mut bytes = std::fs::read(&path).unwrap();
    bytes.truncate(bytes.len() - 1);
    assert!(VectorStore::open_from_bytes(bytes).is_err());
}

// Regression for review 1874: the owned-bytes path must read vectors correctly even
// when the source `Vec<u8>` is NOT 4-byte aligned — the store now copies into an f32-aligned
// backing, so `slot_vector`'s &[u8]→&[f32] cast can never see an unaligned pointer (which is UB).
// Under `cargo +nightly miri test` this also flags any residual misaligned-reference creation.
#[test]
fn open_from_bytes_handles_unaligned_source_buffer() {
    let path = tmp_path("bytes-unaligned.spqv");
    let mut store = VectorStore::create(&path, 3).unwrap();
    store.put(42, &[1.0, 2.0, 3.0]).unwrap();
    store.put(7, &[-4.5, 5.25, 6.0]).unwrap();
    store.finalize().unwrap();
    let file = std::fs::read(&path).unwrap();

    // Force a deliberately ODD-address source: allocate with a leading byte, then drop it via
    // `split_off(1)` so the returned Vec's data pointer is the original base + 1 (alignment 1).
    let mut padded = Vec::with_capacity(file.len() + 1);
    padded.push(0u8);
    padded.extend_from_slice(&file);
    let unaligned = padded.split_off(1);
    // (We don't assert oddness — heap alignment is allocator-dependent — but this maximises the
    // chance the source is unaligned; the copy-into-aligned-backing fix must make reads correct
    // regardless.)
    let mem = VectorStore::open_from_bytes(unaligned).unwrap();
    assert_eq!(mem.get(42), Some(&[1.0f32, 2.0, 3.0][..]));
    assert_eq!(mem.get(7), Some(&[-4.5f32, 5.25, 6.0][..]));
    let pairs: Vec<(u32, Vec<f32>)> = mem.iter().map(|(id, v)| (id, v.to_vec())).collect();
    assert_eq!(pairs, [(42, vec![1.0, 2.0, 3.0]), (7, vec![-4.5, 5.25, 6.0])]);
}
