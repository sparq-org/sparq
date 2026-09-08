//! Term dictionary: bijection between RDF terms and dense `u32` ids.
//!
//! Dictionary encoding is the foundation of every fast triplestore (RDF-3X,
//! QLever, RDFox): triples are stored and joined as fixed-width integers, and the
//! (large, string-heavy) terms live once in the dictionary. Terms are stored COMPACT
//! and prefix-factored (IRIs share a namespace table); the interner is single-storage
//! (the hash table holds only ids) and content-addressed by a hash reproducible from
//! either an `oxrdf::Term` or the parsed byte components — so a byte-level parser can
//! intern straight from slices with no intermediate `Term`.

use hashbrown::HashTable;
use oxrdf::vocab::xsd;
use oxrdf::{Literal, NamedNode, Term};
use rustc_hash::FxHashMap;
use std::hash::Hasher;

/// A dense term id. `u32` (≤ 4.29 B distinct terms) keeps index entries small
/// and cache-friendly; the id space is widened to `u64` only if a dataset needs
/// it. Id 0 is reserved as a sentinel ("no such term").
pub type Id = u32;

pub const NO_ID: Id = 0;

/// Tagged-ValueId base: ids in `[INLINE_BASE, INLINE_BASE + 2^30)` encode an
/// `xsd:integer` whose value is `id - INLINE_BASE` *inline* — no dictionary entry,
/// no string parse. Numeric FILTER / comparison / ORDER BY read the value straight
/// from the id (QLever's value-id idea, kept in `u32` so the index stays compact).
/// Inline integers also sort by value in the permutations (enabling range pruning).
///
/// The `u32` id space is partitioned: dictionary ids `[1, INLINE_BASE)` (≈2.1 billion
/// distinct terms — enough for e.g. full-Wikidata's term count without widening to `u64`,
/// which would double the index), inline integers `[INLINE_BASE, INLINE_BASE + 2^30)`, and
/// the engine's local-vocab ids `[INLINE_BASE + 2^30, 2^32)`. `0` is `NO_ID`.
pub const INLINE_BASE: Id = 1 << 31;
/// The largest value encodable inline (the inline range stays 2^30 wide; bigger integers
/// fall back to the dictionary).
const INLINE_MAX: u32 = (1 << 30) - 1;

/// [OPUS-4.8] (review 1409) On-disk format marker for the mmap dictionary (`dict-meta.bin`).
/// `INLINE_BASE` partitions the `u32` id space, so persisted RAW ids only mean what they say
/// under the SAME partition the file was written with. A file written before `INLINE_BASE`
/// moved (from `1 << 30` to `1 << 31`) encodes inline integers in `[1<<30, 1<<31)`, which the
/// current code would misread as dictionary ids — silent numeric corruption / panics. The
/// header records the partition so `open_mmap` can REJECT a mismatched/legacy store with a
/// clear rebuild-required error instead of silently misinterpreting its ids.
///
/// `dict-meta.bin` previously began with `prefixes.len() as u32` (a small count). This magic
/// is chosen to be distinguishable from any plausible legacy prefix count, so the reader can
/// detect header-less legacy files. ("DMV1" — Dict Meta, V1; little-endian.)
// clippy/dead_code: the on-disk meta header is read/written only by the `mmap`/`dict-spill`
// persistence paths, which are cfg'd out of the default feature set.
#[allow(dead_code)]
pub(crate) const DICT_META_MAGIC: u32 = 0x31_56_4D_44; // b"DMV1" little-endian
/// Bump when the on-disk meta layout changes incompatibly.
#[allow(dead_code)]
pub(crate) const DICT_META_VERSION: u32 = 1;

/// If a literal `value`/`datatype` is a canonical non-negative `xsd:integer` in
/// range, its inline id. Only the canonical lexical form (no leading zeros / sign)
/// inlines, so `"030"^^integer` stays a distinct dictionary term.
#[inline]
fn try_inline_lit(value: &str, datatype: &str) -> Option<Id> {
    if datatype == xsd::INTEGER.as_str() {
        if let Ok(v) = value.parse::<u32>() {
            if v <= INLINE_MAX && v.to_string() == value {
                return Some(INLINE_BASE + v);
            }
        }
    }
    None
}

/// If `term` is a canonical non-negative `xsd:integer` in range, its inline id.
fn try_inline(term: &Term) -> Option<Id> {
    match term {
        Term::Literal(l) => try_inline_lit(l.value(), l.datatype().as_str()),
        _ => None,
    }
}

/// Whether an id encodes an inline integer value. (`INLINE_BASE << 1` would overflow `u32`,
/// so the upper bound is expressed via the inline width.)
#[inline]
pub fn is_inline(id: Id) -> bool {
    id >= INLINE_BASE && id - INLINE_BASE <= INLINE_MAX
}

/// [FABLE-5] (sq-1ivw7) Whether `id` denotes an RDF LITERAL, given the dictionary that owns
/// it. An inline-integer id is always an `xsd:integer` literal; any other id is classified by
/// its stored record (`TermParts::Lit`). `NO_ID` is not a literal. This is the id-level term-kind
/// predicate the engine's predicate-range non-literal inference is built on: an object column of a
/// constant predicate is provably non-literal for a snapshot iff NONE of that predicate's object
/// ids satisfy this. IRIs, blank nodes and triple terms all return `false`.
#[inline]
pub fn is_literal_id(dict: &Dict, id: Id) -> bool {
    if id == NO_ID {
        return false;
    }
    if is_inline(id) {
        return true;
    }
    matches!(dict.term_parts(id), TermParts::Lit { .. })
}

/// The inline id of an integer VALUE in the inline range — the id the canonical
/// `xsd:integer` literal of that value interns/looks up to. The engine's fast path for
/// resolving COMPUTED (BIND/aggregate) integer values to ids without constructing a
/// term, or even its lexical form.
#[inline]
pub fn inline_id_of_int(v: i64) -> Option<Id> {
    if (0..=INLINE_MAX as i64).contains(&v) {
        Some(INLINE_BASE + v as u32)
    } else {
        None
    }
}

/// A compact, single-storage term interner. Each (non-inline) term is stored once, in
/// the `terms` arena, and the hash table holds only its `Id`. IRIs are split into a
/// shared namespace prefix (deduplicated in `prefixes`, the redundancy SPARQL PREFIX
/// directives target) plus a per-term suffix, and literal datatypes are deduplicated in
/// `datatypes` — so a long repeated `http://…/` is stored once, not per term, and the
/// per-term slot (`Stored`) is far smaller than a full `oxrdf::Term`.
/// `Clone` exists for [`fork`](Dict::fork): on an already-forked dict it bumps the
/// shared-base `Arc`s and copies only the small extension; on a flat dict it is the
/// O(n) deep copy that `fork` freezes ONCE into the shared base. It is not intended
/// as a general-purpose copy (a `Graph` remains deliberately non-`Clone`).
#[derive(Default, Clone)]
pub struct Dict {
    prefixes: Vec<Box<str>>,                  // id -> IRI namespace prefix
    prefix_ids: FxHashMap<Box<str>, u32>,     // prefix -> id
    datatypes: Vec<NamedNode>,                // id -> literal datatype (a small set)
    datatype_ids: FxHashMap<Box<str>, u32>,   // datatype IRI -> id
    terms: Vec<Stored>,                       // id-1 -> compact term (empty once compacted)
    table: HashTable<Id>,                     // hash(term) -> id (bare ids, compared via the arena)
    // When `Some` (after `into_blob`), id->term is served from a single concatenated term
    // BLOB + per-term offsets instead of `Vec<Stored>` — no per-term `Box<str>` allocation
    // overhead, ~half the resident dict bytes. `table` is kept (lookup verifies via the
    // blob); `terms` is empty. The memory-bound (browser) storage mode for the dictionary.
    blob: Option<(Vec<u8>, Vec<u32>)>,
    // When `Some` (after `open_mmap`), term(id)/term_parts/lookup are served from mmap'd
    // files and `terms`/`table` are empty — the out-of-core, minimal-RAM dictionary.
    // Behind an `Arc` so a fork shares the (immutable) mapping instead of re-mmapping.
    #[cfg(feature = "mmap")]
    mapped: Option<std::sync::Arc<MappedDict>>,
    // APPEND-ONLY growth over the compacted storage modes (delta-overlay updates, T17):
    // ids `1..=base` are served by the blob / mmap'd record store; freshly interned terms
    // go to the `terms` arena with ids `base + i + 1`. Always 0 in the plain arena mode,
    // so the arena hot paths are unchanged (one predictable comparison).
    base: usize,
    // The SHARED IMMUTABLE BASE of a forked dictionary ([`fork`](Dict::fork)): ids
    // `1..=base` resolve through this complete, never-mutated dict (Arc-shared across
    // snapshot generations); freshly interned terms append to the local `terms` arena
    // above `base`, exactly like the blob/mmap growth modes. `None` for every
    // non-forked dict, so the bulk-load and flat-read hot paths pay only a predictable
    // never-taken branch. INVARIANT: a frozen base is itself flat (`frozen: None`),
    // so delegation never recurses more than one level; the fork's `prefixes` /
    // `datatypes` tables start as clones of the base's, keeping base record prefix /
    // datatype indices valid against the fork's tables.
    frozen: Option<std::sync::Arc<Dict>>,
}

/// The compact per-term storage. An IRI is `(prefix id, suffix)`; a literal is
/// `(value, datatype id, optional language)`; a blank node is its label. An RDF 1.2
/// TRIPLE TERM (`<<( s p o )>>`) is stored STRUCTURALLY as the ids of its three
/// components (which are interned first, so a child id is always lower than — or an
/// inline id distinct from — the triple's own id; nesting recurses through the object).
#[derive(Clone)]
enum Stored {
    Iri { prefix: u32, suffix: Box<str> },
    Lit { value: Box<str>, datatype: u32, lang: Option<Box<str>> },
    Blank(Box<str>),
    Triple([Id; 3]),
}

/// A borrowed view of a dictionary term's string components (no allocation), for
/// serialising results directly from ids. An IRI is its namespace prefix + local
/// suffix; a literal its lexical value, datatype IRI and optional language.
pub enum TermParts<'a> {
    Iri { prefix: &'a str, suffix: &'a str },
    Lit { value: &'a str, datatype: &'a str, lang: Option<&'a str> },
    Blank(&'a str),
    /// An RDF 1.2 triple term: the ids of its subject/predicate/object (resolve each via
    /// `term_parts`/`term` recursively — a child may also be an inline-integer id).
    Triple([Id; 3]),
}

// ---- Content hashing ---------------------------------------------------------
// A term's hash must be identical whether computed from an `oxrdf::Term`, from parsed
// byte slices, or from the compact `Stored` arena form — so interning and rehashing
// never build a `Term` or concatenate an IRI. FxHasher is deterministic (no seed).

#[inline]
fn hash_iri_parts(prefix: &str, suffix: &str) -> u64 {
    // Length-prefix the prefix and make hash_iri route through here on the SAME split,
    // so the two paths issue an identical sequence of writes — FxHasher is word-chunked,
    // so `write(a); write(b)` does NOT equal `write(a++b)`; identical call sequences do.
    let mut h = rustc_hash::FxHasher::default();
    h.write_u8(0);
    h.write_usize(prefix.len());
    h.write(prefix.as_bytes());
    h.write(suffix.as_bytes());
    h.finish()
}

#[inline]
fn hash_iri(iri: &str) -> u64 {
    let (prefix, suffix) = split_iri(iri);
    hash_iri_parts(prefix, suffix)
}

#[inline]
fn hash_lit(value: &str, datatype: &str, lang: Option<&str>) -> u64 {
    let mut h = rustc_hash::FxHasher::default();
    h.write_u8(1);
    h.write_usize(value.len());
    h.write(value.as_bytes());
    h.write_usize(datatype.len());
    h.write(datatype.as_bytes());
    match lang {
        Some(l) => {
            h.write_u8(1);
            h.write(l.as_bytes());
        }
        None => h.write_u8(0),
    }
    h.finish()
}

#[inline]
fn hash_blank(label: &str) -> u64 {
    let mut h = rustc_hash::FxHasher::default();
    h.write_u8(2);
    h.write(label.as_bytes());
    h.finish()
}

/// Content hash of a structurally stored triple term — over the (already-interned) ids of
/// its components, NOT strings. Unlike the other term kinds this hash is dict-relative
/// (the ids are), so a `Term::Triple` is hashed only AFTER its children are resolved in
/// the target dict (`intern`/`lookup` handle this; `hash_term` cannot).
/// [OPUS-4.8] (sq-jvbr) `pub(crate)` so the dict-spill external builder can content-address
/// an RDF 1.2 triple term by its components' FINAL dense ids when it assembles the
/// triple-term dictionary records (the same `(hash, id)` pair `save_mmap`/`into_merged`
/// produce), keeping `dict-hash.bin`/`dict-hid.bin` consistent for triple terms too.
#[inline]
pub(crate) fn hash_triple_ids(ids: [Id; 3]) -> u64 {
    let mut h = rustc_hash::FxHasher::default();
    h.write_u8(3);
    h.write_u32(ids[0]);
    h.write_u32(ids[1]);
    h.write_u32(ids[2]);
    h.finish()
}

/// NOTE: returns a placeholder for `Term::Triple` — a triple term's hash is over its
/// component IDS (dict-relative), so callers must resolve those first (see
/// `Dict::intern` / `Dict::lookup`, which intercept `Term::Triple` before hashing).
/// The STORED language slot of a literal: the BCP47 tag, with an RDF 1.2 base direction
/// appended as `lang--dir` (the SPARQL/Turtle surface syntax — `--` can never occur inside
/// a valid language tag, so the encoding is unambiguous). One string field keeps the
/// storage layout, hashing and equality unchanged; [`reconstruct_ref`] splits it back out.
#[inline]
// [OPUS-4.8] (T1) `pub(crate)` so the Turtle chunk worker's borrowed-slice interner
// (`intern_object_ref` in lib.rs) builds the EXACT same lang(+base-direction) string this
// module's `Dict::intern` uses — required for intern/hash parity (a directional literal must
// land on the identical id whether interned via the owned-Term path or the borrowed path).
pub(crate) fn lang_with_dir(l: &Literal) -> Option<std::borrow::Cow<'_, str>> {
    match l.direction() {
        Some(d) => Some(std::borrow::Cow::Owned(format!("{}--{d}", l.language().unwrap_or("")))),
        None => l.language().map(std::borrow::Cow::Borrowed),
    }
}

/// Inverse of `lang_with_dir`: split a STORED language slot back into its BCP47 tag and
/// (optional) RDF 1.2 base direction. A stored slot of `en--ltr` is `("en", Some("ltr"))`;
/// a plain `en` is `("en", None)`. The returned tag/direction are the spec-distinct
/// components the SPARQL 1.2 results formats keep apart (`xml:lang` vs `its:dir`) — see the
/// engine's `json.rs` and the conformance results reader.
///
/// [OPUS-4.8] sq-bj7o: VALIDATION. The base direction in RDF 1.2 / SPARQL 1.2 is exactly
/// `ltr` or `rtl`, lowercase (oxrdf's `BaseDirection` enum, whose `Display` emits those two
/// strings — the only suffixes `lang_with_dir` ever writes). A trusted store therefore only
/// ever holds `lang--ltr` / `lang--rtl`. But a STORED slot can come from an untrusted/mmap'd
/// index, where the suffix after `--` is attacker-controlled; treating *any* suffix as a
/// direction would (a) emit an arbitrary `its:dir` value and (b) diverge from the
/// materialised `oxrdf::Term` path. So a suffix that is not exactly `ltr`/`rtl` is NOT a
/// direction: the whole slot is returned as the language tag and no direction is reported —
/// the same decision the materialised `reconstruct_ref` path makes (both call
/// [`parse_base_direction`]), so the fast path and the materialised path AGREE.
#[inline]
pub fn split_lang_dir(slot: &str) -> (&str, Option<&str>) {
    match slot.split_once("--") {
        Some((tag, dir)) if parse_base_direction(dir).is_some() => (tag, Some(dir)),
        // No `--`, or a `--suffix` that is not a valid base direction: the entire slot is the
        // language tag and there is no direction. Keeping the full slot intact (rather than
        // dropping the bogus suffix) matches `reconstruct_ref`'s plain-language-tag fallback.
        _ => (slot, None),
    }
}

/// [OPUS-4.8] sq-bj7o / sq-s955: the single source of truth for "is this RDF 1.2 base
/// direction string valid?" — the base-direction validator for the whole stack. Takes a
/// candidate direction string (NOT a `lang--dir` slot; just the direction component) and
/// returns the typed [`oxrdf::BaseDirection`] when it is one of the two valid values, else
/// `None`. The two possible directions are `ltr` / `rtl`, lowercase and case-sensitive
/// (matching oxrdf `BaseDirection::Display` and the W3C RDF 1.2 / SPARQL 1.2 grammar — `LTR`
/// / `Ltr` are NOT directions).
///
/// Returning the typed enum lets every direction-bearing path share exactly one definition
/// and never drift apart:
/// - stored-slot fast path — [`split_lang_dir`] (splits off the `--dir` suffix, then validates
///   it here);
/// - materialised path — `reconstruct_ref` (same split-then-validate of a stored slot);
/// - outbound SPARQL 1.2 results — `sparq-engine`'s `json.rs` (emits the validated direction
///   as a separate `its:dir` field);
/// - inbound SERVICE results parser — `sparq-engine`'s `service.rs` (sq-s955), where `its:dir`
///   arrives as its own JSON/XML field rather than a slot suffix and is validated directly
///   through this function.
///
/// With each caller's fallback, all paths also agree on what an *invalid* direction degrades
/// to (a plain language-tagged literal).
#[inline]
pub fn parse_base_direction(dir: &str) -> Option<oxrdf::BaseDirection> {
    match dir {
        "ltr" => Some(oxrdf::BaseDirection::Ltr),
        "rtl" => Some(oxrdf::BaseDirection::Rtl),
        _ => None,
    }
}

#[inline]
fn hash_term(t: &Term) -> u64 {
    match t {
        Term::NamedNode(n) => hash_iri(n.as_str()),
        Term::Literal(l) => hash_lit(l.value(), l.datatype().as_str(), lang_with_dir(l).as_deref()),
        Term::BlankNode(b) => hash_blank(b.as_str()),
        _ => 0,
    }
}

/// Content hash of a borrowed term — same value as `hash_term` for the same term, computed
/// from already-split components (no `Term`, no IRI concat). Used to route the sharded
/// interner without reconstructing a `Term` per occurrence.
pub(crate) fn hash_termparts(tp: &TermParts) -> u64 {
    match tp {
        TermParts::Iri { prefix, suffix } => hash_iri_parts(prefix, suffix),
        TermParts::Lit { value, datatype, lang } => hash_lit(value, datatype, *lang),
        TermParts::Blank(b) => hash_blank(b),
        // Dict-relative (ids of the SOURCE dict): only meaningful within one dict.
        // The cross-dict parts paths (`intern_partials`) reject triple terms explicitly.
        TermParts::Triple(ids) => hash_triple_ids(*ids),
    }
}

#[inline]
fn hash_stored(s: &Stored, prefixes: &[Box<str>], datatypes: &[NamedNode]) -> u64 {
    match s {
        Stored::Iri { prefix, suffix } => hash_iri_parts(&prefixes[*prefix as usize], suffix),
        Stored::Lit { value, datatype, lang } => hash_lit(value, datatypes[*datatype as usize].as_str(), lang.as_deref()),
        Stored::Blank(b) => hash_blank(b),
        Stored::Triple(ids) => hash_triple_ids(*ids),
    }
}

/// Splits an IRI into (namespace prefix, local suffix) at the last `#` or `/`, the
/// boundary that captures the shared namespace. IRIs with neither get an empty prefix.
#[inline]
fn split_iri(iri: &str) -> (&str, &str) {
    let cut = iri.rfind(['#', '/']).map_or(0, |i| i + 1);
    iri.split_at(cut)
}

// ---- Stored vs query-component equality (no reconstruction on the hot path) ---

#[inline]
fn stored_is_iri(s: &Stored, iri: &str, prefixes: &[Box<str>]) -> bool {
    match s {
        Stored::Iri { prefix, suffix } => {
            let p = &prefixes[*prefix as usize];
            iri.len() == p.len() + suffix.len() && iri.as_bytes().starts_with(p.as_bytes()) && iri[p.len()..] == **suffix
        }
        _ => false,
    }
}

#[inline]
fn stored_is_iri_parts(s: &Stored, prefix: &str, suffix: &str, prefixes: &[Box<str>]) -> bool {
    match s {
        Stored::Iri { prefix: pid, suffix: suf } => prefixes[*pid as usize].as_ref() == prefix && **suf == *suffix,
        _ => false,
    }
}

#[inline]
fn stored_is_lit(s: &Stored, value: &str, datatype: &str, lang: Option<&str>, datatypes: &[NamedNode]) -> bool {
    match s {
        Stored::Lit { value: v, datatype: dt, lang: lg } => {
            **v == *value && lg.as_deref() == lang && datatypes[*dt as usize].as_str() == datatype
        }
        _ => false,
    }
}

/// NOTE: `Term::Triple` always misses here — triple terms are matched by their component
/// IDS (see `Dict::lookup`, which resolves the children and intercepts before this path).
#[inline]
fn stored_eq_term(s: &Stored, q: &Term, prefixes: &[Box<str>], datatypes: &[NamedNode]) -> bool {
    match q {
        Term::NamedNode(n) => stored_is_iri(s, n.as_str(), prefixes),
        Term::Literal(l) => stored_is_lit(s, l.value(), l.datatype().as_str(), lang_with_dir(l).as_deref(), datatypes),
        Term::BlankNode(b) => matches!(s, Stored::Blank(x) if **x == *b.as_str()),
        _ => false,
    }
}

/// [OPUS-4.8] sq-cvug — the safe placeholders `reconstruct_triple` substitutes when a
/// triple-term component id is IN RANGE but resolves to the WRONG term KIND (e.g. a
/// literal id in the subject/predicate position). This can only arise from a tampered,
/// checksum-less mmap'd index (threat-model B5 / T-MMAP-DoS); the trusted in-process
/// interner only ever stores a valid `(IRI|blank, IRI, any-term)` shape. Decoding such a
/// record to a placeholder is wrong-but-safe — the documented trusted-store integrity
/// boundary — never a panic/DoS. Mirrors the [`OOR_TERM`] convention for out-of-range ids.
const OOR_TRIPLE_SUBJECT: &str = "__sparq_wrong_kind_triple_subject__";
const OOR_TRIPLE_PREDICATE: &str = "urn:sparq:wrong-kind-triple-predicate";

/// Rebuilds an RDF 1.2 triple term from its stored component ids. Interning only ever
/// stores a valid (IRI|blank, IRI, any-term) shape, so on ids produced by this dict the
/// inner matches always take a valid arm.
///
/// [OPUS-4.8] sq-cvug — on the UNTRUSTED mmap path a tampered index can carry a component
/// id that is in range yet of the wrong kind (a literal where the subject must be IRI/blank
/// or the predicate an IRI). That used to hit `unreachable!()` → a clean panic (DoS) on a
/// hostile/corrupt file. Fail closed instead: substitute a safe placeholder of the required
/// kind so a corrupt store yields wrong-but-safe results, never an abort. `open_mmap`
/// additionally rejects such an index up front (`MappedDict::validate`).
///
/// `depth` bounds the triple-term nesting recursion (see [`term_at_depth`] / `MAX_TRIPLE_DEPTH`)
/// so a tampered cyclic/self component reference cannot blow the stack.
fn reconstruct_triple(d: &Dict, ids: [Id; 3], depth: u32) -> Term {
    if depth >= MAX_TRIPLE_DEPTH {
        // Pathological (only-from-tampering) nesting depth — stop recursing and return a
        // safe, self-contained placeholder triple rather than risk a stack overflow.
        return Term::Triple(Box::new(oxrdf::Triple::new(
            oxrdf::BlankNode::new_unchecked(OOR_TRIPLE_SUBJECT),
            NamedNode::new_unchecked(OOR_TRIPLE_PREDICATE),
            Term::BlankNode(oxrdf::BlankNode::new_unchecked(OOR_TERM_LABEL)),
        )));
    }
    let subject: oxrdf::NamedOrBlankNode = match term_at_depth(d, ids[0], depth + 1) {
        Term::NamedNode(n) => n.into(),
        Term::BlankNode(b) => b.into(),
        _ => oxrdf::BlankNode::new_unchecked(OOR_TRIPLE_SUBJECT).into(),
    };
    let predicate = match term_at_depth(d, ids[1], depth + 1) {
        Term::NamedNode(n) => n,
        _ => NamedNode::new_unchecked(OOR_TRIPLE_PREDICATE),
    };
    Term::Triple(Box::new(oxrdf::Triple::new(subject, predicate, term_at_depth(d, ids[2], depth + 1))))
}

// ---- Memory-mapped (out-of-core) dictionary --------------------------------------
// The in-memory `Dict` holds `terms: Vec<Stored>` (~1 GB+ at 100M terms) plus the
// lookup `HashTable`, and rebuilds both on open. For querying a dataset whose indexes
// are already mmap'd, that resident RAM (and the rebuild time) is the last big cost.
// `MappedDict` instead serves term(id)/term_parts/lookup straight from mmap'd files —
// nothing big is resident, and open is just `mmap` (no parse, no table rebuild).

