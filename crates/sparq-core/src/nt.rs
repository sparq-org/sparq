//! A byte-level N-Triples parser that interns directly into the [`Dict`], skipping the
//! intermediate `oxrdf::Term` allocation that a general parser makes for every term
//! *occurrence*. The common no-escape case interns the raw UTF-8 slice with zero
//! allocation; terms containing `\` take a correct escape-decoding slow path. Used by
//! the parallel bulk loader for the `ntriples` format (it is one statement per line, so
//! a byte buffer can be split at newlines and parsed in parallel).
//!
//! Blank-node labels are kept verbatim (file-scoped, as the loader folds one document).
//!
//! [OPUS-4.8] (sq-hxgb) RDF 1.2 triple terms `<<( s p o )>>` are accepted in the OBJECT
//! position (object-only, may nest), interned structurally to agree with the Turtle loader
//! and the engine's CONSTRUCT N-Triples serializer (which emits this form). The fast plain-
//! triple path is unchanged: a triple term is only looked for when the object byte is `<`
//! AND the next two bytes are `<(`, so a plain `<…>` IRI pays just two extra byte peeks.

use crate::dict::{Dict, Id, NO_ID};
use std::borrow::Cow;

/// [OPUS-4.8] (sq-7d3dj.2) Rough average N-Triples line length, in bytes, used ONLY to pre-size
/// the per-chunk triple `Vec` (here) and the per-chunk partial `Dict` (in `parse_block`) before
/// filling them — killing the reallocation/rehash churn the ingest profile flagged (~5% of the
/// parallel load). It is a pure capacity HINT: `bytes / AVG_NT_LINE_BYTES` estimates the triple
/// (and, loosely, the distinct-term) count of a chunk, but it NEVER changes the parsed triples,
/// their order, or the interned term/id bijection (pinned by
/// `parse_chunk_dict_capacity_is_pure_hint` here and `presizing_is_pure_capacity_hint` for
/// `parse_block`). Crucially, the reservation is derived ENTIRELY from
/// the real chunk length: it is `bytes.len() / AVG_NT_LINE_BYTES`, hence never exceeds
/// `bytes.len()` and cannot grow independently of the input. So — unlike HDT's independently
/// declared `num_strings` (`decode.rs`), which an attacker could set huge behind a tiny file — it
/// needs no HDT-style clamp against an untrusted count. `64` tracks the mean line of the
/// deterministic CI corpus (`sparq-bench dump`); it is NOT a lower bound on line length, so
/// shorter lines make it a slight under-reservation and longer ones a slight over-reservation —
/// either way the miss is absorbed by ordinary `Vec`/`Dict` growth, since this only reserves
/// capacity and never bounds the parsed count.
pub(crate) const AVG_NT_LINE_BYTES: usize = 64;

/// [OPUS-4.8] (sq-53s1, ASVS V5.5.2) Maximum nesting depth of RDF 1.2 triple terms
/// `<<( s p o )>>` accepted by the byte-level N-Triples/N-Quads parser before it returns a
/// clean parse error instead of recursing until the native stack overflows and aborts the
/// process. `object_term`/`triple_term` (and the no-intern `span_object`/`span_triple_term`
/// graph-scan mirror) recurse once per `<<(` nesting level on the OBJECT axis; an adversarial
/// input such as `<<( <<( … )>> )>>` repeated 10k deep would otherwise blow the stack. The cap
/// is far deeper than any real RDF 1.2 data nests (triple-term nesting beyond a handful of
/// levels is unheard of in practice) yet shallow enough to keep recursion within the stack.
/// Kept in step with spargebra's `MAX_RECURSION_DEPTH` (the SPARQL side of the same control).
const MAX_TRIPLE_TERM_DEPTH: usize = 128;

/// [OPUS-4.8] (sq-25r3) The identity of an N-Quads quad's GRAPH field (4th term): an absolute
/// IRI or a blank-node label, decoded (escapes resolved) so two byte-different-but-equal graph
/// references route to the SAME graph. `None` (the default graph) is represented by the absence
/// of a key, not a variant here. Used purely as a routing key by the chunk-parallel N-Quads
/// loader — the graph name itself never enters any per-graph dictionary; it becomes the
/// `oxrdf::Term` stored in the parent graph's `named` vec. `Ord`/`Hash` give a deterministic
/// per-graph grouping independent of parse order.
#[cfg(feature = "parallel")]
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum GraphKey {
    Iri(String),
    Blank(String),
}

#[cfg(feature = "parallel")]
impl GraphKey {
    /// Rebuilds the `oxrdf::Term` for the parent graph's `named` entry. `new_unchecked` is sound:
    /// these strings came straight out of a successful parse (the same path the serial loader's
    /// `Term::NamedNode`/`Term::BlankNode` take).
    pub fn to_term(&self) -> oxrdf::Term {
        match self {
            GraphKey::Iri(s) => oxrdf::Term::NamedNode(oxrdf::NamedNode::new_unchecked(s)),
            GraphKey::Blank(s) => oxrdf::Term::BlankNode(oxrdf::BlankNode::new_unchecked(s)),
        }
    }
}

/// [OPUS-4.8] (sq-25r3) Relabels an error raised by the SHARED N-Triples term parsers (`term`,
/// `object_term`, `scan_delim`, `decode`, …) so it reads `N-Quads:` when surfaced from the
/// N-Quads fast path — the shared code emits the `N-Triples:` prefix, which is misleading to a
/// caller that handed in N-Quads. Only the leading prefix is rewritten; the byte offset and the
/// rest of the message are preserved.
#[cfg(feature = "parallel")]
fn as_nquads_err(e: String) -> String {
    match e.strip_prefix("N-Triples:") {
        Some(rest) => format!("N-Quads:{rest}"),
        None => e,
    }
}

/// [OPUS-4.8] (sq-25r3) Parses a slice of complete N-QUADS lines, ROUTING each quad's triple to a
/// per-graph partial bucket keyed by its 4th (graph) field — the byte-level analogue of
/// [`parse_chunk`] for datasets. N-Quads is one statement per line, so a byte buffer can be split
/// at newline boundaries and each range parsed independently; this is the per-range worker.
///
/// Returns, IN FIRST-OCCURRENCE ORDER within this byte range, `(Option<GraphKey>, Dict, triples)`
/// buckets — `None` is the default graph. Each bucket carries its OWN partial dict (S/P/O interned
/// locally), mirroring the serial loader's one-dict-per-graph construction; the graph NAME is the
/// routing key and is never interned into any of these dicts. Errors carry the byte offset.
///
/// Cross-range correctness: blank-node labels (subject/object AND graph) are kept verbatim, so the
/// dataset-scoped merge (per graph, by label, exactly as [`parse_chunk`]) unifies them across
/// ranges — see the loader (`load_nquads_parallel`).
#[cfg(feature = "parallel")]
#[allow(clippy::type_complexity)]
pub fn parse_quads_chunk(bytes: &[u8]) -> Result<Vec<(Option<GraphKey>, Dict, Vec<[Id; 3]>)>, String> {
    // [OPUS-5] (sq-7d3dj.21) Establish the module's `CHUNK-UTF8` invariant ONCE, before any byte
    // is scanned — the safety precondition of every `str_of` span extraction below. See `str_of`.
    validate_chunk_utf8(bytes, "N-Quads")?;
    // `buckets` is in first-occurrence order of graph keys (push-on-first-sight); `index` maps a
    // key to its slot so repeated references to the same graph append to the one existing bucket.
    use std::collections::HashMap;
    let mut buckets: Vec<(Option<GraphKey>, Dict, Vec<[Id; 3]>)> = Vec::new();
    let mut index: HashMap<Option<GraphKey>, usize> = HashMap::new();
    let n = bytes.len();
    let mut i = 0;
    while i < n {
        i = skip_ws(bytes, i);
        if i >= n {
            break;
        }
        if bytes[i] == b'#' {
            i = next_line(bytes, i);
            continue;
        }
        if bytes[i] == b'V' {
            i = version_directive(bytes, i).map_err(as_nquads_err)?;
            continue;
        }
        // We must know the GRAPH (which bucket dict to intern into) before interning S/P/O, but the
        // graph is the LAST field. So first locate the optional graph term by SPAN-scanning S/P/O
        // (a no-intern walk, the same cost as a parse), resolve the bucket, THEN intern S/P/O into
        // that bucket's dict by re-scanning from their start offsets. The span walk avoids any
        // throwaway interning into a wrong dict.
        let s_start = i;
        check_position(bytes, i, false).map_err(as_nquads_err)?;
        let j = span_term(bytes, i)?;
        i = skip_hws(bytes, j);
        let p_start = i;
        check_position(bytes, i, true).map_err(as_nquads_err)?;
        let j = span_term(bytes, i)?;
        i = skip_hws(bytes, j);
        let o_start = i;
        let j = span_object(bytes, i)?;
        i = skip_hws(bytes, j);
        // Optional graph term, then the mandatory `.`.
        let graph = if i < n && bytes[i] == b'.' {
            i = end_of_statement(bytes, i + 1).map_err(as_nquads_err)?;
            None
        } else {
            // Span first: it rejects a `<<(` triple term in graph position with the precise
            // object-only message before `graph_key`'s IRI scan sees the inner `<`.
            let jg = span_term(bytes, i)?;
            let g = graph_key(bytes, i)?;
            i = skip_hws(bytes, jg);
            if i >= n || bytes[i] != b'.' {
                return Err(format!("N-Quads: expected '.' at byte {i}"));
            }
            i = end_of_statement(bytes, i + 1).map_err(as_nquads_err)?;
            Some(g)
        };
        // Resolve (or create) the bucket for this graph, then intern S/P/O into ITS dict.
        let slot = *index.entry(graph.clone()).or_insert_with(|| {
            buckets.push((graph.clone(), Dict::new(), Vec::new()));
            buckets.len() - 1
        });
        let (_, dict, triples) = &mut buckets[slot];
        // S/P/O were already span-validated above; the only errors the shared (re-scanning)
        // intern path can still raise here are escape/UTF-8 decode failures, which carry the
        // N-Triples prefix from the shared term parser. Relabel them to N-Quads so a caller of
        // the N-Quads fast path never sees a misleading `N-Triples:` message (sq-25r3).
        let (s, _) = term(bytes, s_start, dict).map_err(as_nquads_err)?;
        let (p, _) = term(bytes, p_start, dict).map_err(as_nquads_err)?;
        let (o, _) = object_term(bytes, o_start, dict).map_err(as_nquads_err)?;
        triples.push([s, p, o]);
    }
    Ok(buckets)
}

/// [OPUS-4.8] (sq-25r3) Parses just the graph (4th) field of an N-Quads quad into a [`GraphKey`]
/// (an absolute IRI or a blank-node label), decoding escapes so the key is the graph's true
/// identity. Literals are NOT valid in graph position and are rejected.
#[cfg(feature = "parallel")]
fn graph_key(b: &[u8], i: usize) -> Result<GraphKey, String> {
    match b.get(i) {
        Some(b'<') => {
            let (start, end, esc, _next) = scan_iri(b, i).map_err(as_nquads_err)?;
            let s = decode(&b[start..end], esc).map_err(as_nquads_err)?;
            check_absolute(&s, i).map_err(as_nquads_err)?;
            Ok(GraphKey::Iri(s.into_owned()))
        }
        Some(b'_') => {
            if b.get(i + 1) != Some(&b':') {
                return Err(format!("N-Quads: bad blank node graph at byte {i}"));
            }
            let start = i + 2;
            let j = blank_label(b, start).map_err(as_nquads_err)?;
            Ok(GraphKey::Blank(str_of(&b[start..j]).to_owned()))
        }
        _ => Err(format!("N-Quads: graph name must be an IRI or blank node at byte {i}")),
    }
}

/// [OPUS-4.8] (sq-25r3) Returns the index just past a term WITHOUT interning — used to locate the
/// optional graph field before deciding which bucket dict to intern S/P/O into (the caller then
/// re-scans from the start offset to intern). Mirrors [`term`]'s grammar.
#[cfg(feature = "parallel")]
fn span_term(b: &[u8], i: usize) -> Result<usize, String> {
    match b.get(i) {
        // [OPUS-4.8] (sq-87bq) Triple terms are OBJECT-ONLY (RDF 1.2); a `<<(` reaching the
        // subject/predicate (or graph) span walk is a non-object triple term — reject with a
        // precise message, mirroring `term`. See the grammar note there (prods. [7]/[8]/[9]).
        Some(b'<') if b.get(i + 1) == Some(&b'<') && b.get(i + 2) == Some(&b'(') => Err(format!(
            "N-Quads: RDF 1.2 triple term `<<( … )>>` is only valid in OBJECT position \
             (subject/predicate/graph must be an IRI or blank node), at byte {i}"
        )),
        Some(b'<') => {
            let (_s, _e, _esc, next) = scan_delim(b, i, b'>')?;
            Ok(next)
        }
        Some(b'_') => {
            if b.get(i + 1) != Some(&b':') {
                return Err(format!("N-Quads: bad blank node at byte {i}"));
            }
            blank_label(b, i + 2).map_err(as_nquads_err)
        }
        Some(b'"') => span_literal(b, i),
        _ => Err(format!("N-Quads: unexpected term start at byte {i}")),
    }
}

/// [OPUS-4.8] (sq-25r3) Index just past an OBJECT-position term (which may be an RDF 1.2 triple
/// term `<<( … )>>`). Mirrors [`object_term`]'s grammar.
#[cfg(feature = "parallel")]
fn span_object(b: &[u8], i: usize) -> Result<usize, String> {
    span_object_depth(b, i, 0)
}

/// [OPUS-4.8] (sq-53s1) `span_object` carrying the triple-term nesting `depth` so the no-intern
/// graph-scan walk is bounded identically to the interning path (`object_term_depth`).
#[cfg(feature = "parallel")]
fn span_object_depth(b: &[u8], i: usize, depth: usize) -> Result<usize, String> {
    if b.get(i) == Some(&b'<') && b.get(i + 1) == Some(&b'<') && b.get(i + 2) == Some(&b'(') {
        span_triple_term(b, i, depth)
    } else {
        span_term(b, i)
    }
}

