//! sq-cafc — behavioural coverage for the direct decoder's least-covered
//! PRODUCTION paths (not vacuous line-touchers):
//!
//!  * the triples-section CONTROL-INFO validation branches (`decode.rs`): a non-`Triples`
//!    control type, the `triplesList` format (explicitly unsupported), an unknown triples
//!    format, and a non-SPO triple order — each a distinct `Err` the decoder returns and
//!    none previously exercised (the round-trip fixtures only ever carry a well-formed
//!    SPO BitmapTriples control info);
//!  * the SINGLE-THREADED dictionary-decode fast path (`decode_dict`'s
//!    `rayon::current_num_threads() <= 1` arm), reached by decoding inside a 1-thread
//!    rayon pool — the default test pool has many threads, so the straight-line arm was
//!    never measured.
//!
//! Each test crafts its input by SPLICING a re-serialised triples `ControlInfo` into an
//! otherwise-valid archive (the `hdt` crate's `ControlInfo` is a public type with public
//! fields + `write`), so the corruption is surgical: every byte before the triples
//! control info is the real archive, so the decoder reaches the triples-format check with
//! a fully-decoded dictionary — i.e. the branch under test is genuinely the thing that
//! rejects the input, not an earlier failure.

use hdt::containers::{ControlInfo, ControlType};
use std::collections::HashMap;

/// A small, valid in-memory `.hdt` archive (shared section + a literal + a blank node),
/// built via the `hdt` crate's `read_nt` + `write` (the same path the round-trip fixtures
/// use). The triples section is a well-formed SPO BitmapTriples, so splicing a mutated
/// triples control info onto its prefix isolates exactly the control-info branch.
fn valid_hdt_bytes() -> Vec<u8> {
    let nt = "<http://example.org/alice> <http://xmlns.com/foaf/0.1/knows> <http://example.org/bob> .\n\
              <http://example.org/bob> <http://xmlns.com/foaf/0.1/knows> <http://example.org/alice> .\n\
              <http://example.org/alice> <http://xmlns.com/foaf/0.1/name> \"Alice\" .\n\
              _:b0 <http://example.org/link> _:b1 .\n";
    let dir = tempfile::tempdir().expect("scratch dir");
    let nt_path = dir.path().join("src.nt");
    std::fs::write(&nt_path, nt).unwrap();
    let written = hdt::Hdt::read_nt(&nt_path).expect("building HDT from N-Triples");
    let mut buf: Vec<u8> = Vec::new();
    written.write(&mut buf).expect("serializing HDT archive");
    buf
}

/// Locates the triples (`$HDT`\x04) control-info cookie in `bytes` and returns
/// `(offset, original_control_info_byte_len, parsed_control_info)`. The triples control
/// info is the LAST `$HDT` cookie in a standard archive (global, header, dictionary,
/// triples), found by scanning for the cookie whose following control-type byte is 4.
fn find_triples_control_info(bytes: &[u8]) -> (usize, usize, ControlInfo) {
    let mut at = 0;
    let mut found = None;
    while let Some(i) = (at..=bytes.len().saturating_sub(4)).find(|&i| &bytes[i..i + 4] == b"$HDT")
    {
        if bytes.get(i + 4) == Some(&(ControlType::Triples as u8)) {
            found = Some(i);
        }
        at = i + 4;
    }
    let off = found.expect("archive must carry a triples control info");
    // Re-read the control info to learn its exact on-disk byte length (cookie + type +
    // null-terminated format + null-terminated properties + 2-byte CRC).
    let mut cur = std::io::Cursor::new(&bytes[off..]);
    let ci = ControlInfo::read(&mut cur).expect("triples control info must parse");
    (off, cur.position() as usize, ci)
}