/// A borrowed view of a compact stored term, parsed zero-copy from a term BLOB (the
/// concatenated record format shared by the in-memory compacted dict and the mmap'd
/// out-of-core dict). Mirrors [`Stored`] but with `&str` slices into the blob.
pub(crate) enum StoredRef<'a> {
    Iri { prefix: u32, suffix: &'a str },
    Lit { value: &'a str, datatype: u32, lang: Option<&'a str> },
    Blank(&'a str),
    Triple([Id; 3]),
}

/// [OPUS-4.8] sq-ky2a — the safe placeholder `record` returns for an OUT-OF-RANGE id (one
/// that can only arise from a tampered, checksum-less permutation file; see `Dict::record`).
/// A `Blank` needs no table lookup, so reconstructing it can never itself OOB. Decoding it
/// is wrong-but-safe — exactly the documented trusted-store integrity boundary, never UB.
const OOR_TERM_LABEL: &str = "__sparq_out_of_range_id__";
const OOR_TERM: StoredRef<'static> = StoredRef::Blank(OOR_TERM_LABEL);

#[inline]
fn rd_u32(b: &[u8], p: &mut usize) -> u32 {
    let v = u32::from_le_bytes([b[*p], b[*p + 1], b[*p + 2], b[*p + 3]]);
    *p += 4;
    v
}

#[inline]
fn rd_str<'a>(b: &'a [u8], p: &mut usize) -> &'a str {
    let n = rd_u32(b, p) as usize;
    let s = &b[*p..*p + n];
    *p += n;
    // SAFETY: written from a `&str` in `save_mmap`; the blob is immutable. This is the
    // TRUSTED fast path: it parses an in-process-built blob (the in-memory compacted dict
    // / forked base), whose records are GUARANTEED valid UTF-8 by construction. The
    // untrusted mmap path (a hostile/corrupt `.spq` file) MUST NOT reach here — it uses
    // the bounds-/UTF-8-checked `rd_str_checked` below and is fully validated at open
    // (`Dict::open_mmap` → `MappedDict::validate`). See bead sq-znld. [OPUS-4.8]
    unsafe { std::str::from_utf8_unchecked(s) }
}

/// Parses one term record (the blob record format) at `b[0..]`.
#[inline]
fn parse_stored_ref(b: &[u8]) -> StoredRef<'_> {
    let mut p = 1;
    match b[0] {
        0 => {
            let prefix = rd_u32(b, &mut p);
            StoredRef::Iri { prefix, suffix: rd_str(b, &mut p) }
        }
        1 => {
            let value = rd_str(b, &mut p);
            let datatype = rd_u32(b, &mut p);
            let lang = if b[p] == 1 {
                p += 1;
                Some(rd_str(b, &mut p))
            } else {
                None
            };
            StoredRef::Lit { value, datatype, lang }
        }
        3 => StoredRef::Triple([rd_u32(b, &mut p), rd_u32(b, &mut p), rd_u32(b, &mut p)]),
        _ => StoredRef::Blank(rd_str(b, &mut p)),
    }
}

// [OPUS-4.8] sq-znld — bounds-/UTF-8-CHECKED record parser for the UNTRUSTED mmap path.
// `parse_stored_ref` / `rd_str` use `from_utf8_unchecked` and unguarded slice indexing,
// which is sound ONLY for in-process-built blobs. A memory-mapped `.spq` file is
// attacker-controlled: an out-of-range offset/length panics (DoS) and a non-UTF-8 byte
// run fed to `from_utf8_unchecked` is immediate UNDEFINED BEHAVIOUR. These `*_checked`
// helpers return `None` on ANY malformation (short read, OOB length, non-UTF-8) so the
// mmap loader can reject a hostile file with a clean `Err` instead — never UB, never a
// panic, never a mis-decoded term.
// [OPUS-4.8] The `*_checked` helpers only serve the mmap loader (gated below); gate them
// too so non-mmap builds don't trip `dead_code` / `-D warnings`.
#[cfg(feature = "mmap")]
#[inline]
fn rd_u32_checked(b: &[u8], p: &mut usize) -> Option<u32> {
    let end = p.checked_add(4)?;
    let v = u32::from_le_bytes(b.get(*p..end)?.try_into().ok()?);
    *p = end;
    Some(v)
}

#[cfg(feature = "mmap")]
#[inline]
fn rd_str_checked<'a>(b: &'a [u8], p: &mut usize) -> Option<&'a str> {
    let n = rd_u32_checked(b, p)? as usize;
    let end = p.checked_add(n)?;
    let s = std::str::from_utf8(b.get(*p..end)?).ok()?;
    *p = end;
    Some(s)
}

/// Parses one term record with FULL bounds + UTF-8 validation; `None` on any malformation.
/// Used for the untrusted memory-mapped term blob (see [`rd_str_checked`]). [OPUS-4.8]
#[cfg(feature = "mmap")]
#[inline]
fn parse_stored_ref_checked(b: &[u8]) -> Option<StoredRef<'_>> {
    let mut p = 1;
    Some(match *b.first()? {
        0 => {
            let prefix = rd_u32_checked(b, &mut p)?;
            StoredRef::Iri { prefix, suffix: rd_str_checked(b, &mut p)? }
        }
        1 => {
            let value = rd_str_checked(b, &mut p)?;
            let datatype = rd_u32_checked(b, &mut p)?;
            let lang = match *b.get(p)? {
                1 => {
                    p += 1;
                    Some(rd_str_checked(b, &mut p)?)
                }
                0 => None,
                _ => return None, // the lang-present flag is a strict 0/1
            };
            StoredRef::Lit { value, datatype, lang }
        }
        3 => StoredRef::Triple([rd_u32_checked(b, &mut p)?, rd_u32_checked(b, &mut p)?, rd_u32_checked(b, &mut p)?]),
        2 => StoredRef::Blank(rd_str_checked(b, &mut p)?),
        _ => return None, // unknown record tag
    })
}

fn reconstruct_ref(d: &Dict, s: &StoredRef) -> Term {
    match *s {
        StoredRef::Iri { prefix, suffix } => {
            let p = &d.prefixes[prefix as usize];
            let mut iri = String::with_capacity(p.len() + suffix.len());
            iri.push_str(p);
            iri.push_str(suffix);
            Term::NamedNode(NamedNode::new_unchecked(iri))
        }
        StoredRef::Lit { value, datatype, lang } => Term::Literal(match lang {
            // A stored `lang--dir` slot is an RDF 1.2 directional language-tagged string
            // (see `lang_with_dir`); a plain tag is an ordinary one. [OPUS-4.8] sq-bj7o: the
            // suffix is only a base direction when it is exactly `ltr`/`rtl`
            // (`parse_base_direction`) — the SAME validation `split_lang_dir` (the JSON fast
            // path) applies, so a malformed slot like `en--bogus` from an untrusted/mmap'd
            // index decodes to a PLAIN language-tagged literal with tag `en--bogus` here AND
            // to `("en--bogus", None)` there. Previously any non-`rtl` suffix was silently
            // coerced to `ltr`, which both fabricated a direction and diverged from the split.
            Some(l) => match l.split_once("--").and_then(|(tag, dir)| parse_base_direction(dir).map(|d| (tag, d))) {
                Some((tag, dir)) => Literal::new_directional_language_tagged_literal_unchecked(
                    value.to_string(),
                    tag.to_string(),
                    dir,
                ),
                None => Literal::new_language_tagged_literal_unchecked(value.to_string(), l.to_string()),
            },
            None => Literal::new_typed_literal(value.to_string(), d.datatypes[datatype as usize].clone()),
        }),
        StoredRef::Blank(b) => Term::BlankNode(oxrdf::BlankNode::new_unchecked(b.to_string())),
        StoredRef::Triple(ids) => reconstruct_triple(d, ids, 0),
    }
}

/// [OPUS-4.8] sq-cvug — depth-bounded term resolution for the triple-term reconstruction
/// recursion. A validly-built store nests triple terms only shallowly (each is interned
/// after its components, so the structure is a finite DAG), but a TAMPERED mmap'd index
/// could encode a self/cyclic component reference that would recurse without bound →
/// stack overflow → process abort. `open_mmap` rejects such an index up front
/// (`MappedDict::validate` enforces component-id < own-id), but the reconstruct path is the
/// fail-closed FLOOR for every storage mode: past this depth we stop and return a safe
/// placeholder rather than recurse further.
const MAX_TRIPLE_DEPTH: u32 = 64;

#[inline]
fn term_at_depth(d: &Dict, id: Id, depth: u32) -> Term {
    if is_inline(id) {
        return Term::Literal(Literal::new_typed_literal((id - INLINE_BASE).to_string(), xsd::INTEGER));
    }
    match d.record(id) {
        StoredRef::Triple(ids) => reconstruct_triple(d, ids, depth),
        other => reconstruct_ref(d, &other),
    }
}

/// NOTE: like `stored_eq_term`, `Term::Triple` always misses here — `Dict::lookup`
/// resolves a triple term's component ids first and matches `StoredRef::Triple` by id.
fn stored_ref_eq_term(s: &StoredRef, q: &Term, prefixes: &[Box<str>], datatypes: &[NamedNode]) -> bool {
    match (s, q) {
        (StoredRef::Iri { prefix, suffix }, Term::NamedNode(n)) => {
            let p = &prefixes[*prefix as usize];
            let iri = n.as_str();
            iri.len() == p.len() + suffix.len() && iri.starts_with(p.as_ref()) && iri[p.len()..] == **suffix
        }
        (StoredRef::Lit { value, datatype, lang }, Term::Literal(l)) => {
            *value == l.value()
                && lang.as_deref() == lang_with_dir(l).as_deref()
                && datatypes[*datatype as usize].as_str() == l.datatype().as_str()
        }
        (StoredRef::Blank(x), Term::BlankNode(b)) => *x == b.as_str(),
        _ => false,
    }
}

/// A zero-copy `StoredRef` view of an arena `Stored` — so the appended-term (arena-over-
/// blob/mmap) paths can share the `StoredRef`-based comparison/serialisation code.
#[inline]
fn stored_as_ref(s: &Stored) -> StoredRef<'_> {
    match s {
        Stored::Iri { prefix, suffix } => StoredRef::Iri { prefix: *prefix, suffix },
        Stored::Lit { value, datatype, lang } => {
            StoredRef::Lit { value, datatype: *datatype, lang: lang.as_deref() }
        }
        Stored::Blank(b) => StoredRef::Blank(b),
        Stored::Triple(ids) => StoredRef::Triple(*ids),
    }
}

/// Content hash of a `StoredRef` record — same value as `hash_stored` for the same term.
#[inline]
fn hash_stored_ref(s: &StoredRef, prefixes: &[Box<str>], datatypes: &[NamedNode]) -> u64 {
    match s {
        StoredRef::Iri { prefix, suffix } => hash_iri_parts(&prefixes[*prefix as usize], suffix),
        StoredRef::Lit { value, datatype, lang } => hash_lit(value, datatypes[*datatype as usize].as_str(), *lang),
        StoredRef::Blank(b) => hash_blank(b),
        StoredRef::Triple(ids) => hash_triple_ids(*ids),
    }
}

// `StoredRef` counterparts of the stored-vs-query-component comparisons, for the
// blob/mmap'd base records consulted by the append-capable intern paths.

#[inline]
fn stored_ref_is_iri(s: &StoredRef, iri: &str, prefixes: &[Box<str>]) -> bool {
    match s {
        StoredRef::Iri { prefix, suffix } => {
            let p = &prefixes[*prefix as usize];
            iri.len() == p.len() + suffix.len() && iri.as_bytes().starts_with(p.as_bytes()) && iri[p.len()..] == **suffix
        }
        _ => false,
    }
}

#[inline]
fn stored_ref_is_iri_parts(s: &StoredRef, prefix: &str, suffix: &str, prefixes: &[Box<str>]) -> bool {
    match s {
        StoredRef::Iri { prefix: pid, suffix: suf } => prefixes[*pid as usize].as_ref() == prefix && *suf == suffix,
        _ => false,
    }
}

#[inline]
fn stored_ref_is_lit(s: &StoredRef, value: &str, datatype: &str, lang: Option<&str>, datatypes: &[NamedNode]) -> bool {
    match s {
        StoredRef::Lit { value: v, datatype: dt, lang: lg } => {
            *v == value && *lg == lang && datatypes[*dt as usize].as_str() == datatype
        }
        _ => false,
    }
}

/// Content hash of any id present in the lookup `table` (blob-base or appended-arena —
/// mapped-base ids are never in the table; they are found via the mmap'd sorted index).
/// Free function so it can serve hashbrown's rehash closures while `table` is borrowed.
#[inline]
fn hash_tabled(
    id: Id,
    base: usize,
    terms: &[Stored],
    blob: &Option<(Vec<u8>, Vec<u32>)>,
    prefixes: &[Box<str>],
    datatypes: &[NamedNode],
) -> u64 {
    let i = (id - 1) as usize;
    if i >= base {
        return hash_stored(&terms[i - base], prefixes, datatypes);
    }
    let (b, offs) = blob.as_ref().expect("a tabled id below `base` must be a blob id");
    hash_stored_ref(&parse_stored_ref(&b[offs[i] as usize..]), prefixes, datatypes)
}

/// The mmap-backed term store + lookup index (out-of-core dictionary).
#[cfg(feature = "mmap")]
struct MappedDict {
    blob: memmap2::Mmap,    // concatenated term records (the format `save_mmap` writes)
    offsets: memmap2::Mmap, // [u64; n] byte offset of each term record in `blob`
    hashes: memmap2::Mmap,  // [u64; n] content hashes, SORTED (for lookup)
    hashids: memmap2::Mmap, // [u32; n] term ids parallel to `hashes`
}

#[cfg(feature = "mmap")]
impl MappedDict {
    #[inline]
    fn slice_u64(m: &memmap2::Mmap) -> &[u64] {
        // SAFETY: `write_pod_slice` wrote these as raw NATIVE-endian u64 (little-endian on
        // every supported target — pinned by `DICT_ON_DISK_LE_GUARD`, sq-lvw8); mmap base is
        // page-aligned (>= 8). This pointer cast reads them back in the same native order.
        unsafe { std::slice::from_raw_parts(m.as_ptr().cast::<u64>(), m.len() / 8) }
    }
    #[inline]
    fn offsets(&self) -> &[u64] {
        Self::slice_u64(&self.offsets)
    }
    #[inline]
    fn hashes(&self) -> &[u64] {
        Self::slice_u64(&self.hashes)
    }
    #[inline]
    fn hashids(&self) -> &[u32] {
        // SAFETY: `write_pod_slice` wrote these as raw NATIVE-endian u32 (little-endian on
        // every supported target — pinned by `DICT_ON_DISK_LE_GUARD`, sq-lvw8); mmap base is
        // page-aligned (>= 4). This pointer cast reads them back in the same native order.
        unsafe { std::slice::from_raw_parts(self.hashids.as_ptr().cast::<u32>(), self.hashids.len() / 4) }
    }
    /// The parsed term record for a 1-based id.
    ///
    /// [OPUS-4.8] sq-znld: this reads from an UNTRUSTED memory-mapped blob, so it uses the
    /// bounds-/UTF-8-CHECKED parser. `Dict::open_mmap` runs [`validate`](Self::validate)
    /// up front, which proves every offset is in range and every record parses, so by the
    /// time `stored` is reached the `expect` is structurally impossible — but the checked
    /// parse remains as a sound floor (no `from_utf8_unchecked` over attacker bytes, ever),
    /// and the cost is negligible against the mmap page-fault that produced the bytes.
    #[inline]
    fn stored(&self, id: Id) -> StoredRef<'_> {
        let off = self.offsets()[(id - 1) as usize] as usize;
        parse_stored_ref_checked(self.blob.get(off..).unwrap_or_default())
            .expect("mmap dict record validated at open (sq-znld); a failure here means the mapped file changed under us")
    }

    /// [OPUS-4.8] sq-znld — validate the four UNTRUSTED data files at open time so the
    /// hostile-on-disk-file boundary (B5 in research/threat-model.md, T-MMAP-UB) never
    /// reaches `unsafe`/unchecked code with attacker-controlled bytes.
    ///
    /// `len` is the term count read from the (already magic/version-validated) meta file.
    /// We require:
    ///   * `dict-offs.bin` is exactly `len * 8` bytes (one `u64` offset per term),
    ///   * `dict-hash.bin` is exactly `len * 8` bytes and `dict-hid.bin` exactly `len * 4`
    ///     (the parallel sorted lookup arrays — a mismatch would make `mapped_find`'s
    ///     binary search read past the hashids array),
    ///   * every offset is `<= blob.len()` AND the record at that offset parses fully with
    ///     valid UTF-8 (via [`parse_stored_ref_checked`]),
    ///   * every record's `prefix` / `datatype` index is within the prefix / datatype
    ///     tables, and every `Triple` component id is a valid 1-based term id — otherwise a
    ///     later `reconstruct_ref` / `hash_stored_ref` would index those tables out of
    ///     bounds (a panic) on a hostile record.
    ///
    /// Any violation is a clean `InvalidData` error — never a panic, never UB.
    #[cfg(feature = "mmap")]
    fn validate(&self, len: usize, prefixes: &[Box<str>], datatypes: &[NamedNode]) -> std::io::Result<()> {
        // [OPUS-4.8] sq-ueuk — delegate to the mmap-free `&[u8]` seam below. The four
        // mmap'd files are passed as plain byte slices (a `memmap2::Mmap` derefs to `&[u8]`),
        // so ALL of the header/length/bounds/record/index logic lives in a pure function
        // that has no `mmap`/file-I/O/FFI dependency and can be Kani-proof-harnessed (the
        // MS-G4 follow-up the .spqv validator already enjoys — see .github/workflows/kani.yml).
        //
        // [OPUS-4.8] sq-lvw8 (sq-ueuk/#418 verify note) — the seam reads the offset/hashid
        // arrays with EXPLICIT `from_le_bytes`, while the runtime mmap readers (`slice_u64` /
        // `hashids`) reinterpret the same bytes with a NATIVE-endian pointer cast. PR #418's
        // claim that this delegation is "byte-for-byte unchanged" from the old inline validator
        // therefore holds only on a LITTLE-ENDIAN target, where native == LE. That precondition
        // is enforced for the whole persistence format by `DICT_ON_DISK_LE_GUARD` (a build error
        // on a big-endian target), so on every platform this validator actually runs on, the
        // explicit-LE seam and the native-endian readers observe identical integers.
        validate_dict_bytes(&self.blob, &self.offsets, &self.hashes, &self.hashids, len, prefixes, datatypes)
    }
}

/// [OPUS-4.8] sq-ueuk — the mmap-FREE core of [`MappedDict::validate`].
///
/// Takes the four on-disk regions as plain `&[u8]` (a `memmap2::Mmap` derefs to one) so the
/// whole validator is a pure function of its byte inputs: no `mmap`, no file I/O, no FFI, no
/// `unsafe`. That makes it BOTH unit-testable from in-memory buffers AND amenable to a Kani
/// bounded proof (`#[cfg(kani)]` harness below), exactly like `VectorStore::open_from_bytes`
/// is the mmap-free twin of the `.spqv` validator (the MS-G4 gap; sq-hkud, epic sq-toze).
///
/// The byte regions:
///   * `blob`    — the concatenated term records (`dict-terms.bin`),
///   * `offsets` — `len` little-endian `u64` byte offsets into `blob` (`dict-offs.bin`),
///   * `hashes`  — `len` little-endian `u64` content hashes (`dict-hash.bin`); only its
///     length is load-bearing here (it is binary-searched, not indexed, at query time),
///   * `hashids` — `len` little-endian `u32` term ids parallel to `hashes` (`dict-hid.bin`).
///
/// `len` is the term count read from the (already magic/version-validated) meta file.
/// We require:
///   * `dict-offs.bin` is exactly `len * 8` bytes (one `u64` offset per term),
///   * `dict-hash.bin` is exactly `len * 8` bytes and `dict-hid.bin` exactly `len * 4`
///     (the parallel sorted lookup arrays — a mismatch would make `mapped_find`'s binary
///     search read past the hashids array),
///   * every offset is `<= blob.len()` AND the record at that offset parses fully with valid
///     UTF-8 (via [`parse_stored_ref_checked`]),
///   * every record's `prefix` / `datatype` index is within the prefix / datatype tables,
///     and every `Triple` component id is a valid 1-based term id — otherwise a later
///     `reconstruct_ref` / `hash_stored_ref` would index those tables out of bounds.
///
/// Any violation is a clean `InvalidData` error — never a panic, never UB.
#[cfg(feature = "mmap")]
fn validate_dict_bytes(
    blob: &[u8],
    offsets: &[u8],
    hashes: &[u8],
    hashids: &[u8],
    len: usize,
    prefixes: &[Box<str>],
    datatypes: &[NamedNode],
) -> std::io::Result<()> {
    let bad = |m: &str| std::io::Error::new(std::io::ErrorKind::InvalidData, format!("mmap dict: {m}"));
    if offsets.len() != len.checked_mul(8).ok_or_else(|| bad("term count overflows offset-array size"))? {
        return Err(bad("dict-offs.bin length is not term_count * 8 (corrupt or truncated)"));
    }
    if hashes.len() != len * 8 {
        return Err(bad("dict-hash.bin length is not term_count * 8 (corrupt or truncated)"));
    }
    if hashids.len() != len.checked_mul(4).ok_or_else(|| bad("term count overflows hashid-array size"))? {
        return Err(bad("dict-hid.bin length is not term_count * 4 (corrupt or truncated)"));
    }
    let len_id = u32::try_from(len).map_err(|_| bad("term count exceeds u32 id space"))?;
    // Read the `idx`-th little-endian `u64` offset directly from the byte region. The length
    // check above guarantees `offsets` is exactly `len * 8` bytes, so `0..len` is in range;
    // we still index via `get` so this stays panic-free for ANY input (the Kani obligation).
    let offset_at = |idx: usize| -> Option<usize> {
        let start = idx.checked_mul(8)?;
        let end = start.checked_add(8)?;
        let bytes: [u8; 8] = offsets.get(start..end)?.try_into().ok()?;
        Some(u64::from_le_bytes(bytes) as usize)
    };
    // [OPUS-4.8] sq-cvug — triple-term `(own_id, subject, predicate)` ids collected
    // during the parse pass, verified for KIND once every record offset is known in range.
    let mut triple_sp: Vec<(Id, Id, Id)> = Vec::new();
    for idx in 0..len {
        let own_id = (idx as Id) + 1; // records are stored in 1-based id order
        let off = offset_at(idx).ok_or_else(|| bad("offset-array index out of range (corrupt or truncated)"))?;
        let rec = blob.get(off..).ok_or_else(|| bad("term offset is past the end of dict-terms.bin"))?;
        let parsed = parse_stored_ref_checked(rec)
            .ok_or_else(|| bad("a term record is malformed (bad tag/length or non-UTF-8 bytes)"))?;
        // Range-check the table indices / component ids the record carries: these are
        // attacker-controlled u32s in the blob, used unchecked as `prefixes[..]` /
        // `datatypes[..]` indices and as ids when the term is reconstructed.
        match parsed {
            StoredRef::Iri { prefix, .. } => {
                if prefix as usize >= prefixes.len() {
                    return Err(bad("an IRI record references an out-of-range prefix id"));
                }
            }
            StoredRef::Lit { datatype, .. } => {
                if datatype as usize >= datatypes.len() {
                    return Err(bad("a literal record references an out-of-range datatype id"));
                }
            }
            StoredRef::Triple(ids) => {
                // A triple term's components are themselves dictionary ids; an
                // inline-integer id (>= INLINE_BASE) or a real term id (1..=len) is in
                // range, but 0 or a dangling id would later index OOB. Beyond range, a
                // validly-built store interns each component BEFORE the triple (see
                // `intern_triple_ids`), so a non-inline component id is always STRICTLY
                // LESS than the triple's own id. Enforcing that here rejects a tampered
                // self/forward/cyclic reference (e.g. a triple whose object id is the
                // triple itself), which would otherwise drive `reconstruct_triple` into
                // UNBOUNDED RECURSION → stack overflow → process abort (a DoS). [OPUS-4.8]
                if ids
                    .iter()
                    .any(|&c| c == 0 || (c < INLINE_BASE && (c > len_id || c >= own_id)))
                {
                    return Err(bad("a quoted-triple record references an out-of-range, forward, or self component id"));
                }
                triple_sp.push((own_id, ids[0], ids[1]));
            }
            StoredRef::Blank(_) => {}
        }
    }
    // [OPUS-4.8] sq-cvug — KIND check: a component id can be in range yet of the wrong
    // kind (e.g. a literal where the subject must be IRI/blank or the predicate an IRI).
    // `reconstruct_triple` now fails closed to a placeholder rather than panicking, but
    // reject such an index here too so a corrupt store never opens. Resolving each id is
    // safe now: every offset above was confirmed in range and every record parseable, so
    // `record_kind` cannot OOB. (Inline-integer ids resolve to a literal — invalid in a
    // subject/predicate slot.)
    let record_kind = |c: Id| -> Option<u8> {
        if is_inline(c) {
            return Some(1); // inline integer => literal
        }
        let off = offset_at((c - 1) as usize)?;
        // tag byte: 0=IRI, 1=literal, 2=blank, 3=triple
        blob.get(off).copied()
    };
    for (_own, s, p) in triple_sp {
        match record_kind(s) {
            Some(0 | 2) => {} // IRI or blank — valid subject
            _ => return Err(bad("a quoted-triple subject component is not an IRI or blank node")),
        }
        match record_kind(p) {
            Some(0) => {} // IRI — valid predicate
            _ => return Err(bad("a quoted-triple predicate component is not an IRI")),
        }
    }
    // Every id in the sorted lookup array must be a valid 1-based term id: `mapped_find`
    // feeds it straight to `stored` (→ `offsets()[id-1]`), so `id == 0` (underflow) or
    // `id > len` (OOB) would be an out-of-bounds read on a hostile dict-hid.bin. Read each
    // little-endian `u32` from the byte region; the length check above guarantees the slice
    // is exactly `len * 4` bytes, and `chunks_exact(4)` never panics on a short tail.
    for chunk in hashids.chunks_exact(4) {
        let id = u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
        if id == 0 || id > len_id {
            return Err(bad("dict-hid.bin contains an out-of-range term id (corrupt lookup index)"));
        }
    }
    Ok(())
}