/// [OPUS-4.8] (sq-25r3) Index just past an RDF 1.2 triple term `<<( s p o )>>`, recursing through
/// its nested object. Mirrors [`triple_term`].
#[cfg(feature = "parallel")]
fn span_triple_term(b: &[u8], i: usize, depth: usize) -> Result<usize, String> {
    // [OPUS-4.8] (sq-53s1, ASVS V5.5.2) Mirror the interning path's depth bound (`triple_term`).
    if depth >= MAX_TRIPLE_TERM_DEPTH {
        return Err(format!(
            "N-Quads: RDF 1.2 triple term nested more than {MAX_TRIPLE_TERM_DEPTH} levels deep at byte {i}"
        ));
    }
    let mut j = skip_hws(b, i + 3);
    check_position(b, j, false).map_err(as_nquads_err)?;
    let k = span_term(b, j)?;
    j = skip_hws(b, k);
    check_position(b, j, true).map_err(as_nquads_err)?;
    let k = span_term(b, j)?;
    j = skip_hws(b, k);
    let k = span_object_depth(b, j, depth + 1)?;
    j = skip_hws(b, k);
    if b.get(j) == Some(&b')') && b.get(j + 1) == Some(&b'>') && b.get(j + 2) == Some(&b'>') {
        Ok(j + 3)
    } else {
        Err(format!("N-Quads: expected ')>>' closing triple term at byte {j}"))
    }
}

/// [OPUS-4.8] (sq-25r3) Span of a literal (value + optional `^^<dt>` / `@lang`), returning the
/// index just past it. Mirrors [`literal`]'s grammar without interning.
#[cfg(feature = "parallel")]
fn span_literal(b: &[u8], i: usize) -> Result<usize, String> {
    let (_value, after) = lex_string(b, i)?;
    let after = annotation_start(b, after);
    match b.get(after) {
        Some(b'^') if b.get(after + 1) == Some(&b'^') => {
            let open = skip_hws(b, after + 2);
            if b.get(open) != Some(&b'<') {
                return Err(format!("N-Quads: bad datatype at byte {open}"));
            }
            let (_ds, _de, _desc, next) = scan_delim(b, open, b'>')?;
            Ok(next)
        }
        Some(b'@') => {
            let mut j = after + 1;
            while j < b.len() && (b[j].is_ascii_alphanumeric() || b[j] == b'-') {
                j += 1;
            }
            Ok(j)
        }
        _ => Ok(after),
    }
}

/// [SONNET-4.6] (sq-7d3dj.31.3) Maximum byte length of a subject/predicate token stored
/// in the inline intern caches of [`parse_chunk`]. Tokens longer than this threshold fall
/// through to the normal `term()` intern path with no caching overhead.
///
/// 128 bytes covers virtually all realistic N-Triples subjects and predicates (Wikidata
/// entities are ~42 bytes, DBpedia resources ~50 bytes, SP2Bench articles ~55 bytes).
/// The inline copy is stack-allocated (two 128-byte arrays) and stays in L1 cache across
/// the hot inner loop, avoiding the cache-hostile read-from-file-position that a plain
/// `bytes[old_start..old_end]` comparison would cause on shuffled (non-grouped) input.
pub(crate) const SUBJ_PRED_CACHE_LEN: usize = 128;

/// Parses a slice of complete N-Triples lines, interning each term and returning the
/// id-triples. Errors carry the byte offset.
pub fn parse_chunk(bytes: &[u8], dict: &mut Dict) -> Result<Vec<[Id; 3]>, String> {
    // [OPUS-5] (sq-7d3dj.21) Establish the module's `CHUNK-UTF8` invariant ONCE, before any byte
    // is scanned — the safety precondition of every `str_of` span extraction below. See `str_of`.
    validate_chunk_utf8(bytes, "N-Triples")?;
    // [OPUS-4.8] (sq-7d3dj.2) Pre-size the id-triple output from the chunk's byte length so the
    // Vec does not re-grow (and copy) through ~log2(triples) doublings while parsing. Pure
    // capacity hint (see `AVG_NT_LINE_BYTES`); a tiny chunk (`< AVG_NT_LINE_BYTES` bytes) yields
    // `with_capacity(0)` == no allocation, and any slack is dropped with this Vec once the caller
    // remaps it into the final store — so it never reaches the `store_bytes_per_triple` ratchet.
    let mut out = Vec::with_capacity(bytes.len() / AVG_NT_LINE_BYTES);
    let n = bytes.len();
    let mut i = 0;
    // [SONNET-4.6] (sq-7d3dj.31.3) Subject and predicate intern caches. Grouped N-Triples
    // (the common dump shape: SP2Bench, WatDiv, DBpedia) repeat the same subject across
    // consecutive lines; the same small predicate set recurs throughout. Caching the last-seen
    // raw bytes and their interned id lets us skip the hash_iri + hash-table probe on a
    // run-length hit.
    //
    // KEY DESIGN: the cached bytes are stored in a FIXED STACK BUFFER, not compared against
    // `bytes[old_start..]`. A file-position comparison would cause L3 cache misses on
    // shuffled (non-grouped) input (the previous subject is at a random position in a
    // potentially 100s-of-MB file). The stack buffer is always in L1 — hits and misses are
    // both cache-friendly, keeping the shuffled path at baseline cost.
    //
    // Correctness invariants:
    //   • Exact byte equality: cache_s_buf[..cache_s_len] == bytes[s_start..s_after]
    //     is the HIT condition — no hash collisions, no false positives.
    //   • Boundary byte (whitespace or EOB) after the matched bytes is required for
    //     blank-node subjects (no closing delimiter): prevents `_:b0` (4 bytes) from
    //     falsely matching `_:b0extra` whose first 4 bytes are identical.
    //   • For IRI subjects/predicates, the `>` delimiter makes the token end unambiguous;
    //     the boundary check is always satisfied for valid N-Triples and adds no cost.
    //   • Cache miss: normal `term()` path, id-assignment order unchanged.
    //   • Subjects/predicates longer than SUBJ_PRED_CACHE_LEN bytes fall through to
    //     normal intern on every line (cache disabled for that token).
    //   • `NO_ID` (= 0) is the "empty" sentinel; `term()` never returns id 0.
    let mut cache_s_buf = [0u8; SUBJ_PRED_CACHE_LEN];
    let mut cache_s_len: usize = 0; // 0 = empty (also see NO_ID guard)
    let mut cache_s_id: Id = NO_ID;
    let mut cache_p_buf = [0u8; SUBJ_PRED_CACHE_LEN];
    let mut cache_p_len: usize = 0;
    let mut cache_p_id: Id = NO_ID;
    while i < n {
        i = skip_ws(bytes, i);
        if i >= n {
            break;
        }
        if bytes[i] == b'#' {
            i = next_line(bytes, i);
            continue;
        }
        if bytes[i] == b'V' {
            i = version_directive(bytes, i)?;
            continue;
        }
        // Subject: consult the stack-buffer cache before interning.
        let s_start = i;
        let s = {
            let s_after = s_start + cache_s_len;
            // `cache_s_id != NO_ID` guards both the non-empty check and prevents the
            // `cache_s_len == 0` (invalidated) path from spuriously matching an empty range.
            if cache_s_id != NO_ID
                && s_after <= n
                && bytes[s_start..s_after] == cache_s_buf[..cache_s_len]
                && (s_after >= n
                    || matches!(bytes[s_after], b' ' | b'\t' | b'\r' | b'\n'))
            {
                // Cache hit: same raw bytes → same interned id.
                i = s_after;
                cache_s_id
            } else {
                // Cache miss: intern normally, update the stack buffer.
                check_position(bytes, i, false)?;
                let (sid, j) = term(bytes, i, dict)?;
                let raw = &bytes[s_start..j];
                if raw.len() <= SUBJ_PRED_CACHE_LEN {
                    cache_s_buf[..raw.len()].copy_from_slice(raw);
                    cache_s_len = raw.len();
                    cache_s_id = sid;
                } else {
                    // Token too long for the inline buffer: disable the cache.
                    cache_s_len = 0;
                    cache_s_id = NO_ID;
                }
                i = j;
                sid
            }
        };
        i = skip_hws(bytes, i);
        // Predicate: same stack-buffer cache (predicates repeat even more than subjects).
        let p_start = i;
        let p = {
            let p_after = p_start + cache_p_len;
            if cache_p_id != NO_ID
                && p_after <= n
                && bytes[p_start..p_after] == cache_p_buf[..cache_p_len]
                && (p_after >= n
                    || matches!(bytes[p_after], b' ' | b'\t' | b'\r' | b'\n'))
            {
                i = p_after;
                cache_p_id
            } else {
                check_position(bytes, i, true)?;
                let (pid, j) = term(bytes, i, dict)?;
                let raw = &bytes[p_start..j];
                if raw.len() <= SUBJ_PRED_CACHE_LEN {
                    cache_p_buf[..raw.len()].copy_from_slice(raw);
                    cache_p_len = raw.len();
                    cache_p_id = pid;
                } else {
                    cache_p_len = 0;
                    cache_p_id = NO_ID;
                }
                i = j;
                pid
            }
        };
        i = skip_hws(bytes, i);
        // [OPUS-4.8] (sq-hxgb) Only the OBJECT position may be an RDF 1.2 triple term
        // `<<( s p o )>>` (matching the Turtle loader / RDF 1.2 grammar — triple terms
        // are object-only and may nest through their own object).
        let (o, j) = object_term(bytes, i, dict)?;
        i = skip_hws(bytes, j);
        if i >= n || bytes[i] != b'.' {
            return Err(format!("N-Triples: expected '.' at byte {}", i));
        }
        i = end_of_statement(bytes, i + 1)?;
        out.push([s, p, o]);
    }
    Ok(out)
}

#[inline]
fn skip_ws(b: &[u8], mut i: usize) -> usize {
    while i < b.len() && matches!(b[i], b' ' | b'\t' | b'\r' | b'\n') {
        i += 1;
    }
    i
}

/// Index of the first byte at or after `i` that is not a space or tab (`WS` inside a line).
#[inline]
fn skip_hws(b: &[u8], mut i: usize) -> usize {
    while i < b.len() && matches!(b[i], b' ' | b'\t') {
        i += 1;
    }
    i
}

/// After a string literal's closing quote: the index of its `^^` / `@` annotation when only
/// spaces/tabs separate them (the grammar allows WS between terminals), else `after`.
#[inline]
fn annotation_start(b: &[u8], after: usize) -> usize {
    let k = skip_hws(b, after);
    match b.get(k) {
        Some(b'@') => k,
        Some(b'^') if b.get(k + 1) == Some(&b'^') => k,
        _ => after,
    }
}

#[inline]
fn next_line(b: &[u8], mut i: usize) -> usize {
    // EOL is `[#xD#xA]+`: a lone CR ends a comment too.
    while i < b.len() && b[i] != b'\n' && b[i] != b'\r' {
        i += 1;
    }
    i
}

/// [OPUS-5] (sq-7d3dj.21) Establishes the module's `CHUNK-UTF8` invariant: proves the WHOLE
/// chunk buffer is valid UTF-8 in ONE contiguous pass, so every subsequent span extraction
/// (`str_of`) is O(1) instead of O(span). This is the safety precondition of the whole
/// module — see the `CHUNK-UTF8` block on `str_of` — and is called at the TOP of both public
/// entry points (`parse_chunk`, `parse_quads_chunk`) before any byte is scanned.
///
/// This is also STRICTER than the per-span validation it replaces: bytes OUTSIDE a term span
/// (comments, the inter-term filler) are now validated too. N-Triples/N-Quads documents are
/// defined to be UTF-8 (RDF 1.1 N-Triples §2, RDF 1.2 N-Quads §2), so rejecting a chunk with
/// invalid UTF-8 anywhere is the conformant answer; the serial `oxttl` loader rejects such a
/// document as well, so this moves the byte-level fast path TOWARD parity, not away from it.
///
/// Under the chunk-parallel loader each worker validates only ITS chunk. That is equivalent to
/// validating the whole file, because the splitter (`newline_chunk_bounds`) cuts exclusively at
/// `\n` — an ASCII byte, so no multi-byte character can straddle a cut. And it is fail-SAFE if
/// that ever changed: a straddling character would make both chunks fail validation (a spurious
/// rejection), never let an unvalidated span reach `str_of`.
///
/// `format` is the caller's grammar label (`"N-Triples"` / `"N-Quads"`) so the error message
/// matches the rest of that entry point's diagnostics.
#[inline]
fn validate_chunk_utf8(bytes: &[u8], format: &str) -> Result<(), String> {
    if let Err(e) = std::str::from_utf8(bytes) {
        // Positional `format!` args: the CodeQL `rust/unused-variable` query false-positives on
        // inline captures. [OPUS-5]
        return Err(format!("{}: invalid UTF-8 at byte {}", format, e.valid_up_to()));
    }
    Ok(())
}