/// Re-serialises `ci` and splices it over the original triples control info in `bytes`,
/// returning the mutated archive. The dictionary + bitmap/sequence payload bytes that
/// follow are left untouched, so only the control-info branch under test changes outcome.
fn splice_triples_control_info(bytes: &[u8], ci: &ControlInfo) -> Vec<u8> {
    let (off, orig_len, _) = find_triples_control_info(bytes);
    let mut new_ci = Vec::new();
    ci.write(&mut new_ci).expect("re-serialising control info");
    let mut out = Vec::with_capacity(bytes.len());
    out.extend_from_slice(&bytes[..off]);
    out.extend_from_slice(&new_ci);
    out.extend_from_slice(&bytes[off + orig_len..]);
    out
}

/// Loads `bytes` and reduces the outcome to a `Debug`-able shape: `Ok(triple_count)` or
/// `Err(error_message)`. `sparq_core::Graph` is not `Debug`, so `Result::expect_err` can't
/// be used directly; this also makes the rejection-message assertions read cleanly.
fn load_outcome(bytes: Vec<u8>) -> Result<usize, String> {
    sparq_hdt::load_reader(std::io::Cursor::new(bytes))
        .map(|g| g.store.len())
        .map_err(|e| e.to_string())
}

/// The pristine archive (with its real SPO BitmapTriples control info) loads — so the
/// rejections below are genuinely caused by the spliced control info, not a broken prefix.
#[test]
fn pristine_archive_loads_so_splices_are_meaningful() {
    let g = sparq_hdt::load_reader(std::io::Cursor::new(valid_hdt_bytes())).expect("pristine load");
    assert_eq!(g.store.len(), 4, "the 4-triple fixture must load");
}

/// A re-serialised but UNCHANGED triples control info still loads — proving the splice
/// mechanism itself is faithful (the CRC + byte layout round-trip), so a rejection in the
/// later tests is attributable to the field we mutate, not to splicing noise.
#[test]
fn identity_splice_still_loads() {
    let bytes = valid_hdt_bytes();
    let (_, _, ci) = find_triples_control_info(&bytes);
    let spliced = splice_triples_control_info(&bytes, &ci);
    let g = sparq_hdt::load_reader(std::io::Cursor::new(spliced)).expect("identity-splice load");
    assert_eq!(g.store.len(), 4);
}

/// A control info whose TYPE is not `Triples` (here: `Dictionary`) at the triples-section
/// position must be rejected by the `control_type != Triples` guard — never decoded as a
/// triples section, never a panic.
#[test]
fn wrong_control_type_at_triples_position_is_rejected() {
    let bytes = valid_hdt_bytes();
    let (_, _, mut ci) = find_triples_control_info(&bytes);
    ci.control_type = ControlType::Dictionary;
    let spliced = splice_triples_control_info(&bytes, &ci);
    let Err(msg) = load_outcome(spliced) else {
        panic!("a non-Triples control type at the triples position must be Err");
    };
    assert!(
        msg.contains("expected triples control info"),
        "expected the triples-control-type rejection message, got: {msg}"
    );
}

/// The `triplesList` format is a valid HDT triples encoding the direct decoder does NOT
/// support (it only reads BitmapTriples); it must be rejected with the documented message,
/// not silently mis-read as a bitmap.
#[test]
fn triples_list_format_is_rejected() {
    let bytes = valid_hdt_bytes();
    let (_, _, mut ci) = find_triples_control_info(&bytes);
    ci.format = "<http://purl.org/HDT/hdt#triplesList>".to_owned();
    let spliced = splice_triples_control_info(&bytes, &ci);
    let Err(msg) = load_outcome(spliced) else {
        panic!("triplesList format must be rejected");
    };
    assert!(
        msg.contains("triples list format is not supported"),
        "expected the triplesList rejection message, got: {msg}"
    );
}