// [OPUS-4.8] sq-ueuk (epic sq-toze, gap MS-G4) — Kani bounded proof of the mmap dict
// validator, now that the `&[u8]` seam above lets Kani reach the header/length/bounds/record
// logic WITHOUT a real `mmap` (which Kani — like Miri — cannot model; see the WHY scope note
// in .github/workflows/kani.yml). This is the dict-side twin of `sparq-vectors`'
// `open_from_bytes_*` harnesses: it model-checks that `validate_dict_bytes` is TOTAL over a
// bounded symbolic byte domain — it always returns `Ok`/`Err`, never panics, never reads out
// of bounds, never hits UB — for EVERY hostile input in that domain.
//
// [SONNET-4.6] sq-sqtk2.2 — extended with PROVED (complete-domain) Kani harnesses for the
// inline-id round-trip and four-region id-partition disjointness properties (§3.3 C-1 of
// research/mechanized-proof-program.md). These prove purely numeric invariants of the `u32`
// id space; CBMC models each type as a bit-vector and explores the full symbolic domain, so
// "complete-domain" is a machine-verified claim (not just a bound). Unlike the mmap validator
// harnesses below, these do NOT require the `mmap` feature and run under both
// `cargo kani -p sparq-core` and `cargo kani -p sparq-core --features mmap`.
//
// TESTED in-worktree during sq-sqtk2.2 (see PR body for run log + mutation spot-check).
// The `kani` cfg is registered in crates/sparq-core/Cargo.toml [lints.rust] so `-D warnings`
// (unexpected_cfgs) is happy on the normal build.
#[cfg(kani)]
mod kani_proofs {
    use super::*;

    // ---- [SONNET-4.6] sq-sqtk2.2 — inline-id round-trip + id-partition disjointness ----
    //
    // Both harnesses are pure integer arithmetic — no loops, no heap, no strings. CBMC
    // models `u32` and `i64` as bit-vectors and exhausts the full symbolic domain; no
    // `#[kani::unwind]` annotation is needed (there is nothing to unroll).

    /// PROPERTY — encode half of the inline-id round-trip: `inline_id_of_int(v)` encodes
    /// every in-range value to a distinct inline id from which the value is exactly
    /// recoverable by subtracting `INLINE_BASE`.
    ///
    /// Domain coverage: **complete** — `v` is any `u32`; the `assume` restricts to the
    /// inline sub-domain `[0, INLINE_MAX]`, so CBMC exhausts the whole 30-bit value space.
    /// Together with `inline_id_out_of_domain_is_none` this covers the full `Option` spec.
    #[kani::proof]
    fn inline_id_round_trip() {
        let v: u32 = kani::any();
        kani::assume(v <= INLINE_MAX);
        let id = inline_id_of_int(v as i64)
            .expect("inline_id_of_int must return Some for every v in [0, INLINE_MAX]");
        assert!(is_inline(id), "inline_id_of_int result must satisfy is_inline");
        assert_eq!(
            id - INLINE_BASE,
            v,
            "id minus INLINE_BASE must recover the original value"
        );
    }

    /// PROPERTY — complement half of the inline-id spec: `inline_id_of_int(v)` returns
    /// `None` for every value outside `[0, INLINE_MAX]`.
    ///
    /// Domain coverage: **complete** — `v` is any `i64`; the `assume` restricts to the
    /// complement of the inline domain (negative values and values above `INLINE_MAX`).
    #[kani::proof]
    fn inline_id_out_of_domain_is_none() {
        let v: i64 = kani::any();
        kani::assume(v < 0 || v > INLINE_MAX as i64);
        assert!(
            inline_id_of_int(v).is_none(),
            "inline_id_of_int must return None for values outside [0, INLINE_MAX]"
        );
    }

    /// PROPERTY — four-region id-partition disjointness: the four named regions of the
    /// `u32` id space are pairwise disjoint and together exhaustive over all `u32` values.
    ///
    /// Regions (per the `INLINE_BASE` / `INLINE_MAX` partition constants):
    ///   - `NO_ID`:      `{0}`
    ///   - dict:         `[1, INLINE_BASE)` — non-inline dictionary ids
    ///   - inline:       `[INLINE_BASE, INLINE_BASE + INLINE_MAX]` — `is_inline` ids
    ///   - local-vocab:  `(INLINE_BASE + INLINE_MAX, u32::MAX]` — engine-private ids
    ///
    /// Domain coverage: **complete** — `id` is any `u32` bit-vector; CBMC exhausts the
    /// full 2^32 symbolic value space. Each `in_*` predicate is stated INDEPENDENTLY
    /// (not as a complement), so disjointness and exhaustiveness are non-trivial checks.
    #[kani::proof]
    fn id_partition_disjoint() {
        let id: u32 = kani::any();
        let in_no_id       = id == NO_ID;
        let in_dict        = id > NO_ID && id < INLINE_BASE;
        let in_inline      = is_inline(id);
        let in_local_vocab = id > INLINE_BASE + INLINE_MAX;
        // Disjointness: every pair is mutually exclusive.
        assert!(!(in_no_id && in_dict),         "NO_ID region overlaps dict region");
        assert!(!(in_no_id && in_inline),       "NO_ID region overlaps inline region");
        assert!(!(in_no_id && in_local_vocab),  "NO_ID region overlaps local-vocab region");
        assert!(!(in_dict && in_inline),        "dict region overlaps inline region");
        assert!(!(in_dict && in_local_vocab),   "dict region overlaps local-vocab region");
        assert!(!(in_inline && in_local_vocab), "inline region overlaps local-vocab region");
        // Exhaustiveness: every id belongs to at least one region (combined with
        // disjointness this gives exactly one).
        assert!(
            in_no_id || in_dict || in_inline || in_local_vocab,
            "id does not belong to any region (partition is not exhaustive)"
        );
    }

    /// DOMAIN-COVERAGE SELF-CHECK (the sq-og8u8 anti-vacuity pattern). Lesson of sq-sqtk2.1
    /// (2026-07-04): a bound can SILENTLY prune the very inputs a harness means to cover, so
    /// it passes VACUOUSLY. The three id harnesses above `assume`-restrict a symbolic id/value
    /// to ONE partition sub-domain; a disjointness or round-trip proof over a sub-domain that
    /// turned out EMPTY (or collapsed to fewer than four regions) would be vacuously true. This
    /// self-check pins that every region is genuinely NON-EMPTY and the boundaries are sharp —
    /// concrete arithmetic over the partition constants (no loop, no unwind, so nothing here
    /// can itself be pruned) — plus two `kani::cover!`s proving the `inline_id_round_trip`
    /// assume-domain (`v <= INLINE_MAX`) actually REACHES both its adversarial boundary
    /// (`INLINE_MAX`) and zero. Runs in BOTH feature states (no `mmap`). [OPUS-4.8] sq-og8u8
    #[kani::proof]
    fn domain_id_partition_regions_are_all_nonempty() {
        // Every one of the four regions has at least one member — else a "four-region"
        // disjointness/exhaustiveness proof is secretly a three- or two-region one.
        assert!(NO_ID == 0, "the NO_ID region is the singleton {0}");
        assert!(INLINE_BASE > 1, "the dict region [1, INLINE_BASE) is non-empty");
        assert!(INLINE_MAX > 0, "the inline region [INLINE_BASE, INLINE_BASE + INLINE_MAX] is non-empty");
        // The local-vocab region (INLINE_BASE + INLINE_MAX, u32::MAX] is non-empty: its low
        // bound is strictly below u32::MAX. Widen to u64 so the sum cannot wrap.
        assert!(
            u64::from(INLINE_BASE) + u64::from(INLINE_MAX) < u64::from(u32::MAX),
            "the local-vocab region above the inline range is non-empty"
        );
        // Sharp boundaries: the ends of the inline region are inline; the ids just outside
        // it (the last dict id, the first local-vocab id) are NOT.
        assert!(is_inline(INLINE_BASE) && is_inline(INLINE_BASE + INLINE_MAX));
        assert!(!is_inline(INLINE_BASE - 1), "the last dict id must not be classed inline");
        assert!(
            !is_inline(INLINE_BASE + INLINE_MAX + 1),
            "the first local-vocab id must not be classed inline"
        );

        // The `inline_id_round_trip` assume-domain reaches BOTH ends — its adversarial
        // boundary `INLINE_MAX` is not silently excluded by the `assume`.
        let v: u32 = kani::any();
        kani::assume(v <= INLINE_MAX);
        kani::cover!(v == INLINE_MAX, "the round-trip domain reaches its inline boundary");
        kani::cover!(v == 0, "the round-trip domain reaches zero");
    }

    // ---- [OPUS-4.8] sq-ueuk — mmap dict validator bounded proofs ----
    //
    // These harnesses require the `mmap` feature (they call `validate_dict_bytes`, which
    // is `#[cfg(feature = "mmap")]`). They run only when both `kani` AND `mmap` are active,
    // i.e. under `cargo kani -p sparq-core --features mmap`.

    /// Symbolic buffer ceiling. The four byte regions are independently symbolic but length-
    /// coupled to `len` by the early size checks, so a small `len` plus a small blob exercises
    /// every branch — the size-arithmetic rejections, the record parse + table-index range
    /// checks, the triple-term component/kind checks, and the hashid range loop — while
    /// keeping the bounded exploration tractable.
    #[cfg(feature = "mmap")]
    const MAX_LEN: usize = 2; // up to 2 terms
    #[cfg(feature = "mmap")]
    const MAX_BLOB: usize = 24; // a couple of small records / one triple term (1+12) + slack

    /// Fills a `Vec<u8>` of a symbolic length `<= cap` with symbolic bytes.
    #[cfg(feature = "mmap")]
    fn symbolic_bytes(cap: usize) -> Vec<u8> {
        let n: usize = kani::any();
        kani::assume(n <= cap);
        let mut v = vec![0u8; n];
        for b in v.iter_mut() {
            *b = kani::any();
        }
        v
    }

    /// PROPERTY: `validate_dict_bytes` never panics / OOB / UB for ANY four byte regions and
    /// term count up to the bounds, with empty prefix/datatype tables. Empty tables make every
    /// non-zero IRI-prefix / literal-datatype index out of range (so that rejection branch is
    /// exercised) and keep the symbolic surface minimal.
    #[cfg(feature = "mmap")]
    #[kani::proof]
    #[kani::unwind(28)] // > MAX_BLOB so every bounded record/slice loop fully unrolls.
    fn validate_dict_bytes_never_panics() {
        let len: usize = kani::any();
        kani::assume(len <= MAX_LEN);
        let blob = symbolic_bytes(MAX_BLOB);
        let offsets = symbolic_bytes(MAX_LEN * 8);
        let hashes = symbolic_bytes(MAX_LEN * 8);
        let hashids = symbolic_bytes(MAX_LEN * 4);
        // The property is simply: this call returns (must not panic / OOB / UB). Both `Ok`
        // and `Err` are correct outcomes; the harness proves the validator is TOTAL over the
        // bounded input domain.
        let _ = validate_dict_bytes(&blob, &offsets, &hashes, &hashids, len, &[], &[]);
    }

    /// PROPERTY (focused): with the array LENGTHS already correct for `len`, the record-parse,
    /// component-id, kind, and hashid-range tail still never panics. Pinning the lengths past
    /// the size checks spends Kani's budget on the loop/arithmetic logic the corpus is least
    /// likely to have exhausted, with one prefix + one datatype so the in-range index branch
    /// is also taken.
    #[cfg(feature = "mmap")]
    #[kani::proof]
    #[kani::unwind(28)]
    fn validate_dict_bytes_sized_tail_never_panics() {
        const LEN: usize = 1;
        let blob = symbolic_bytes(MAX_BLOB);
        let mut offsets = vec![0u8; LEN * 8];
        for b in offsets.iter_mut() {
            *b = kani::any();
        }
        let mut hashes = vec![0u8; LEN * 8];
        for b in hashes.iter_mut() {
            *b = kani::any();
        }
        let mut hashids = vec![0u8; LEN * 4];
        for b in hashids.iter_mut() {
            *b = kani::any();
        }
        let prefixes: [Box<str>; 1] = [Box::from("p")];
        let datatypes = [NamedNode::new_unchecked("http://example.org/dt")];
        let _ = validate_dict_bytes(&blob, &offsets, &hashes, &hashids, LEN, &prefixes, &datatypes);
    }

    /// DOMAIN-COVERAGE SELF-CHECK (the sq-og8u8 anti-vacuity pattern) for the two mmap
    /// validator totality harnesses above. `..._never_panics` proves `validate_dict_bytes`
    /// is TOTAL (returns `Ok`/`Err`, never panics/OOB/UB) over the bounded symbolic domain —
    /// but that is worthless if the ACCEPT path is never reachable within the bounds and every
    /// symbolic input trips a size check before any record is parsed (a VACUOUS "never panics
    /// on the tail"). This pins that the bounded domain genuinely CONTAINS an accepted dict:
    /// the empty dict (`len == 0`, empty tables — in the harnesses' `len <= MAX_LEN` domain)
    /// validates `Ok`. Concrete, so it cannot itself be pruned. The DEEPER accept path (a
    /// `>= 1`-record dict that exercises the record-parse / triple-kind / hashid-range loops
    /// to an `Ok`) is pinned by the unit test
    /// [`validate_dict_bytes_seam_accepts_valid_and_rejects_corruption`], which builds a REAL
    /// valid dict and asserts `Ok` — so the symbolic totality proof is non-vacuous on both the
    /// shallow (here) and the deep (that unit test) accept paths. [OPUS-4.8] sq-og8u8
    #[cfg(feature = "mmap")]
    #[kani::proof]
    fn domain_dict_validator_accepts_the_empty_dict() {
        // len == 0 lies in the harnesses' `len <= MAX_LEN` domain and every table is empty.
        assert!(
            validate_dict_bytes(&[], &[], &[], &[], 0, &[], &[]).is_ok(),
            "the empty dict must validate — else the accept path is vacuous"
        );
    }
}

impl Dict {
    pub fn new() -> Self {
        Dict::default()
    }

    pub fn with_capacity(n: usize) -> Self {
        Dict {
            terms: Vec::with_capacity(n),
            table: HashTable::with_capacity(n),
            ..Default::default()
        }
    }

    #[inline]
    fn intern_prefix(&mut self, prefix: &str) -> u32 {
        if let Some(&id) = self.prefix_ids.get(prefix) {
            return id;
        }
        let id = self.prefixes.len() as u32;
        let boxed: Box<str> = prefix.into();
        self.prefixes.push(boxed.clone());
        self.prefix_ids.insert(boxed, id);
        id
    }

    #[inline]
    fn intern_datatype(&mut self, dt: &str) -> u32 {
        if let Some(&id) = self.datatype_ids.get(dt) {
            return id;
        }
        let id = self.datatypes.len() as u32;
        self.datatypes.push(NamedNode::new_unchecked(dt));
        self.datatype_ids.insert(dt.into(), id);
        id
    }

    /// Assigns the next id to a freshly built `Stored` and indexes it. With a compacted
    /// base (blob/mmap), the new term is APPENDED to the arena above `base` — the dict's
    /// append-only growth path for delta-overlay updates.
    #[inline]
    fn push(&mut self, hash: u64, stored: Stored) -> Id {
        let id = (self.base + self.terms.len()) as Id + 1; // 1-based
        // Enforced in release too: once the (non-inline) dictionary reaches INLINE_BASE
        // distinct terms, new ids would collide with the inline-integer range and decode
        // as integers — silent corruption. 2^31 ≈ 2.1B distinct non-integer terms is the
        // hard capacity limit of the u32 scheme; fail loudly (widen Id to u64).
        assert!(id < INLINE_BASE, "dictionary exceeded the id capacity (2^31 distinct non-integer terms); widen Id to u64");
        self.terms.push(stored);
        let (base, terms, blob, prefixes, datatypes) =
            (self.base, &self.terms, &self.blob, &self.prefixes, &self.datatypes);
        self.table.insert_unique(hash, id, |&i| hash_tabled(i, base, terms, blob, prefixes, datatypes));
        id
    }

    /// Finds a term in the MAPPED base by content hash (binary search of the mmap'd
    /// sorted-hash index, verifying candidates with `eq`). `None` when not mapped or
    /// absent — the caller then consults the table (blob base + appended arena terms).
    #[cfg(feature = "mmap")]
    #[inline]
    fn mapped_find(&self, hash: u64, eq: impl Fn(&StoredRef) -> bool) -> Option<Id> {
        let m = self.mapped.as_ref()?;
        let hashes = m.hashes();
        let mut i = hashes.partition_point(|&h| h < hash);
        let ids = m.hashids();
        while i < hashes.len() && hashes[i] == hash {
            let id = ids[i];
            if eq(&m.stored(id)) {
                return Some(id);
            }
            i += 1;
        }
        None
    }