/// Converts a term span of the chunk buffer to `&str` in O(1).
///
/// # `CHUNK-UTF8` — unsafe justification (sq-7d3dj.21; register row `src/nt.rs`)
///
/// This is NOT a threat-model **B5** site (hostile on-disk file → mmap → unchecked reinterpret).
/// The input is an in-memory chunk that this process fully validates before any unchecked
/// access, so hostile bytes produce a clean `Err` carrying a byte offset, never UB.
///
/// **Precondition.** `raw` is a subslice `b[x..y]` of the chunk buffer `b` that the calling
/// entry point already proved valid UTF-8 via `validate_chunk_utf8`, and `x`/`y` are UTF-8
/// character boundaries of `b`.
///
/// **Why the boundaries hold — the ASCII-delimiter argument.** Every span index this module
/// produces is `0`, `b.len()`, or an index *at* or *immediately after* a byte the scanner
/// matched literally against an ASCII constant: `<` `>` `"` `\` `_` `:` `.` `@`, the four
/// whitespace bytes, or an `is_ascii_alphanumeric()` / `-` test. UTF-8 is self-synchronising —
/// in a valid encoding every byte `< 0x80` is a complete one-byte character and can never be a
/// continuation byte — so any position at or just past such a byte is a character boundary.
/// Concretely, the four call sites, grouped by the scanner that produced the span:
///
/// * `decode` via `scan_iri` / `scan_string` (same shape as `scan_delim`) — `x = open + 1`
///   where `b[open]` is `<` or `"`; `y` is the index where the scan found the ASCII closer
///   (every other loop exit returns `Err` before any span is taken). The `\` escape skip can only ever step over bytes,
///   never change which byte the closer test matched.
/// * `blank` and `graph_key` — `x = i + 2` past the ASCII `_:`; `y` from `blank_label`,
///   which stops at `b.len()`, an ASCII non-label byte, or the LEAD byte of a rejected
///   non-ASCII scalar (accepted scalars are consumed whole), then backs over ASCII `.` bytes.
///   Its one-scalar decode slices `b[j..j + len]` from a lead byte by that byte's UTF-8 width.
/// * `literal`'s `@lang` tag — `x = after + 1` past the ASCII `@`; `y` is the first index whose
///   byte fails `is_ascii_alphanumeric() || == b'-'`, i.e. either `b.len()` or the index OF a
///   non-ASCII byte. A non-ASCII byte reached by scanning forward over ASCII bytes in a valid
///   encoding is a LEAD byte, which is a character boundary.
///
/// Both halves of the precondition are therefore structural, not incidental. The invariant is
/// module-local and cheap to audit: `nt` has exactly two public entry points and both validate
/// on their first line.
///
/// **How it is bounded.** The `debug_assert!` below re-checks the full precondition on every
/// call in every debug/test build; the nightly `miri` lane interprets this module's tests under
/// Tree Borrows, where an invalid `&str` produced here is caught as UB; and the
/// `parse_chunk_rejects_invalid_utf8` / `parse_quads_chunk_rejects_invalid_utf8` tests pin the
/// entry-point validation while `multibyte_spans_at_every_delimiter_round_trip`,
/// `escaped_and_multibyte_literal_span_round_trips` and
/// `language_tag_terminated_by_multibyte_errors_cleanly` pin the boundary argument at each
/// delimiter (each mutation-checked: mis-cutting a span end by one byte reds them).
#[inline]
fn str_of(raw: &[u8]) -> &str {
    debug_assert!(
        std::str::from_utf8(raw).is_ok(),
        "nt CHUNK-UTF8 invariant violated: a term span was not valid UTF-8 — the entry point \
         must call validate_chunk_utf8 before any span is taken (sq-7d3dj.21)"
    );
    // SAFETY: `raw` is an ASCII-delimited span of a buffer `validate_chunk_utf8` already proved
    // is valid UTF-8, so both span ends are character boundaries and the span is itself valid
    // UTF-8 — see the `CHUNK-UTF8` justification above for the per-call-site argument. The
    // `debug_assert!` re-checks it on every debug/test call and the `miri` lane interprets it.
    unsafe { std::str::from_utf8_unchecked(raw) }
}

fn term(b: &[u8], i: usize, dict: &mut Dict) -> Result<(Id, usize), String> {
    match b.get(i) {
        // [OPUS-4.8] (sq-87bq) A `<<(` here is an RDF 1.2 triple term in a NON-object
        // position (subject, predicate, or a triple term's own subject/predicate). The RDF
        // 1.2 N-Triples grammar makes triple terms OBJECT-ONLY — `subject ::= IRIREF |
        // BLANK_NODE_LABEL` (prod. [7]) and `predicate ::= IRIREF` (prod. [8]); only
        // `object ::= IRIREF | BLANK_NODE_LABEL | literal | tripleTerm` (prod. [9]) admits a
        // `tripleTerm` (prod. [11]). (This differs from the legacy RDF-star CG `<< s p o >>`
        // QUOTED-triple syntax, which DID allow subject quoting; RDF 1.2 deliberately dropped
        // that.) Reject with a precise message instead of the generic "unexpected term start".
        Some(b'<') if b.get(i + 1) == Some(&b'<') && b.get(i + 2) == Some(&b'(') => Err(format!(
            "N-Triples: RDF 1.2 triple term `<<( … )>>` is only valid in OBJECT position \
             (subject/predicate must be an IRI or blank node), at byte {i}"
        )),
        Some(b'<') => iri(b, i, dict),
        Some(b'_') => blank(b, i, dict),
        Some(b'"') => literal(b, i, dict),
        _ => Err(format!("N-Triples: unexpected term start at byte {i}")),
    }
}

/// [OPUS-4.8] (sq-hxgb) Parses a term in the OBJECT position, where (and only where) RDF 1.2
/// N-Triples permits a triple term `<<( s p o )>>`. The `<<(` opener disambiguates a triple
/// term from a plain IRIREF (`<…>`); anything else falls through to the shared [`term`] path.
fn object_term(b: &[u8], i: usize, dict: &mut Dict) -> Result<(Id, usize), String> {
    object_term_depth(b, i, dict, 0)
}

/// [OPUS-4.8] (sq-53s1) `object_term` carrying the current triple-term nesting `depth` so a
/// pathologically nested `<<( … )>>` chain returns a clean parse error at `MAX_TRIPLE_TERM_DEPTH`
/// instead of overflowing the stack. `depth` is the number of `<<(` openers already entered above
/// this call (0 at the top-level object position).
fn object_term_depth(b: &[u8], i: usize, dict: &mut Dict, depth: usize) -> Result<(Id, usize), String> {
    if b.get(i) == Some(&b'<') && b.get(i + 1) == Some(&b'<') && b.get(i + 2) == Some(&b'(') {
        triple_term(b, i, dict, depth)
    } else {
        term(b, i, dict)
    }
}

/// [OPUS-4.8] (sq-hxgb) Parses an RDF 1.2 triple term `<<( s p o )>>` and interns it
/// STRUCTURALLY (subject/predicate/object interned first, then the triple by their ids — the
/// same `Dict::intern_triple_ids` path `intern(&Term::Triple(_))` and the Turtle loader take,
/// so the two loaders content-address triple terms identically). The subject is an IRI or
/// blank node, the predicate an IRI, and the object any term — including a NESTED triple term
/// (recursion through `object_term`). Returns the interned id and the index past the `>>`.
fn triple_term(b: &[u8], i: usize, dict: &mut Dict, depth: usize) -> Result<(Id, usize), String> {
    // [OPUS-4.8] (sq-53s1, ASVS V5.5.2) Bound the triple-term nesting before recursing into the
    // object, so deeply-nested input yields a clean parse error rather than a stack overflow.
    if depth >= MAX_TRIPLE_TERM_DEPTH {
        return Err(format!(
            "N-Triples: RDF 1.2 triple term nested more than {MAX_TRIPLE_TERM_DEPTH} levels deep at byte {i}"
        ));
    }
    // Skip the `<<(` opener.
    let mut j = skip_hws(b, i + 3);
    check_position(b, j, false)?;
    let (s, k) = term(b, j, dict)?;
    j = skip_hws(b, k);
    check_position(b, j, true)?;
    let (p, k) = term(b, j, dict)?;
    j = skip_hws(b, k);
    let (o, k) = object_term_depth(b, j, dict, depth + 1)?;
    j = skip_hws(b, k);
    // Closer `)>>`.
    if b.get(j) == Some(&b')') && b.get(j + 1) == Some(&b'>') && b.get(j + 2) == Some(&b'>') {
        Ok((dict.intern_triple_ids([s, p, o]), j + 3))
    } else {
        Err(format!("N-Triples: expected ')>>' closing triple term at byte {j}"))
    }
}

/// Subject / predicate position check before the shared [`term`] parser: neither may be a
/// literal, and a predicate may not be a blank node (`subject ::= IRIREF | BLANK_NODE_LABEL`,
/// `predicate ::= IRIREF`). A triple term keeps `term`'s own object-only message.
#[inline]
fn check_position(b: &[u8], i: usize, predicate: bool) -> Result<(), String> {
    match b.get(i) {
        Some(b'"') => Err(format!(
            "N-Triples: a literal cannot be a subject or predicate, at byte {i}"
        )),
        Some(b'_') if predicate => Err(format!(
            "N-Triples: a blank node cannot be a predicate, at byte {i}"
        )),
        _ => Ok(()),
    }
}

/// RDF 1.2 `versionDirective ::= 'VERSION' STRING_LITERAL_QUOTE` (any version string is
/// accepted; `"1.2"` and `"1.2-basic"` are the defined ones). It must end its line. Returns
/// the index just past the directive.
fn version_directive(b: &[u8], i: usize) -> Result<usize, String> {
    if !b[i..].starts_with(b"VERSION") {
        return Err(format!("N-Triples: unexpected term start at byte {i}"));
    }
    // No line-start check is needed: every statement ends through [`end_of_statement`], and a
    // chunk always starts at a line start (the splitter cuts after `\n`).
    let mut j = i + 7;
    while matches!(b.get(j), Some(b' ' | b'\t')) {
        j += 1;
    }
    if b.get(j) != Some(&b'"') {
        return Err(format!(
            "N-Triples: VERSION needs a quoted version string at byte {j}"
        ));
    }
    // The value is ignored, but it goes through the same lexer as a literal's.
    let (_value, j) = lex_string(b, j)?;
    end_of_statement(b, j)
}

/// THE end of every statement and directive (triple, quad, `VERSION`), called right after its
/// terminator on both entry points: statements are separated by `EOL`, so only spaces/tabs and
/// an optional comment may follow before a line break or the end of input. Returns the index of
/// that line break (or `b.len()`).
#[inline]
fn end_of_statement(b: &[u8], i: usize) -> Result<usize, String> {
    let mut j = skip_hws(b, i);
    if b.get(j) == Some(&b'#') {
        j = next_line(b, j);
    }
    match b.get(j) {
        None | Some(b'\n' | b'\r') => Ok(j),
        _ => Err(format!(
            "N-Triples: expected end of line after statement, at byte {j}"
        )),
    }
}

/// Byte classes for [`scan_string`]: `1` = closing `"`, `2` = `\`, `3` = raw LF / CR (not
/// allowed in `STRING_LITERAL_QUOTE`), `0` = anything else.
const STRING_CLASS: [u8; 256] = {
    let mut t = [0u8; 256];
    t[b'"' as usize] = 1;
    t[b'\\' as usize] = 2;
    t[b'\n' as usize] = 3;
    t[b'\r' as usize] = 3;
    t
};

/// [`scan_delim`] for a `STRING_LITERAL_QUOTE`: a raw newline or carriage return inside the
/// quotes is an error. `\` skips the next byte (the escape is decoded and checked later).
#[inline]
fn scan_string(b: &[u8], open: usize) -> Result<(usize, usize, bool, usize), String> {
    let start = open + 1;
    let mut j = start;
    let mut esc = false;
    while j < b.len() {
        match STRING_CLASS[b[j] as usize] {
            0 => j += 1,
            1 => return Ok((start, j, esc, j + 1)),
            2 => {
                esc = true;
                j += 2;
            }
            _ => return Err(format!("N-Triples: raw line break inside a string literal at byte {j}")),
        }
    }
    Err(format!("N-Triples: unterminated delimiter from byte {open}"))
}

/// THE lexer for a `STRING_LITERAL_QUOTE` (`'"' ([^#x22#x5C#xA#xD] | ECHAR | UCHAR)* '"'`):
/// every quoted string — literal values on every entry point, the N-Quads span walk, and
/// `VERSION` specifiers — goes through here, so all of them get the same raw-line-break,
/// ECHAR and UCHAR (scalar-value) checks. Returns the decoded value and the index past the
/// closing quote.
#[inline]
fn lex_string(b: &[u8], open: usize) -> Result<(Cow<'_, str>, usize), String> {
    let (start, end, esc, next) = scan_string(b, open)?;
    Ok((decode(&b[start..end], esc)?, next))
}

/// Scans `<...>` returning the (escaped?) content range and the index past `>` (used only by
/// the no-intern N-Quads span walk; the interning paths use [`scan_iri`] / [`scan_string`]).
#[cfg(feature = "parallel")]
fn scan_delim(b: &[u8], open: usize, close: u8) -> Result<(usize, usize, bool, usize), String> {
    let start = open + 1;
    let mut j = start;
    let mut esc = false;
    while j < b.len() && b[j] != close {
        if b[j] == b'\\' {
            esc = true;
            j += 1;
        }
        j += 1;
    }
    if j >= b.len() {
        return Err(format!("N-Triples: unterminated delimiter from byte {open}"));
    }
    Ok((start, j, esc, j + 1))
}

/// Byte classes for [`scan_iri`]: `0` = ordinary IRIREF byte, `1` = the closing `>`, `2` = `\`
/// (must start a `\u` / `\U` UCHAR), `3` = a byte the IRIREF production forbids
/// (`` [#x00-#x20<>"{}|^`\] ``). Bytes `>= 0x80` are ordinary (UTF-8 already validated).
const IRI_CLASS: [u8; 256] = {
    let mut t = [0u8; 256];
    let mut c = 0;
    while c <= 0x20 {
        t[c] = 3;
        c += 1;
    }
    t[b'<' as usize] = 3;
    t[b'"' as usize] = 3;
    t[b'{' as usize] = 3;
    t[b'}' as usize] = 3;
    t[b'|' as usize] = 3;
    t[b'^' as usize] = 3;
    t[b'`' as usize] = 3;
    t[b'>' as usize] = 1;
    t[b'\\' as usize] = 2;
    t
};

/// [`scan_delim`] for an `IRIREF` (`` '<' ([^#x00-#x20<>"{}|^`\] | UCHAR)* '>' ``): the same
/// `(start, end, escaped, next)` result, but a forbidden byte or a non-UCHAR `\` escape is an
/// error (W3C `nt-syntax-bad-uri-01`/`-04`). One table lookup per byte.
#[inline]
fn scan_iri(b: &[u8], open: usize) -> Result<(usize, usize, bool, usize), String> {
    let start = open + 1;
    let mut j = start;
    let mut esc = false;
    while j < b.len() {
        match IRI_CLASS[b[j] as usize] {
            0 => j += 1,
            1 => return Ok((start, j, esc, j + 1)),
            2 if matches!(b.get(j + 1), Some(b'u' | b'U')) => {
                esc = true;
                j += 2;
            }
            _ => return Err(format!("N-Triples: invalid IRI character at byte {j}")),
        }
    }
    Err(format!("N-Triples: unterminated delimiter from byte {open}"))
}