/// An UNKNOWN triples format string must be rejected by the catch-all arm.
#[test]
fn unknown_triples_format_is_rejected() {
    let bytes = valid_hdt_bytes();
    let (_, _, mut ci) = find_triples_control_info(&bytes);
    ci.format = "<http://example.org/HDT/madeUpFormat>".to_owned();
    let spliced = splice_triples_control_info(&bytes, &ci);
    let Err(msg) = load_outcome(spliced) else {
        panic!("an unknown triples format must be rejected");
    };
    assert!(
        msg.contains("unknown triples format"),
        "expected the unknown-format rejection message, got: {msg}"
    );
}

/// A non-SPO triple ORDER (order != 1) must be rejected: the direct decoder's adjacency
/// walk is SPO-only (mirroring the upstream reader). The format stays the valid
/// BitmapTriples; only the `order` property changes, so the order guard is the sole reason
/// for the rejection.
#[test]
fn non_spo_order_is_rejected() {
    let bytes = valid_hdt_bytes();
    let (_, _, mut ci) = find_triples_control_info(&bytes);
    // 2 == OPS in HDT's order enum; any value != 1 (SPO) must be refused.
    ci.properties.insert("order".to_owned(), "2".to_owned());
    let spliced = splice_triples_control_info(&bytes, &ci);
    let Err(msg) = load_outcome(spliced) else {
        panic!("a non-SPO triple order must be rejected");
    };
    assert!(
        msg.contains("unsupported HDT triples order"),
        "expected the order rejection message, got: {msg}"
    );
}

/// A MISSING / unparsable `order` property is also non-SPO (`order != Some(1)`) and must
/// be rejected — the `and_then(parse)` returns `None`, which the guard treats as not-SPO.
#[test]
fn missing_order_property_is_rejected() {
    let bytes = valid_hdt_bytes();
    let (_, _, mut ci) = find_triples_control_info(&bytes);
    ci.properties = HashMap::new(); // drop `order` (and numTriples) entirely
    let spliced = splice_triples_control_info(&bytes, &ci);
    let Err(msg) = load_outcome(spliced) else {
        panic!("a missing order property must be rejected");
    };
    assert!(
        msg.contains("unsupported HDT triples order"),
        "expected the order rejection message for a missing order, got: {msg}"
    );
}

/// sq-cafc: the SINGLE-THREADED dictionary-decode fast path. `decode_dict`
/// takes a straight-line (no `rayon::join`) arm when `rayon::current_num_threads() <= 1`;
/// the default test pool is multi-threaded, so that arm was never measured. Decoding the
/// SAME archive inside a 1-thread rayon pool (via `ThreadPool::install`, which makes
/// `current_num_threads()` report 1 for work run on it) must yield the IDENTICAL triple
/// set as the default many-threaded decode — proving the two arms are behaviourally
/// equivalent (the whole point of keeping the fast path).
#[test]
fn single_threaded_pool_decode_matches_default() {
    let bytes = valid_hdt_bytes();
    let default = sparq_hdt::load_reader(std::io::Cursor::new(bytes.clone()))
        .expect("default many-threaded decode");

    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(1)
        .build()
        .expect("1-thread rayon pool");
    let single = pool
        .install(|| sparq_hdt::load_reader(std::io::Cursor::new(bytes)))
        .expect("single-threaded-pool decode");

    let triples = |g: &sparq_core::Graph| {
        let scan = g.store.scan(&[None, None, None]);
        scan.rows
            .iter()
            .map(|r| {
                let [s, p, o] = scan.to_spo(r);
                [
                    g.dict.term(s).to_string(),
                    g.dict.term(p).to_string(),
                    g.dict.term(o).to_string(),
                ]
            })
            .collect::<std::collections::BTreeSet<_>>()
    };
    assert_eq!(single.store.len(), default.store.len());
    assert_eq!(
        triples(&single),
        triples(&default),
        "the single-threaded dict-decode fast path must produce the identical triple set"
    );
    assert_eq!(
        single.dict.len(),
        default.dict.len(),
        "same distinct-term count on both decode arms"
    );
}