    /// The record of a TABLED id (blob base, or appended arena) as needed by the intern
    /// comparison closures. Pure-arena dicts (`base == 0`) always take the arena branch,
    /// so the bulk-load hot path is unchanged beyond one predictable comparison.
    #[inline]
    fn tabled_base_ref(&self, id: Id) -> StoredRef<'_> {
        let (blob, offs) = self.blob.as_ref().expect("a tabled id below `base` requires the blob");
        parse_stored_ref(&blob[offs[(id - 1) as usize] as usize..])
    }

    #[inline]
    fn tabled_is_iri(&self, id: Id, iri: &str) -> bool {
        let i = (id - 1) as usize;
        if i >= self.base {
            stored_is_iri(&self.terms[i - self.base], iri, &self.prefixes)
        } else {
            stored_ref_is_iri(&self.tabled_base_ref(id), iri, &self.prefixes)
        }
    }

    #[inline]
    fn tabled_is_iri_parts(&self, id: Id, prefix: &str, suffix: &str) -> bool {
        let i = (id - 1) as usize;
        if i >= self.base {
            stored_is_iri_parts(&self.terms[i - self.base], prefix, suffix, &self.prefixes)
        } else {
            stored_ref_is_iri_parts(&self.tabled_base_ref(id), prefix, suffix, &self.prefixes)
        }
    }

    #[inline]
    fn tabled_is_lit(&self, id: Id, value: &str, datatype: &str, lang: Option<&str>) -> bool {
        let i = (id - 1) as usize;
        if i >= self.base {
            stored_is_lit(&self.terms[i - self.base], value, datatype, lang, &self.datatypes)
        } else {
            stored_ref_is_lit(&self.tabled_base_ref(id), value, datatype, lang, &self.datatypes)
        }
    }

    #[inline]
    fn tabled_is_blank(&self, id: Id, label: &str) -> bool {
        let i = (id - 1) as usize;
        if i >= self.base {
            matches!(&self.terms[i - self.base], Stored::Blank(b) if **b == *label)
        } else {
            matches!(self.tabled_base_ref(id), StoredRef::Blank(b) if b == label)
        }
    }

    #[inline]
    fn tabled_is_triple(&self, id: Id, ids: [Id; 3]) -> bool {
        let i = (id - 1) as usize;
        if i >= self.base {
            matches!(&self.terms[i - self.base], Stored::Triple(x) if *x == ids)
        } else {
            matches!(self.tabled_base_ref(id), StoredRef::Triple(x) if x == ids)
        }
    }

    #[inline]
    fn tabled_eq_term(&self, id: Id, term: &Term) -> bool {
        let i = (id - 1) as usize;
        if i >= self.base {
            stored_eq_term(&self.terms[i - self.base], term, &self.prefixes, &self.datatypes)
        } else {
            stored_ref_eq_term(&self.tabled_base_ref(id), term, &self.prefixes, &self.datatypes)
        }
    }

    // ---- Non-mutating finders shared by the intern and lookup paths ----------------
    // Search order per term kind: the mmap'd base index (when mapped), then the FROZEN
    // shared base of a forked dict (its own complete finder — one level, the base is
    // flat by invariant), then the table (blob base + appended arena). On a plain
    // arena dict only the table branch does work, so the bulk-load hot path keeps its
    // shape (two predictable never-taken branches).
    //
    // [OPUS-5] (sq-7d3dj.21b) HASH-BEFORE-MEMCMP — SCOPED HONESTLY, PER PATH. The bead asked to
    // compare a stored 64-bit hash before falling through to the string compare ("skip if
    // already done — verify first"). It is in place on the MMAP path only; the in-memory path
    // keeps a weaker, tag-only prefilter BY CHOICE. What each dedup path actually does:
    //   * `mapped_find` — FULL 64-bit gate. It binary-searches the SORTED 64-bit `dict-hash.bin`
    //     array and calls `eq` only on entries whose FULL 64-bit hash matches; a miss costs zero
    //     string compares. Pinned by `mapped_find_compares_full_hash_before_eq`.
    //   * `self.table` (blob base + appended arena) — NO full-hash gate. It is a
    //     `hashbrown::HashTable<Id>` storing bare ids, so there is no stored 64-bit hash to
    //     compare: `find(hash, eq)` matches only the hash's top-7-bit `h2` tag against a SIMD
    //     control group, so `eq` — and its string compare — CAN run for a probed entry whose
    //     other 57 hash bits differ (~1 in 128 of probed slots). Correctness therefore rests on
    //     `eq` itself, not on the tag. Pinned by `in_memory_dedup_has_only_the_h2_tag_prefilter`.
    //   * the sharded merge (`ShardedDict::intern_partials`/`intern_terms`) and the spill path
    //     (`dictspill`) route by `hash % n_leaf` — a shard SELECTOR, not a hash-equality gate —
    //     and then land in one of the two above, inheriting whichever prefilter that path has.
    // The in-memory full gate is deliberately not implemented: a per-entry stored `u64` means
    // `HashTable<(Id, u64)>` — 4 → 16 bytes per entry after padding, i.e. ~3-4x the lookup table
    // on a 100M-term dict (a `dict_bytes_per_term` ratchet regression) — to filter the ~0.8% of
    // probes the `h2` tag already lets through. Rejected on that trade, not on effort. So: the
    // bead's requirement is MET on the mmap path and CONSCIOUSLY DECLINED in memory; do not read
    // the mmap guarantee as covering every dedup path.

    #[inline]
    fn find_iri(&self, hash: u64, iri: &str) -> Option<Id> {
        #[cfg(feature = "mmap")]
        if let Some(id) = self.mapped_find(hash, |s| stored_ref_is_iri(s, iri, &self.prefixes)) {
            return Some(id);
        }
        if let Some(f) = &self.frozen {
            if let Some(id) = f.find_iri(hash, iri) {
                return Some(id);
            }
        }
        self.table.find(hash, |&id| self.tabled_is_iri(id, iri)).copied()
    }

    #[inline]
    fn find_iri_parts(&self, hash: u64, prefix: &str, suffix: &str) -> Option<Id> {
        #[cfg(feature = "mmap")]
        if let Some(id) = self.mapped_find(hash, |s| stored_ref_is_iri_parts(s, prefix, suffix, &self.prefixes)) {
            return Some(id);
        }
        if let Some(f) = &self.frozen {
            if let Some(id) = f.find_iri_parts(hash, prefix, suffix) {
                return Some(id);
            }
        }
        self.table.find(hash, |&id| self.tabled_is_iri_parts(id, prefix, suffix)).copied()
    }

    #[inline]
    fn find_lit(&self, hash: u64, value: &str, datatype: &str, lang: Option<&str>) -> Option<Id> {
        #[cfg(feature = "mmap")]
        if let Some(id) = self.mapped_find(hash, |s| stored_ref_is_lit(s, value, datatype, lang, &self.datatypes)) {
            return Some(id);
        }
        if let Some(f) = &self.frozen {
            if let Some(id) = f.find_lit(hash, value, datatype, lang) {
                return Some(id);
            }
        }
        self.table.find(hash, |&id| self.tabled_is_lit(id, value, datatype, lang)).copied()
    }

    #[inline]
    fn find_blank(&self, hash: u64, label: &str) -> Option<Id> {
        #[cfg(feature = "mmap")]
        if let Some(id) = self.mapped_find(hash, |s| matches!(s, StoredRef::Blank(b) if *b == label)) {
            return Some(id);
        }
        if let Some(f) = &self.frozen {
            if let Some(id) = f.find_blank(hash, label) {
                return Some(id);
            }
        }
        self.table.find(hash, |&id| self.tabled_is_blank(id, label)).copied()
    }

    #[inline]
    fn find_triple_ids(&self, hash: u64, ids: [Id; 3]) -> Option<Id> {
        #[cfg(feature = "mmap")]
        if let Some(id) = self.mapped_find(hash, |s| matches!(s, StoredRef::Triple(x) if *x == ids)) {
            return Some(id);
        }
        if let Some(f) = &self.frozen {
            if let Some(id) = f.find_triple_ids(hash, ids) {
                return Some(id);
            }
        }
        self.table.find(hash, |&id| self.tabled_is_triple(id, ids)).copied()
    }

    #[inline]
    fn find_term(&self, hash: u64, term: &Term) -> Option<Id> {
        #[cfg(feature = "mmap")]
        if let Some(id) = self.mapped_find(hash, |s| stored_ref_eq_term(s, term, &self.prefixes, &self.datatypes)) {
            return Some(id);
        }
        if let Some(f) = &self.frozen {
            if let Some(id) = f.find_term(hash, term) {
                return Some(id);
            }
        }
        self.table.find(hash, |&id| self.tabled_eq_term(id, term)).copied()
    }

    /// Interns an IRI term from its string, returning its id.
    #[inline]
    pub fn intern_iri(&mut self, iri: &str) -> Id {
        let hash = hash_iri(iri);
        if let Some(id) = self.find_iri(hash, iri) {
            return id;
        }
        let (p, suffix) = split_iri(iri);
        let prefix = self.intern_prefix(p);
        self.push(hash, Stored::Iri { prefix, suffix: suffix.into() })
    }

    /// Interns an IRI already split into (prefix, suffix) — used by the parallel-load
    /// merge, where the canonical split is already known (no re-split, no concat).
    #[inline]
    fn intern_iri_parts(&mut self, prefix: &str, suffix: &str) -> Id {
        let hash = hash_iri_parts(prefix, suffix);
        if let Some(id) = self.find_iri_parts(hash, prefix, suffix) {
            return id;
        }
        let prefix = self.intern_prefix(prefix);
        self.push(hash, Stored::Iri { prefix, suffix: suffix.into() })
    }

    /// Interns a literal from its components, returning its id. Canonical small
    /// `xsd:integer`s are encoded inline and never stored.
    #[inline]
    pub fn intern_lit(&mut self, value: &str, datatype: &str, lang: Option<&str>) -> Id {
        if let Some(id) = try_inline_lit(value, datatype) {
            return id;
        }
        let hash = hash_lit(value, datatype, lang);
        if let Some(id) = self.find_lit(hash, value, datatype, lang) {
            return id;
        }
        let datatype = self.intern_datatype(datatype);
        self.push(hash, Stored::Lit { value: value.into(), datatype, lang: lang.map(Into::into) })
    }

    /// Interns a blank node from its label, returning its id.
    #[inline]
    pub fn intern_blank(&mut self, label: &str) -> Id {
        let hash = hash_blank(label);
        if let Some(id) = self.find_blank(hash, label) {
            return id;
        }
        self.push(hash, Stored::Blank(label.into()))
    }

    /// Interns an RDF 1.2 triple term whose components are ALREADY interned in this dict
    /// (ids may include inline-integer ids). Content-addressed by the component ids, so
    /// separately-interned identical triples share one id.
    ///
    /// [OPUS-4.8] `pub(crate)` so the byte-level N-Triples parser (`nt`) can intern a
    /// `<<( s p o )>>` triple-term object STRUCTURALLY (interning s/p/o first, then the
    /// triple by their ids) — the identical path `intern(&Term::Triple(_))` takes — so the
    /// N-Triples and Turtle loaders agree on triple-term content-addressing.
    pub(crate) fn intern_triple_ids(&mut self, ids: [Id; 3]) -> Id {
        let hash = hash_triple_ids(ids);
        if let Some(id) = self.find_triple_ids(hash, ids) {
            return id;
        }
        self.push(hash, Stored::Triple(ids))
    }

    /// Interns a term, returning its id (creating it if new). Dispatches to the
    /// component interners so the `Term` and byte-slice paths share one code path.
    /// An RDF 1.2 triple term is stored STRUCTURALLY: its s/p/o are interned first
    /// (recursing through a nested triple-term object) and the triple holds their ids.
    #[inline]
    pub fn intern(&mut self, term: &Term) -> Id {
        match term {
            Term::NamedNode(n) => self.intern_iri(n.as_str()),
            Term::Literal(l) => self.intern_lit(l.value(), l.datatype().as_str(), lang_with_dir(l).as_deref()),
            Term::BlankNode(b) => self.intern_blank(b.as_str()),
            Term::Triple(t) => {
                let s = match t.subject {
                    oxrdf::NamedOrBlankNode::NamedNode(ref n) => self.intern_iri(n.as_str()),
                    oxrdf::NamedOrBlankNode::BlankNode(ref b) => self.intern_blank(b.as_str()),
                };
                let p = self.intern_iri(t.predicate.as_str());
                let o = self.intern(&t.object);
                self.intern_triple_ids([s, p, o])
            }
        }
    }

    /// Interns a borrowed term (already-split components) without building a `Term` or
    /// concatenating the IRI — the fast path used by the sharded parallel merge for LEAF
    /// terms.
    ///
    /// Triple terms are NOT handled here: `TermParts::Triple` carries ids of the SOURCE
    /// dict, meaningless in another dict. [OPUS-4.8] (sq-t3rt) The sharded merge no longer
    /// reaches this with a triple term — `ShardedDict::intern_partials` SKIPS triple terms
    /// in the (hash-routed) leaf pass and interns them structurally into a dedicated triple
    /// shard via `intern_triple_ids` instead. This stays a loud guard, not a path.
    #[inline]
    fn intern_parts(&mut self, tp: &TermParts) -> Id {
        match tp {
            TermParts::Iri { prefix, suffix } => self.intern_iri_parts(prefix, suffix),
            TermParts::Lit { value, datatype, lang } => self.intern_lit(value, datatype, *lang),
            TermParts::Blank(b) => self.intern_blank(b),
            TermParts::Triple(_) => {
                panic!("RDF 1.2 triple terms must be interned structurally (intern_triple_ids), not via the parts-based leaf path")
            }
        }
    }

    /// Returns the id for a term if present, else `NO_ID`.
    #[inline]
    pub fn lookup(&self, term: &Term) -> Id {
        // An RDF 1.2 triple term is matched STRUCTURALLY: resolve its components first
        // (a missing component means the triple term cannot be present either), then
        // find the triple by its component ids.
        if let Term::Triple(t) = term {
            let s = match t.subject {
                oxrdf::NamedOrBlankNode::NamedNode(ref n) => self.lookup(&Term::NamedNode(n.clone())),
                oxrdf::NamedOrBlankNode::BlankNode(ref b) => self.lookup(&Term::BlankNode(b.clone())),
            };
            let p = self.lookup(&Term::NamedNode(t.predicate.clone()));
            let o = self.lookup(&t.object);
            if s == NO_ID || p == NO_ID || o == NO_ID {
                return NO_ID;
            }
            return self.lookup_triple_ids([s, p, o]);
        }
        if let Some(id) = try_inline(term) {
            return id;
        }
        let hash = hash_term(term);
        // Mapped base first (binary search of the sorted hash index, verifying each
        // equal-hash candidate — content hashes can collide), then the frozen shared
        // base of a forked dict, then the table, which covers the blob base and any
        // APPENDED terms (delta-overlay growth).
        self.find_term(hash, term).unwrap_or(NO_ID)
    }

    /// Returns the id for a literal given its components, else `NO_ID` — `lookup`
    /// without constructing an `oxrdf::Term` (the fast path for resolving computed
    /// BIND/aggregate values against the dictionary). Canonical small `xsd:integer`s
    /// resolve to their inline id, exactly like `lookup`/`intern_lit`.
    #[inline]
    pub fn lookup_lit(&self, value: &str, datatype: &str, lang: Option<&str>) -> Id {
        if let Some(id) = try_inline_lit(value, datatype) {
            return id;
        }
        let hash = hash_lit(value, datatype, lang);
        self.find_lit(hash, value, datatype, lang).unwrap_or(NO_ID)
    }

    /// Returns the id of a triple term whose components resolved to `ids`, else `NO_ID`.
    /// Serves all three storage modes (arena, blob, mmap).
    fn lookup_triple_ids(&self, ids: [Id; 3]) -> Id {
        let hash = hash_triple_ids(ids);
        self.find_triple_ids(hash, ids).unwrap_or(NO_ID)
    }

    /// Borrows a dictionary term's components WITHOUT reconstructing an `oxrdf::Term`
    /// (no allocation) — for serialising results straight from ids. Only valid for a
    /// real dictionary id (`1..INLINE_BASE`); the caller handles inline / local ids.
    #[inline]
    pub fn term_parts(&self, id: Id) -> TermParts<'_> {
        // The &str slices borrow the mmap/blob record store (or the arena), plus the
        // in-RAM prefix+datatype tables, all owned by `self` for `'_` — zero-copy.
        match self.record(id) {
            StoredRef::Iri { prefix, suffix } => TermParts::Iri { prefix: &self.prefixes[prefix as usize], suffix },
            StoredRef::Lit { value, datatype, lang } => {
                TermParts::Lit { value, datatype: self.datatypes[datatype as usize].as_str(), lang }
            }
            StoredRef::Blank(b) => TermParts::Blank(b),
            StoredRef::Triple(ids) => TermParts::Triple(ids),
        }
    }

    /// [FABLE-5] sq-7d3dj.30.21 — if `id` is a PLAIN `xsd:string` literal (datatype exactly
    /// `xsd:string`, NO language tag), returns its value as a ZERO-COPY `&str` borrowed from
    /// the record store; otherwise `None`. A single-probe eligibility test AND value accessor
    /// for the engine's lazy top-k ORDER-BY string key — it matches EXACTLY the engine's
    /// `LiteralKind::String` set (no lang tag + `datatype == xsd:string`), so routing such an
    /// id to a zero-allocation id-carrying sort cell preserves the SPARQL `value()`-order
    /// exactly (a plain string sorts by its value bytes). Inline-integer ids are numeric
    /// literals, so they return `None` (no allocation, no reconstruction, unlike `term(id)`).
    #[inline]
    pub fn plain_string_value(&self, id: Id) -> Option<&str> {
        if is_inline(id) {
            return None;
        }
        match self.record(id) {
            StoredRef::Lit { value, datatype, lang: None }
                if self.datatypes[datatype as usize].as_ref() == xsd::STRING =>
            {
                Some(value)
            }
            _ => None,
        }
    }

    /// Returns the term for an id. Inline-integer ids are decoded directly; others are
    /// reconstructed from the compact record store (panics on an invalid index — ids
    /// come from the store).
    #[inline]
    pub fn term(&self, id: Id) -> Term {
        if is_inline(id) {
            return Term::Literal(Literal::new_typed_literal((id - INLINE_BASE).to_string(), xsd::INTEGER));
        }
        reconstruct_ref(self, &self.record(id))
    }

    /// The record for ANY 1-based dictionary id, across all storage modes: ids above
    /// `base` come from the (appended) arena; ids at or below it from the mmap'd files
    /// or the in-memory blob. Pure-arena dicts always take the first branch (base = 0).
    #[inline]
    fn record(&self, id: Id) -> StoredRef<'_> {
        // [OPUS-4.8] sq-ky2a: `id == 0` is never a valid 1-based id, but a tampered perm can
        // contain it; `id - 1` would underflow (a panic in debug). Treat it as out-of-range.
        if id == 0 {
            return OOR_TERM;
        }
        let i = (id - 1) as usize;
        if i >= self.base {
            // [OPUS-4.8] sq-ky2a: `id` can originate from a TAMPERED permutation file (the
            // raw/compressed perm is a trusted-format asset with no per-row checksum — see
            // research/threat-model.md T-MMAP-DoS). A garbage id beyond the arena would
            // index `self.terms` OUT OF BOUNDS (a panic, and with the unchecked blob path a
            // potential OOB read). Resolve it through a bounds-CHECKED access and fall back
            // to a safe placeholder so a corrupt store yields wrong-but-safe results, never
            // UB / OOB. The valid-id hot path is one predictable bounds check (always in
            // range for an untampered store).
            return self
                .terms
                .get(i - self.base)
                .map(stored_as_ref)
                .unwrap_or(OOR_TERM);
        }
        if let Some(f) = &self.frozen {
            return f.record(id); // the shared immutable base of a forked dict
        }
        #[cfg(feature = "mmap")]
        if let Some(m) = &self.mapped {
            return m.stored(id);
        }
        let (blob, offs) = self.blob.as_ref().expect("an id below `base` requires the blob or mmap store");
        match offs.get(i) {
            Some(&off) => parse_stored_ref(&blob[off as usize..]),
            None => OOR_TERM,
        }
    }

    /// Compacts the id→term storage into a single concatenated BLOB + per-term `u32`
    /// offsets, freeing the `Vec<Stored>` (with its per-term `Box<str>` allocations). The
    /// hash table is kept (lookup verifies candidates against the blob). Roughly halves
    /// the resident dictionary bytes — the memory-bound (browser) storage mode. A no-op if
    /// already compacted, memory-mapped, or empty.
    pub fn into_blob(mut self) -> Dict {
        #[cfg(feature = "mmap")]
        if self.mapped.is_some() {
            return self;
        }
        if self.frozen.is_some() {
            return self; // forked: the base is already compact-shared; ids must keep resolving through it
        }
        if self.blob.is_some() || self.terms.is_empty() {
            return self;
        }
        let mut blob: Vec<u8> = Vec::with_capacity(self.terms.len() * 8);
        let mut offsets: Vec<u32> = Vec::with_capacity(self.terms.len());
        for t in &self.terms {
            assert!(
                blob.len() <= u32::MAX as usize,
                "dictionary term blob exceeds 4 GiB; use the mmap dict (u64 offsets) at this scale"
            );
            offsets.push(blob.len() as u32);
            // Writing to a `Vec<u8>` is infallible.
            let _ = write_record(&mut blob, &stored_as_ref(t));
        }
        blob.shrink_to_fit();
        self.base = offsets.len(); // appended (delta-overlay) terms continue above the blob
        self.blob = Some((blob, offsets));
        self.terms = Vec::new(); // free the Stored arena (the win)
        self
    }

    /// Merges another (partial) dictionary into this one, returning the remap from the
    /// other's local ids to this dictionary's global ids: `remap[local - 1]` is the
    /// global id for the other's 1-based local id `local`. Used by the parallel bulk
    /// loader. Interns each of the other's terms directly from its compact components —
    /// no `Term` reconstruction, no IRI concatenation.
    pub fn merge_remap(&mut self, other: &Dict) -> Vec<Id> {
        self.terms.reserve(other.terms.len());
        {
            let (base, terms, blob, prefixes, datatypes) =
                (self.base, &self.terms, &self.blob, &self.prefixes, &self.datatypes);
            self.table
                .reserve(other.terms.len(), |&i| hash_tabled(i, base, terms, blob, prefixes, datatypes));
        }
        let mut remap: Vec<Id> = Vec::with_capacity(other.terms.len());
        for s in &other.terms {
            let id = match s {
                Stored::Iri { prefix, suffix } => self.intern_iri_parts(&other.prefixes[*prefix as usize], suffix),
                Stored::Lit { value, datatype, lang } => self.intern_lit(value, other.datatypes[*datatype as usize].as_str(), lang.as_deref()),
                Stored::Blank(b) => self.intern_blank(b),
                // A triple term's components always precede it in `other` (interning
                // pushes children first), so their remapped ids are already known.
                Stored::Triple(ids) => {
                    let m = |id: Id| if is_inline(id) { id } else { remap[(id - 1) as usize] };
                    self.intern_triple_ids([m(ids[0]), m(ids[1]), m(ids[2])])
                }
            };
            remap.push(id);
        }
        remap
    }

    /// [GPT-5.6] (sq-eiv) Consumes this dictionary and builds a dense dictionary containing
    /// only the real ids for which `retain` returns `true`. Returns the rebuilt dictionary
    /// and an old-to-new remap where `remap[(old_id - 1) as usize]` is `NO_ID` for a dropped
    /// term. Inline integers are not dictionary records, are not passed to `retain`, and keep
    /// their value-carrying id when remapping payloads.
    ///
    /// RDF 1.2 triple terms make the filter dependency-aware: retaining a triple term also
    /// retains its subject, predicate, object, and any recursively nested triple-term
    /// components. Their remap entries are therefore populated even when `retain` returned
    /// `false` for those component ids.
    ///
    /// The rebuild never materialises an `oxrdf::Term` and never re-interns survivors. It moves
    /// the compact `Stored` payloads out of an arena dictionary (preserving their string
    /// allocations), or copies compact records directly from a blob, mmap, or frozen base, then
    /// rewrites only metadata and triple-component ids before building the lookup table once.
    /// Prefix and datatype tables are filtered too, so dead metadata does not accumulate.
    pub fn rebuild_filtered(mut self, mut retain: impl FnMut(Id) -> bool) -> (Dict, Vec<Id>) {
        let old_len = self.len();

        // First select the caller-visible survivors. The closure runs exactly once for each
        // real dictionary id; dependency expansion below never calls it a second time.
        let mut retained = vec![false; old_len];
        let mut pending = Vec::new();
        for (slot, retained_slot) in retained.iter_mut().enumerate() {
            let id = slot as Id + 1;
            if retain(id) {
                *retained_slot = true;
                pending.push(id);
            }
        }

        // A structural triple record cannot outlive its components. Walk a small explicit
        // worklist rather than reconstructing Terms (and rather than assuming only one nesting
        // level). Valid dictionaries always reference earlier ids, but the visited bit also
        // makes this total for a cyclic record forged by an in-crate producer.
        while let Some(id) = pending.pop() {
            let StoredRef::Triple(ids) = self.record(id) else {
                continue;
            };
            for child in ids {
                if is_inline(child) || child == NO_ID {
                    continue;
                }
                let child_slot = (child - 1) as usize;
                if child_slot < retained.len() && !retained[child_slot] {
                    retained[child_slot] = true;
                    pending.push(child);
                }
            }
        }

        // Dense ids preserve old-id order. That keeps every well-formed triple's children before
        // the triple itself and makes rewriting structural component ids a direct table lookup.
        let mut next_id: Id = 1;
        let remap: Vec<Id> = retained
            .iter()
            .map(|&keep| {
                if keep {
                    let id = next_id;
                    next_id += 1;
                    id
                } else {
                    NO_ID
                }
            })
            .collect();
        let retained_len = (next_id - 1) as usize;

        // Identify metadata referenced by survivors before consuming any arena fields.
        let mut used_prefixes = vec![false; self.prefixes.len()];
        let mut used_datatypes = vec![false; self.datatypes.len()];
        for (slot, &keep) in retained.iter().enumerate() {
            if !keep {
                continue;
            }
            match self.record(slot as Id + 1) {
                StoredRef::Iri { prefix, .. } => used_prefixes[prefix as usize] = true,
                StoredRef::Lit { datatype, .. } => used_datatypes[datatype as usize] = true,
                StoredRef::Blank(_) | StoredRef::Triple(_) => {}
            }
        }

        // Arena mode can move each Box<str> survivor unchanged. Compact/frozen modes expose
        // borrowed StoredRef records, so copy those compact fields directly exactly once.
        let mut terms = Vec::with_capacity(retained_len);
        if self.base == 0 {
            for (slot, stored) in std::mem::take(&mut self.terms).into_iter().enumerate() {
                if retained[slot] {
                    terms.push(stored);
                }
            }
        } else {
            for (slot, &keep) in retained.iter().enumerate() {
                if !keep {
                    continue;
                }
                terms.push(match self.record(slot as Id + 1) {
                    StoredRef::Iri { prefix, suffix } => Stored::Iri {
                        prefix,
                        suffix: suffix.into(),
                    },
                    StoredRef::Lit {
                        value,
                        datatype,
                        lang,
                    } => Stored::Lit {
                        value: value.into(),
                        datatype,
                        lang: lang.map(Into::into),
                    },
                    StoredRef::Blank(label) => Stored::Blank(label.into()),
                    StoredRef::Triple(ids) => Stored::Triple(ids),
                });
            }
        }

        // Move only live metadata, preserving its old-id order, and build the small reverse maps.
        self.prefix_ids = FxHashMap::default();
        let mut prefix_remap = vec![u32::MAX; self.prefixes.len()];
        let mut prefixes = Vec::with_capacity(used_prefixes.iter().filter(|&&used| used).count());
        let mut prefix_ids: FxHashMap<Box<str>, u32> = FxHashMap::default();
        for (old, prefix) in std::mem::take(&mut self.prefixes).into_iter().enumerate() {
            if used_prefixes[old] {
                let new = prefixes.len() as u32;
                prefix_remap[old] = new;
                prefix_ids.insert(prefix.clone(), new);
                prefixes.push(prefix);
            }
        }

        self.datatype_ids = FxHashMap::default();
        let mut datatype_remap = vec![u32::MAX; self.datatypes.len()];
        let mut datatypes = Vec::with_capacity(used_datatypes.iter().filter(|&&used| used).count());
        let mut datatype_ids: FxHashMap<Box<str>, u32> = FxHashMap::default();
        for (old, datatype) in std::mem::take(&mut self.datatypes).into_iter().enumerate() {
            if used_datatypes[old] {
                let new = datatypes.len() as u32;
                datatype_remap[old] = new;
                datatype_ids.insert(datatype.as_str().into(), new);
                datatypes.push(datatype);
            }
        }

        let map_component = |old: Id| {
            if is_inline(old) {
                old
            } else {
                old.checked_sub(1)
                    .and_then(|slot| remap.get(slot as usize).copied())
                    .unwrap_or(NO_ID)
            }
        };
        for stored in &mut terms {
            match stored {
                Stored::Iri { prefix, .. } => *prefix = prefix_remap[*prefix as usize],
                Stored::Lit { datatype, .. } => {
                    *datatype = datatype_remap[*datatype as usize];
                }
                Stored::Blank(_) => {}
                Stored::Triple(ids) => {
                    for id in ids {
                        *id = map_component(*id);
                    }
                }
            }
        }

        let mut rebuilt = Dict {
            prefixes,
            prefix_ids,
            datatypes,
            datatype_ids,
            terms,
            table: HashTable::new(),
            blob: None,
            #[cfg(feature = "mmap")]
            mapped: None,
            base: 0,
            frozen: None,
        };
        // Release the old table/blob/mapping before allocating the rebuilt lookup table.
        drop(self);
        rebuilt.build_table();
        (rebuilt, remap)
    }

    /// (Re)builds the content-hash lookup table from the term arena — for dictionaries
    /// assembled WITHOUT incremental table inserts (`ShardedDict::into_merged`), so the
    /// result supports `lookup`/`intern` like any serially-built dict. The per-term
    /// hashing (the dominant cost) runs in parallel; the inserts are a fast serial pass
    /// (the table is pre-sized, so no resize re-hashing). Arena-mode only.
    pub fn build_table(&mut self) {
        debug_assert!(self.base == 0 && self.blob.is_none(), "build_table is arena-mode only");
        let hashes: Vec<u64> = {
            #[cfg(feature = "parallel")]
            {
                use rayon::prelude::*;
                self.terms.par_iter().map(|t| hash_stored(t, &self.prefixes, &self.datatypes)).collect()
            }
            #[cfg(not(feature = "parallel"))]
            self.terms.iter().map(|t| hash_stored(t, &self.prefixes, &self.datatypes)).collect()
        };
        let mut table = HashTable::with_capacity(self.terms.len());
        for (i, &h) in hashes.iter().enumerate() {
            table.insert_unique(h, (i as Id) + 1, |&j| hashes[(j - 1) as usize]);
        }
        self.table = table;
    }

    /// Serialises the dictionary (prefixes, datatypes, compact terms) to `path` in a
    /// compact binary format. The hash table is NOT written — it is rebuilt on `open`.
    /// Arena-mode only (it serialises `terms`); the compacted/mmap'd modes persist via
    /// [`save_mmap`](Self::save_mmap), which handles appended terms too.
    pub fn save(&self, path: &std::path::Path) -> std::io::Result<()> {
        use std::io::Write;
        assert_eq!(self.base, 0, "Dict::save is arena-mode only; use save_mmap for blob/mmap'd (or grown) dicts");
        let mut w = std::io::BufWriter::new(std::fs::File::create(path)?);
        w.write_all(&(self.prefixes.len() as u32).to_le_bytes())?;
        for p in &self.prefixes {
            write_str(&mut w, p)?;
        }
        w.write_all(&(self.datatypes.len() as u32).to_le_bytes())?;
        for d in &self.datatypes {
            write_str(&mut w, d.as_str())?;
        }
        w.write_all(&(self.terms.len() as u32).to_le_bytes())?;
        for t in &self.terms {
            match t {
                Stored::Iri { prefix, suffix } => {
                    w.write_all(&[0])?;
                    w.write_all(&prefix.to_le_bytes())?;
                    write_str(&mut w, suffix)?;
                }
                Stored::Lit { value, datatype, lang } => {
                    w.write_all(&[1])?;
                    write_str(&mut w, value)?;
                    w.write_all(&datatype.to_le_bytes())?;
                    match lang {
                        Some(l) => {
                            w.write_all(&[1])?;
                            write_str(&mut w, l)?;
                        }
                        None => w.write_all(&[0])?,
                    }
                }
                Stored::Blank(b) => {
                    w.write_all(&[2])?;
                    write_str(&mut w, b)?;
                }
                Stored::Triple(ids) => {
                    w.write_all(&[3])?;
                    for id in ids {
                        w.write_all(&id.to_le_bytes())?;
                    }
                }
            }
        }
        w.flush()
    }

    /// Loads a dictionary written by [`save`](Self::save), rebuilding the hash table.
    pub fn open(path: &std::path::Path) -> std::io::Result<Dict> {
        let file = std::fs::File::open(path)?;
        // [OPUS-4.8] sq-f5jh: the legacy single-file `dict.bin` is an UNTRUSTED on-disk file
        // (trust boundary B5), the same as the mmap dict files. Its entry COUNTS (np/nd/nt)
        // and every string LENGTH are read straight from it; `Vec::with_capacity(n)` /
        // `vec![0u8; n]` on a hostile count drove a multi-GB allocation that ABORTS the
        // process (uncatchable OOM DoS — the residual coverage-flake / rc=101 trigger under
        // llvm-cov memory pressure). The file length is a hard upper bound: every prefix /
        // datatype / term entry costs >= 1 byte on disk, and every string length costs
        // <= the bytes that can still remain. Clamp each pre-allocation and bound each
        // `read_str` to it; the per-entry `read_exact`s still error cleanly if the file
        // actually ends early. Mirrors the `open_mmap` hardening.
        let file_len = file.metadata()?.len() as usize;
        let mut r = std::io::BufReader::new(file);
        let np = read_u32(&mut r)? as usize;
        let mut prefixes = Vec::with_capacity(np.min(file_len));
        let mut prefix_ids = FxHashMap::default();
        for i in 0..np {
            let p = read_str_bounded_io(&mut r, file_len)?;
            prefix_ids.insert(p.clone(), i as u32);
            prefixes.push(p);
        }
        let nd = read_u32(&mut r)? as usize;
        let mut datatypes = Vec::with_capacity(nd.min(file_len));
        let mut datatype_ids = FxHashMap::default();
        for i in 0..nd {
            let d = read_str_bounded_io(&mut r, file_len)?;
            datatype_ids.insert(d.clone(), i as u32);
            datatypes.push(NamedNode::new_unchecked(String::from(d)));
        }
        let timing = std::env::var("SPARQ_DICT_TIMING").is_ok();
        let t0 = std::time::Instant::now();
        let nt = read_u32(&mut r)? as usize;
        let mut terms = Vec::with_capacity(nt.min(file_len));
        for _ in 0..nt {
            terms.push(match read_u8(&mut r)? {
                0 => Stored::Iri { prefix: read_u32(&mut r)?, suffix: read_str_bounded_io(&mut r, file_len)? },
                1 => {
                    let value = read_str_bounded_io(&mut r, file_len)?;
                    let datatype = read_u32(&mut r)?;
                    let lang = if read_u8(&mut r)? == 1 { Some(read_str_bounded_io(&mut r, file_len)?) } else { None };
                    Stored::Lit { value, datatype, lang }
                }
                2 => Stored::Blank(read_str_bounded_io(&mut r, file_len)?),
                3 => Stored::Triple([read_u32(&mut r)?, read_u32(&mut r)?, read_u32(&mut r)?]),
                other => return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, format!("bad term tag {other}"))),
            });
        }
        let t_parse = t0.elapsed();
        // Rebuild the hash table from the arena.
        let t1 = std::time::Instant::now();
        let mut table = HashTable::with_capacity(nt);
        for (i, t) in terms.iter().enumerate() {
            let id = (i as Id) + 1;
            let hash = hash_stored(t, &prefixes, &datatypes);
            table.insert_unique(hash, id, |&j| hash_stored(&terms[(j - 1) as usize], &prefixes, &datatypes));
        }
        if timing {
            eprintln!(
                "[dict open] {nt} terms: read+parse {:.2}s, hashtable rebuild {:.2}s",
                t_parse.as_secs_f64(),
                t1.elapsed().as_secs_f64(),
            );
        }
        Ok(Dict {
            prefixes,
            prefix_ids,
            datatypes,
            datatype_ids,
            terms,
            table,
            blob: None,
            #[cfg(feature = "mmap")]
            mapped: None,
            base: 0,
            frozen: None,
        })
    }

    /// Serialises the dictionary for the MEMORY-MAPPED, out-of-core open path: a small
    /// meta file (prefixes + datatypes) plus four mmap-friendly blobs — the term records,
    /// their byte offsets, and a hash-sorted `(hash, id)` lookup index. Opened by
    /// [`open_mmap`](Self::open_mmap) with NOTHING large resident and no table rebuild.
    #[cfg(feature = "mmap")]
    pub fn save_mmap(&self, dir: &std::path::Path) -> std::io::Result<()> {
        use std::io::Write;
        std::fs::create_dir_all(dir)?;
        // ALL ids — the blob/mmap'd base plus any APPENDED (delta-overlay) terms — so a
        // grown dictionary persists totally (the compaction path relies on this).
        let n = self.len();

        // meta: [OPUS-4.8] (review 1409) version header (magic + version + id partition), then
        // prefixes, datatypes, term count.
        let mut meta = std::io::BufWriter::new(std::fs::File::create(dir.join("dict-meta.bin"))?);
        meta.write_all(&DICT_META_MAGIC.to_le_bytes())?;
        meta.write_all(&DICT_META_VERSION.to_le_bytes())?;
        meta.write_all(&INLINE_BASE.to_le_bytes())?; // the id-space partition this file encodes
        meta.write_all(&(self.prefixes.len() as u32).to_le_bytes())?;
        for p in &self.prefixes {
            write_str(&mut meta, p)?;
        }
        meta.write_all(&(self.datatypes.len() as u32).to_le_bytes())?;
        for d in &self.datatypes {
            write_str(&mut meta, d.as_str())?;
        }
        meta.write_all(&(n as u64).to_le_bytes())?;
        meta.flush()?;

        // term blob + per-term byte offsets; collect (hash, id) for the lookup index.
        let mut blob = std::io::BufWriter::new(std::fs::File::create(dir.join("dict-terms.bin"))?);
        let mut offsets: Vec<u64> = Vec::with_capacity(n);
        let mut pairs: Vec<(u64, u32)> = Vec::with_capacity(n);
        let mut pos: u64 = 0;
        for id in 1..=n as Id {
            offsets.push(pos);
            let r = self.record(id);
            pos += write_record(&mut blob, &r)?;
            pairs.push((hash_stored_ref(&r, &self.prefixes, &self.datatypes), id));
        }
        blob.flush()?;
        write_pod_slice(&dir.join("dict-offs.bin"), &offsets)?;

        // hash-sorted parallel arrays (binary-searchable lookup). Sorted by (hash, id) —
        // not just hash — so equal-hash tie order is CANONICAL: the spilled-dict build
        // (`dictspill`) produces these files by external sort and must match byte-for-byte.
        // Lookup semantics are unchanged (it walks the whole equal-hash range).
        pairs.sort_unstable();
        let hashes: Vec<u64> = pairs.iter().map(|&(h, _)| h).collect();
        let ids: Vec<u32> = pairs.iter().map(|&(_, id)| id).collect();
        write_pod_slice(&dir.join("dict-hash.bin"), &hashes)?;
        write_pod_slice(&dir.join("dict-hid.bin"), &ids)?;
        Ok(())
    }

    /// Opens a dictionary written by [`save_mmap`](Self::save_mmap) with the term store +
    /// lookup index MEMORY-MAPPED: open is just `mmap` (no term parse, no table rebuild),
    /// and the large data stays off-heap. Only the small prefix/datatype tables are read
    /// into RAM. Read-only (no further interning).
    #[cfg(feature = "mmap")]
    pub fn open_mmap(dir: &std::path::Path) -> std::io::Result<Dict> {
        use std::io::Read;
        let meta_file = std::fs::File::open(dir.join("dict-meta.bin"))?;
        // [OPUS-4.8] sq-znld: `dict-meta.bin` is attacker-controlled. Its entry COUNTS (np,
        // nd) and every string LENGTH are u32s read from it; the original code fed them
        // straight to `Vec::with_capacity` / `vec![0u8; n]`, so one flipped count byte drove
        // a multi-GB allocation that ABORTS the process (uncatchable OOM DoS). The file
        // length is a hard upper bound on any count (each entry costs >= 4 bytes) and on any
        // string length, so we clamp pre-allocations to it and bound each `read_str`.
        let meta_len = meta_file.metadata()?.len() as usize;
        let mut r = std::io::BufReader::new(meta_file);
        // [OPUS-4.8] (review 1409) Validate the on-disk id-space partition before trusting any
        // persisted RAW id. A header-less file (legacy, written before this marker) or one
        // whose INLINE_BASE differs from the running build must be REJECTED — its inline vs
        // dictionary id ranges would be misinterpreted, silently corrupting numeric values.
        let magic = read_u32(&mut r)?;
        if magic != DICT_META_MAGIC {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "dict-meta.bin is in a legacy (pre-versioning) format whose id-space partition \
                 is unknown; rebuild the store with the current version",
            ));
        }
        let version = read_u32(&mut r)?;
        if version != DICT_META_VERSION {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!(
                    "dict-meta.bin format version {version} is not supported (this build expects {DICT_META_VERSION}); rebuild the store"
                ),
            ));
        }
        let file_inline_base = read_u32(&mut r)?;
        if file_inline_base != INLINE_BASE {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!(
                    "store id-space partition mismatch: file INLINE_BASE={file_inline_base}, this build={INLINE_BASE}; \
                     persisted ids would be misinterpreted — rebuild the store"
                ),
            ));
        }
        // Each entry occupies >= 4 bytes (its length prefix), so a count larger than the
        // meta file is corrupt; clamp the reservation regardless so it can never abort. The
        // read loop then errors cleanly via `read_exact` if the file actually ends early.
        let np = read_u32(&mut r)? as usize;
        let mut prefixes = Vec::with_capacity(np.min(meta_len / 4));
        for _ in 0..np {
            prefixes.push(read_str_bounded(&mut r, meta_len)?);
        }
        let nd = read_u32(&mut r)? as usize;
        let mut datatypes = Vec::with_capacity(nd.min(meta_len / 4));
        for _ in 0..nd {
            datatypes.push(NamedNode::new_unchecked(String::from(read_str_bounded(&mut r, meta_len)?)));
        }
        let mut nbuf = [0u8; 8];
        r.read_exact(&mut nbuf)?;
        let len = u64::from_le_bytes(nbuf) as usize;

        let map = |name: &str| -> std::io::Result<memmap2::Mmap> {
            let f = std::fs::File::open(dir.join(name))?;
            // SAFETY: read-only mapping of a file owned by this dict for its lifetime.
            unsafe { memmap2::Mmap::map(&f) }
        };
        let mapped = MappedDict {
            blob: map("dict-terms.bin")?,
            offsets: map("dict-offs.bin")?,
            hashes: map("dict-hash.bin")?,
            hashids: map("dict-hid.bin")?,
        };
        // [OPUS-4.8] sq-znld: the four data files above are attacker-controlled. Validate
        // their sizes + every offset + every record (bounds + UTF-8 + table-index range)
        // BEFORE handing back a Dict whose `term`/`lookup` paths would otherwise
        // dereference them unchecked.
        mapped.validate(len, &prefixes, &datatypes)?;
        Ok(Dict { prefixes, datatypes, mapped: Some(std::sync::Arc::new(mapped)), base: len, ..Default::default() })
    }

    /// A structural FORK of this dictionary (the dict half of `Graph::fork`): the
    /// immutable base is SHARED (an `Arc` bump) and only the small extension — terms
    /// interned since the base was frozen, plus the prefix/datatype tables — is
    /// copied. New terms interned in the fork get ids above the base's high-water
    /// mark (`base + 1 ..`), still below [`INLINE_BASE`], so the id-space partition
    /// (dict / inline-integer / local-vocab) is untouched and ids never collide
    /// across generations: a term present in the shared base resolves to the SAME id
    /// in every fork.
    ///
    /// The FIRST fork of a flat (never-forked) dict pays a one-time O(n) deep copy
    /// to freeze the shared base; every later fork of the resulting lineage is
    /// O(extension). `Graph::compact` re-freezes (folds the extension into a fresh
    /// shared base), so the extension — and with it the per-fork cost — stays
    /// bounded by the compaction policy.
    pub fn fork(&self) -> Dict {
        if self.frozen.is_some() {
            // Already forked: Clone bumps the frozen (and mapped) Arcs and copies
            // only the extension arena/table + the small prefix/datatype tables.
            return self.clone();
        }
        let base_len = self.len();
        Dict {
            prefixes: self.prefixes.clone(),
            prefix_ids: self.prefix_ids.clone(),
            datatypes: self.datatypes.clone(),
            datatype_ids: self.datatype_ids.clone(),
            terms: Vec::new(),
            table: HashTable::new(),
            blob: None,
            #[cfg(feature = "mmap")]
            mapped: None,
            base: base_len,
            // The one-time freeze: a full copy of the flat dict becomes the shared
            // immutable base (`self` itself cannot be moved into the Arc — it stays
            // usable, and is typically a published snapshot behind its own Arc).
            frozen: Some(std::sync::Arc::new(self.clone())),
        }
    }

    /// Whether this dictionary was produced by [`fork`](Self::fork) (it resolves a
    /// shared frozen base plus a local extension).
    pub fn is_forked(&self) -> bool {
        self.frozen.is_some()
    }

    /// Number of terms in the growth extension above the (blob / mmap'd / frozen)
    /// base — the dict half of a fork's per-generation copy cost, and an input to
    /// the compaction threshold policy. 0 for a plain arena dict.
    pub fn appended_len(&self) -> usize {
        if self.base > 0 || self.frozen.is_some() {
            self.terms.len()
        } else {
            0
        }
    }

    /// Folds a forked dictionary's extension into a FRESH frozen base (ids
    /// unchanged), so subsequent [`fork`](Self::fork)s are O(1) again — the dict
    /// half of `Graph::compact`. Non-forked dicts are returned unchanged (well,
    /// cloned); O(n) — call it from a compaction path only.
    pub fn compacted(&self) -> Dict {
        if self.frozen.is_none() {
            return self.clone();
        }
        let n = self.len();
        let mut terms: Vec<Stored> = Vec::with_capacity(n);
        for id in 1..=n as Id {
            terms.push(match self.record(id) {
                StoredRef::Iri { prefix, suffix } => Stored::Iri { prefix, suffix: suffix.into() },
                StoredRef::Lit { value, datatype, lang } => {
                    Stored::Lit { value: value.into(), datatype, lang: lang.map(Into::into) }
                }
                StoredRef::Blank(b) => Stored::Blank(b.into()),
                StoredRef::Triple(ids) => Stored::Triple(ids),
            });
        }
        // The fork's prefix/datatype tables are supersets of the frozen base's with
        // identical indices (fork-time clone + append-only growth), so every record
        // copied above stays valid against them.
        let mut flat = Dict {
            prefixes: self.prefixes.clone(),
            prefix_ids: self.prefix_ids.clone(),
            datatypes: self.datatypes.clone(),
            datatype_ids: self.datatype_ids.clone(),
            terms,
            table: HashTable::new(),
            blob: None,
            #[cfg(feature = "mmap")]
            mapped: None,
            base: 0,
            frozen: None,
        };
        flat.build_table();
        Dict {
            prefixes: flat.prefixes.clone(),
            prefix_ids: flat.prefix_ids.clone(),
            datatypes: flat.datatypes.clone(),
            datatype_ids: flat.datatype_ids.clone(),
            terms: Vec::new(),
            table: HashTable::new(),
            blob: None,
            #[cfg(feature = "mmap")]
            mapped: None,
            base: n,
            frozen: Some(std::sync::Arc::new(flat)),
        }
    }

    pub fn len(&self) -> usize {
        // `base` is the blob/mmap'd record count (0 for a plain arena dict); appended
        // (delta-overlay) terms live in the arena above it.
        self.base + self.terms.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// [OPUS-4.8] (sq-hxgb) Whether this dict stores any RDF 1.2 triple term
    /// (`Stored::Triple`).
    //
    // [OPUS-4.8] (sq-jvbr) The SOLE caller is the DICT-SPILL external builder's
    // `SpillInterner::intern_batch` (`#[cfg(feature = "dict-spill")]`), which uses it to SKIP
    // the serial triple-term arena pass on the common triple-term-free hot path. Gated to
    // `dict-spill` so the method is not dead code on the default / wasm builds, which
    // `clippy -D warnings` (`-D dead-code`) would otherwise reject. (`any_triple_terms`, the
    // implementation, stays ungated — `intern_partials` uses it on every build.)
    #[cfg(feature = "dict-spill")]
    pub(crate) fn has_triple_terms(&self) -> bool {
        self.any_triple_terms()
    }

    /// [OPUS-4.8] (sq-perf) Cheap enum-tag scan for any stored RDF 1.2 triple term, used by
    /// `ShardedDict::intern_partials` to SKIP the serial triple-term second pass on the common
    /// triple-term-free hot path. Unlike [`has_triple_terms`](Self::has_triple_terms) this is
    /// NOT gated on `parallel` (the sharded interner runs in the non-parallel build too, and the
    /// dict-spill builders call `intern_partials` directly), and it short-circuits on the first
    /// `Stored::Triple`, so plain N-Triples/Turtle pay one bail-fast pass, not a `term_parts`
    /// reconstruction over every term.
    pub(crate) fn any_triple_terms(&self) -> bool {
        self.terms.iter().any(|s| matches!(s, Stored::Triple(_)))
    }

    /// Iterates every REAL dictionary term in id order as `(id, parts)` — the official
    /// form of the dense-id contract (real ids are exactly `1..=len()`, assigned in
    /// interning order) that downstream crates previously leaned on by hand via
    /// `term_parts(1..=len())`. Borrowing and allocation-free per item
    /// ([`term_parts`](Self::term_parts)); reconstruct an `oxrdf::Term` with
    /// [`term`](Self::term) where needed. Inline integer ids (see [`is_inline`]) are
    /// value-carrying, not dictionary entries, so they do not appear here.
    pub fn iter(&self) -> impl ExactSizeIterator<Item = (Id, TermParts<'_>)> {
        // 0..len (a `Range<usize>`, which is ExactSize) mapped to 1-based ids.
        (0..self.len()).map(move |i| {
            let id = (i + 1) as Id;
            (id, self.term_parts(id))
        })
    }

    /// A rough estimate of the dictionary's heap footprint in bytes (for
    /// benchmarking). Counts the compact `terms` arena (slots + suffix/value/lang
    /// bytes), the shared prefix + datatype tables, and the hash table (bare ids).
    pub fn heap_bytes(&self) -> usize {
        // A memory-mapped dictionary keeps only the small prefix/datatype tables resident;
        // the term blob + offsets + lookup index are mmap'd (OS page cache, not the heap).
        // Appended (delta-overlay) terms live in the in-RAM arena in every mode.
        // A forked dict's Arc-shared frozen base counts in full (reachable footprint,
        // like the store's Arc-shared permutations) — after `compacted()` the ENTIRE
        // dictionary lives there (roborev 1945, same omission class as the caches).
        let frozen_base: usize = self.frozen.as_ref().map_or(0, |f| f.heap_bytes());
        let appended: usize = self.terms.capacity() * std::mem::size_of::<Stored>()
            + self.terms.iter().map(stored_owned_bytes).sum::<usize>();
        #[cfg(feature = "mmap")]
        if self.mapped.is_some() {
            let prefix_bytes: usize = self.prefixes.iter().map(|p| p.len() + std::mem::size_of::<Box<str>>()).sum();
            let dt_bytes: usize = self.datatypes.iter().map(|d| d.as_str().len() + 32).sum();
            let table = self.table.capacity() * (std::mem::size_of::<Id>() + 1);
            return frozen_base + appended + table + prefix_bytes + dt_bytes;
        }
        // Compacted (blob) mode: the term blob + u32 offsets + the kept hash table, plus
        // the small prefix/datatype tables — no per-`Stored` slot or per-`Box<str>` term.
        if let Some((blob, offs)) = &self.blob {
            let prefix_bytes: usize = self.prefixes.iter().map(|p| p.len() + std::mem::size_of::<Box<str>>()).sum::<usize>() * 2;
            let dt_bytes: usize = self.datatypes.iter().map(|d| d.as_str().len() + 32).sum();
            let table = self.table.capacity() * (std::mem::size_of::<Id>() + 1);
            return frozen_base + appended + blob.capacity() + offs.capacity() * 4 + table + prefix_bytes + dt_bytes;
        }
        let term_slots = self.terms.capacity() * std::mem::size_of::<Stored>();
        let owned: usize = self.terms.iter().map(stored_owned_bytes).sum();
        let prefix_bytes: usize =
            self.prefixes.iter().map(|p| p.len() + std::mem::size_of::<Box<str>>()).sum::<usize>() * 2; // Vec + map key
        let dt_bytes: usize = self.datatypes.iter().map(|d| d.as_str().len() + 32).sum();
        let table = self.table.capacity() * (std::mem::size_of::<Id>() + 1);
        frozen_base + term_slots + owned + prefix_bytes + dt_bytes + table
    }
}