/// N-Triples / N-Quads IRIs must be absolute: the (decoded) IRI starts with a scheme
/// `ALPHA *( ALPHA / DIGIT / "+" / "-" / "." ) ":"` (W3C `nt-syntax-bad-uri-06..09`).
#[inline]
fn check_absolute(iri: &str, at: usize) -> Result<(), String> {
    let b = iri.as_bytes();
    if b.first().is_some_and(u8::is_ascii_alphabetic) {
        for &c in &b[1..] {
            if c == b':' {
                return Ok(());
            }
            if !(c.is_ascii_alphanumeric() || matches!(c, b'+' | b'-' | b'.')) {
                break;
            }
        }
    }
    Err(format!("N-Triples: relative IRI <{}> at byte {}", iri, at))
}

fn decode<'a>(raw: &'a [u8], esc: bool) -> Result<Cow<'a, str>, String> {
    let s = str_of(raw);
    if esc {
        Ok(Cow::Owned(unescape(s)?))
    } else {
        Ok(Cow::Borrowed(s))
    }
}

// [OPUS-4.8] (sq-7d3dj.18) Per-thread `scheme://authority/` prefix memo for the opt-in IRI
// validation fast path. A thread-local is sound under the rayon chunk-parallel loader — each
// worker keeps its own ring — and needs no clearing between parses: every memoized prefix was
// accepted by `oxiri`, and IRI validity is a pure function of the bytes, so a retained (or
// cross-parse) prefix can only change SPEED, never the accept/reject answer.
#[cfg(feature = "iri-fast")]
thread_local! {
    static IRI_MEMO: std::cell::RefCell<crate::iri::IriPrefixMemo> =
        std::cell::RefCell::new(crate::iri::IriPrefixMemo::new());
}

fn iri(b: &[u8], i: usize, dict: &mut Dict) -> Result<(Id, usize), String> {
    let (start, end, esc, next) = scan_iri(b, i)?;
    let s = decode(&b[start..end], esc)?;
    check_absolute(&s, i)?;
    // [OPUS-4.8] (sq-7d3dj.18) When the opt-in `iri-fast` feature is on, validate every IRIREF
    // against RFC-3987 (fast path in front of `oxiri`) so the byte-level loader rejects the
    // malformed IRIs the serial oxttl path already rejects — reaching conformance parity at
    // fast-path cost. OFF by default: the loader keeps its current unvalidated-fast behaviour.
    #[cfg(feature = "iri-fast")]
    {
        let valid = IRI_MEMO.with(|m| m.borrow_mut().validate(&s));
        if !valid {
            return Err(format!("N-Triples: invalid IRI <{}> at byte {}", s, i));
        }
    }
    Ok((dict.intern_iri(&s), next))
}

/// End index (exclusive) of a `BLANK_NODE_LABEL` body starting at `start` (the byte after
/// `_:`): `(PN_CHARS_U | [0-9]) ((PN_CHARS | '.')* PN_CHARS)?`. The scan takes ASCII bytes
/// through a fast path and decodes each non-ASCII scalar against the full `PN_CHARS_BASE` /
/// `PN_CHARS` ranges, stopping at the first character that cannot continue the label, then
/// backs over trailing `.` bytes, which are the statement terminator, not
/// part of the label (`_:a.b` → `a.b`; `_:abc.` → `abc`, matching oxttl). Stopping at the first
/// non-label byte lets a label abut the next term (`_:s<http://p>`, W3C `minimal_whitespace`);
/// a label whose first character is not `PN_CHARS_U` or a digit (`-`, `.`, U+00B7, U+00D7, ...)
/// is empty and so an error (`nt-syntax-bad-bnode-01`); a non-label character later in the
/// label (a stray `:` or U+00D7) ends it and is left for the caller to reject as the next
/// term (`nt-syntax-bad-bnode-02`).
/// The single source of truth shared by [`blank`] (interning), [`span_term`] (no-intern span)
/// and [`graph_key`] (graph field) so the three never diverge.
fn blank_label(b: &[u8], start: usize) -> Result<usize, String> {
    let mut j = start;
    while j < b.len() {
        let c = b[j];
        let first = j == start;
        if c < 0x80 {
            // ASCII fast path: PN_CHARS_U | [0-9] first, then PN_CHARS | '.'.
            if c.is_ascii_alphanumeric() || c == b'_' || (!first && matches!(c, b'-' | b'.')) {
                j += 1;
                continue;
            }
            break;
        }
        // Non-ASCII: decode one scalar (chunk is valid UTF-8 and `j` is a lead byte).
        let len = match c {
            0xF0.. => 4,
            0xE0.. => 3,
            _ => 2,
        };
        let ch = str_of(&b[j..(j + len).min(b.len())]).chars().next().unwrap_or('\0');
        if is_pn_chars_base(ch) || (!first && matches!(ch, '\u{B7}' | '\u{300}'..='\u{36F}' | '\u{203F}'..='\u{2040}')) {
            j += len;
            continue;
        }
        break;
    }
    while j > start && b[j - 1] == b'.' {
        j -= 1;
    }
    if j == start {
        Err(format!("N-Triples: bad or empty blank node label at byte {start}"))
    } else {
        Ok(j)
    }
}

/// The non-ASCII part of `PN_CHARS_BASE` (the ASCII letters are handled by the fast path).
#[inline]
fn is_pn_chars_base(ch: char) -> bool {
    matches!(ch,
        '\u{C0}'..='\u{D6}' | '\u{D8}'..='\u{F6}' | '\u{F8}'..='\u{2FF}' | '\u{370}'..='\u{37D}'
        | '\u{37F}'..='\u{1FFF}' | '\u{200C}'..='\u{200D}' | '\u{2070}'..='\u{218F}'
        | '\u{2C00}'..='\u{2FEF}' | '\u{3001}'..='\u{D7FF}' | '\u{F900}'..='\u{FDCF}'
        | '\u{FDF0}'..='\u{FFFD}' | '\u{10000}'..='\u{EFFFF}')
}

fn blank(b: &[u8], i: usize, dict: &mut Dict) -> Result<(Id, usize), String> {
    if b.get(i + 1) != Some(&b':') {
        return Err(format!("N-Triples: bad blank node at byte {i}"));
    }
    let start = i + 2;
    let j = blank_label(b, start)?;
    Ok((dict.intern_blank(str_of(&b[start..j])), j))
}

const XSD_STRING: &str = "http://www.w3.org/2001/XMLSchema#string";
const RDF_LANG_STRING: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#langString";
const RDF_DIR_LANG_STRING: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#dirLangString";

/// [OPUS-4.8] (sq-langcase / #1119) ASCII-lowercases a parsed language tag, BORROWING the input
/// when it is already all-lowercase (the common case) so the no-uppercase fast path stays
/// allocation-free. RDF language tags are ASCII (`(PN_CHARS_BASE)('-' alnum*)*` plus the RDF 1.2
/// `--ltr`/`--rtl` direction), so `make_ascii_lowercase` is the correct, locale-independent fold —
/// the SAME normalisation `oxrdf`/`oxttl` apply, keeping the byte parser's stored language slot
/// byte-identical to the oxttl paths' for the same tag.
fn lowercase_lang(raw: &str) -> Cow<'_, str> {
    if raw.bytes().any(|c| c.is_ascii_uppercase()) {
        Cow::Owned(raw.to_ascii_lowercase())
    } else {
        Cow::Borrowed(raw)
    }
}

/// RDF 1.2 `LANG_DIR`: a well-formed BCP47 language tag (RDF 1.2 Concepts requires one; the
/// grammar's `[a-zA-Z]+ ('-' [a-zA-Z0-9]+)*` alone admits `en-123456789`), then an optional
/// base direction that is exactly `--ltr` or `--rtl` (case-sensitive: W3C
/// `ntriples-langdir-bad-2` rejects `--LTR`). The tag check is the serial parser's own
/// (`oxrdf` → `oxilangtag`), behind an allocation-free fast path for the common
/// `ll` / `lll` / `ll-RR` / `ll-999` shapes.
fn valid_lang_tag(t: &[u8]) -> bool {
    let (tag, dir) = match t.windows(2).position(|w| w == b"--") {
        Some(k) => (&t[..k], Some(&t[k + 2..])),
        None => (t, None),
    };
    if !dir.is_none_or(|d| d == b"ltr" || d == b"rtl") {
        return false;
    }
    let alpha = |p: &[u8]| p.iter().all(u8::is_ascii_alphabetic);
    let common = match tag.iter().position(|&c| c == b'-') {
        None => matches!(tag.len(), 2 | 3) && alpha(tag),
        Some(k) => {
            let (lang, region) = (&tag[..k], &tag[k + 1..]);
            matches!(lang.len(), 2 | 3)
                && alpha(lang)
                && ((region.len() == 2 && alpha(region))
                    || (region.len() == 3 && region.iter().all(u8::is_ascii_digit)))
        }
    };
    common
        || std::str::from_utf8(tag)
            .is_ok_and(|tag| oxrdf::Literal::new_language_tagged_literal("", tag).is_ok())
}

fn literal(b: &[u8], i: usize, dict: &mut Dict) -> Result<(Id, usize), String> {
    let (value, after) = lex_string(b, i)?;
    let after = annotation_start(b, after);
    match b.get(after) {
        // ^^<datatype>
        Some(b'^') if b.get(after + 1) == Some(&b'^') => {
            let open = skip_hws(b, after + 2);
            if b.get(open) != Some(&b'<') {
                return Err(format!("N-Triples: bad datatype at byte {open}"));
            }
            let (dstart, dend, desc, next) = scan_iri(b, open)?;
            let dt = decode(&b[dstart..dend], desc)?;
            check_absolute(&dt, open)?;
            // A language-tagged string needs its tag (W3C `ntriples-langdir-bad-3` / `-5`).
            if dt == RDF_LANG_STRING || dt == RDF_DIR_LANG_STRING {
                return Err(format!(
                    "N-Triples: {dt} literal without a language tag at byte {open}"
                ));
            }
            Ok((dict.intern_lit(&value, &dt, None), next))
        }
        // @lang
        Some(b'@') => {
            let lstart = after + 1;
            let mut j = lstart;
            while j < b.len() && (b[j].is_ascii_alphanumeric() || b[j] == b'-') {
                j += 1;
            }
            // RDF 1.2 `@lang--dir` keeps the combined slot (the dict's stored encoding)
            // but is typed rdf:dirLangString.
            let raw = str_of(&b[lstart..j]);
            if !valid_lang_tag(raw.as_bytes()) {
                return Err(format!("N-Triples: invalid language tag at byte {lstart}"));
            }
            let dt = if raw.contains("--") { RDF_DIR_LANG_STRING } else { RDF_LANG_STRING };
            // [OPUS-4.8] (sq-langcase / KamiQuasi #1119) NORMALISE the language tag to ASCII
            // lowercase, matching the oxttl/oxrdf paths (`Literal::new_language_tagged_literal`
            // lowercases via `make_ascii_lowercase`; oxttl's Turtle/N-Quads/TriG recognisers call
            // `language.to_ascii_lowercase()` before interning). RDF 1.1/1.2 language tags are
            // case-INSENSITIVE, so `"x"@en-US` and `"x"@en-us` are the SAME RDF term and must
            // intern to the SAME dict id (term identity is byte-equality of the stored slot).
            // Without this the byte-level N-Triples/N-Quads fast path preserved the written case
            // while every oxttl path lowercased it — a format-dependent inconsistency (a Turtle
            // doc lowercased, the same triple in N-Triples did not) AND a latent term-identity
            // bug (case-variant tags interned as two distinct terms). The `--dir` suffix is only
            // ever `ltr`/`rtl` (already lowercase) so lowercasing the whole slot is equivalent to
            // lowercasing just the BCP47 language part. The all-lowercase case (the overwhelming
            // majority) borrows with no allocation; only a tag carrying uppercase pays one alloc.
            let lang = lowercase_lang(raw);
            Ok((dict.intern_lit(&value, dt, Some(&lang)), j))
        }
        // simple literal -> xsd:string
        _ => Ok((dict.intern_lit(&value, XSD_STRING, None), after)),
    }
}

/// Decodes N-Triples ECHAR (`\t \b \n \r \f \" \' \\`) and UCHAR (`\uXXXX`, `\UXXXXXXXX`).
fn unescape(s: &str) -> Result<String, String> {
    let mut out = String::with_capacity(s.len());
    let mut it = s.chars();
    while let Some(c) = it.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match it.next() {
            Some('t') => out.push('\t'),
            Some('b') => out.push('\u{8}'),
            Some('n') => out.push('\n'),
            Some('r') => out.push('\r'),
            Some('f') => out.push('\u{C}'),
            Some('"') => out.push('"'),
            Some('\'') => out.push('\''),
            Some('\\') => out.push('\\'),
            Some('u') => out.push(hex(&mut it, 4)?),
            Some('U') => out.push(hex(&mut it, 8)?),
            _ => return Err("N-Triples: bad escape".into()),
        }
    }
    Ok(out)
}

fn hex(it: &mut std::str::Chars<'_>, n: usize) -> Result<char, String> {
    let mut cp = 0u32;
    for _ in 0..n {
        let d = it.next().and_then(|c| c.to_digit(16)).ok_or("N-Triples: bad \\u escape")?;
        cp = cp * 16 + d;
    }
    char::from_u32(cp).ok_or_else(|| "N-Triples: invalid code point".into())
}

// [OPUS-4.8] (sq-hxgb) Triple-term parsing in the OBJECT position: structural interning,
// nesting, blank-node components, fast-path-unchanged, and the rejection cases.
#[cfg(test)]
mod tests {
    use super::*;
    use oxrdf::{BlankNode, Literal, NamedNode, Term, Triple};

    fn iri(s: &str) -> Term {
        Term::NamedNode(NamedNode::new_unchecked(s))
    }