// ---- (de)serialisation helpers for save/open --------------------------------

pub(crate) fn write_str(w: &mut impl std::io::Write, s: &str) -> std::io::Result<()> {
    w.write_all(&(s.len() as u32).to_le_bytes())?;
    w.write_all(s.as_bytes())
}

/// Writes one compact term record (the format `parse_stored_ref` reads) and returns the
/// number of bytes written — for building the mmap term blob + its offset index.
pub(crate) fn write_record(w: &mut impl std::io::Write, t: &StoredRef) -> std::io::Result<u64> {
    Ok(match t {
        StoredRef::Iri { prefix, suffix } => {
            w.write_all(&[0])?;
            w.write_all(&prefix.to_le_bytes())?;
            write_str(w, suffix)?;
            1 + 4 + 4 + suffix.len() as u64
        }
        StoredRef::Lit { value, datatype, lang } => {
            w.write_all(&[1])?;
            write_str(w, value)?;
            w.write_all(&datatype.to_le_bytes())?;
            let lang_bytes = match lang {
                Some(l) => {
                    w.write_all(&[1])?;
                    write_str(w, l)?;
                    1 + 4 + l.len() as u64
                }
                None => {
                    w.write_all(&[0])?;
                    1
                }
            };
            1 + (4 + value.len() as u64) + 4 + lang_bytes
        }
        StoredRef::Blank(b) => {
            w.write_all(&[2])?;
            write_str(w, b)?;
            1 + 4 + b.len() as u64
        }
        StoredRef::Triple(ids) => {
            w.write_all(&[3])?;
            for id in ids {
                w.write_all(&id.to_le_bytes())?;
            }
            1 + 12
        }
    })
}

/// [OPUS-4.8] sq-lvw8 (sq-ueuk/#418 follow-up): the mmap on-disk dictionary format is
/// little-endian. [`write_pod_slice`] / [`MappedDict::slice_u64`] / [`MappedDict::hashids`]
/// move the offset/hash/id arrays through a raw native-endian byte cast, while
/// [`validate_dict_bytes`] and the `dict-spill` external builder use explicit
/// `from_le_bytes` / `to_le_bytes`; the two only agree (and the in-RAM vs spilled build only
/// stay byte-identical) on a little-endian host. Make that a BUILD error, not silent on-disk
/// corruption, on the day someone targets a big-endian platform — at which point both the raw
/// writer and the unsafe readers must be reworked onto explicit `*_le_bytes`. (`compile_error!`
/// here would be evaluated even when the `mmap` cfg is inactive, so this uses a const guard,
/// which is monomorphised only when the feature is on.)
#[cfg(feature = "mmap")]
const DICT_ON_DISK_LE_GUARD: () = assert!(
    cfg!(target_endian = "little"),
    "the mmap/dict-spill on-disk dictionary format is little-endian only (sq-lvw8): \
     write_pod_slice + the MappedDict pointer-cast readers are native-endian, but \
     validate_dict_bytes and the dict-spill builder are explicit-LE, so they diverge on a \
     big-endian target. Port both to explicit *_le_bytes before enabling these features on BE."
);

/// Writes a slice of plain-old-data (u32/u64) as raw bytes (for the mmap offset / hash /
/// id arrays — `dict-offs.bin` / `dict-hash.bin` / `dict-hid.bin`).
///
/// [OPUS-4.8] sq-lvw8 (sq-ueuk/#418 follow-up): this is a raw `transmute`-style byte copy,
/// so the on-disk byte order is the HOST's native endianness — little-endian on every target
/// sparq currently builds for. The whole mmap dictionary format relies on that:
///   * the mmap readers ([`MappedDict::slice_u64`] / [`MappedDict::hashids`]) reinterpret
///     these bytes as `&[u64]` / `&[u32]` via a pointer cast — also native-endian — so the
///     in-RAM write/read round-trip is self-consistent on ANY single target, BUT
///   * [`validate_dict_bytes`] and the `dict-spill` external builder (`dictspill.rs`) read
///     and WRITE the very same files with EXPLICIT `from_le_bytes` / `to_le_bytes`.
///
/// On a little-endian host all three agree, so `save_mmap`'s output is byte-for-byte identical
/// to the spilled-build output and `validate_dict_bytes` accepts it — exactly the equivalence
/// PR #418 relied on when it called its validator refactor "byte-for-byte unchanged". On a
/// BIG-endian host the native-endian writer/reader here would diverge from the explicit-LE
/// `validate_dict_bytes` / `dictspill` paths, so a freshly-written store would FAIL its own
/// `open_mmap` validation and the two build paths would no longer be byte-identical. Rather
/// than carry that silent-corruption hazard, the persistence features are STATICALLY pinned to
/// little-endian targets by [`DICT_ON_DISK_LE_GUARD`]; the only correct way to support BE would
/// be to route BOTH this writer and the unsafe readers through explicit `*_le_bytes` too.
#[cfg(feature = "mmap")]
fn write_pod_slice<T: Copy>(path: &std::path::Path, data: &[T]) -> std::io::Result<()> {
    // Force-evaluate the little-endian on-disk-format guard (sq-lvw8): referencing the const
    // both keeps it from being dead-code-eliminated and makes a big-endian build fail HERE,
    // at the native-endian raw byte write, rather than silently emit a BE file.
    let () = DICT_ON_DISK_LE_GUARD;
    // SAFETY: T is u32/u64 (POD); we only read its bytes.
    let bytes = unsafe { std::slice::from_raw_parts(data.as_ptr().cast::<u8>(), std::mem::size_of_val(data)) };
    std::fs::write(path, bytes)
}

fn read_u32(r: &mut impl std::io::Read) -> std::io::Result<u32> {
    let mut b = [0u8; 4];
    r.read_exact(&mut b)?;
    Ok(u32::from_le_bytes(b))
}

fn read_u8(r: &mut impl std::io::Read) -> std::io::Result<u8> {
    let mut b = [0u8; 1];
    r.read_exact(&mut b)?;
    Ok(b[0])
}

/// [OPUS-4.8] sq-f5jh — `read_str` for the UNTRUSTED legacy `dict.bin` loader (always
/// compiled, unlike the `mmap`-gated [`read_str_bounded`]). Rejects a declared length
/// larger than `max` (the bytes that can possibly remain in the source file) BEFORE
/// allocating, so a hostile u32 length cannot trigger a multi-GB `vec![0u8; n]`
/// allocation/abort. The subsequent `read_exact` still errors cleanly if the file ends
/// early.
fn read_str_bounded_io(r: &mut impl std::io::Read, max: usize) -> std::io::Result<Box<str>> {
    let n = read_u32(r)? as usize;
    if n > max {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "dictionary string length exceeds the file size (corrupt or truncated)",
        ));
    }
    let mut buf = vec![0u8; n];
    r.read_exact(&mut buf)?;
    String::from_utf8(buf)
        .map(String::into_boxed_str)
        .map_err(|_| std::io::Error::new(std::io::ErrorKind::InvalidData, "invalid utf-8 in dictionary"))
}

/// [OPUS-4.8] sq-znld — `read_str` for UNTRUSTED metadata: rejects a length larger than
/// `max` (the bytes that can possibly remain in the source file) BEFORE allocating, so a
/// hostile u32 length cannot trigger a multi-GB `vec![0u8; n]` allocation/abort. The
/// subsequent `read_exact` still errors cleanly if the file actually ends early.
#[cfg(feature = "mmap")]
fn read_str_bounded(r: &mut impl std::io::Read, max: usize) -> std::io::Result<Box<str>> {
    let n = read_u32(r)? as usize;
    if n > max {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "dictionary string length exceeds the metadata file size (corrupt or truncated)",
        ));
    }
    let mut buf = vec![0u8; n];
    r.read_exact(&mut buf)?;
    String::from_utf8(buf)
        .map(String::into_boxed_str)
        .map_err(|_| std::io::Error::new(std::io::ErrorKind::InvalidData, "invalid utf-8 in dictionary"))
}

/// The owned heap bytes of a compact stored term's string content (suffix / value /
/// language / blank label) — the part outside the fixed `Stored` slot. The IRI prefix
/// and datatype live once in the shared tables, not here.
fn stored_owned_bytes(s: &Stored) -> usize {
    match s {
        Stored::Iri { suffix, .. } => suffix.len(),
        Stored::Lit { value, lang, .. } => value.len() + lang.as_ref().map_or(0, |l| l.len()),
        Stored::Blank(b) => b.len(),
        Stored::Triple(_) => 0, // the three ids live inline in the slot
    }
}

// ---- Hash-sharded dictionary for PARALLEL bulk interning --------------------------
// The external-memory build's serial `merge_remap` (re-interning every parsed term into one
// global dict) is the ingest bottleneck — it's latency-bound on the single growing global
// hash table. `ShardedDict` splits it into N independent shards (a term routes to
// `hash % N`), so the interning parallelises with NO cross-shard contention. A term gets a
// TEMPORARY id `shard*STRIDE + shard_local_id` (STRIDE = INLINE_BASE/N ≥ max shard size);
// these temp ids sort in the SAME order as the final dense ids `base[shard] + local`
// (base = prefix-sum of shard sizes, monotonic), so the externally-sorted permutations need
// only an order-preserving remap — no re-sort. `into_merged` consumes the shards into one
// regular `Dict` (MOVING the term strings; only the small prefix/datatype id fields are
// remapped to unified tables), reusing the existing `save_mmap`/`numerics_of` path. Inline
// integer ids (≥ INLINE_BASE) are global and never enter a shard.

/// A hash-sharded interner — see the module comment above.
///
/// [OPUS-4.8] (sq-t3rt) RDF 1.2 triple terms `<<( s p o )>>` cannot be hash-routed like leaf
/// terms: a triple term's content IS its components' ids, and those final ids are only known
/// after every leaf consolidates (a component may live in any shard). So `shards` holds
/// `n_leaf` LEAF shards (`shards[0..n_leaf]`, routed by `hash % n_leaf`, unchanged) PLUS one
/// extra TRIPLE-TERM shard (`shards[n_leaf]`) interned in a serial second pass: its entries
/// are `Stored::Triple([temp_s, temp_p, temp_o])` content-addressed by their components' TEMP
/// ids, resolved to final ids in `into_merged`. The triple shard sorts LAST (highest `base`),
/// so every triple term's final id exceeds every leaf's — a triple term can only reference
/// already-finalised leaf / earlier-triple ids, exactly the children-first ordering serial
/// `merge_remap` relies on. `stride` is sized for `n_leaf + 1` shards so the triple shard's
/// temp ids stay below `INLINE_BASE`; FINAL ids are `base[shard] + local`, which is
/// stride-INDEPENDENT, so leaf-only output stays byte-identical to before.
pub struct ShardedDict {
    /// `n_leaf` leaf shards followed by one triple-term shard (`shards[n_leaf]`).
    shards: Vec<Dict>,
    /// Number of LEAF shards (`shards.len() - 1`); leaves route to `hash % n_leaf`.
    n_leaf: usize,
    stride: u32,
}

impl ShardedDict {
    pub fn new(n: usize) -> ShardedDict {
        let n_leaf = n.max(1);
        // `n_leaf` leaf shards + 1 triple-term shard. `stride = INLINE_BASE / (n_leaf + 1)`
        // reserves the top stride-band for the triple shard so its temp ids stay below
        // INLINE_BASE; leaf shards' FINAL ids are unaffected (final = base + local).
        ShardedDict {
            // [OPUS-4.8] `0..(n_leaf + 1)` (== `0..=n_leaf`) — `n_leaf` leaf shards plus the
            // one trailing triple-term shard. Parenthesised so the bound reads unambiguously.
            shards: (0..(n_leaf + 1)).map(|_| Dict::new()).collect(),
            n_leaf,
            stride: INLINE_BASE / (n_leaf as u32 + 1),
        }
    }

    /// The number of LEAF shards (the historical shard count; the triple shard is internal).
    pub fn n(&self) -> usize {
        self.n_leaf
    }

    /// Route + intern a batch of `(tag, idx, term)` items (the terms must be NON-inline —
    /// inline integers are handled by the caller), returning `(tag, idx, temp-id)`. The
    /// per-shard interning runs in parallel (each shard is single-writer → no contention).
    ///
    /// RDF 1.2 triple terms are NOT supported by the sharded interner (a triple's
    /// component terms would be interned both in their hash-routed shard and alongside
    /// the triple, breaking the term↔id bijection on merge); the N-Triples bulk loaders
    /// that feed it reject triple-term syntax before it could reach here.
    pub fn intern_terms(&mut self, items: Vec<(u32, Id, Term)>) -> Vec<(u32, Id, Id)> {
        // Leaves route across the `n_leaf` LEAF shards only; the trailing triple shard is
        // reserved for `intern_partials`'s structural second pass (see the type doc).
        let n = self.n_leaf;
        let stride = self.stride;
        let mut buckets: Vec<Vec<(u32, Id, Term)>> = (0..n).map(|_| Vec::new()).collect();
        for (tag, idx, t) in items {
            assert!(
                !matches!(t, Term::Triple(_)),
                "RDF 1.2 triple terms are not supported by the sharded bulk interner; use the serial loader"
            );
            let s = (hash_term(&t) % n as u64) as usize;
            buckets[s].push((tag, idx, t));
        }
        let intern_bucket = |s: usize, shard: &mut Dict, bucket: Vec<(u32, Id, Term)>| -> Vec<(u32, Id, Id)> {
            bucket
                .into_iter()
                .map(|(tag, idx, t)| {
                    let lid = shard.intern(&t);
                    // [OPUS-4.8] Always-on (not debug_assert): a shard overflowing its STRIDE
                    // makes temp-id ranges of adjacent shards overlap, which silently corrupts
                    // the order-preserving remap (`remap_sharded`) in release builds. Fail hard.
                    assert!(lid < stride, "shard {s} exceeded STRIDE ({stride}) — raise shard count / widen Id");
                    (tag, idx, (s as u32) * stride + lid)
                })
                .collect()
        };
        #[cfg(feature = "parallel")]
        {
            use rayon::prelude::*;
            self.shards
                .par_iter_mut()
                .zip(buckets)
                .enumerate()
                .map(|(s, (shard, bucket))| intern_bucket(s, shard, bucket))
                .flatten()
                .collect()
        }
        #[cfg(not(feature = "parallel"))]
        {
            self.shards
                .iter_mut()
                .zip(buckets)
                .enumerate()
                .flat_map(|(s, (shard, bucket))| intern_bucket(s, shard, bucket))
                .collect()
        }
    }

    /// The fast bulk-merge path: route every (non-inline) term of each parsed partial dict to
    /// its shard and intern it IN PARALLEL across shards — interning straight from the
    /// partial's borrowed components (`term_parts`), so there is no `Term` allocation, no IRI
    /// concat, and the cheaper part-based hashing is reused (mirrors the serial `merge_remap`
    /// per-term cost, but parallel and contention-free). Returns a per-partial remap:
    /// `remap[p][local_id] = temp_id` (`remap[p][0]` unused).
    ///
    /// FULLY parallel (measured: the old serial hash-routing scan + serial remap-vec
    /// scatter were ~half the "merge" bucket): the routing runs per-partial in parallel
    /// (each partial fills its own per-shard sub-buckets — no shared state), the interning
    /// runs per-shard in parallel (each shard walks the partials IN ORDER, so per-shard id
    /// assignment — and therefore the merged dict and every downstream byte — is identical
    /// to the old serial-routed version), and each shard scatters its temp ids straight
    /// into the shared remap table (disjoint writes: a term instance `(partial, local-id)`
    /// is hash-routed to exactly one shard).
    ///
    /// [OPUS-4.8] (sq-t3rt) RDF 1.2 triple terms (`Stored::Triple`) are handled in a SERIAL
    /// second pass into the dedicated triple shard (`shards[n_leaf]`): they cannot be
    /// hash-routed (their content is their components' ids, only resolvable once the leaves
    /// are interned), so each is content-addressed by its components' already-known TEMP ids
    /// (children precede the triple in a partial's arena — `intern` pushes them first — so
    /// the per-partial remap entry of each component, leaf or nested triple, is already
    /// filled when the triple is reached). Triple terms are reification metadata (rare), so
    /// the serial pass is not on the hot path.
    ///
    /// # Errors
    /// [OPUS-4.8] Returns `Err` if a partial is malformed — a triple term whose component
    /// local id is out of the partial's range, or references a slot that was never
    /// routed/interned (e.g. children-first arena ordering was violated). Such input is
    /// rejected with a descriptive message rather than silently mis-indexing the remap.
    pub fn intern_partials(&mut self, partials: &[(Dict, Vec<[Id; 3]>)]) -> Result<Vec<Vec<Id>>, String> {
        let n = self.n_leaf;
        let stride = self.stride;
        // Route each partial's LEAF terms to per-leaf-shard sub-buckets (parallel over
        // partials). Triple terms (`TermParts::Triple`) are skipped here — they take the
        // serial second pass below.
        fn route<'a>(pd: &'a Dict, n: usize) -> Vec<Vec<(Id, TermParts<'a>)>> {
            let mut b: Vec<Vec<(Id, TermParts<'a>)>> = (0..n).map(|_| Vec::with_capacity(pd.len() / n + 1)).collect();
            for i in 1..=pd.len() as Id {
                let tp = pd.term_parts(i);
                if matches!(tp, TermParts::Triple(_)) {
                    continue; // a triple term — interned structurally in the serial pass
                }
                let s = (hash_termparts(&tp) % n as u64) as usize;
                b[s].push((i, tp));
            }
            b
        }
        #[cfg(feature = "parallel")]
        let routed: Vec<Vec<Vec<(Id, TermParts)>>> = {
            use rayon::prelude::*;
            partials.par_iter().map(|(pd, _)| route(pd, n)).collect()
        };
        #[cfg(not(feature = "parallel"))]
        let routed: Vec<Vec<Vec<(Id, TermParts)>>> = partials.iter().map(|(pd, _)| route(pd, n)).collect();

        // Pre-size the remap table; shards scatter into it with DISJOINT writes.
        let mut remaps: Vec<Vec<Id>> = partials.iter().map(|(pd, _)| vec![0 as Id; pd.len() + 1]).collect();
        // Raw view of `remaps` so each shard can write its own (partial, local-id) slots
        // from a parallel context. SAFETY (disjointness): the hash routing above assigns
        // every (pidx, i) leaf slot to exactly ONE leaf shard, so no two shards write the
        // same slot, and nobody reads until the parallel scope ends. (Triple-term slots are
        // written afterwards by the serial pass, never concurrently.)
        #[derive(Clone, Copy)]
        struct SlotPtr(*mut Id, usize);
        // SAFETY: `SlotPtr` (a `*mut Id`) only ever touches its own disjoint slot — the
        // hash routing above assigns each (pidx, i) leaf slot to exactly one shard, so the
        // raw-pointer handle can be sent/shared across the parallel scatter without
        // aliasing; no reads happen until the parallel scope ends. [OPUS-4.8 sq-8wbn]
        unsafe impl Send for SlotPtr {}
        // SAFETY: shared read of a `Copy` raw-pointer handle; the writes it performs are
        // disjoint by the hash routing above (see the `Send` argument). [OPUS-4.8 sq-8wbn]
        unsafe impl Sync for SlotPtr {}
        let scatter: Vec<SlotPtr> = remaps.iter_mut().map(|v| SlotPtr(v.as_mut_ptr(), v.len())).collect();

        // Intern each LEAF shard's terms in parallel (single-writer per shard → no
        // contention), walking partials in order so per-shard id assignment matches the
        // serial routing.
        let intern_shard = |s: usize, shard: &mut Dict| {
            for (pidx, per_partial) in routed.iter().enumerate() {
                let SlotPtr(ptr, len) = scatter[pidx];
                for (i, tp) in &per_partial[s] {
                    let lid = shard.intern_parts(tp);
                    // [OPUS-4.8] Always-on (not debug_assert): STRIDE overflow makes adjacent
                    // shards' temp-id ranges overlap and silently corrupts `remap_sharded`
                    // (the scatter below would also write a wrong final id). Fail hard.
                    assert!(lid < stride, "shard {s} exceeded STRIDE ({stride}) — raise shard count / widen Id");
                    debug_assert!((*i as usize) < len);
                    // SAFETY: i < len (local ids are 1..=pd.len() < pd.len()+1) and this
                    // (pidx, i) slot is written by exactly this shard — see SlotPtr.
                    unsafe { ptr.add(*i as usize).write((s as u32) * stride + lid) };
                }
            }
        };
        // Only the `n_leaf` leaf shards take the parallel pass; the triple shard (index
        // `n_leaf`) is filled serially below.
        #[cfg(feature = "parallel")]
        {
            use rayon::prelude::*;
            self.shards[..n].par_iter_mut().enumerate().for_each(|(s, shard)| intern_shard(s, shard));
        }
        #[cfg(not(feature = "parallel"))]
        self.shards[..n].iter_mut().enumerate().for_each(|(s, shard)| intern_shard(s, shard));

        // [OPUS-4.8] (sq-t3rt) Serial second pass — triple terms into the triple shard.
        // [OPUS-4.8] (sq-perf) FAST PATH: triple terms are rare reification metadata, but the
        // common N-Triples/Turtle hot path is triple-term-FREE. Skip the whole serial second
        // pass (a per-term `term_parts` walk over every partial) unless some partial actually
        // carries one — `any_triple_terms` short-circuits on a cheap enum-tag scan that bails
        // on the first `Stored::Triple`, so plain input no longer pays the redundant O(terms)
        // walk #91 added. (Measured: this restores the parse hot path to its pre-#91 cost.)
        if partials.iter().any(|(pd, _)| pd.any_triple_terms()) {
            self.intern_triple_terms(partials, &mut remaps)?;
        }
        Ok(remaps)
    }

    /// [OPUS-4.8] (sq-t3rt) Intern every partial's RDF 1.2 triple terms into the dedicated
    /// triple shard (`shards[n_leaf]`), filling their slots in `remaps`. Runs AFTER the leaf
    /// pass, so every component's temp id is already in `remaps` (children precede their
    /// triple in a partial's arena). Each triple is content-addressed by its components'
    /// TEMP ids, so identical triple terms across partials share one triple-shard id (the
    /// term↔id bijection the hash-routed path could not preserve). Serial: triple-shard id
    /// assignment must follow first-occurrence (partial, then arena) order deterministically,
    /// and triple terms are rare metadata.
    fn intern_triple_terms(&mut self, partials: &[(Dict, Vec<[Id; 3]>)], remaps: &mut [Vec<Id>]) -> Result<(), String> {
        let tshard_idx = self.n_leaf; // the triple shard
        let stride = self.stride;
        // Resolve a component LOCAL id to its TEMP id via this partial's remap. Inline
        // integers (≥ INLINE_BASE) are global and pass through unchanged. A component is a
        // leaf (filled by the parallel pass) or a NESTED triple term (filled earlier in this
        // same ordered walk — children always precede their parent in arena order).
        //
        // [OPUS-4.8] (sq-t3rt review) A component id is VALIDATED before it indexes the remap:
        // a malformed partial that violated children-first arena ordering, or that carried a
        // component id which was never routed/interned, would otherwise index a wrong/empty
        // slot silently. Both failures are recoverable bad input, not invariant breaches, so
        // we return a descriptive `Err` rather than panicking or producing a garbage id. The
        // leaf pass fills every routed slot with `base + local ≥ stride > 0`, so a remaining
        // `0` marks an unpopulated (hence un-resolvable) component slot.
        for (pidx, (pd, _)) in partials.iter().enumerate() {
            for i in 1..=pd.len() as Id {
                if let TermParts::Triple(local_ids) = pd.term_parts(i) {
                    let rm = &remaps[pidx];
                    let resolve = |id: Id| -> Result<Id, String> {
                        if is_inline(id) {
                            return Ok(id);
                        }
                        let slot = *rm.get(id as usize).ok_or_else(|| {
                            format!(
                                "malformed triple term in partial {pidx}: component local id {id} out of range (remap len {})",
                                rm.len()
                            )
                        })?;
                        if slot == 0 {
                            return Err(format!(
                                "malformed triple term in partial {pidx}: component local id {id} references an unpopulated slot \
                                 (not routed/interned, or violates children-first ordering)"
                            ));
                        }
                        Ok(slot)
                    };
                    let temp_ids = [resolve(local_ids[0])?, resolve(local_ids[1])?, resolve(local_ids[2])?];
                    let lid = self.shards[tshard_idx].intern_triple_ids(temp_ids);
                    // The triple shard, like every shard, must not overflow its STRIDE band.
                    assert!(lid < stride, "triple shard exceeded STRIDE ({stride}) — raise shard count / widen Id");
                    remaps[pidx][i as usize] = (tshard_idx as u32) * stride + lid;
                }
            }
        }
        Ok(())
    }

    /// `base[s]` = number of terms in shards `< s` (the final id of shard `s`'s local 1 is
    /// `base[s] + 1`).
    fn bases(&self) -> Vec<u64> {
        let mut base = Vec::with_capacity(self.shards.len() + 1);
        let mut acc = 0u64;
        base.push(0);
        for sh in &self.shards {
            acc += sh.terms.len() as u64;
            base.push(acc);
        }
        base
    }

    /// Consume the shards into one merged `Dict` (final dense ids in shard order) + the
    /// `(base, stride)` to remap temp ids → final ids. Moves term strings; only the small
    /// prefix/datatype id fields are remapped to the unified tables.
    pub fn into_merged(self) -> (Dict, Vec<u64>, u32) {
        let base = self.bases();
        let stride = self.stride;
        // [OPUS-4.8] Validate capacity before producing the merged dict. (1) No shard may
        // have overflowed its STRIDE — otherwise its temp ids overlapped the next shard's
        // range and `remap_sharded` produced corrupt final ids. (2) The total distinct-term
        // count must fit in the dictionary id partition `[1, INLINE_BASE)`; otherwise final
        // dense ids would collide with the inline-integer space. Both are silent-corruption
        // bugs in release, so assert hard rather than debug_assert.
        for (s, sh) in self.shards.iter().enumerate() {
            assert!(
                (sh.terms.len() as u64) <= stride as u64,
                "shard {s} has {} terms, exceeding STRIDE ({stride}) — raise shard count / widen Id",
                sh.terms.len()
            );
        }
        let total_terms = base[base.len() - 1];
        assert!(
            total_terms < INLINE_BASE as u64,
            "merged dictionary has {total_terms} terms, exceeding the dictionary id partition (< {INLINE_BASE}) — widen Id to u64"
        );
        // Unify prefix + datatype tables; build per-shard local-id -> unified-id remaps.
        let (mut uni_prefixes, mut pidx): (Vec<Box<str>>, FxHashMap<Box<str>, u32>) = Default::default();
        let (mut uni_dts, mut didx): (Vec<NamedNode>, FxHashMap<Box<str>, u32>) = Default::default();
        let mut prefix_remap: Vec<Vec<u32>> = Vec::with_capacity(self.shards.len());
        let mut dt_remap: Vec<Vec<u32>> = Vec::with_capacity(self.shards.len());
        for sh in &self.shards {
            let pr = sh
                .prefixes
                .iter()
                .map(|p| {
                    *pidx.entry(p.clone()).or_insert_with(|| {
                        uni_prefixes.push(p.clone());
                        (uni_prefixes.len() - 1) as u32
                    })
                })
                .collect();
            prefix_remap.push(pr);
            let dr = sh
                .datatypes
                .iter()
                .map(|d| {
                    *didx.entry(d.as_str().into()).or_insert_with(|| {
                        uni_dts.push(d.clone());
                        (uni_dts.len() - 1) as u32
                    })
                })
                .collect();
            dt_remap.push(dr);
        }
        // Build the merged arena by MOVING each shard's Stored (only the id field remaps).
        // The move runs PER-SHARD IN PARALLEL into disjoint slices of the target arena
        // (offsets = the same `base` prefix sums) — at 1 B-triple scale this single pass
        // over every distinct term was a measurable serial tail of the consolidation.
        let total: usize = self.shards.iter().map(|s| s.terms.len()).sum();
        let mut terms: Vec<Stored> = Vec::with_capacity(total);
        {
            let remap_one = |stored: Stored, pr: &[u32], dr: &[u32]| -> Stored {
                match stored {
                    Stored::Iri { prefix, suffix } => Stored::Iri { prefix: pr[prefix as usize], suffix },
                    Stored::Lit { value, datatype, lang } => Stored::Lit { value, datatype: dr[datatype as usize], lang },
                    Stored::Blank(b) => Stored::Blank(b),
                    // [OPUS-4.8] (sq-t3rt) A triple term lives only in the triple shard
                    // (`shards[n_leaf]`); its component ids are TEMP ids (leaf or earlier
                    // nested-triple temps) — resolve each to its FINAL dense id with the same
                    // order-preserving `remap_sharded` the perm files use. Children sort
                    // before parents (lower base / earlier triple-shard local), so every
                    // component's final id is already well-defined.
                    Stored::Triple(ids) => {
                        Stored::Triple([
                            remap_sharded(ids[0], &base, stride),
                            remap_sharded(ids[1], &base, stride),
                            remap_sharded(ids[2], &base, stride),
                        ])
                    }
                }
            };
            #[cfg(feature = "parallel")]
            {
                use rayon::prelude::*;
                // Split the spare capacity into one disjoint slice per shard.
                let mut spare: &mut [std::mem::MaybeUninit<Stored>] = &mut terms.spare_capacity_mut()[..total];
                let mut slices: Vec<&mut [std::mem::MaybeUninit<Stored>]> = Vec::with_capacity(self.shards.len());
                for sh in &self.shards {
                    let (head, tail) = spare.split_at_mut(sh.terms.len());
                    slices.push(head);
                    spare = tail;
                }
                self.shards
                    .into_par_iter()
                    .zip(slices)
                    .enumerate()
                    .for_each(|(s, (sh, out))| {
                        let (pr, dr) = (&prefix_remap[s], &dt_remap[s]);
                        for (slot, stored) in out.iter_mut().zip(sh.terms) {
                            slot.write(remap_one(stored, pr, dr));
                        }
                    });
                // SAFETY: every one of the `total` slots was initialised exactly once above
                // (the slices partition [0, total) and each shard fills its slice fully).
                unsafe { terms.set_len(total) };
            }
            #[cfg(not(feature = "parallel"))]
            for (s, sh) in self.shards.into_iter().enumerate() {
                let (pr, dr) = (&prefix_remap[s], &dt_remap[s]);
                for stored in sh.terms {
                    terms.push(remap_one(stored, pr, dr));
                }
            }
        }
        let prefix_ids = uni_prefixes.iter().enumerate().map(|(i, p)| (p.clone(), i as u32)).collect();
        let datatype_ids = uni_dts.iter().enumerate().map(|(i, d)| (d.as_str().into(), i as u32)).collect();
        let merged = Dict { prefixes: uni_prefixes, prefix_ids, datatypes: uni_dts, datatype_ids, terms, ..Default::default() };
        (merged, base, stride)
    }
}

/// Remap a temporary sharded id to its final dense id (inline ids pass through unchanged).
#[inline]
pub fn remap_sharded(id: Id, base: &[u64], stride: u32) -> Id {
    if id >= INLINE_BASE {
        return id; // inline integer — global, never sharded
    }
    let shard = (id / stride) as usize;
    let local = (id % stride) as u64;
    (base[shard] + local) as Id
}

#[cfg(test)]
mod tests {
    use super::*;

    fn int(s: &str) -> Term {
        Term::Literal(Literal::new_typed_literal(s, xsd::INTEGER))
    }

    #[test]
    fn inline_integer_roundtrip_and_boundaries() {
        let mut d = Dict::new();
        for v in [0u32, 1, 42, INLINE_MAX] {
            let id = d.intern(&int(&v.to_string()));
            assert!(is_inline(id), "{v} should inline");
            assert_eq!(id, INLINE_BASE + v);
            assert_eq!(d.term(id), int(&v.to_string()), "round-trips to the canonical term");
            assert_eq!(d.lookup(&int(&v.to_string())), id, "lookup agrees with intern");
        }
        assert_eq!(d.len(), 0, "no inline integer is stored in the dictionary");
    }