    /// #2716: the W3C N-Triples syntax cases the byte parser used to accept (bad IRI chars,
    /// relative IRIs, bad blank-node labels, bad language tags) are rejected on both entry points.
    #[test]
    fn w3c_negative_syntax_cases_rejected() {
        let bad: &[&str] = &[
            "<http://example/ space> <http://example/p> <http://example/o> .",
            "<http://example/\\n> <http://example/p> <http://example/o> .",
            "<http://example/{x}> <http://example/p> <http://example/o> .",
            "<s> <http://example/p> <http://example/o> .",
            "<http://example/s> <p> <http://example/o> .",
            "<http://example/s> <http://example/p> <o> .",
            "<http://example/s> <http://example/p> \"foo\"^^<dt> .",
            "_::a  <http://example/p> <http://example/o> .",
            "_:abc:def  <http://example/p> <http://example/o> .",
            "_:-a <http://example/p> <http://example/o> .",
            "<http://example/s> <http://example/p> \"string\"@1 .",
            "<http://example/s> <http://example/p> \"string\"@en- .",
        ];
        for doc in bad {
            assert!(parse_chunk(doc.as_bytes(), &mut Dict::new()).is_err(), "N-Triples accepted {doc}");
            #[cfg(feature = "parallel")]
            assert!(parse_quads_chunk(doc.as_bytes()).is_err(), "N-Quads accepted {doc}");
        }
        #[cfg(feature = "parallel")]
        assert!(parse_quads_chunk(b"<http://example/s> <http://example/p> <http://example/o> <g>.").is_err());
    }

    /// #2716 review round: further grammar gaps — a label starting with U+00B7, a raw line
    /// break inside a string, literal / blank-node predicates, a literal subject, a direction
    /// other than ltr/rtl, and a VERSION directive not ending its line — are rejected.
    #[test]
    fn grammar_gaps_rejected() {
        let bad: &[&str] = &[
            "_:\u{B7}a <http://example/p> <http://example/o> .",
            "_:\u{301}a <http://example/p> <http://example/o> .",
            "<http://example/s> <http://example/p> \"a\nb\" .",
            "<http://example/s> <http://example/p> \"a\rb\" .",
            "<http://example/s> \"p\" <http://example/o> .",
            "<http://example/s> _:p <http://example/o> .",
            "\"s\" <http://example/p> <http://example/o> .",
            "<http://example/s> <http://example/p> <<( <http://example/a> \"b\" <http://example/c> )>> .",
            "<http://example/s> <http://example/p> \"x\"@en--foo .",
            "<http://example/s> <http://example/p> \"x\"@en--LTR .",
            "<http://example/s> <http://example/p> \"x\"@en--Rtl .",
            "<http://example/s> <http://example/p> \"x\"@en-123456789 .",
            "<http://example/s> <http://example/p> \"x\"@abcdefghi .",
            "<http://example/s> <http://example/p> \"x\"^^<http://www.w3.org/1999/02/22-rdf-syntax-ns#langString> .",
            "<http://example/s> <http://example/p> \"x\"^^<http://www.w3.org/1999/02/22-rdf-syntax-ns#dirLangString> .",
            "VERSION \"1.2\" <http://example/s> <http://example/p> <http://example/o> .",
            "VERSION 1.2\n",
            "VERSIONX \"1.2\"\n",
            "VERSION \"\\q\"\n",
            "<http://example/s> <http://example/p> <http://example/o> . VERSION \"1.2\"\n",
            "<http://example/s> <http://example/p> <http://example/o> .\t VERSION \"1.2\"\n",
            "<http://example/s> <http://example/p> \"x\"^^ \n<http://example/dt> .",
            "VERSION \"\\uD800\"\n",
            "_:\u{D7} <http://example/p> <http://example/o> .",
            "_:a\u{D7} <http://example/p> <http://example/o> .",
            "_:a\u{D7}b <http://example/p> <http://example/o> .",
            "<http://example/s> <http://example/p> _:o\u{D7} .",
        ];
        for doc in bad {
            assert!(parse_chunk(doc.as_bytes(), &mut Dict::new()).is_err(), "N-Triples accepted {doc:?}");
            #[cfg(feature = "parallel")]
            assert!(parse_quads_chunk(doc.as_bytes()).is_err(), "N-Quads accepted {doc:?}");
        }
    }

    /// Every statement form against every separator that can follow its terminator, on both
    /// fast entry points and the serial parser: statements are separated by `EOL`, so after `.`
    /// (or a `VERSION` string) only spaces/tabs and a comment may come before a line break or
    /// EOF, and no line break may fall between the terms of a statement.
    #[test]
    fn statement_terminator_and_separator_table() {
        // (statement, is a quad, is a VERSION directive)
        let forms: &[(&str, bool, bool)] = &[
            ("<urn:s> <urn:p> <urn:o> .", false, false),
            ("<urn:s> <urn:p> <urn:o>.", false, false),
            ("<urn:s> <urn:p> _:o .", false, false),
            ("<urn:s> <urn:p> _:o.", false, false),
            ("_:s <urn:p> \"x\" .", false, false),
            ("<urn:s> <urn:p> \"x\"@en .", false, false),
            ("<urn:s> <urn:p> \"x\"^^<urn:dt>.", false, false),
            (
                "<urn:s> <urn:p> <<( <urn:a> <urn:b> <urn:c> )>> .",
                false,
                false,
            ),
            ("<urn:s> <urn:p> <urn:o> <urn:g> .", true, false),
            ("<urn:s> <urn:p> <urn:o> _:g.", true, false),
            ("VERSION \"1.2\"", false, true),
        ];
        // (separator, accepted). `next` is a full second statement on its own line, so
        // `sep + next` either keeps the first statement alone on its line or does not.
        let next = "<urn:s2> <urn:p2> <urn:o2> .\n";
        let seps: &[(&str, bool)] = &[
            ("\n", true),
            ("\r\n", true),
            ("\r", true),
            ("\n\n", true),
            (" \t\n", true),
            ("#c\n", true),
            (" # c\n", true),
            ("\t#c\r", true),
            ("", false),
            (" ", false),
            ("\t", false),
            (" _:b <urn:p> <urn:o> .\n", false),
            ("_:b <urn:p> <urn:o> .\n", false),
            (" \"x\" .\n", false),
            (" . \n", false),
            (" VERSION \"1.2\"\n", false),
            ("VERSION \"1.2\"\n", false),
        ];
        // EOF straight after the statement, with optional trailing WS/comment.
        let eofs = ["", " ", "\t", "#c", " # c"];

        let fast_nt = |doc: &str| {
            parse_chunk(doc.as_bytes(), &mut Dict::new())
                .map(|t| t.len())
                .ok()
        };
        #[cfg(feature = "parallel")]
        let fast_nq = |doc: &str| {
            parse_quads_chunk(doc.as_bytes())
                .map(|b| b.iter().map(|(_, _, t)| t.len()).sum::<usize>())
                .ok()
        };
        let serial_nt = |doc: &str| {
            oxttl::NTriplesParser::new()
                .for_slice(doc.as_bytes())
                .collect::<Result<Vec<_>, _>>()
                .map(|v| v.len())
                .ok()
        };
        let serial_nq = |doc: &str| {
            oxttl::NQuadsParser::new()
                .for_slice(doc.as_bytes())
                .collect::<Result<Vec<_>, _>>()
                .map(|v| v.len())
                .ok()
        };
        let check = |doc: &str, quad: bool, version: bool, n_ok: Option<usize>| {
            let nt_expect = if quad { None } else { n_ok };
            assert_eq!(fast_nt(doc), nt_expect, "parse_chunk on {doc:?}");
            #[cfg(feature = "parallel")]
            assert_eq!(fast_nq(doc), n_ok, "parse_quads_chunk on {doc:?}");
            // The serial parser predates `VERSION`; it must agree on every other form.
            if !version {
                assert_eq!(serial_nt(doc), nt_expect, "serial N-Triples on {doc:?}");
                assert_eq!(serial_nq(doc), n_ok, "serial N-Quads on {doc:?}");
            }
        };
        for &(form, quad, version) in forms {
            let first = usize::from(!version);
            for &(sep, ok) in seps {
                let doc = format!("{form}{sep}{next}");
                let n = if ok { Some(first + 1) } else { None };
                // A separator carrying `VERSION` is outside the serial parser's grammar.
                check(&doc, quad, version || sep.contains("VERSION"), n);
            }
            for eof in eofs {
                check(&format!("{form}{eof}"), quad, version, Some(first));
            }
        }

        // No line break between the terms of a statement (or inside a triple term), from any
        // entry point; spaces and tabs are fine there.
        let split: &[(&str, bool)] = &[
            ("<urn:s>\t<urn:p>  <urn:o>\t.\n", true),
            (
                "<urn:s> <urn:p> <<(\t<urn:a>\t<urn:b>  <urn:c> )>>\t.\n",
                true,
            ),
            ("<urn:s>\n<urn:p> <urn:o> .\n", false),
            ("<urn:s> <urn:p>\n<urn:o> .\n", false),
            ("<urn:s> <urn:p> <urn:o>\n.\n", false),
            ("<urn:s>\r<urn:p> <urn:o> .\n", false),
            ("<urn:s> <urn:p>\r\n<urn:o> .\n", false),
            ("_:s\n<urn:p> <urn:o> .\n", false),
            ("<urn:s> <urn:p> \"x\"\n.\n", false),
            ("<urn:s> <urn:p> \"x\"\n@en .\n", false),
            (
                "<urn:s> <urn:p> <<(\n<urn:a> <urn:b> <urn:c> )>> .\n",
                false,
            ),
            (
                "<urn:s> <urn:p> <<( <urn:a> <urn:b> <urn:c>\n)>> .\n",
                false,
            ),
        ];
        for &(doc, ok) in split {
            check(doc, false, false, ok.then_some(1));
        }
        let quad_split = [
            "<urn:s> <urn:p> <urn:o>\n<urn:g> .\n",
            "<urn:s> <urn:p> <urn:o> <urn:g>\n.\n",
        ];
        for doc in quad_split {
            check(doc, true, false, None);
        }
        check("VERSION\n\"1.2\"\n", false, true, None);
    }

    /// Language tags are well-formed BCP47 and directions are exactly `ltr` / `rtl`, on both
    /// entry points, agreeing with the serial parser tag by tag.
    #[test]
    fn language_tags_match_serial_bcp47() {
        let tags: &[(&str, bool)] = &[
            ("en", true),
            ("EN", true),
            ("deu", true),
            ("en-US", true),
            ("en-419", true),
            ("abcd", true),
            ("abcdefgh", true),
            ("zh-Hant-TW", true),
            ("en-12345678", true),
            ("de-CH-1901", true),
            ("en-US-u-ca-gregory", true),
            ("x-private", true),
            ("i-klingon", true),
            ("en--ltr", true),
            ("en-US--rtl", true),
            ("en-123456789", false),
            ("abcdefghi", false),
            ("en-x-abcdefghi", false),
            ("a", false),
            ("en-a", false),
            ("en-US-US", false),
            ("en--LTR", false),
            ("en--Ltr", false),
            ("en--RTL", false),
            ("en--up", false),
        ];
        for &(tag, ok) in tags {
            let doc = format!("<urn:s> <urn:p> \"x\"@{tag} .\n");
            assert_eq!(
                parse_chunk(doc.as_bytes(), &mut Dict::new()).is_ok(),
                ok,
                "N-Triples @{tag}"
            );
            #[cfg(feature = "parallel")]
            assert_eq!(
                parse_quads_chunk(doc.as_bytes()).is_ok(),
                ok,
                "N-Quads @{tag}"
            );
            let serial = oxttl::NTriplesParser::new()
                .for_slice(doc.as_bytes())
                .all(|r| r.is_ok());
            assert_eq!(serial, ok, "serial N-Triples @{tag}");
        }
    }

    /// The W3C grammar's character classes, transcribed independently of the parser
    /// (RDF 1.2 N-Triples `PN_CHARS_BASE`, `PN_CHARS_U`, `PN_CHARS`).
    const PN_CHARS_BASE_RANGES: &[(u32, u32)] = &[
        (0x41, 0x5A),
        (0x61, 0x7A),
        (0xC0, 0xD6),
        (0xD8, 0xF6),
        (0xF8, 0x2FF),
        (0x370, 0x37D),
        (0x37F, 0x1FFF),
        (0x200C, 0x200D),
        (0x2070, 0x218F),
        (0x2C00, 0x2FEF),
        (0x3001, 0xD7FF),
        (0xF900, 0xFDCF),
        (0xFDF0, 0xFFFD),
        (0x10000, 0xEFFFF),
    ];
    const PN_CHARS_EXTRA_RANGES: &[(u32, u32)] = &[
        (0x2D, 0x2D),
        (0x30, 0x39),
        (0xB7, 0xB7),
        (0x300, 0x36F),
        (0x203F, 0x2040),
    ];
    fn in_ranges(c: u32, r: &[(u32, u32)]) -> bool {
        r.iter().any(|&(lo, hi)| (lo..=hi).contains(&c))
    }
    fn spec_pn_chars_u(c: u32) -> bool {
        in_ranges(c, PN_CHARS_BASE_RANGES) || c == 0x5F
    }
    fn spec_pn_chars(c: u32) -> bool {
        spec_pn_chars_u(c) || in_ranges(c, PN_CHARS_EXTRA_RANGES)
    }

    /// Every boundary code point (lo-1, lo, hi, hi+1) of every PN_CHARS_BASE / PN_CHARS_U /
    /// PN_CHARS range, as a label's first character and as a middle character, on both entry
    /// points: accepted exactly when the grammar allows it there.
    #[test]
    fn blank_label_matches_grammar_at_every_class_boundary() {
        let mut points: Vec<u32> = vec![0x5E, 0x5F, 0x60, 0x2E];
        for &(lo, hi) in PN_CHARS_BASE_RANGES.iter().chain(PN_CHARS_EXTRA_RANGES) {
            points.extend([lo.saturating_sub(1), lo, hi, hi + 1]);
        }
        for cp in points {
            let Some(c) = char::from_u32(cp) else { continue };
            // First position: PN_CHARS_U or [0-9].
            let first_ok = spec_pn_chars_u(cp) || c.is_ascii_digit();
            let doc = format!("_:{c}x <http://example/p> <http://example/o> .\n");
            let got = parse_chunk(doc.as_bytes(), &mut Dict::new()).is_ok();
            assert_eq!(got, first_ok, "first char U+{cp:04X}");
            #[cfg(feature = "parallel")]
            assert_eq!(parse_quads_chunk(doc.as_bytes()).is_ok(), first_ok, "N-Quads first char U+{cp:04X}");
            // Middle position: PN_CHARS or '.'. A disallowed character ends the label and the
            // rest (`{c}y`) is not a term, so the document is rejected.
            let mid_ok = spec_pn_chars(cp) || c == '.';
            let doc = format!("_:x{c}y <http://example/p> <http://example/o> .\n");
            let mut d = Dict::new();
            let got = parse_chunk(doc.as_bytes(), &mut d);
            assert_eq!(got.is_ok(), mid_ok, "middle char U+{cp:04X}");
            if mid_ok {
                let label = format!("x{c}y");
                assert!(d.lookup(&Term::BlankNode(BlankNode::new_unchecked(label))) != 0, "U+{cp:04X} kept in label");
            }
            #[cfg(feature = "parallel")]
            assert_eq!(parse_quads_chunk(doc.as_bytes()).is_ok(), mid_ok, "N-Quads middle char U+{cp:04X}");
        }
    }