    /// [FABLE-5] sq-7d3dj.30.21 — `plain_string_value` returns the ZERO-COPY value slice for
    /// EXACTLY a plain `xsd:string` literal (no lang tag, datatype `xsd:string`) and `None` for
    /// every other term kind — the eligibility gate the engine's lazy top-k string key relies
    /// on to stay byte-identical to the `LiteralKind::String` set.
    #[test]
    fn plain_string_value_only_plain_strings() {
        let mut d = Dict::new();
        // Plain xsd:string (explicit datatype) and a "simple" literal (implicit xsd:string):
        // both are plain strings and must return their value slice.
        let s_typed = d.intern(&Term::Literal(Literal::new_typed_literal("hello", xsd::STRING)));
        let s_simple = d.intern(&Term::Literal(Literal::new_simple_literal("world")));
        let s_empty = d.intern(&Term::Literal(Literal::new_simple_literal("")));
        assert_eq!(d.plain_string_value(s_typed), Some("hello"));
        assert_eq!(d.plain_string_value(s_simple), Some("world"));
        assert_eq!(d.plain_string_value(s_empty), Some(""));
        // Same lexical, canonical id: a simple and an explicit-xsd:string literal of the same
        // value intern to ONE id (both plain strings).
        assert_eq!(
            d.intern(&Term::Literal(Literal::new_simple_literal("hello"))),
            s_typed,
            "simple and explicit xsd:string of the same value share one id"
        );

        // Every non-plain-string kind must be `None`.
        let lang = d.intern(&Term::Literal(Literal::new_language_tagged_literal("hello", "en").unwrap()));
        let int_lit = d.intern(&Term::Literal(Literal::new_typed_literal("42", xsd::INTEGER)));
        let other_dt = d.intern(&Term::Literal(Literal::new_typed_literal(
            "x",
            NamedNode::new("http://ex/dt").unwrap(),
        )));
        let iri = d.intern(&Term::NamedNode(NamedNode::new("http://ex/a").unwrap()));
        let blank = d.intern(&Term::BlankNode(oxrdf::BlankNode::new("b0").unwrap()));
        let inline_int = d.intern(&Term::Literal(Literal::new_typed_literal("7", xsd::INTEGER)));
        assert_eq!(d.plain_string_value(lang), None, "lang-tagged is not a plain string");
        assert_eq!(d.plain_string_value(int_lit), None, "large integer literal");
        assert_eq!(d.plain_string_value(other_dt), None, "other datatype");
        assert_eq!(d.plain_string_value(iri), None, "IRI");
        assert_eq!(d.plain_string_value(blank), None, "blank node");
        assert!(is_inline(inline_int), "small int inlines");
        assert_eq!(d.plain_string_value(inline_int), None, "inline integer id is never a string");
    }

    /// [OPUS-4.8] sq-cvug — a triple-term component id that is IN RANGE but resolves to the
    /// WRONG KIND (a literal where the subject must be IRI/blank, or where the predicate must
    /// be an IRI) used to hit `reconstruct_triple`'s `unreachable!()` → a clean panic (DoS) on
    /// a hostile/corrupt mmap'd index. It must now FAIL CLOSED to a safe placeholder, never
    /// panic. The trusted interner never produces such a record, so we craft one directly via
    /// the raw `intern_triple_ids` (the same path a tampered store would feed `reconstruct`).
    #[test]
    fn reconstruct_triple_wrong_kind_component_is_safe_not_panic() {
        let mut d = Dict::new();
        let lit = d.intern(&Term::Literal(Literal::new_simple_literal("not a node")));
        let iri = d.intern(&Term::NamedNode(NamedNode::new_unchecked("http://ex/p")));
        let blank = d.intern(&Term::BlankNode(oxrdf::BlankNode::new_unchecked("b0")));
        let obj = d.intern(&Term::NamedNode(NamedNode::new_unchecked("http://ex/o")));

        // (a) Literal in the SUBJECT slot (subject must be IRI|blank). Must not panic; the
        // result is a triple term whose subject is the safe blank-node placeholder.
        let bad_subj = d.intern_triple_ids([lit, iri, obj]);
        let Term::Triple(t) = d.term(bad_subj) else { panic!("expected a triple term") };
        assert_eq!(
            t.subject,
            oxrdf::BlankNode::new_unchecked(OOR_TRIPLE_SUBJECT).into(),
            "wrong-kind subject falls back to the safe placeholder"
        );

        // (b) Literal in the PREDICATE slot (predicate must be IRI). Must not panic; the
        // predicate is the safe placeholder IRI.
        let bad_pred = d.intern_triple_ids([iri, lit, obj]);
        let Term::Triple(t) = d.term(bad_pred) else { panic!("expected a triple term") };
        assert_eq!(
            t.predicate,
            NamedNode::new_unchecked(OOR_TRIPLE_PREDICATE),
            "wrong-kind predicate falls back to the safe placeholder"
        );

        // A blank-node subject and an inline-integer object are valid and decode normally.
        let inline_obj = d.intern(&int("7"));
        let ok = d.intern_triple_ids([blank, iri, inline_obj]);
        let Term::Triple(t) = d.term(ok) else { panic!("expected a triple term") };
        assert_eq!(t.subject, oxrdf::BlankNode::new_unchecked("b0").into());
        assert_eq!(t.predicate, NamedNode::new_unchecked("http://ex/p"));
        assert_eq!(t.object, int("7"));

        // The same fail-closed behaviour must hold after compaction (the blob record path).
        let d = d.into_blob();
        let Term::Triple(t) = d.term(bad_subj) else { panic!("expected a triple term") };
        assert_eq!(t.subject, oxrdf::BlankNode::new_unchecked(OOR_TRIPLE_SUBJECT).into());
    }

    /// [OPUS-4.8] sq-bj7o — base-direction VALIDATION + cross-path PARITY. The stored
    /// language slot can come from an untrusted/mmap'd index whose `--<suffix>` is
    /// attacker-controlled. The split used by the JSON fast path (`split_lang_dir`) and the
    /// materialised-`Term` path (`reconstruct_ref`, reached via `Dict::term`) must AGREE on
    /// every slot — and an invalid suffix must NOT fabricate an `its:dir`. This test feeds
    /// raw slots directly through `intern_lit` (the same entry an mmap record takes) and
    /// asserts both paths agree.
    #[test]
    fn lang_dir_suffix_validated_and_paths_agree() {
        // A valid base direction must be lowercase `ltr`/`rtl` (case-sensitive, per
        // BaseDirection::Display / RDF 1.2). A trusted store only ever writes those two; the
        // others model a tampered/untrusted slot.
        // (slot, expected (tag, dir) from split_lang_dir)
        let cases: &[(&str, (&str, Option<&str>))] = &[
            // Valid directions: split out, direction reported.
            ("en--ltr", ("en", Some("ltr"))),
            ("ar--rtl", ("ar", Some("rtl"))),
            // Plain language tag: no `--`, no direction.
            ("en", ("en", None)),
            // Malformed / hostile suffixes: NOT a direction → whole slot is the tag, no dir.
            ("en--bogus", ("en--bogus", None)),
            ("en--LTR", ("en--LTR", None)), // case-sensitive: uppercase is NOT a direction
            ("en--", ("en--", None)),       // empty suffix
            ("en--ltr--rtl", ("en--ltr--rtl", None)), // suffix `ltr--rtl` is not a direction
        ];

        for &(slot, (want_tag, want_dir)) in cases {
            // (1) The fast-path split validates the suffix.
            assert_eq!(
                split_lang_dir(slot),
                (want_tag, want_dir),
                "split_lang_dir disagreed for slot {slot:?}"
            );

            // (2) Round-trip the SAME raw slot through the dictionary and materialise it
            //     (the `oxrdf::Term` path). Both the in-arena and compacted-blob records must
            //     agree with the split.
            let mut d = Dict::new();
            let id = d.intern_lit("v", xsd::STRING.as_str(), Some(slot));
            for d in [d.clone(), d.clone().into_blob()] {
                // The stored slot is preserved verbatim (the fast path reads exactly this).
                let TermParts::Lit { lang: Some(stored), .. } = d.term_parts(id) else {
                    panic!("expected a lang literal for slot {slot:?}");
                };
                assert_eq!(stored, slot, "stored slot mutated for {slot:?}");

                // The materialised Term: direction() and language() MUST match the split.
                let Term::Literal(l) = d.term(id) else { panic!("expected a literal for {slot:?}") };
                let mat_dir = l.direction().map(|x| match x {
                    oxrdf::BaseDirection::Ltr => "ltr",
                    oxrdf::BaseDirection::Rtl => "rtl",
                });
                assert_eq!(mat_dir, want_dir, "materialised direction != split for {slot:?}");
                assert_eq!(l.language(), Some(want_tag), "materialised lang != split tag for {slot:?}");
            }
        }
    }

    #[test]
    fn into_blob_roundtrips() {
        // Build a dict spanning all term kinds (prefixed IRIs, typed/lang/plain literals,
        // a blank node, an inline integer), then compact it and verify every term still
        // round-trips: term(id), term_parts(id), and lookup(term)->id are unchanged, and
        // a non-present term still misses.
        let mut d = Dict::new();
        let terms = vec![
            Term::NamedNode(NamedNode::new_unchecked("http://ex/alice")),
            Term::NamedNode(NamedNode::new_unchecked("http://ex/bob")),
            Term::NamedNode(NamedNode::new_unchecked("http://other.org/x#y")),
            Term::Literal(Literal::new_simple_literal("a plain string")),
            Term::Literal(Literal::new_language_tagged_literal_unchecked("café", "fr")),
            Term::Literal(Literal::new_typed_literal("1.5", xsd::DECIMAL)),
            Term::BlankNode(oxrdf::BlankNode::new_unchecked("b0")),
            int("42"),       // inline
            int("99999999"), // also inline (within range)
        ];
        let ids: Vec<Id> = terms.iter().map(|t| d.intern(t)).collect();
        let before_len = d.len();

        let d = d.into_blob();
        assert_eq!(d.len(), before_len, "len unchanged after compaction");
        for (t, &id) in terms.iter().zip(&ids) {
            assert_eq!(d.term(id), *t, "term({id}) round-trips");
            assert_eq!(d.lookup(t), id, "lookup round-trips");
        }
        // A term that was never interned must still miss.
        assert_eq!(d.lookup(&Term::NamedNode(NamedNode::new_unchecked("http://ex/absent"))), NO_ID);
        // Compaction is idempotent.
        let d = d.into_blob();
        assert_eq!(d.term(ids[0]), terms[0]);
    }

    #[test]
    fn sharded_dict_roundtrips_and_preserves_order() {
        // Intern terms (with cross-shard duplicates) through the sharded interner, merge,
        // and verify: distinct count, every term round-trips via remap→term, duplicates get
        // the same temp id, and temp-id order == final-id order (the no-re-sort invariant).
        let mut sd = ShardedDict::new(4);
        let mk = |i: usize| -> Term {
            let v = (i / 3) % 30; // value decorrelated from the type selector (i % 3)
            match i % 3 {
                0 => Term::NamedNode(NamedNode::new_unchecked(format!("http://ex/n{v}"))),
                1 => Term::Literal(Literal::new_simple_literal(format!("lit{v}"))),
                _ => Term::BlankNode(oxrdf::BlankNode::new_unchecked(format!("b{v}"))),
            }
        };
        let terms: Vec<Term> = (0..300).map(mk).collect();
        let items: Vec<(u32, Id, Term)> = terms.iter().enumerate().map(|(j, t)| (0, (j + 1) as Id, t.clone())).collect();
        let resolved = sd.intern_terms(items);
        let mut temp_of: std::collections::HashMap<Id, Id> = std::collections::HashMap::new();
        for (_, j, temp) in &resolved {
            temp_of.insert(*j, *temp);
        }
        let (merged, base, stride) = sd.into_merged();
        // 90 distinct terms: 30 IRIs + 30 literals + 30 blanks.
        assert_eq!(merged.len(), 90, "distinct term count");
        // Every term round-trips through remap→term.
        for (j, t) in terms.iter().enumerate() {
            let temp = temp_of[&((j + 1) as Id)];
            let fin = remap_sharded(temp, &base, stride);
            assert_eq!(merged.term(fin), *t, "term {j} round-trips");
        }
        // Duplicates (same content) share a temp id.
        assert_eq!(temp_of[&1], temp_of[&91], "j=0 and j=90 are the same term (n0)");
        // Order preservation: sorting distinct temp ids == sorting their final ids.
        let mut temps: Vec<Id> = temp_of.values().copied().collect();
        temps.sort_unstable();
        temps.dedup();
        let mut prev = 0;
        for &temp in &temps {
            let fin = remap_sharded(temp, &base, stride);
            assert!(fin > prev, "final ids strictly increase with temp ids (no re-sort needed)");
            prev = fin;
        }
        // Final ids are dense [1, 90].
        assert_eq!(prev, 90, "final ids are dense and 1-based");
    }

    #[test]
    fn merged_dict_lookup_and_intern_after_build_table() {
        // [OPUS-4.8] Regression for review 1373 (Medium): `into_merged` returns a dict with an
        // empty lookup table; `build_table` must rebuild it so the merged dict honours the
        // normal lookup/intern invariant — lookup hits every term, and re-interning an existing
        // term returns its id (no duplicate), while a fresh term appends above the merged ids.
        let mut sd = ShardedDict::new(4);
        let mk = |i: usize| -> Term { Term::NamedNode(NamedNode::new_unchecked(format!("http://ex/n{i}"))) };
        let terms: Vec<Term> = (0..50).map(mk).collect();
        let items: Vec<(u32, Id, Term)> =
            terms.iter().enumerate().map(|(j, t)| (0, (j + 1) as Id, t.clone())).collect();
        let resolved = sd.intern_terms(items);
        let temp_of: std::collections::HashMap<Id, Id> = resolved.iter().map(|(_, j, temp)| (*j, *temp)).collect();
        let (mut merged, base, stride) = sd.into_merged();
        merged.build_table();
        assert_eq!(merged.len(), 50, "distinct term count");
        for (j, t) in terms.iter().enumerate() {
            let fin = remap_sharded(temp_of[&((j + 1) as Id)], &base, stride);
            // lookup must HIT (was an empty table before build_table) and agree with the final id.
            assert_eq!(merged.lookup(t), fin, "lookup hits term {j}");
            // re-interning an existing term must NOT add a duplicate.
            assert_eq!(merged.intern(t), fin, "re-intern of existing term {j} is idempotent");
        }
        assert_eq!(merged.len(), 50, "no duplicates created by re-interning");
        // A fresh term appends above the merged ids.
        let fresh = Term::NamedNode(NamedNode::new_unchecked("http://ex/fresh"));
        let fid = merged.intern(&fresh);
        assert_eq!(merged.len(), 51);
        assert_eq!(merged.lookup(&fresh), fid);
    }

    #[test]
    fn non_canonical_and_out_of_range_do_not_inline() {
        let mut d = Dict::new();
        for s in ["05", "+5", "-3", "007"] {
            let id = d.intern(&int(s));
            assert!(!is_inline(id), "{s:?} must not inline");
        }
        assert_ne!(d.intern(&int("05")), d.intern(&int("5")));
        let typed = Term::Literal(Literal::new_typed_literal("5", xsd::INT));
        assert!(!is_inline(d.intern(&typed)));
        assert!(is_inline(d.intern(&int(&INLINE_MAX.to_string()))));
        assert!(!is_inline(d.intern(&int(&(INLINE_BASE).to_string()))));
    }

    #[test]
    fn iri_prefix_factoring_roundtrips() {
        // IRIs sharing a namespace dedupe the prefix; every term round-trips exactly,
        // and lookup agrees with intern (the content hash matches across paths).
        let mut d = Dict::new();
        let iris = ["http://ex/a", "http://ex/b", "http://www.w3.org/2001/XMLSchema#date", "urn:x"];
        let ids: Vec<Id> = iris.iter().map(|i| d.intern(&Term::NamedNode(NamedNode::new_unchecked(*i)))).collect();
        for (iri, id) in iris.iter().zip(&ids) {
            let t = Term::NamedNode(NamedNode::new_unchecked(*iri));
            assert_eq!(d.term(*id), t);
            assert_eq!(d.lookup(&t), *id);
        }
        // Distinct IRIs get distinct ids; re-interning is idempotent.
        assert_eq!(ids.iter().collect::<std::collections::HashSet<_>>().len(), iris.len());
        assert_eq!(d.intern(&Term::NamedNode(NamedNode::new_unchecked("http://ex/a"))), ids[0]);
        // A language-tagged literal and a typed literal round-trip with their components.
        let lang = Term::Literal(Literal::new_language_tagged_literal_unchecked("hi", "en"));
        let dec = Term::Literal(Literal::new_typed_literal("1.0", xsd::DECIMAL));
        let (lid, did) = (d.intern(&lang), d.intern(&dec));
        assert_eq!(d.term(lid), lang);
        assert_eq!(d.term(did), dec);
    }

    fn triple(s: &str, p: &str, o: Term) -> Term {
        Term::Triple(Box::new(oxrdf::Triple::new(
            NamedNode::new_unchecked(s),
            NamedNode::new_unchecked(p),
            o,
        )))
    }

    #[test]
    fn filtered_rebuild_moves_arena_records_and_filters_metadata() {
        let mut d = Dict::new();
        let kept_iri = Term::NamedNode(NamedNode::new_unchecked("http://keep.example/a"));
        let dead_iri = Term::NamedNode(NamedNode::new_unchecked("http://dead.example/b"));
        let kept_lit = Term::Literal(Literal::new_typed_literal("1.5", xsd::DECIMAL));
        let dead_lit = Term::Literal(Literal::new_typed_literal(
            "gone",
            NamedNode::new_unchecked("http://dead.example/type"),
        ));
        let kept_blank = Term::BlankNode(oxrdf::BlankNode::new_unchecked("kept"));
        let kept_iri_id = d.intern(&kept_iri);
        let dead_iri_id = d.intern(&dead_iri);
        let kept_lit_id = d.intern(&kept_lit);
        let dead_lit_id = d.intern(&dead_lit);
        let kept_blank_id = d.intern(&kept_blank);
        let old_len = d.len();

        // Raw payload pointers prove the arena path MOVES the compact Boxes instead of
        // reconstructing Terms (which would allocate new suffix/value strings).
        let kept_suffix_ptr = match d.record(kept_iri_id) {
            StoredRef::Iri { suffix, .. } => suffix.as_ptr(),
            _ => panic!("kept IRI did not have an IRI record"),
        };
        let kept_value_ptr = match d.record(kept_lit_id) {
            StoredRef::Lit { value, .. } => value.as_ptr(),
            _ => panic!("kept literal did not have a literal record"),
        };

        let (rebuilt, remap) =
            d.rebuild_filtered(|id| id == kept_iri_id || id == kept_lit_id || id == kept_blank_id);

        assert_eq!(remap.len(), old_len);
        assert_eq!(remap[(kept_iri_id - 1) as usize], 1);
        assert_eq!(remap[(dead_iri_id - 1) as usize], NO_ID);
        assert_eq!(remap[(kept_lit_id - 1) as usize], 2);
        assert_eq!(remap[(dead_lit_id - 1) as usize], NO_ID);
        assert_eq!(remap[(kept_blank_id - 1) as usize], 3);
        assert_eq!(rebuilt.len(), 3);
        assert_eq!(rebuilt.lookup(&kept_iri), 1);
        assert_eq!(rebuilt.lookup(&kept_lit), 2);
        assert_eq!(rebuilt.lookup(&kept_blank), 3);
        assert_eq!(rebuilt.lookup(&dead_iri), NO_ID);
        assert_eq!(rebuilt.lookup(&dead_lit), NO_ID);

        // Metadata belonging only to dead records must be reclaimed as well.
        assert_eq!(rebuilt.prefixes.len(), 1);
        assert_eq!(rebuilt.prefix_ids.len(), 1);
        assert_eq!(rebuilt.datatypes.len(), 1);
        assert_eq!(rebuilt.datatype_ids.len(), 1);
        match rebuilt.record(1) {
            StoredRef::Iri { suffix, .. } => {
                assert!(std::ptr::eq(suffix.as_ptr(), kept_suffix_ptr));
            }
            _ => panic!("rebuilt IRI did not have an IRI record"),
        }
        match rebuilt.record(2) {
            StoredRef::Lit { value, .. } => {
                assert!(std::ptr::eq(value.as_ptr(), kept_value_ptr));
            }
            _ => panic!("rebuilt literal did not have a literal record"),
        }
    }

    #[test]
    fn filtered_rebuild_from_blob_retains_nested_triple_dependencies() {
        let mut d = Dict::new();
        let inner = triple(
            "http://ex/a",
            "http://ex/b",
            Term::Literal(Literal::new_simple_literal("v")),
        );
        let outer = triple("http://ex/x", "http://ex/p", inner.clone());
        let outer_id = d.intern(&outer);
        let inner_id = d.lookup(&inner);
        let dead = Term::NamedNode(NamedNode::new_unchecked("http://dead.example/gone"));
        let dead_id = d.intern(&dead);
        let old_len = d.len();

        // Select ONLY the outer triple. Its leaves and nested triple are implicit survivors.
        let (rebuilt, remap) = d.into_blob().rebuild_filtered(|id| id == outer_id);
        let new_outer = remap[(outer_id - 1) as usize];
        let new_inner = remap[(inner_id - 1) as usize];
        assert_ne!(new_outer, NO_ID);
        assert_ne!(
            new_inner, NO_ID,
            "nested triple dependency must be retained"
        );
        assert_eq!(remap[(dead_id - 1) as usize], NO_ID);
        assert_eq!(
            rebuilt.len(),
            old_len - 1,
            "only the unrelated dead term is dropped"
        );
        assert!(
            rebuilt.len() > 1,
            "the one selected triple must retain its dependencies"
        );
        assert_eq!(rebuilt.term(new_outer), outer);
        assert_eq!(rebuilt.term(new_inner), inner);
        assert_eq!(rebuilt.lookup(&outer), new_outer);
        assert_eq!(rebuilt.lookup(&dead), NO_ID);
    }

    #[test]
    fn triple_terms_intern_structurally_and_roundtrip() {
        let mut d = Dict::new();
        let tt = triple("http://ex/alice", "http://ex/age", int("30"));
        let id = d.intern(&tt);
        assert!((1..INLINE_BASE).contains(&id));
        // Children were interned (subject + predicate; the object is inline).
        assert_eq!(d.len(), 3, "subject + predicate + the triple itself (inline object not stored)");
        // term() rebuilds a structural Term::Triple, not a literal.
        assert_eq!(d.term(id), tt);
        // lookup agrees with intern; a separately-built identical triple shares the id.
        assert_eq!(d.lookup(&tt), id);
        assert_eq!(d.intern(&triple("http://ex/alice", "http://ex/age", int("30"))), id);
        // A different triple gets a different id; lookup with absent components misses.
        let other = triple("http://ex/bob", "http://ex/age", int("30"));
        let oid = d.intern(&other);
        assert_ne!(oid, id);
        assert_eq!(d.lookup(&triple("http://ex/carol", "http://ex/age", int("30"))), NO_ID);
        // A triple whose components exist but that was never interned also misses.
        assert_eq!(d.lookup(&triple("http://ex/alice", "http://ex/age", int("25"))), NO_ID);
    }

    #[test]
    fn nested_triple_terms_roundtrip() {
        // RDF 1.2 nests triple terms through the OBJECT position.
        let mut d = Dict::new();
        let inner = triple("http://ex/a", "http://ex/b", Term::NamedNode(NamedNode::new_unchecked("http://ex/c")));
        let outer = triple("http://ex/x", "http://ex/p", inner.clone());
        let oid = d.intern(&outer);
        let iid = d.intern(&inner);
        assert_ne!(oid, iid);
        assert_eq!(d.term(oid), outer);
        assert_eq!(d.term(iid), inner);
        assert_eq!(d.lookup(&outer), oid);
        // Blank-node subject round-trips too.
        let bsubj = Term::Triple(Box::new(oxrdf::Triple::new(
            oxrdf::BlankNode::new_unchecked("r0"),
            NamedNode::new_unchecked("http://ex/p"),
            Term::Literal(Literal::new_simple_literal("v")),
        )));
        let bid = d.intern(&bsubj);
        assert_eq!(d.term(bid), bsubj);
        assert_eq!(d.lookup(&bsubj), bid);
    }

    #[test]
    fn triple_terms_survive_blob_compaction() {
        let mut d = Dict::new();
        let inner = triple("http://ex/a", "http://ex/b", int("7"));
        let outer = triple("http://ex/x", "http://ex/p", inner.clone());
        let plain = Term::NamedNode(NamedNode::new_unchecked("http://ex/other"));
        let ids = [d.intern(&outer), d.intern(&inner), d.intern(&plain)];
        let d = d.into_blob();
        assert_eq!(d.term(ids[0]), outer);
        assert_eq!(d.term(ids[1]), inner);
        assert_eq!(d.term(ids[2]), plain);
        assert_eq!(d.lookup(&outer), ids[0]);
        assert_eq!(d.lookup(&inner), ids[1]);
        assert_eq!(d.lookup(&triple("http://ex/x", "http://ex/p", int("7"))), NO_ID);
    }