    /// ECHAR / UCHAR boundaries through the one string lexer, in a literal and in a VERSION
    /// specifier, on both entry points.
    #[test]
    fn string_escapes_match_grammar_at_boundaries() {
        let cases: &[(&str, bool)] = &[
            (r"\t", true), (r"\b", true), (r"\n", true), (r"\r", true), (r"\f", true),
            (r#"\""#, true), (r"\'", true), (r"\\", true),
            (r"\a", false), (r"\v", false), (r"\/", false), (r"\q", false), (r"\0", false),
            (r"\u0000", true), (r"퟿", true), (r"\uD800", false), (r"\uDFFF", false),
            (r"", true), (r"￿", true), (r"\U0010FFFF", true), (r"\U00110000", false),
            (r"\U0000D800", false), (r"\u12", false), (r"\uGGGG", false), (r"\U0001F6Z0", false),
        ];
        for &(esc, ok) in cases {
            for doc in [
                format!("<http://example/s> <http://example/p> \"a{esc}b\" .\n"),
                format!("VERSION \"1.2{esc}\"\n"),
            ] {
                assert_eq!(parse_chunk(doc.as_bytes(), &mut Dict::new()).is_ok(), ok, "{doc:?}");
                #[cfg(feature = "parallel")]
                assert_eq!(parse_quads_chunk(doc.as_bytes()).is_ok(), ok, "N-Quads {doc:?}");
            }
        }
    }

    /// A lone CR ends a comment line (EOL is `[#xD#xA]+`), RDF 1.2 VERSION directives are
    /// accepted, and a mid-label U+00B7 / `--rtl` direction still parse.
    #[test]
    fn cr_comment_version_and_valid_unicode_accepted() {
        let doc = "VERSION \"1.2\"\r\nVERSION \"1.2-basic\" # c\n#comment\r<urn:s> <urn:p> <urn:o> .\n\
                   _:a\u{B7}b <urn:p> \"x\"@ar--rtl .\n\
                   _:\u{E9}t\u{E9}\u{301}-\u{3042}.\u{10000}_9 <urn:p> _:\u{D8}\u{2C00}\u{203F}x .\n\
                   VERSION \"1.2\\u0041\"\n  \tVERSION \"1.2\"\n\
                   <urn:s> <urn:p> \"x\"^^ <urn:dt> .\n<urn:s> <urn:p> \"y\" \t^^\t<urn:dt> .\n\
                   <urn:s> <urn:p> \"z\" @en .\n";
        let mut d = Dict::new();
        let t = parse_chunk(doc.as_bytes(), &mut d).expect("valid N-Triples 1.2");
        assert_eq!(t.len(), 6, "the triple after a CR-terminated comment is kept");
        let dt_lit = Term::Literal(Literal::new_typed_literal("x", NamedNode::new_unchecked("urn:dt")));
        assert!(d.lookup(&dt_lit) != 0, "`^^ <dt>` with whitespace keeps its datatype");
        assert!(d.lookup(&Term::BlankNode(BlankNode::new_unchecked("\u{E9}t\u{E9}\u{301}-\u{3042}.\u{10000}_9"))) != 0);
        #[cfg(feature = "parallel")]
        assert_eq!(parse_quads_chunk(doc.as_bytes()).expect("valid N-Quads 1.2")[0].2.len(), 6);
    }

    /// #2716: a blank-node label ends at the first non-label byte, so it may abut the next term
    /// (W3C `minimal_whitespace`); valid escapes, dotted labels and direction tags still parse.
    #[test]
    fn w3c_positive_syntax_cases_accepted() {
        let doc = "_:s<http://example/p><http://example/o>.\n_:s<http://example/p>\"Alice\".\n\
                   _:s<http://example/p>_:bnode1.\n<http://example/s><http://example/p>_:o.\n\
                   _:a.b <http://example/\\u0041> \"x\"@en-US--ltr .\n\
                   _:a.b <urn:x-y.z+w:p> \"x\"^^<http://example/dt> .\n";
        let mut d = Dict::new();
        let t = parse_chunk(doc.as_bytes(), &mut d).expect("valid N-Triples");
        assert_eq!(t.len(), 6);
        assert_eq!(t[0][0], t[1][0], "`_:s` abutting `<` is the same label each time");
        assert!(d.lookup(&Term::BlankNode(BlankNode::new_unchecked("a.b"))) != 0);
        assert!(d.lookup(&iri("http://example/A")) != 0);
        #[cfg(feature = "parallel")]
        assert_eq!(parse_quads_chunk(doc.as_bytes()).expect("valid N-Quads")[0].2.len(), 6);
    }

    #[test]
    fn triple_term_object_parses_and_interns_structurally() {
        // The exact byte form the CONSTRUCT serializer emits.
        let nt = b"<http://ex/r1> <http://ex/reifies> <<( <http://ex/alice> <http://ex/age> \"30\"^^<http://www.w3.org/2001/XMLSchema#integer> )>> .\n";
        let mut d = Dict::new();
        let t = parse_chunk(nt, &mut d).unwrap();
        assert_eq!(t.len(), 1);
        let [s, p, o] = t[0];
        assert_eq!(d.term(s), iri("http://ex/r1"));
        assert_eq!(d.term(p), iri("http://ex/reifies"));
        // Object is a structural Term::Triple — not a literal or opaque IRI.
        let want = Term::Triple(Box::new(Triple::new(
            NamedNode::new_unchecked("http://ex/alice"),
            NamedNode::new_unchecked("http://ex/age"),
            Term::Literal(Literal::new_typed_literal("30", NamedNode::new_unchecked("http://www.w3.org/2001/XMLSchema#integer"))),
        )));
        assert_eq!(d.term(o), want);
        // Content-addressed: re-interning the identical term shares the id.
        assert_eq!(d.lookup(&want), o);
    }

    #[test]
    fn nested_and_blank_node_triple_terms_parse() {
        let nt = b"<http://ex/r> <http://ex/p> <<( _:b0 <http://ex/q> <<( <http://ex/a> <http://ex/b> <http://ex/c> )>> )>> .\n";
        let mut d = Dict::new();
        let t = parse_chunk(nt, &mut d).unwrap();
        assert_eq!(t.len(), 1);
        let inner = Term::Triple(Box::new(Triple::new(
            NamedNode::new_unchecked("http://ex/a"),
            NamedNode::new_unchecked("http://ex/b"),
            iri("http://ex/c"),
        )));
        let outer = Term::Triple(Box::new(Triple::new(
            BlankNode::new_unchecked("b0"),
            NamedNode::new_unchecked("http://ex/q"),
            inner,
        )));
        assert_eq!(d.term(t[0][2]), outer);
    }

    #[test]
    fn plain_triples_unaffected() {
        // A plain IRI object (`<…>`) must NOT be mistaken for a triple term, and a literal
        // object beginning with `<`-free bytes parses as before.
        let nt = b"<http://ex/s> <http://ex/p> <http://ex/o> .\n<http://ex/s> <http://ex/p> \"v\" .\n";
        let mut d = Dict::new();
        let t = parse_chunk(nt, &mut d).unwrap();
        assert_eq!(t.len(), 2);
        assert_eq!(d.term(t[0][2]), iri("http://ex/o"));
        assert_eq!(d.term(t[1][2]), Term::Literal(Literal::new_simple_literal("v")));
    }

    // [OPUS-4.8] (sq-langcase / KamiQuasi #1119) The byte-level N-Triples/N-Quads parser
    // NORMALISES language tags to ASCII lowercase, matching every oxttl path (the Turtle /
    // N-Quads / TriG recognisers + `oxrdf::Literal::new_language_tagged_literal`). Previously the
    // byte path PRESERVED the written case (`en-US`) while oxttl lowercased it (`en-us`), giving a
    // format-dependent result for the SAME triple. RDF 1.1/1.2 language tags are case-insensitive,
    // so the lowercased slot is the canonical RDF term — and the regression target is twofold:
    // (a) cross-format consistency, (b) `"x"@en-US` and `"x"@en-us` interning to the SAME id.
    #[test]
    fn language_tag_lowercased_to_match_oxttl() {
        let nt = b"<http://ex/s> <http://ex/p> \"hi\"@en-US .\n";
        let mut d = Dict::new();
        let t = parse_chunk(nt, &mut d).unwrap();
        assert_eq!(t.len(), 1);
        // The stored term carries the lowercased tag — byte-identical to what oxttl produces.
        let want = Term::Literal(Literal::new_language_tagged_literal_unchecked("hi", "en-us"));
        assert_eq!(d.term(t[0][2]), want);
        // And it equals what the checked oxrdf constructor (which lowercases) yields from the
        // mixed-case tag, i.e. the byte parser and the oxttl/oxrdf paths agree.
        let oxttl_like =
            Term::Literal(Literal::new_language_tagged_literal("hi", "en-US").unwrap());
        assert_eq!(d.term(t[0][2]), oxttl_like);
    }

    #[test]
    fn case_variant_language_tags_intern_to_same_id() {
        // Two lines whose ONLY difference is the language-tag case must content-address to one id
        // (term identity is case-insensitive on the tag) — they did NOT before this fix.
        let nt = b"<http://ex/s> <http://ex/p> \"hi\"@en-US .\n<http://ex/s2> <http://ex/p> \"hi\"@en-us .\n";
        let mut d = Dict::new();
        let t = parse_chunk(nt, &mut d).unwrap();
        assert_eq!(t.len(), 2);
        assert_eq!(t[0][2], t[1][2], "case-variant language tags must share one dict id");
    }

    #[test]
    fn directional_language_tag_lowercases_lang_part_only() {
        // RDF 1.2 `@lang--dir`: the BCP47 language part lowercases; the direction (`ltr`/`rtl`,
        // already lowercase) is unchanged. The stored slot must match the oxttl encoding exactly.
        let nt = b"<http://ex/s> <http://ex/p> \"hi\"@en-US--ltr .\n";
        let mut d = Dict::new();
        let t = parse_chunk(nt, &mut d).unwrap();
        assert_eq!(t.len(), 1);
        let want = Term::Literal(
            Literal::new_directional_language_tagged_literal_unchecked(
                "hi",
                "en-us",
                oxrdf::BaseDirection::Ltr,
            ),
        );
        assert_eq!(d.term(t[0][2]), want);
    }

    // The N-Quads byte fast path interns S/P/O through the SAME `literal` path, so it lowercases
    // too — guarding against a future divergence between the two byte loaders.
    #[cfg(feature = "parallel")]
    #[test]
    fn nquads_language_tag_lowercased() {
        let nq = b"<http://ex/s> <http://ex/p> \"hi\"@en-US <http://ex/g> .\n";
        let buckets = parse_quads_chunk(nq).unwrap();
        assert_eq!(buckets.len(), 1);
        let (_g, dict, triples) = &buckets[0];
        let want = Term::Literal(Literal::new_language_tagged_literal_unchecked("hi", "en-us"));
        assert_eq!(dict.term(triples[0][2]), want);
    }

    #[test]
    fn already_lowercase_language_tag_unchanged() {
        // The common all-lowercase tag is untouched (and the fast borrow path is exercised).
        let nt = b"<http://ex/s> <http://ex/p> \"hi\"@fr .\n";
        let mut d = Dict::new();
        let t = parse_chunk(nt, &mut d).unwrap();
        let want = Term::Literal(Literal::new_language_tagged_literal_unchecked("hi", "fr"));
        assert_eq!(d.term(t[0][2]), want);
    }

    #[test]
    fn unterminated_triple_term_errors() {
        let nt = b"<http://ex/s> <http://ex/p> <<( <http://ex/a> <http://ex/b> <http://ex/c> .\n";
        let mut d = Dict::new();
        assert!(parse_chunk(nt, &mut d).is_err(), "missing )>> must error, not silently accept");
    }

    #[test]
    fn triple_term_only_in_object_position() {
        // [OPUS-4.8] (sq-87bq) RDF 1.2 makes triple terms OBJECT-ONLY (N-Triples grammar:
        // `subject ::= IRIREF | BLANK_NODE_LABEL` [7], `predicate ::= IRIREF` [8], only
        // `object` [9] admits a `tripleTerm` [11]). The parser must REJECT a `<<( … )>>` in
        // subject OR predicate position with a precise, position-aware message — not misparse
        // it, and not silently accept the legacy RDF-star CG `<< … >>` subject-quoting that
        // RDF 1.2 dropped.

        // Subject position.
        let subj = b"<<( <http://ex/a> <http://ex/b> <http://ex/c> )>> <http://ex/p> <http://ex/o> .\n";
        let mut d = Dict::new();
        let e = parse_chunk(subj, &mut d).expect_err("triple term in subject position must be rejected");
        assert!(e.contains("only valid in OBJECT position"), "message must explain the object-only rule, got: {e}");

        // Predicate position.
        let pred = b"<http://ex/s> <<( <http://ex/a> <http://ex/b> <http://ex/c> )>> <http://ex/o> .\n";
        let mut d = Dict::new();
        let e = parse_chunk(pred, &mut d).expect_err("triple term in predicate position must be rejected");
        assert!(e.contains("only valid in OBJECT position"), "message must explain the object-only rule, got: {e}");

        // A triple term's OWN subject must also be a plain term (recursion goes through
        // `term`, not `object_term`): a nested `<<( … )>>` in the inner subject is rejected.
        let inner_subj = b"<http://ex/s> <http://ex/p> <<( <<( <http://ex/a> <http://ex/b> <http://ex/c> )>> <http://ex/q> <http://ex/r> )>> .\n";
        let mut d = Dict::new();
        assert!(
            parse_chunk(inner_subj, &mut d).is_err(),
            "a triple term in a triple term's SUBJECT slot must be rejected (object-only nesting)"
        );

        // Sanity: the same triple term in OBJECT position parses fine (the rejection is
        // strictly position-driven, not a blanket ban).
        let obj = b"<http://ex/s> <http://ex/p> <<( <http://ex/a> <http://ex/b> <http://ex/c> )>> .\n";
        let mut d = Dict::new();
        assert!(parse_chunk(obj, &mut d).is_ok(), "the same triple term in object position must parse");
    }

    // [OPUS-4.8] (sq-53s1, ASVS V5.5.2) Builds one N-Triples line whose OBJECT is `depth` levels
    // of nested RDF 1.2 triple terms `<<( s p <<( … )>> )>>`. Each level wraps the previous object
    // in a fresh triple term; the innermost object is a plain IRI.
    fn nested_triple_term_line(depth: usize) -> Vec<u8> {
        let mut line = b"<http://ex/s> <http://ex/p> ".to_vec();
        for _ in 0..depth {
            line.extend_from_slice(b"<<( <http://ex/a> <http://ex/b> ");
        }
        line.extend_from_slice(b"<http://ex/leaf>");
        for _ in 0..depth {
            line.extend_from_slice(b" )>>");
        }
        line.extend_from_slice(b" .\n");
        line
    }

    #[test]
    fn deeply_nested_triple_term_returns_clean_error_not_stack_overflow() {
        // 10k levels of `<<( … )>>` nesting must NOT recurse until the native stack overflows
        // (which would abort the whole process); it must return a normal parse `Err`.
        let line = nested_triple_term_line(10_000);
        let mut d = Dict::new();
        let e = parse_chunk(&line, &mut d).expect_err("deeply-nested triple terms must be rejected, not overflow the stack");
        assert!(
            e.contains("nested more than") && e.contains("levels deep"),
            "must be the depth-limit error, got: {e}"
        );
        // Sanitized per the #241 posture: the error reports the limit and a byte offset, never
        // echoes the (attacker-controlled) offending input.
        assert!(!e.contains("http://ex/leaf"), "error must not echo the offending input");
    }

    #[test]
    fn nested_triple_term_at_limit_parses() {
        // One level below the cap must still parse cleanly — the bound is high enough that no
        // real RDF 1.2 data is rejected. `MAX_TRIPLE_TERM_DEPTH` is the number of `<<(` levels,
        // so a line nested exactly `MAX_TRIPLE_TERM_DEPTH` deep is accepted.
        let line = nested_triple_term_line(MAX_TRIPLE_TERM_DEPTH);
        let mut d = Dict::new();
        assert!(
            parse_chunk(&line, &mut d).is_ok(),
            "nesting up to the cap ({MAX_TRIPLE_TERM_DEPTH}) must parse"
        );

        // One level beyond the cap is the first rejected depth.
        let over = nested_triple_term_line(MAX_TRIPLE_TERM_DEPTH + 1);
        let mut d = Dict::new();
        assert!(
            parse_chunk(&over, &mut d).is_err(),
            "nesting one beyond the cap must be rejected"
        );
    }

    // [OPUS-4.8] (sq-53s1) The N-Quads fast path uses a separate no-intern span walk
    // (`span_object`/`span_triple_term`) to locate the graph field; it must be bounded too.
    #[cfg(feature = "parallel")]
    #[test]
    fn deeply_nested_triple_term_in_nquads_span_returns_clean_error() {
        let mut line = b"<http://ex/s> <http://ex/p> ".to_vec();
        for _ in 0..10_000 {
            line.extend_from_slice(b"<<( <http://ex/a> <http://ex/b> ");
        }
        line.extend_from_slice(b"<http://ex/leaf>");
        for _ in 0..10_000 {
            line.extend_from_slice(b" )>>");
        }
        // N-Quads: explicit graph field after the object.
        line.extend_from_slice(b" <http://ex/g> .\n");
        let e = match parse_quads_chunk(&line) {
            Err(e) => e,
            Ok(_) => panic!("deeply-nested triple terms in N-Quads must be rejected, not overflow"),
        };
        assert!(
            e.contains("nested more than") && e.contains("levels deep"),
            "must be the depth-limit error, got: {e}"
        );
    }

    // [OPUS-4.8] (sq-cpm) The N-Quads 4th field is an OPTIONAL `graphLabel`, and the RDF 1.2
    // N-Quads grammar restricts it to `IRIREF | BLANK_NODE_LABEL` (prod. `graphLabel`) — a
    // literal or a triple term in graph position is a syntax error, not a fourth quad component.
    // `parse_quads_chunk` resolves the graph bucket via `graph_key` (IRI/blank only) and
    // `span_term` (object-only triple-term rule), so these cases must be rejected; an absent 4th
    // field routes to the default (`None`) graph.
    // `parse_quads_chunk` returns `Dict` (no `Debug`/`PartialEq`), so these helpers pull just the
    // graph-key column and the per-graph triple counts out of the bucket vec for assertions.
    #[cfg(feature = "parallel")]
    fn quad_graph_keys(line: &[u8]) -> Result<Vec<(Option<GraphKey>, usize)>, String> {
        parse_quads_chunk(line).map(|buckets| {
            buckets.into_iter().map(|(g, _dict, triples)| (g, triples.len())).collect()
        })
    }

    #[cfg(feature = "parallel")]
    #[test]
    fn nquads_graph_label_accepts_iri_and_blank_only() {
        // IRI graph label: routes the triple into a named-graph bucket keyed by that IRI.
        let iri_g = b"<http://ex/s> <http://ex/p> <http://ex/o> <http://ex/g> .\n";
        assert_eq!(
            quad_graph_keys(iri_g).expect("IRI graph label must parse"),
            vec![(Some(GraphKey::Iri("http://ex/g".to_owned())), 1)],
            "the triple lands in the IRI-keyed graph"
        );

        // Blank-node graph label.
        let blank_g = b"<http://ex/s> <http://ex/p> <http://ex/o> _:g0 .\n";
        assert_eq!(
            quad_graph_keys(blank_g).expect("blank-node graph label must parse"),
            vec![(Some(GraphKey::Blank("g0".to_owned())), 1)],
            "blank-node graph label key"
        );

        // No 4th field: the triple is in the default graph (`None`).
        let default_g = b"<http://ex/s> <http://ex/p> <http://ex/o> .\n";
        assert_eq!(
            quad_graph_keys(default_g).expect("triple with no graph label must parse"),
            vec![(None, 1)],
            "absent graph label is the default graph"
        );
    }

    #[cfg(feature = "parallel")]
    #[test]
    fn nquads_graph_label_rejects_literal() {
        // A literal in the 4th position is not a valid `graphLabel`.
        let lit_g = b"<http://ex/s> <http://ex/p> <http://ex/o> \"g\" .\n";
        let e = quad_graph_keys(lit_g).expect_err("a literal graph label must be rejected");
        assert!(
            e.contains("graph name must be an IRI or blank node"),
            "message must explain the IRI/blank-only rule, got: {e}"
        );

        // A typed literal likewise.
        let typed_g = b"<http://ex/s> <http://ex/p> <http://ex/o> \"5\"^^<http://www.w3.org/2001/XMLSchema#integer> .\n";
        assert!(
            quad_graph_keys(typed_g).is_err(),
            "a typed-literal graph label must be rejected"
        );
    }

    #[cfg(feature = "parallel")]
    #[test]
    fn nquads_graph_label_rejects_triple_term() {
        // A triple term `<<( … )>>` is OBJECT-ONLY; it is not a valid `graphLabel`. The span walk
        // (`span_term`) reaches the `<<(` opener in graph position and rejects it with the
        // position-aware message.
        let tt_g = b"<http://ex/s> <http://ex/p> <http://ex/o> <<( <http://ex/a> <http://ex/b> <http://ex/c> )>> .\n";
        let e = quad_graph_keys(tt_g).expect_err("a triple-term graph label must be rejected");
        assert!(
            e.contains("only valid in OBJECT position"),
            "message must explain the object-only rule, got: {e}"
        );
    }

    /// [OPUS-4.8] (sq-7d3dj.2) The `Dict` capacity is a PURE hint: parsing the same bytes into a
    /// `Dict::new()` vs a `Dict::with_capacity(BIG)` must yield byte-identical triples AND an
    /// identical term↔id bijection (same distinct-term count, same `term(id)` for every id) —
    /// the reserved capacity changes the allocation, never the parsed result or id-assignment
    /// order. (The out-`Vec` reservation is a fixed `bytes.len() / AVG_NT_LINE_BYTES` and is not
    /// varied here; its own purity — being derived solely from the input length — is argued at
    /// `AVG_NT_LINE_BYTES`.)
    #[test]
    fn parse_chunk_dict_capacity_is_pure_hint() {
        // Repeated predicate + type IRIs, distinct subjects/objects, a typed literal, a lang
        // literal, a blank node, and a nested triple term — a mix that grows both the term arena
        // and the lookup table across many interns.
        let mut nt = String::new();
        for i in 0..200 {
            nt.push_str(&format!("<http://ex/n{}> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://ex/Person> .\n", i));
            nt.push_str(&format!("<http://ex/n{}> <http://ex/name> \"name{}\"@en .\n", i, i));
            nt.push_str(&format!("<http://ex/n{}> <http://ex/age> \"{}\"^^<http://www.w3.org/2001/XMLSchema#integer> .\n", i, i % 37));
            nt.push_str(&format!("_:b{} <http://ex/reifies> <<( <http://ex/n{}> <http://ex/knows> <http://ex/n{}> )>> .\n", i, i, (i + 1) % 200));
        }
        let bytes = nt.as_bytes();

        let mut d_plain = Dict::new();
        let t_plain = parse_chunk(bytes, &mut d_plain).expect("plain parse");

        // A deliberately OVER-sized reservation — the hint must still not perturb the result.
        let mut d_big = Dict::with_capacity(10_000);
        let t_big = parse_chunk(bytes, &mut d_big).expect("presized parse");

        assert_eq!(t_plain, t_big, "id-triples (and their order) diverge under a capacity hint");
        assert_eq!(d_plain.len(), d_big.len(), "distinct-term count diverges under a capacity hint");
        for id in 1..=d_plain.len() as Id {
            assert_eq!(
                d_plain.term(id),
                d_big.term(id),
                "term for id {} diverges under a capacity hint (the term↔id bijection is not identical)",
                id
            );
        }
    }

    // [SONNET-4.6] (sq-7d3dj.31.3) Subject/predicate intern-cache tests.

    /// A grouped N-Triples corpus (many triples per subject) exercises the cache hit path.
    /// We verify: (a) the correct triples are parsed, (b) the subject cache produces the same
    /// id for each occurrence of the same subject, and (c) distinct subjects get distinct ids.
    #[test]
    fn subject_run_cache_hits_produce_correct_ids() {
        // Three subjects, each with three predicates — a genuine subject run.
        let nt = concat!(
            "<http://ex/s1> <http://ex/p1> <http://ex/o1> .\n",
            "<http://ex/s1> <http://ex/p2> <http://ex/o2> .\n",
            "<http://ex/s1> <http://ex/p3> <http://ex/o3> .\n",
            "<http://ex/s2> <http://ex/p1> <http://ex/o4> .\n",
            "<http://ex/s2> <http://ex/p2> <http://ex/o5> .\n",
            "<http://ex/s3> <http://ex/p1> <http://ex/o6> .\n",
        );
        let mut d = Dict::new();
        let triples = parse_chunk(nt.as_bytes(), &mut d).unwrap();
        assert_eq!(triples.len(), 6);
        // All three triples whose subject is s1 share the same subject id.
        let s1_id = triples[0][0];
        assert_eq!(triples[1][0], s1_id, "s1 second triple has wrong subject id");
        assert_eq!(triples[2][0], s1_id, "s1 third triple has wrong subject id");
        // s2 and s3 have distinct ids from s1 and each other.
        let s2_id = triples[3][0];
        let s3_id = triples[5][0];
        assert_ne!(s1_id, s2_id, "s1 and s2 must have distinct ids");
        assert_ne!(s1_id, s3_id, "s1 and s3 must have distinct ids");
        assert_ne!(s2_id, s3_id, "s2 and s3 must have distinct ids");
        // Predicate p1 is used for all three subjects — verify the predicate cache returns
        // the same id for both occurrences of p1.
        let p1_id_s1 = triples[0][1];
        let p1_id_s2 = triples[3][1];
        let p1_id_s3 = triples[5][1];
        assert_eq!(p1_id_s1, p1_id_s2, "predicate p1 must have the same id across subjects");
        assert_eq!(p1_id_s1, p1_id_s3, "predicate p1 must have the same id across subjects");
        // Verify via dict round-trip.
        use oxrdf::{NamedNode, Term};
        assert_eq!(d.term(s1_id), Term::NamedNode(NamedNode::new_unchecked("http://ex/s1")));
        assert_eq!(d.term(s2_id), Term::NamedNode(NamedNode::new_unchecked("http://ex/s2")));
        assert_eq!(d.term(s3_id), Term::NamedNode(NamedNode::new_unchecked("http://ex/s3")));
    }

    /// A blank-node subject run exercises the blank-node boundary check — `_:b0` must NOT
    /// collide with `_:b0extra` even though their first four bytes are identical.
    #[test]
    fn blank_node_subject_cache_no_false_hit_on_prefix_match() {
        let nt = concat!(
            "_:b0 <http://ex/p> <http://ex/o1> .\n",
            "_:b0extra <http://ex/p> <http://ex/o2> .\n",
            "_:b0 <http://ex/p> <http://ex/o3> .\n",
        );
        let mut d = Dict::new();
        let triples = parse_chunk(nt.as_bytes(), &mut d).unwrap();
        assert_eq!(triples.len(), 3);
        let b0_id = triples[0][0];
        let b0extra_id = triples[1][0];
        let b0_again_id = triples[2][0];
        // _:b0 and _:b0extra must be distinct.
        assert_ne!(b0_id, b0extra_id, "_:b0 and _:b0extra must have different ids");
        // The third line (_:b0) must re-use the first line's id (cache hit after eviction).
        assert_eq!(b0_id, b0_again_id, "_:b0 in third line must share id with first line");
        // Dict round-trip.
        assert_eq!(
            d.term(b0_id),
            oxrdf::Term::BlankNode(oxrdf::BlankNode::new_unchecked("b0"))
        );
        assert_eq!(
            d.term(b0extra_id),
            oxrdf::Term::BlankNode(oxrdf::BlankNode::new_unchecked("b0extra"))
        );
    }

    /// Shuffled input (no consecutive subject runs) must produce the SAME interned store as
    /// the grouped input — the cache only affects speed, never correctness.
    #[test]
    fn shuffled_input_store_matches_grouped() {
        // Build a small grouped corpus: 4 subjects × 3 predicates each.
        let grouped = concat!(
            "<http://ex/s1> <http://ex/p1> <http://ex/o11> .\n",
            "<http://ex/s1> <http://ex/p2> <http://ex/o12> .\n",
            "<http://ex/s1> <http://ex/p3> <http://ex/o13> .\n",
            "<http://ex/s2> <http://ex/p1> <http://ex/o21> .\n",
            "<http://ex/s2> <http://ex/p2> <http://ex/o22> .\n",
            "<http://ex/s2> <http://ex/p3> <http://ex/o23> .\n",
            "<http://ex/s3> <http://ex/p1> <http://ex/o31> .\n",
            "<http://ex/s3> <http://ex/p2> <http://ex/o32> .\n",
            "<http://ex/s3> <http://ex/p3> <http://ex/o33> .\n",
        );
        // Same triples, adversarially re-ordered (no two consecutive same-subject lines).
        let shuffled = concat!(
            "<http://ex/s1> <http://ex/p1> <http://ex/o11> .\n",
            "<http://ex/s2> <http://ex/p1> <http://ex/o21> .\n",
            "<http://ex/s3> <http://ex/p1> <http://ex/o31> .\n",
            "<http://ex/s1> <http://ex/p2> <http://ex/o12> .\n",
            "<http://ex/s2> <http://ex/p2> <http://ex/o22> .\n",
            "<http://ex/s3> <http://ex/p2> <http://ex/o32> .\n",
            "<http://ex/s1> <http://ex/p3> <http://ex/o13> .\n",
            "<http://ex/s2> <http://ex/p3> <http://ex/o23> .\n",
            "<http://ex/s3> <http://ex/p3> <http://ex/o33> .\n",
        );
        let mut d_grouped = Dict::new();
        let t_grouped = parse_chunk(grouped.as_bytes(), &mut d_grouped).unwrap();
        let mut d_shuffled = Dict::new();
        let t_shuffled = parse_chunk(shuffled.as_bytes(), &mut d_shuffled).unwrap();

        // Both must yield the same number of triples.
        assert_eq!(t_grouped.len(), t_shuffled.len(), "triple counts must match");

        // Both dicts must contain the same terms (same count, same term set).
        assert_eq!(d_grouped.len(), d_shuffled.len(), "distinct-term counts must match");

        // All nine triples must appear in both outputs (order may differ due to dict internment
        // order, so compare via term reconstruction).
        use oxrdf::{NamedNode, Term};
        let grouped_set: std::collections::HashSet<[Term; 3]> = t_grouped
            .iter()
            .map(|&[s, p, o]| [d_grouped.term(s), d_grouped.term(p), d_grouped.term(o)])
            .collect();
        let shuffled_set: std::collections::HashSet<[Term; 3]> = t_shuffled
            .iter()
            .map(|&[s, p, o]| [d_shuffled.term(s), d_shuffled.term(p), d_shuffled.term(o)])
            .collect();
        assert_eq!(
            grouped_set, shuffled_set,
            "grouped and shuffled parse must produce the same triple set"
        );
        // Verify one concrete triple is present in both.
        let triple_s1_p2_o12 = [
            Term::NamedNode(NamedNode::new_unchecked("http://ex/s1")),
            Term::NamedNode(NamedNode::new_unchecked("http://ex/p2")),
            Term::NamedNode(NamedNode::new_unchecked("http://ex/o12")),
        ];
        assert!(grouped_set.contains(&triple_s1_p2_o12), "grouped must contain s1/p2/o12");
        assert!(shuffled_set.contains(&triple_s1_p2_o12), "shuffled must contain s1/p2/o12");
    }

    #[test]
    // [SONNET-4.6] (sq-7d3dj.31.3) Verify that subjects/predicates longer than
    // SUBJ_PRED_CACHE_LEN bytes are parsed correctly even though they bypass the stack-buffer
    // cache (the else branch that sets cache_s_len=0 / cache_s_id=NO_ID).  Two consecutive
    // triples with the same oversized subject must receive the SAME interned id.
    fn oversized_subject_bypasses_cache_parses_correctly() {
        // Build a subject IRI of exactly SUBJ_PRED_CACHE_LEN+1 bytes (the `<…>` delimiters add 2,
        // but the raw bytes stored in the buffer include the delimiters, so the test subject IRI
        // must produce raw bytes > SUBJ_PRED_CACHE_LEN in the parser).
        // Each inner char is 'a', giving us SUBJ_PRED_CACHE_LEN+1 'a's inside "<http://…>".
        let inner: String = "a".repeat(SUBJ_PRED_CACHE_LEN + 1);
        let subj = format!("<http://ex/{}>", inner);
        // Two consecutive triples share the same oversized subject and predicate.
        let input = format!(
            "{} <http://ex/p> <http://ex/o1> .\n{} <http://ex/p> <http://ex/o2> .\n",
            subj, subj
        );
        let mut dict = Dict::new();
        let triples = parse_chunk(input.as_bytes(), &mut dict).expect("parse should succeed");
        assert_eq!(triples.len(), 2, "expected two triples");
        // Both triples must share the SAME subject id (intern de-duplication).
        assert_eq!(
            triples[0][0], triples[1][0],
            "oversized subject must be interned to the same id on each occurrence"
        );
        // Both triples must share the same predicate id.
        assert_eq!(
            triples[0][1], triples[1][1],
            "predicate id must match across both triples"
        );
        // Object ids must differ.
        assert_ne!(
            triples[0][2], triples[1][2],
            "distinct objects must get distinct ids"
        );
    }

    // ---- sq-7d3dj.21 — the `CHUNK-UTF8` unsafe invariant ---------------------------------
    // `str_of` converts a term span with `from_utf8_unchecked`, relying on (i) the entry point
    // having proved the WHOLE chunk is valid UTF-8 and (ii) every span end being an ASCII
    // delimiter, hence a character boundary. These tests pin BOTH halves. Under the nightly
    // `miri` lane they are also the UB canary: an invalid `&str` minted here is caught as UB,
    // and in any debug/test build `str_of`'s `debug_assert!` re-checks the precondition on
    // every single call — so every test in this module is really an invariant check too.

    /// Multi-byte UTF-8 pressed directly against EVERY span delimiter the parser cuts on
    /// (`<`/`>`, `"`/`"`, `_:`, `^^<>`), round-tripping byte-exactly. If any span end were
    /// mis-computed to land mid-codepoint, `str_of`'s `debug_assert!` fires here (and Miri
    /// reports UB in a release-shaped interpretation). [OPUS-5]
    #[test]
    fn multibyte_spans_at_every_delimiter_round_trip() {
        // `é` (U+00E9, 2 bytes), `→` (U+2192, 3 bytes), `𝔄` (U+1D504, 4 bytes) — one of each
        // UTF-8 length, each adjacent to a delimiter.
        let input = concat!(
            "<http://ex/é> <http://ex/→> \"𝔄\" .\n",
            "_:é <http://ex/p> \"é→𝔄\"@en .\n",
            "<http://ex/𝔄> <http://ex/p> \"→\"^^<http://ex/dt𝔄> .\n",
        );
        let mut dict = Dict::new();
        let triples = parse_chunk(input.as_bytes(), &mut dict).expect("multi-byte terms must parse");
        assert_eq!(triples.len(), 3);
        // Reconstruct every term and compare against the exact source text: a mis-cut span
        // would surface as a truncated / replacement-charred string, not just as UB.
        assert_eq!(dict.term(triples[0][0]), iri("http://ex/é"));
        assert_eq!(dict.term(triples[0][1]), iri("http://ex/→"));
        assert_eq!(dict.term(triples[0][2]), Term::Literal(Literal::new_simple_literal("𝔄")));
        assert_eq!(dict.term(triples[1][0]), Term::BlankNode(BlankNode::new_unchecked("é")));
        assert_eq!(
            dict.term(triples[1][2]),
            Term::Literal(Literal::new_language_tagged_literal_unchecked("é→𝔄", "en"))
        );
        assert_eq!(dict.term(triples[2][0]), iri("http://ex/𝔄"));
        assert_eq!(
            dict.term(triples[2][2]),
            Term::Literal(Literal::new_typed_literal("→", NamedNode::new_unchecked("http://ex/dt𝔄")))
        );
    }

    /// The escape slow path (`decode` with `esc == true`) over a span that ALSO carries raw
    /// multi-byte bytes: the `\` skip in `scan_delim` must not move the closer match off a
    /// character boundary. [OPUS-5]
    #[test]
    fn escaped_and_multibyte_literal_span_round_trips() {
        let input = "<http://ex/s> <http://ex/p> \"caf\\u00E9 → \\\"é\\\"\" .\n";
        let mut dict = Dict::new();
        let triples = parse_chunk(input.as_bytes(), &mut dict).expect("escaped multi-byte literal must parse");
        assert_eq!(
            dict.term(triples[0][2]),
            Term::Literal(Literal::new_simple_literal("café → \"é\""))
        );
    }

    /// A language tag terminated by a MULTI-BYTE byte is the one span end that is not an ASCII
    /// delimiter but the index OF a non-ASCII lead byte — still a character boundary. The parse
    /// must fail CLEANLY (the byte is not a valid statement terminator), never trip the
    /// `debug_assert!` and never mis-slice. [OPUS-5]
    #[test]
    fn language_tag_terminated_by_multibyte_errors_cleanly() {
        let input = "<http://ex/s> <http://ex/p> \"x\"@ené .\n";
        let mut dict = Dict::new();
        let err = parse_chunk(input.as_bytes(), &mut dict).expect_err("a non-terminator after @lang must error");
        assert!(err.contains("expected '.'"), "unexpected error: {}", err);
    }

    /// Entry-point validation, half (i): a chunk carrying invalid UTF-8 is REJECTED before any
    /// span is taken, with the offending byte offset. Covers a bad byte inside an IRI, inside a
    /// literal, and — the deliberately STRICTER case — inside a comment, i.e. outside any term
    /// span. N-Triples documents are defined to be UTF-8, so all three are conformant
    /// rejections and match what the serial `oxttl` loader does. [OPUS-5]
    #[test]
    fn parse_chunk_rejects_invalid_utf8() {
        // (label, prefix, the invalid byte, suffix) — the expected `valid_up_to` offset is
        // `prefix.len()`, derived rather than hand-counted so the cases cannot drift.
        let cases: [(&str, &[u8], u8, &[u8]); 3] = [
            // A byte that is never valid anywhere in UTF-8.
            ("inside an IRI", b"<http://ex/", 0xFF, b"> <http://ex/p> <http://ex/o> .\n"),
            // A lone continuation byte with no lead byte.
            ("inside a literal", b"<http://ex/s> <http://ex/p> \"a", 0x80, b"b\" .\n"),
            // A truncated 2-byte sequence, OUTSIDE any term span — the deliberately stricter case.
            ("inside a comment (outside every term span)", b"# note ", 0xC3, b"\n<http://ex/s> <http://ex/p> <http://ex/o> .\n"),
        ];
        for (label, prefix, bad, suffix) in cases {
            let at = prefix.len();
            let mut bytes = prefix.to_vec();
            bytes.push(bad);
            bytes.extend_from_slice(suffix);
            let mut dict = Dict::new();
            let err = parse_chunk(&bytes, &mut dict).expect_err(label);
            assert_eq!(
                err,
                format!("N-Triples: invalid UTF-8 at byte {}", at),
                "wrong rejection for the case: {}",
                label
            );
            // Nothing may have been interned: validation runs before any byte is scanned.
            assert!(dict.is_empty(), "chunk rejected for {} must not have interned anything", label);
        }
    }

    /// The N-Quads entry point carries the SAME validation, labelled for its own grammar. [OPUS-5]
    #[cfg(feature = "parallel")]
    #[test]
    fn parse_quads_chunk_rejects_invalid_utf8() {
        let mut bytes = b"<http://ex/s> <http://ex/p> <http://ex/o> <http://ex/".to_vec();
        let at = bytes.len();
        bytes.push(0xFE);
        bytes.extend_from_slice(b"g> .\n");
        // `Dict` is not `Debug`, so unwrap the `Err` by hand rather than via `expect_err`.
        let err = match parse_quads_chunk(&bytes) {
            Err(e) => e,
            Ok(_) => panic!("invalid UTF-8 must be rejected"),
        };
        assert_eq!(err, format!("N-Quads: invalid UTF-8 at byte {}", at));
    }

    /// The N-Quads path takes spans through its own `graph_key` / re-scan route; multi-byte
    /// graph names and terms must survive it byte-exactly. [OPUS-5]
    #[cfg(feature = "parallel")]
    #[test]
    fn parse_quads_chunk_multibyte_graph_and_terms_round_trip() {
        // (`→` U+2192 is fine in a literal but is not `PN_CHARS`, so the label uses `ā`.)
        let input = "<http://ex/s> <http://ex/p> \"é\" <http://ex/g𝔄> .\n_:é <http://ex/p> \"→\" _:gā .\n";
        let buckets = parse_quads_chunk(input.as_bytes()).expect("multi-byte quads must parse");
        let keys: Vec<Option<GraphKey>> = buckets.iter().map(|(g, _, _)| g.clone()).collect();
        assert_eq!(
            keys,
            vec![
                Some(GraphKey::Iri("http://ex/g𝔄".to_string())),
                Some(GraphKey::Blank("gā".to_string())),
            ]
        );
        assert_eq!(buckets[0].1.term(buckets[0].2[0][2]), Term::Literal(Literal::new_simple_literal("é")));
        assert_eq!(buckets[1].1.term(buckets[1].2[0][0]), Term::BlankNode(BlankNode::new_unchecked("é")));
    }
}