    #[test]
    fn triple_terms_survive_save_open() {
        let mut d = Dict::new();
        let inner = triple("http://ex/a", "http://ex/b", int("7"));
        let outer = triple("http://ex/x", "http://ex/p", inner.clone());
        let (oid, iid) = (d.intern(&outer), d.intern(&inner));
        let dir = std::env::temp_dir().join(format!("sparq-dict-star-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("dict.bin");
        d.save(&path).unwrap();
        let d2 = Dict::open(&path).unwrap();
        assert_eq!(d2.term(oid), outer);
        assert_eq!(d2.term(iid), inner);
        assert_eq!(d2.lookup(&outer), oid);
        std::fs::remove_dir_all(&dir).ok();
    }

    /// [OPUS-4.8] (sq-t3rt review) `intern_triple_terms` must REJECT a malformed partial whose
    /// triple-term component id is out of range or references an unpopulated remap slot, rather
    /// than silently mis-indexing the per-partial remap (returning garbage / panicking).
    #[test]
    fn intern_partials_rejects_malformed_triple_component() {
        // Build a partial Dict with two real leaf terms (local ids 1, 2) followed by a triple
        // term whose components reference an OUT-OF-RANGE local id (> pd.len()). The valid
        // path always interns components first (children-first), so we use the low-level
        // `intern_triple_ids` to forge the malformed arena a tampered/buggy producer might.
        let mut pd = Dict::new();
        let s = pd.intern(&Term::NamedNode(NamedNode::new_unchecked("http://ex/s"))); // local 1
        let p = pd.intern(&Term::NamedNode(NamedNode::new_unchecked("http://ex/p"))); // local 2
        let bogus = pd.len() as Id + 5; // out of range: no such local id exists
        let _tt = pd.intern_triple_ids([s, p, bogus]);
        let partials = vec![(pd, Vec::<[Id; 3]>::new())];

        let mut sd = ShardedDict::new(4);
        let err = sd
            .intern_partials(&partials)
            .expect_err("an out-of-range triple component id must be a descriptive Err, not garbage/panic");
        assert!(err.contains("out of range"), "error should name the out-of-range cause: {err:?}");

        // Second case: an IN-RANGE but UNPOPULATED component slot. The triple references a
        // non-inline local id whose remap slot the leaf pass never fills (here the triple's
        // own not-yet-written slot — a children-first ordering violation), so `rm[id]` is the
        // `0` sentinel rather than a routed `base + local` temp id.
        let mut pd2 = Dict::new();
        let a = pd2.intern(&Term::NamedNode(NamedNode::new_unchecked("http://ex/a"))); // local 1
        let phantom = pd2.len() as Id + 1; // == 2: the triple term's OWN slot, unwritten when resolved
        let _tt2 = pd2.intern_triple_ids([a, a, phantom]);
        let partials2 = vec![(pd2, Vec::<[Id; 3]>::new())];

        let mut sd2 = ShardedDict::new(4);
        let err2 = sd2
            .intern_partials(&partials2)
            .expect_err("an unpopulated triple component slot must be a descriptive Err, not garbage/panic");
        assert!(
            err2.contains("out of range") || err2.contains("unpopulated"),
            "error should name the out-of-range / unpopulated cause: {err2:?}"
        );

        // Sanity: a WELL-FORMED triple-term partial still succeeds (no false rejection).
        let mut pdok = Dict::new();
        let _ok = pdok.intern(&triple("http://ex/x", "http://ex/y", int("7")));
        let mut sdok = ShardedDict::new(4);
        sdok.intern_partials(&[(pdok, Vec::<[Id; 3]>::new())])
            .expect("a well-formed triple-term partial must intern without error");
    }

    /// [OPUS-5] sq-7d3dj.21b — HASH-BEFORE-MEMCMP on the MMAP dedup path (and that path only —
    /// the in-memory table's weaker tag-only prefilter is pinned separately by
    /// `in_memory_dedup_has_only_the_h2_tag_prefilter`). Here it holds in its strongest form:
    /// `mapped_find` binary-searches the sorted 64-bit hash array and calls the `eq`
    /// (string-compare) closure ONLY for entries whose FULL 64-bit hash matches. The bead asked
    /// for this filter to be added; the verification says it already exists here, so this test
    /// pins it instead — a refactor that dropped the hash gate and linearly `eq`-scanned would
    /// turn every miss from O(log n) hash probes + 0 compares into O(n) compares, which this
    /// catches by COUNTING `eq` invocations rather than by timing.
    #[cfg(feature = "mmap")]
    #[test]
    fn mapped_find_compares_full_hash_before_eq() {
        use std::cell::Cell;
        let mut d = Dict::new();
        for i in 0..64 {
            d.intern(&Term::NamedNode(NamedNode::new_unchecked(format!("http://ex/t{}", i))));
        }
        let dir = std::env::temp_dir().join(format!("sparq-dict-hashfirst-{}", std::process::id()));
        d.save_mmap(&dir).unwrap();
        let m = Dict::open_mmap(&dir).unwrap();

        // (a) A hash that is present: `eq` runs for exactly the entries carrying that hash.
        //     With 64 distinct terms and a 64-bit hash that is one entry, so one compare —
        //     NOT the 64 a compare-everything scan would make.
        let present = hash_iri("http://ex/t7");
        let calls = Cell::new(0usize);
        let found = m.mapped_find(present, |s| {
            calls.set(calls.get() + 1);
            stored_ref_is_iri(s, "http://ex/t7", &m.prefixes)
        });
        assert!(found.is_some(), "the term must be found through the mapped index");
        assert_eq!(calls.get(), 1, "eq must run once — only for the entry whose 64-bit hash matched");

        // (b) A hash that is absent: `eq` must NEVER run. This is the load-bearing half — it is
        //     what makes a dictionary MISS cost zero string compares.
        let calls = Cell::new(0usize);
        let found = m.mapped_find(hash_iri("http://ex/absent"), |_| {
            calls.set(calls.get() + 1);
            true // would report a bogus hit if the hash gate were ever removed
        });
        assert_eq!(found, None, "an absent hash must not resolve to any id");
        assert_eq!(calls.get(), 0, "eq must not run at all when no stored 64-bit hash matches");

        std::fs::remove_dir_all(&dir).ok();
    }

    /// [OPUS-5] sq-7d3dj.21b — the OTHER half of the HASH-BEFORE-MEMCMP scope note on
    /// `find_iri` & co., pinned so the mmap guarantee is never read as covering every dedup
    /// path. The in-memory `self.table` stores bare ids and so has NO full-64-bit hash gate:
    /// hashbrown matches only the top-7-bit `h2` tag, which means
    ///   (a) `eq` DOES run for a probed entry whose remaining 57 hash bits differ, and
    ///   (b) correctness therefore rests entirely on `eq` — an `h2` collision must never dedup
    ///       two distinct terms onto one id.
    /// Both are asserted below on a deterministically-found tag-colliding IRI pair (FxHasher is
    /// seedless, so the search result is stable across runs and hosts).
    #[test]
    fn in_memory_dedup_has_only_the_h2_tag_prefilter() {
        use std::cell::Cell;
        use std::collections::HashMap;

        // hashbrown's `h2` tag is the top 7 bits of the 64-bit hash.
        let tag = |h: u64| h >> 57;
        let mut first_per_tag: HashMap<u64, (String, u64)> = HashMap::new();
        let mut pair = None;
        for i in 0..4096u32 {
            let iri = format!("http://ex/h{}", i);
            let h = hash_iri(&iri);
            match first_per_tag.get(&tag(h)) {
                Some((prev_iri, prev_h)) if *prev_h != h => {
                    pair = Some((prev_iri.clone(), *prev_h, iri, h));
                    break;
                }
                Some(_) => {}
                None => {
                    first_per_tag.insert(tag(h), (iri, h));
                }
            }
        }
        let (a, ha, b, hb) =
            pair.expect("two IRIs sharing the 7-bit h2 tag must exist among 4096 candidates");
        assert_eq!(tag(ha), tag(hb), "the pair must share the h2 tag: {} / {}", a, b);
        assert_ne!(ha, hb, "the pair must differ in the FULL 64-bit hash: {} / {}", a, b);

        // (a) The in-memory prefilter is tag-only. Probing a `HashTable<Id>` exactly as
        //     `find_iri` does (`table.find(hash, eq)`) invokes `eq` for the tag-colliding entry
        //     even though the full hashes differ — a stored-full-hash gate would call it zero
        //     times. This is the claim the source comment makes, asserted rather than asserted-
        //     by-prose.
        let mut table: HashTable<Id> = HashTable::new();
        table.insert_unique(ha, 1, |_| ha);
        let calls = Cell::new(0usize);
        let hit = table.find(hb, |_| {
            calls.set(calls.get() + 1);
            false
        });
        assert!(hit.is_none(), "a tag-only collision must not resolve to an id");
        assert_eq!(
            calls.get(),
            1,
            "eq must run for the h2-tag-colliding entry — the in-memory path has no full-hash gate"
        );

        // (b) So `eq` is load-bearing for correctness: the colliding pair must intern to two
        //     distinct ids, and each must still look up to its own id.
        let (ta, tb) = (
            Term::NamedNode(NamedNode::new_unchecked(a.clone())),
            Term::NamedNode(NamedNode::new_unchecked(b.clone())),
        );
        let mut d = Dict::new();
        let (ia, ib) = (d.intern(&ta), d.intern(&tb));
        assert_ne!(ia, ib, "an h2-tag collision must not dedup {} and {} onto one id", a, b);
        assert_eq!(d.intern(&ta), ia, "re-interning must dedup to the same id");
        assert_eq!(d.intern(&tb), ib, "re-interning must dedup to the same id");
        assert_eq!(d.lookup(&ta), ia);
        assert_eq!(d.lookup(&tb), ib);
        assert_eq!(d.term(ia), ta);
        assert_eq!(d.term(ib), tb);
    }

    #[cfg(feature = "mmap")]
    #[test]
    fn triple_terms_survive_save_open_mmap() {
        let mut d = Dict::new();
        let inner = triple("http://ex/a", "http://ex/b", int("7"));
        let outer = triple("http://ex/x", "http://ex/p", inner.clone());
        let (oid, iid) = (d.intern(&outer), d.intern(&inner));
        let dir = std::env::temp_dir().join(format!("sparq-dict-star-mmap-{}", std::process::id()));
        d.save_mmap(&dir).unwrap();
        let d2 = Dict::open_mmap(&dir).unwrap();
        assert_eq!(d2.term(oid), outer);
        assert_eq!(d2.term(iid), inner);
        assert_eq!(d2.lookup(&outer), oid);
        assert_eq!(d2.lookup(&triple("http://ex/x", "http://ex/p", int("8"))), NO_ID);
        std::fs::remove_dir_all(&dir).ok();
    }

    /// [OPUS-4.8] sq-lvw8 (sq-ueuk/#418 follow-up) — endianness invariant of the mmap on-disk
    /// dictionary format. The format mixes two byte-order conventions for the SAME files: the
    /// runtime writer/readers (`write_pod_slice`, `MappedDict::slice_u64`/`hashids`) move the
    /// `u64`/`u32` arrays through a raw NATIVE-endian byte cast, while `validate_dict_bytes` and
    /// the `dict-spill` external builder use explicit `to_le_bytes` / `from_le_bytes`. They
    /// agree, and the in-RAM `save_mmap` output is byte-for-byte identical to the spilled build
    /// (the claim PR #418 relied on), ONLY on a little-endian host — which is exactly what
    /// `DICT_ON_DISK_LE_GUARD` pins at compile time. This test makes the assumption a CHECKED
    /// invariant: it evaluates the compile-time guard, confirms the writer's raw bytes equal the
    /// explicit-LE encoding the validator expects, and that re-opening round-trips. On a
    /// (currently unbuildable) big-endian target `DICT_ON_DISK_LE_GUARD` fails the build before
    /// this test — or any silent on-disk corruption — can be reached.
    #[cfg(feature = "mmap")]
    #[test]
    fn mmap_dict_on_disk_arrays_are_little_endian() {
        // Force-evaluate the compile-time little-endian guard: on a big-endian target this
        // const initializer (and thus the whole crate) fails to build, so the persistence
        // format is never silently emitted in the wrong byte order. `cfg!(target_endian)` is a
        // compile-time constant here, hence the const guard rather than a runtime `assert!`.
        let () = DICT_ON_DISK_LE_GUARD;

        let mut d = Dict::new();
        d.intern(&Term::NamedNode(NamedNode::new_unchecked("http://ex/a")));
        d.intern(&Term::NamedNode(NamedNode::new_unchecked("http://ex/b")));
        let dir = std::env::temp_dir().join(format!("sparq-dict-endian-mmap-{}", std::process::id()));
        d.save_mmap(&dir).unwrap();

        // (a) `write_pod_slice` is a raw native-endian byte copy; on this LE host the on-disk
        //     `dict-offs.bin` must therefore be the EXPLICIT little-endian encoding of the same
        //     u64 offsets that `validate_dict_bytes` (which uses `from_le_bytes`) reads back.
        let offs_bytes = std::fs::read(dir.join("dict-offs.bin")).unwrap();
        assert_eq!(offs_bytes.len() % 8, 0, "offset array is whole u64s");
        for chunk in offs_bytes.chunks_exact(8) {
            // Re-encoding the LE-decoded value must reproduce the on-disk bytes exactly.
            let v = u64::from_le_bytes(chunk.try_into().unwrap());
            assert_eq!(&v.to_le_bytes(), chunk, "dict-offs.bin is little-endian on this host");
        }
        // (b) Same for `dict-hid.bin` (the u32 lookup ids the seam reads via `from_le_bytes`).
        let hid_bytes = std::fs::read(dir.join("dict-hid.bin")).unwrap();
        assert_eq!(hid_bytes.len() % 4, 0, "hashid array is whole u32s");
        for chunk in hid_bytes.chunks_exact(4) {
            let v = u32::from_le_bytes(chunk.try_into().unwrap());
            assert_eq!(&v.to_le_bytes(), chunk, "dict-hid.bin is little-endian on this host");
        }

        // (c) End-to-end: the store re-opens and resolves — i.e. the native-endian mmap readers
        //     and the explicit-LE `validate_dict_bytes` (run inside `open_mmap`) agree.
        let d2 = Dict::open_mmap(&dir).unwrap();
        assert_eq!(d2.lookup(&Term::NamedNode(NamedNode::new_unchecked("http://ex/a"))), 1);
        assert_eq!(d2.lookup(&Term::NamedNode(NamedNode::new_unchecked("http://ex/b"))), 2);
        std::fs::remove_dir_all(&dir).ok();
    }

    /// [OPUS-4.8] Regression for review 1409: the mmap dictionary must carry an on-disk
    /// id-space partition marker so a store written under a DIFFERENT `INLINE_BASE` (or a
    /// legacy header-less file) is REJECTED rather than silently misinterpreting its ids.
    #[cfg(feature = "mmap")]
    #[test]
    fn mmap_dict_rejects_legacy_and_mismatched_partition() {
        use std::io::Write;
        let mut d = Dict::new();
        d.intern(&Term::NamedNode(NamedNode::new_unchecked("http://ex/a")));
        d.intern(&int("42")); // an inline integer — its meaning depends on INLINE_BASE
        let dir = std::env::temp_dir().join(format!("sparq-dict-ver-mmap-{}", std::process::id()));

        // (a) A current-format store opens fine, and the header records the current partition.
        d.save_mmap(&dir).unwrap();
        let meta = std::fs::read(dir.join("dict-meta.bin")).unwrap();
        assert_eq!(&meta[0..4], &DICT_META_MAGIC.to_le_bytes(), "meta starts with the magic");
        assert_eq!(&meta[4..8], &DICT_META_VERSION.to_le_bytes(), "then the version");
        assert_eq!(&meta[8..12], &INLINE_BASE.to_le_bytes(), "then the id partition");
        assert!(Dict::open_mmap(&dir).is_ok(), "current-format store must open");

        // (b) A store with a MISMATCHED partition (simulating the old INLINE_BASE = 1<<30) must
        //     be rejected — its inline ids would be misread as dictionary ids.
        let mut tampered = meta.clone();
        tampered[8..12].copy_from_slice(&(1u32 << 30).to_le_bytes());
        std::fs::File::create(dir.join("dict-meta.bin")).unwrap().write_all(&tampered).unwrap();
        let err = Dict::open_mmap(&dir).err().expect("mismatched partition must be rejected");
        assert!(err.to_string().contains("partition mismatch"), "clear rebuild error: {err}");

        // (c) A LEGACY header-less file (begins directly with the prefix count) must be rejected.
        let legacy_body = &meta[12..]; // drop magic+version+partition → pre-versioning layout
        std::fs::File::create(dir.join("dict-meta.bin")).unwrap().write_all(legacy_body).unwrap();
        let err = Dict::open_mmap(&dir).err().expect("legacy header-less file must be rejected");
        assert!(err.to_string().contains("legacy"), "clear rebuild error: {err}");

        std::fs::remove_dir_all(&dir).ok();
    }

    /// [OPUS-4.8] sq-ueuk — exercise the mmap-FREE `&[u8]` validator seam directly from
    /// in-memory buffers (no real `mmap`, no file I/O), the same seam the Kani harness proves
    /// over a bounded symbolic domain. Builds a well-formed two-term store in RAM (one IRI +
    /// one literal) and then mutates each region to hit every rejection branch.
    #[cfg(feature = "mmap")]
    #[test]
    fn validate_dict_bytes_seam_accepts_valid_and_rejects_corruption() {
        // A well-formed store: ids 1 = IRI(prefix 0, suffix "a"), 2 = literal "v" of datatype 0.
        let prefixes: Vec<Box<str>> = vec![Box::from("http://ex/")];
        let datatypes = vec![NamedNode::new_unchecked("http://ex/dt")];
        let recs = [
            StoredRef::Iri { prefix: 0, suffix: "a" },
            StoredRef::Lit { value: "v", datatype: 0, lang: None },
        ];
        let mut blob: Vec<u8> = Vec::new();
        let mut offsets: Vec<u8> = Vec::new();
        for r in &recs {
            offsets.extend_from_slice(&(blob.len() as u64).to_le_bytes());
            write_record(&mut blob, r).unwrap();
        }
        let len = recs.len();
        // Sorted lookup arrays: hashes are not indexed here (length-only), ids must be 1..=len.
        let hashes: Vec<u8> = vec![0u8; len * 8];
        let mut hashids: Vec<u8> = Vec::new();
        hashids.extend_from_slice(&1u32.to_le_bytes());
        hashids.extend_from_slice(&2u32.to_le_bytes());

        // (a) The well-formed buffers validate.
        validate_dict_bytes(&blob, &offsets, &hashes, &hashids, len, &prefixes, &datatypes)
            .expect("a well-formed in-memory store validates");

        // (b) Wrong offset-array length (truncated dict-offs.bin) is rejected.
        let err = validate_dict_bytes(&blob, &offsets[..8], &hashes, &hashids, len, &prefixes, &datatypes)
            .expect_err("a truncated offset array must be rejected");
        assert!(err.to_string().contains("dict-offs.bin"), "{err}");

        // (c) An offset past the end of the blob is rejected (no panic / OOB).
        let mut bad_offsets = offsets.clone();
        bad_offsets[0..8].copy_from_slice(&((blob.len() + 1) as u64).to_le_bytes());
        let err = validate_dict_bytes(&blob, &bad_offsets, &hashes, &hashids, len, &prefixes, &datatypes)
            .expect_err("an out-of-range offset must be rejected");
        assert!(err.to_string().contains("past the end"), "{err}");

        // (d) An out-of-range prefix id is rejected (empty prefix table makes prefix 0 invalid).
        let err = validate_dict_bytes(&blob, &offsets, &hashes, &hashids, len, &[], &datatypes)
            .expect_err("an out-of-range prefix id must be rejected");
        assert!(err.to_string().contains("prefix id"), "{err}");

        // (e) A hashid of 0 (would underflow `offsets[id-1]`) is rejected.
        let mut bad_hashids = hashids.clone();
        bad_hashids[0..4].copy_from_slice(&0u32.to_le_bytes());
        let err = validate_dict_bytes(&blob, &offsets, &hashes, &bad_hashids, len, &prefixes, &datatypes)
            .expect_err("a zero hashid must be rejected");
        assert!(err.to_string().contains("dict-hid.bin"), "{err}");

        // (f) A hashid > len (would index past the offset array) is rejected.
        let mut over_hashids = hashids.clone();
        over_hashids[4..8].copy_from_slice(&(len as u32 + 1).to_le_bytes());
        let err = validate_dict_bytes(&blob, &offsets, &hashes, &over_hashids, len, &prefixes, &datatypes)
            .expect_err("an out-of-range hashid must be rejected");
        assert!(err.to_string().contains("dict-hid.bin"), "{err}");

        // (g) Totally-empty inputs with len 0 validate (vacuously well-formed).
        validate_dict_bytes(&[], &[], &[], &[], 0, &[], &[]).expect("an empty store validates");
    }

    /// [OPUS-4.8] sq-ueuk — the seam must reject a tampered triple term whose component id
    /// forward-references (or self-references) its own id, which would otherwise drive the
    /// reconstruct recursion unbounded. Build a single triple term whose subject id equals its
    /// own id and confirm it is refused at the `&[u8]` boundary (no panic / no stack blowup).
    #[cfg(feature = "mmap")]
    #[test]
    fn validate_dict_bytes_seam_rejects_self_referential_triple() {
        // id 1 = Triple([1, 1, 1]) — every component is the triple's own id (forward/self).
        let mut blob: Vec<u8> = Vec::new();
        write_record(&mut blob, &StoredRef::Triple([1, 1, 1])).unwrap();
        let offsets = 0u64.to_le_bytes().to_vec();
        let hashes = vec![0u8; 8];
        let hashids = 1u32.to_le_bytes().to_vec();
        let err = validate_dict_bytes(&blob, &offsets, &hashes, &hashids, 1, &[], &[])
            .expect_err("a self/forward-referential quoted-triple must be rejected");
        assert!(err.to_string().contains("quoted-triple"), "{err}");
    }

    #[test]
    fn merge_remap_translates_triple_term_components() {
        // A partial dict's triple term must be re-interned against the GLOBAL ids of its
        // components (which differ from the partial's local ids).
        let mut global = Dict::new();
        // Pre-populate the global dict so ids diverge from the partial's.
        for i in 0..5 {
            global.intern(&Term::NamedNode(NamedNode::new_unchecked(format!("http://pre/{i}"))));
        }
        let mut partial = Dict::new();
        let inner = triple("http://ex/a", "http://ex/b", int("7"));
        let outer = triple("http://ex/x", "http://ex/p", inner.clone());
        let local_outer = partial.intern(&outer);
        let remap = global.merge_remap(&partial);
        let gid = remap[(local_outer - 1) as usize];
        assert_eq!(global.term(gid), outer);
        assert_eq!(global.lookup(&outer), gid);
        assert_eq!(global.lookup(&inner), remap[(partial.lookup(&inner) - 1) as usize]);
    }

    #[test]
    fn non_integer_terms_get_dictionary_ids_below_inline_base() {
        let mut d = Dict::new();
        let iri = Term::NamedNode(oxrdf::NamedNode::new("http://ex/x").unwrap());
        let id = d.intern(&iri);
        assert!((1..INLINE_BASE).contains(&id));
        assert!(!is_inline(id));
        assert_eq!(d.term(id), iri);
    }

    /// [OPUS-4.8] sq-bif — `inline_id_of_int` is the engine's fast path for resolving a
    /// COMPUTED integer (BIND / aggregate result) directly to its inline id, with no term or
    /// lexical built. Previously it had no test. The id it returns MUST be the very id the
    /// canonical `xsd:integer` literal of that value interns to (so a computed value and a
    /// parsed literal collide on one id), and the in-range/out-of-range boundary must match
    /// `is_inline`. Values below 0 or above `INLINE_MAX` fall back to the dictionary (`None`).
    #[test]
    fn inline_id_of_int_matches_intern_and_respects_boundaries() {
        let mut d = Dict::new();
        // In-range values: the fast-path id equals interning the canonical integer literal,
        // and decodes back to that value via the inline partition.
        for v in [0i64, 1, 2, 42, 1_000_000, INLINE_MAX as i64 - 1, INLINE_MAX as i64] {
            let id = inline_id_of_int(v).unwrap_or_else(|| panic!("{v} is in range"));
            assert!(is_inline(id), "{v} maps to an inline id");
            assert_eq!(id, INLINE_BASE + v as u32, "exact inline encoding for {v}");
            assert_eq!(id, d.intern(&int(&v.to_string())), "fast-path id == canonical literal's id for {v}");
        }
        // Out-of-range: negative and just past INLINE_MAX both fall back to the dictionary.
        assert_eq!(inline_id_of_int(-1), None, "negative integers are not inline");
        assert_eq!(inline_id_of_int(i64::MIN), None);
        assert_eq!(inline_id_of_int(INLINE_MAX as i64 + 1), None, "one past the inline ceiling is not inline");
        assert_eq!(inline_id_of_int(i64::MAX), None);
        // The dictionary stayed empty: no inline value was ever stored as a term.
        assert_eq!(d.len(), 0);
    }

    /// [OPUS-4.8] sq-bif — `is_inline` must classify the id partition EXACTLY at its
    /// boundaries: `NO_ID`(0) and every dictionary id are NOT inline; `INLINE_BASE` (value 0)
    /// up to `INLINE_BASE + INLINE_MAX` (the inline ceiling) ARE; the first id ABOVE the
    /// inline range (the engine's local-vocab space) is NOT. A drift here would silently
    /// misread a local-vocab id as an integer value.
    #[test]
    fn is_inline_partition_boundaries() {
        assert!(!is_inline(NO_ID), "0 / NO_ID is not inline");
        assert!(!is_inline(1), "the first dictionary id is not inline");
        assert!(!is_inline(INLINE_BASE - 1), "the last dictionary id is not inline");
        assert!(is_inline(INLINE_BASE), "INLINE_BASE (value 0) is inline");
        assert!(is_inline(INLINE_BASE + INLINE_MAX), "the inline ceiling is inline");
        // One past the inline range is the engine's local-vocab space, NOT an inline integer.
        assert!(!is_inline(INLINE_BASE + INLINE_MAX + 1), "above the inline range is not inline");
        assert!(!is_inline(u32::MAX), "the very top of the id space is not inline");
    }
}
