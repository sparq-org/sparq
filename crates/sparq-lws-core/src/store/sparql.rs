// AUTHORED-BY Claude Opus 4.8
//! SPARQL injection-safe term builders + the queries the [`SparqClient`](super::SparqClient) data
//! path issues.
//!
//! This is the SPARQL analogue of the suite's RDF-injection guard: an untrusted IRI or literal is
//! **never** string-concatenated into a query — every term goes through [`iri`] / [`literal`], which
//! validate + escape per the SPARQL 1.1 grammar so a crafted resource IRI (or content-type/blob-key)
//! cannot break out of its term position and inject clauses. The builders are pure + total: a value
//! that cannot be safely represented is escaped to a form that still parses as the same term, never
//! as syntax.
//!
//! The data model (the maintainer's "tight SPARQ integration" direction, per the
//! `solid-server-rs-wac.md` design): every resource is a **named graph whose graph IRI equals the
//! resource IRI**; the resource's authoritative index record (content type + blob-key byte-pointer +
//! ETag) and its containment edges live as triples *in that graph* under the reserved `pss:` vocab.
//! Existence/metadata/listing are SELECT/ASK over those graphs; the resource RDF itself is the rest
//! of the graph (read via CONSTRUCT). See the module docs in `super::http` (behind the opt-in
//! `http-sparq` feature) for the live-endpoint
//! deviation note (today's `sparq-server` HTTP store folds named graphs into one default graph, so
//! the isolation the model assumes is enforced at the engine layer but not yet over HTTP — FR-4).

/// The reserved metadata vocabulary namespace for the index records this server writes (mirrors the
/// production server's `pss:` index predicates). Kept as one constant so the predicate IRIs below
/// cannot drift apart.
pub const PSS_NS: &str = "urn:pss:index#";

/// `ldp:contains`, the containment predicate.
pub const LDP_CONTAINS: &str = "http://www.w3.org/ns/ldp#contains";

/// Predicate: a resource's stored RDF content type (a literal).
pub fn p_content_type() -> String {
    format!("{PSS_NS}contentType")
}
/// Predicate: a resource's opaque blob-store byte-pointer (a literal).
pub fn p_blob_key() -> String {
    format!("{PSS_NS}blobKey")
}
/// Predicate: a resource's opaque entity tag (a literal).
pub fn p_etag() -> String {
    format!("{PSS_NS}etag")
}
/// Predicate: a resource's server-recorded modification time (an `xsd:dateTime` literal). Read back
/// into [`ResourceMeta::last_modified`](super::sparq::ResourceMeta::last_modified) to make
/// `If-Modified-Since` live (jx3c). It is a RESERVED `urn:pss:index#` predicate, so the existing
/// reserved-term guard (`is_reserved_term`) already forbids untrusted body RDF from writing it.
pub fn p_modified() -> String {
    format!("{PSS_NS}modified")
}
/// The `xsd:dateTime` datatype IRI — the datatype of the `pss:modified` literal.
pub const XSD_DATETIME: &str = "http://www.w3.org/2001/XMLSchema#dateTime";
/// Predicate: a per-operation create marker (a unique-nonce literal), written atomically with a
/// guarded `create_child` so the success confirm is race-resistant (no other op touches it).
pub fn p_create_marker() -> String {
    format!("{PSS_NS}createMarker")
}
/// Predicate: a per-operation completion marker (a unique-nonce literal), written atomically
/// with a guarded index write into a separate scratch graph. Each operation confirms its own nonce.
pub fn p_delete_marker() -> String {
    format!("{PSS_NS}deleteMarker")
}
/// The reserved scratch graph that per-operation delete markers are written into. It is a DISTINCT
/// graph from any resource's own graph, so dropping a resource's graph never touches a marker, and a
/// marker can confirm a delete whose own graph is gone. (Markers are operation-scoped + pruned by the
/// reconciler — M3-next.)
pub fn g_delete_markers() -> String {
    format!("{PSS_NS}deleteMarkers")
}
/// The reserved graph that write fences are written into ([`update_fence`]): like the delete
/// markers' graph, never a resource's own graph, so no resource's create or delete touches it.
pub fn g_fences() -> String {
    format!("{PSS_NS}fences")
}
/// Predicate: a write fence (a unique-nonce literal, the nonce of the write it fences). See
/// [`update_fence`].
pub fn p_fenced() -> String {
    format!("{PSS_NS}fenced")
}
/// The subject every index record hangs off, *within* a resource's own named graph: a stable,
/// reserved IRI so the record triples never collide with the resource's own RDF.
pub fn s_record() -> String {
    format!("{PSS_NS}record")
}

/// An error rendering an untrusted value into a SPARQL term — surfaced as a fatal backend error.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum BuildError {
    /// The value is not a valid SPARQL `IRIREF` (it contains an IRIREF-forbidden character).
    #[error("invalid IRI for a SPARQL term: contains a forbidden character")]
    InvalidIri,
    /// A language tag is empty or contains a character outside the BCP-47-subset
    /// (`[A-Za-z0-9-]`, non-empty) — rejected fail-closed rather than silently rewritten.
    #[error("invalid language tag for a SPARQL literal")]
    InvalidLangTag,
    /// A body triple touches a RESERVED metadata term (the `urn:pss:index#record` subject or a
    /// `urn:pss:index#` predicate) — rejected so untrusted RDF can never corrupt/spoof the index
    /// records that share the resource's named graph.
    #[error("body triple collides with a reserved metadata term")]
    ReservedTermCollision,
}

/// Render a string as a **SPARQL IRI reference** (`<...>`), REJECTING any IRIREF-forbidden character
/// rather than escaping it.
///
/// Per the SPARQL 1.1 `IRIREF` production an IRI ref may not contain `<`, `>`, `"`, `{`, `}`, `|`,
/// `^`, `` ` ``, `\`, or any char ≤ U+0020. A well-formed resource IRI (the server's own
/// target-parsed absolute IRI) never contains these; a value that DOES is rejected with
/// [`BuildError::InvalidIri`] — NOT percent-escaped. Rejection (not escaping) is what keeps the
/// mapping **injective**: percent-escaping a raw `>` to `%3E` would alias it onto a distinct IRI that
/// already literally contained `%3E` (two different resources → the same graph IRI — the round-4
/// finding). Because forbidden chars are refused, the surviving output is the input wrapped verbatim
/// in `<...>`: a single well-formed IRI term that cannot terminate early or inject. Fail-closed.
pub fn iri(value: &str) -> Result<String, BuildError> {
    for ch in value.chars() {
        let forbidden = matches!(ch, '<' | '>' | '"' | '{' | '}' | '|' | '^' | '`' | '\\')
            || (ch as u32) <= 0x20;
        if forbidden {
            return Err(BuildError::InvalidIri);
        }
    }
    let mut out = String::with_capacity(value.len() + 2);
    out.push('<');
    out.push_str(value);
    out.push('>');
    Ok(out)
}

/// Render a string as a **safely-escaped SPARQL string literal** (`"..."`), with the optional
/// datatype/lang left to the caller (these data-path literals are plain `xsd:string`).
///
/// Escapes per the SPARQL `STRING_LITERAL_QUOTE` production: `"`, `\`, and the line terminators
/// `\n`/`\r` (and `\t` for cleanliness). A literal value containing `" . } DROP …` is therefore
/// rendered as inert escaped characters inside one string term — it can never close the literal and
/// inject syntax. The output is ALWAYS a single well-formed `"..."` token.
pub fn literal(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for ch in value.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Render an `xsd:dateTime` lexical form as a datatyped SPARQL literal
/// (`"<lexical>"^^<http://www.w3.org/2001/XMLSchema#dateTime>`).
///
/// The lexical value is server-generated (from [`super::timestamp::to_xsd_datetime`], not untrusted
/// input), but it STILL flows through [`literal`] — so even a hypothetically malformed value can
/// only ever be an inert escaped string term, never injected syntax. The datatype IRI is a
/// compile-time constant wrapped via `iri_const`.
pub fn datetime_literal(lexical: &str) -> String {
    format!(
        "{lit}^^{dt}",
        lit = literal(lexical),
        dt = iri_const(XSD_DATETIME)
    )
}

/// Map a 4-bit nibble to an uppercase hex digit (for the injective bnode-label byte escapes).
fn nibble_hex(n: u8) -> char {
    match n {
        0..=9 => (b'0' + n) as char,
        _ => (b'A' + (n - 10)) as char,
    }
}

// ---------------------------------------------------------------------------
// Query/Update builders — every untrusted value flows through the fallible `iri` / `literal`.
//
// All builders return `Result<String, BuildError>`: an untrusted IRI that is not a valid IRIREF is
// REJECTED (fail-closed), never escaped-and-aliased. The reserved-vocab constants (`s_record()`,
// the `p_*()` predicates, `LDP_CONTAINS`) are known-valid by construction, so they go through
// `iri_const` — an infallible wrapper that ASSERTS validity (a panic there is a build-time bug, not a
// runtime/untrusted-input path).
// ---------------------------------------------------------------------------

/// Wrap a KNOWN-VALID reserved-vocab IRI (a compile-time-fixed constant or a `pss:`/`ldp:` IRI this
/// crate mints) into a `<...>` term. These never contain IRIREF-forbidden chars, so an error here is
/// a programming bug — surfaced via `expect`, never reachable from untrusted input.
fn iri_const(value: &str) -> String {
    iri(value).expect("reserved-vocab IRI must be a valid IRIREF (build-time invariant)")
}

/// The graph, and its subject, where the blob keys whose bytes are being reclaimed are named
/// ([`update_begin_reclaim`]): outside every storage, so no request can name it.
pub const RECLAIMING: &str = "urn:x-sparq-lws:reclaiming";

/// Predicate: a blob key whose bytes are being reclaimed (a literal, on [`RECLAIMING`]).
pub fn p_reclaiming() -> String {
    format!("{PSS_NS}reclaiming")
}

/// UPDATE: name `blob_key` as being reclaimed, only while no record in any graph names it as its
/// bytes (`pss:blobKey`, a hold's record included): one atomic `INSERT … WHERE`, so no record
/// that names it can commit in between (the caller reads [`ask_reclaiming`] back to tell
/// whether it did).
pub fn update_begin_reclaim(blob_key: &str) -> String {
    format!(
        "INSERT {{ GRAPH {r} {{ {r} {pr} {k} }} }} \
         WHERE {{ FILTER NOT EXISTS {{ GRAPH ?g {{ ?s {pbk} {k} }} }} }}",
        r = iri_const(RECLAIMING),
        pr = iri_const(&p_reclaiming()),
        pbk = iri_const(&p_blob_key()),
        k = literal(blob_key),
    )
}

/// ASK whether any record in any graph names `blob_key` as its bytes.
pub fn ask_blob_referenced(blob_key: &str) -> String {
    format!(
        "ASK {{ GRAPH ?g {{ ?s {pbk} {k} }} }}",
        pbk = iri_const(&p_blob_key()),
        k = literal(blob_key),
    )
}

/// ASK whether `blob_key` is named as being reclaimed.
pub fn ask_reclaiming(blob_key: &str) -> String {
    format!(
        "ASK {{ GRAPH {r} {{ {r} {pr} {k} }} }}",
        r = iri_const(RECLAIMING),
        pr = iri_const(&p_reclaiming()),
        k = literal(blob_key),
    )
}

/// Confirm a reclaim and observe its append directory in the same query. No row
/// means the claim was refused; an unbound `?hasAppends` means no directory exists.
pub fn select_reclaim_with_appends(blob_key: &str) -> String {
    format!("SELECT ?hasAppends WHERE {{ GRAPH {r} {{ {r} {pr} {key} }} OPTIONAL {{ GRAPH {dir} {{ {s} {pct} ?ct }} BIND(true AS ?hasAppends) }} }}",
        r = iri_const(RECLAIMING), pr = iri_const(&p_reclaiming()), key = literal(blob_key),
        dir = iri_const(&super::append_dir(blob_key)), s = iri_const(&s_record()), pct = iri_const(&p_content_type()))
}

/// UPDATE: stop naming `blob_key` as being reclaimed (its bytes are gone).
pub fn update_end_reclaim(blob_key: &str) -> String {
    format!(
        "DELETE DATA {{ GRAPH {r} {{ {r} {pr} {k} }} }}",
        r = iri_const(RECLAIMING),
        pr = iri_const(&p_reclaiming()),
        k = literal(blob_key),
    )
}

/// SELECT every blob key named as being reclaimed (`?k`).
pub fn select_reclaiming() -> String {
    format!(
        "SELECT ?k WHERE {{ GRAPH {r} {{ {r} {pr} ?k }} }}",
        r = iri_const(RECLAIMING),
        pr = iri_const(&p_reclaiming()),
    )
}

/// ASK whether a resource is indexed: its graph holds the reserved index record.
pub fn ask_exists(resource: &str) -> Result<String, BuildError> {
    Ok(format!(
        "ASK {{ GRAPH {g} {{ {s} {p} ?ct }} }}",
        g = iri(resource)?,
        s = iri_const(&s_record()),
        p = iri_const(&p_content_type()),
    ))
}

/// ASK whether a specific `container ldp:contains child` edge exists — the *containment-edge*
/// confirmation [`update_create_child`] needs.
///
/// This is the correct post-create check (NOT `ask_exists(child)`): the atomic guarded insert only
/// adds the edge when the container existed, so the presence of the EDGE — not merely the child's
/// own existence — is what distinguishes "guard matched, child created here" from "guard rejected
/// (container missing) while a same-named child happened to already exist". Fail-closed.
pub fn ask_contains_edge(container: &str, child: &str) -> Result<String, BuildError> {
    Ok(format!(
        "ASK {{ GRAPH {g} {{ {s} {p} {c} }} }}",
        g = iri(container)?,
        s = iri_const(&s_record()),
        p = iri_const(LDP_CONTAINS),
        c = iri(child)?,
    ))
}

/// ASK whether a unique per-operation marker triple is present in a child's graph — the
/// *race-resistant* create-confirmation [`update_create_child`] uses.
///
/// The marker is written ATOMICALLY with the guarded create (only when the container existed), under
/// a reserved predicate with a caller-supplied unique nonce object. Unlike the containment edge, NO
/// other operation (`remove_child`, a sibling create) ever writes or removes THIS marker, so a
/// concurrent containment mutation between the create and the confirm cannot flip the result — the
/// marker's presence is an operation-scoped success signal. Fail-closed.
pub fn ask_create_marker(child: &str, nonce: &str) -> Result<String, BuildError> {
    Ok(format!(
        "ASK {{ GRAPH {g} {{ {s} {p} {n} }} }}",
        g = iri(child)?,
        s = iri_const(&s_record()),
        p = iri_const(&p_create_marker()),
        n = literal(nonce),
    ))
}

/// SELECT a resource's index-record metadata (content type, blob-key, ETag, + the OPTIONAL modified
/// time) from its graph.
///
/// `pss:modified` is bound via `OPTIONAL` — a record written before the modification time was tracked
/// (or by a path that records none) still yields its `ct`/`bk`/`etag` row, with `?mod` simply
/// unbound. The read side then treats an absent `?mod` as "no modification time" ⇒ a fresh 200 on
/// `If-Modified-Since`, never a spurious 304.
pub fn select_meta(resource: &str) -> Result<String, BuildError> {
    Ok(format!(
        "SELECT ?ct ?bk ?etag ?mod ?appendSeq WHERE {{ GRAPH {g} {{ \
            {s} {pct} ?ct ; {pbk} ?bk ; {pet} ?etag . \
            OPTIONAL {{ {s} {pmod} ?mod }} OPTIONAL {{ {s} {pseq} ?appendSeq }} }} }} LIMIT 1",
        g = iri(resource)?,
        s = iri_const(&s_record()),
        pct = iri_const(&p_content_type()),
        pbk = iri_const(&p_blob_key()),
        pet = iri_const(&p_etag()),
        pmod = iri_const(&p_modified()),
        pseq = iri_const(&p_append_seq()),
    ))
}

// Comparative measurements favor a conservative boundary beyond ordinary pod walks.
// Larger walks retain VALUES rather than repeating graph-name lookups for every candidate.
pub(super) const READ_PLAN_CONSTANT_CANDIDATES: usize = 64;

/// SELECT the target and ACL candidates in one query. Short walks use balanced
/// constant GRAPH branches; longer walks retain the VALUES plan.
/// Absent records yield no row in either form.
///
/// Every name passes through the fallible [`iri`] builder before dispatch. Duplicate
/// names are evaluated once; the reader restores the caller's candidate order and roles.
pub fn select_read_plan(target: &str, acl_candidates: &[String]) -> Result<String, BuildError> {
    let names = read_plan_names(target, acl_candidates);
    if acl_candidates.len() > READ_PLAN_CONSTANT_CANDIDATES {
        let mut values = String::new();
        for name in names {
            let name = name?;
            if !values.is_empty() {
                values.push(' ');
            }
            values.push_str(&name);
        }
        return Ok(format!(
            "SELECT ?g ?ct ?bk ?etag ?mod ?appendSeq WHERE {{ VALUES ?g {{ {values} }} {record} }}",
            record = read_plan_record("?g"),
        ));
    }
    let names = names.collect::<Result<Vec<_>, _>>()?;
    let mut query = String::from("SELECT ?g ?ct ?bk ?etag ?mod ?appendSeq WHERE { ");
    read_plan_union(&names, &mut query);
    query.push_str(" }");
    Ok(query)
}

fn read_plan_names<'a>(
    target: &'a str,
    candidates: &'a [String],
) -> impl Iterator<Item = Result<String, BuildError>> + 'a {
    let mut seen = Vec::with_capacity(1 + candidates.len());
    std::iter::once(target)
        .chain(candidates.iter().map(String::as_str))
        .filter_map(move |raw| {
            if seen.contains(&raw) {
                None
            } else {
                seen.push(raw);
                Some(iri(raw))
            }
        })
}

fn read_plan_union(names: &[String], output: &mut String) {
    if let [graph] = names {
        output.push_str("{ BIND(");
        output.push_str(graph);
        output.push_str(" AS ?g) ");
        output.push_str(&read_plan_record(graph));
        output.push_str(" }");
        return;
    }
    let (left, right) = names.split_at(names.len() / 2);
    output.push_str("{ ");
    read_plan_union(left, output);
    output.push_str(" UNION ");
    read_plan_union(right, output);
    output.push_str(" }");
}

// Keep all rows of a multi-valued record: the reader validates every row before
// choosing the first per graph. Do not add LIMIT to individual graph branches.
fn read_plan_record(graph: &str) -> String {
    format!(
        "GRAPH {graph} {{ {s} {pct} ?ct ; {pbk} ?bk ; {pet} ?etag . \
            OPTIONAL {{ {s} {pmod} ?mod }} OPTIONAL {{ {s} {pseq} ?appendSeq }} }}",
        s = iri_const(&s_record()),
        pct = iri_const(&p_content_type()),
        pbk = iri_const(&p_blob_key()),
        pet = iri_const(&p_etag()),
        pmod = iri_const(&p_modified()),
        pseq = iri_const(&p_append_seq()),
    )
}

/// CONSTRUCT the resource's own RDF (everything in its graph EXCEPT the reserved index record).
pub fn construct_resource(resource: &str) -> Result<String, BuildError> {
    Ok(format!(
        "CONSTRUCT {{ ?s ?p ?o }} WHERE {{ GRAPH {g} {{ ?s ?p ?o . \
            FILTER (?s != {rec}) }} }}",
        g = iri(resource)?,
        rec = iri_const(&s_record()),
    ))
}

/// SELECT the direct children of a container (its `ldp:contains` members), held in the container's
/// own graph against the reserved record subject.
pub fn select_children(container: &str) -> Result<String, BuildError> {
    Ok(format!(
        "SELECT ?child WHERE {{ GRAPH {g} {{ {s} {p} ?child }} }}",
        g = iri(container)?,
        s = iri_const(&s_record()),
        p = iri_const(LDP_CONTAINS),
    ))
}

/// [`select_children`] cut to at most `limit` rows (ask for one more than will be kept, so a cut
/// shows itself).
pub fn select_children_limited(container: &str, limit: usize) -> Result<String, BuildError> {
    Ok(format!("{} LIMIT {limit}", select_children(container)?))
}

/// SELECT every blob-key the index references, across ALL resource graphs — the *referenced set* the
/// reconciler diffs against the physically-stored blobs to find orphans.
///
/// This is the efficient shape for the orphan sweep: ONE query returns the WHOLE referenced set
/// (`SELECT DISTINCT ?bk` over the reserved `pss:blobKey` predicate against the reserved record
/// subject, in any graph), so the reconciler asks SPARQ *once* and then does O(1) set-membership per
/// blob — rather than N per-key `ASK`s (one network round-trip per stored blob), which would make a GC
/// over a large pod O(N) backend calls. `DISTINCT` because two index records could (pathologically)
/// name the same blob; we only care whether a key is referenced AT ALL.
///
/// It takes NO untrusted input — the predicate + subject are reserved-vocab constants — so it cannot
/// be an injection vector and is infallible-by-construction (returned as `String`, not a `Result`).
pub fn select_referenced_blob_keys() -> String {
    format!(
        "SELECT DISTINCT ?bk WHERE {{ GRAPH ?g {{ {s} {p} ?bk }} FILTER(?bk != \"\") }}",
        s = iri_const(&s_record()),
        p = iri_const(&p_blob_key()),
    )
}

/// The append sequence attached to a document's current byte pointer.
pub fn p_append_seq() -> String {
    format!("{PSS_NS}appendSeq")
}

/// Read an append's own result; another operation cannot overwrite its receipt.
pub fn select_append_receipt(receipt: &str) -> Result<String, BuildError> {
    Ok(format!(
        "SELECT ?bk ?seq WHERE {{ GRAPH {} {{ {} <urn:pss:index#appendBlobKey> ?bk ; {} ?seq }} }}",
        iri(receipt)?,
        iri_const(&s_record()),
        iri_const(&p_append_seq())
    ))
}

/// Switch an exactly matching append snapshot to merged bytes without changing
/// its representation validators. The receipt is in the same atomic update.
#[cfg(feature = "http-sparq")]
pub(crate) fn update_fold_appends(
    resource: &str,
    expected: &super::sparq::ResourceMeta,
    blob_key: &str,
    receipt: &str,
    nonce: &str,
    epoch: u64,
) -> Result<String, BuildError> {
    let modified = match expected
        .last_modified
        .and_then(super::timestamp::to_xsd_datetime)
    {
        Some(value) => format!("FILTER (?modified = {})", datetime_literal(&value)),
        None => "FILTER (!BOUND(?modified))".into(),
    };
    Ok(format!(
        "DELETE {{ GRAPH {g} {{ {s} {pbk} ?bk ; {pseq} ?seq }} }} \
         INSERT {{ GRAPH {g} {{ {s} {pbk} {new_key} ; {pseq} 0 }} \
                   GRAPH {receipt} {{ {s} <urn:pss:index#appendBlobKey> {new_key} ; {pseq} 0 }} }} \
         WHERE {{ GRAPH {g} {{ {s} {pbk} ?bk ; {pseq} ?seq ; {pet} {tag} ; {pct} {ct} . \
                   OPTIONAL {{ {s} {pmod} ?modified }} }} \
                   FILTER (?bk = {old_key} && ?seq = {seq}) {modified} {fence} }}",
        g = iri(resource)?,
        s = iri_const(&s_record()),
        pbk = iri_const(&p_blob_key()),
        pseq = iri_const(&p_append_seq()),
        pet = iri_const(&p_etag()),
        pct = iri_const(&p_content_type()),
        pmod = iri_const(&p_modified()),
        new_key = literal(blob_key),
        old_key = literal(&expected.blob_key),
        tag = literal(&expected.etag),
        ct = literal(&expected.content_type),
        seq = expected.append_seq,
        receipt = iri(receipt)?,
        fence = fence_guard(Some(epoch), nonce),
    ))
}

/// Atomically publish an insert and advance its target only while the original
/// byte pointer is current. Allocate the visible sequence at publication time.
#[cfg(any(feature = "http-sparq", feature = "embedded-sparq"))]
pub(crate) fn update_publish_append(
    resource: &str,
    key: &str,
    blob_key: &str,
    nonce: &str,
    modified: &str,
    epoch: Option<u64>,
) -> Result<String, BuildError> {
    let dir = super::append_dir(key);
    let receipt = format!("urn:x-sparq-lws:append-results/{nonce}");
    let type_essence = format!(
        "LCASE(REPLACE(STRBEFORE(CONCAT(?ct, \";\"), \";\"), {}, \"\"))",
        literal(r"^\s+|\s+$")
    );
    Ok(format!(
        "DELETE {{ GRAPH {g} {{ {s} {pseq} ?oldSeq ; {pet} ?oldTag ; {pmod} ?oldMod }} \
                   GRAPH {dir} {{ {s} {pseq} ?dirSeq }} }} \
         INSERT {{ GRAPH {g} {{ {s} {pseq} ?next ; {pet} {tag} ; {pmod} {modified} }} \
                   GRAPH {dir} {{ {s} {pct} \"application/n-triples\" ; {pbk} \"\" ; {pet} \"\" ; {pseq} ?next ; {contains} ?record }} \
                   GRAPH ?record {{ {s} {pct} \"application/n-triples\" ; {pbk} {blob_key} ; {pet} \"\" ; {pseq} ?parent ; {pmod} {modified} }} \
                   GRAPH {receipt} {{ {s} <urn:pss:index#appendBlobKey> {blob_key} ; {pseq} ?next }} }} \
         WHERE {{ GRAPH {g} {{ {s} {pct} ?ct ; {pbk} {key} ; {pet} ?oldTag . \
                   OPTIONAL {{ {s} {pseq} ?oldSeq }} OPTIONAL {{ {s} {pmod} ?oldMod }} }} \
                   FILTER ({type_essence} IN (\"text/turtle\", \"application/n-triples\", \"application/n-quads\")) \
                   OPTIONAL {{ GRAPH {dir} {{ {s} {pseq} ?dirSeq }} }} \
                   BIND(COALESCE(?oldSeq, 0) AS ?parent) \
                   BIND(IF(COALESCE(?dirSeq, 0) > ?parent, ?dirSeq, ?parent) + 1 AS ?next) FILTER (?next <= 18446744073709551615) \
                   BIND(IRI(CONCAT({dir_text}, SUBSTR(CONCAT(\"00000000000000000000\", STR(?next)), STRLEN(STR(?next)) + 1), \"-\", {nonce_text})) AS ?record) {fence} }}",
        g = iri(resource)?, s = iri_const(&s_record()), pseq = iri_const(&p_append_seq()),
        pet = iri_const(&p_etag()), pmod = iri_const(&p_modified()),
        pct = iri_const(&p_content_type()), pbk = iri_const(&p_blob_key()),
        dir = iri(&dir)?, dir_text = literal(&dir), contains = iri_const(LDP_CONTAINS),
        receipt = iri(&receipt)?, key = literal(key), blob_key = literal(blob_key),
        tag = literal(&format!("\"append-{nonce}\"")), modified = datetime_literal(modified),
        nonce_text = literal(nonce), fence = fence_guard(epoch, nonce),
    ))
}

/// Create or replace a legacy metadata row with no stored append sequence.
pub fn update_put_meta(
    resource: &str,
    content_type: &str,
    blob_key: &str,
    etag: &str,
    modified: Option<&str>,
) -> Result<String, BuildError> {
    update_put_meta_with_append_seq(resource, content_type, blob_key, etag, modified, 0)
}

/// UPDATE: create-or-replace ONLY a resource's reserved index-record triples (content type, blob-key,
/// ETag, and — when a value is given — modification time) — leaving everything else in the graph
/// (containment `ldp:contains` edges, the resource's own RDF) untouched.
///
/// This must NOT `DROP` the whole graph: a container's graph also holds its `ldp:contains` edges, and
/// a resource's graph holds its user RDF, so a blanket drop would silently erase children / body
/// data on every metadata re-write (the bug roborev flagged). Instead it `DELETE`s only the reserved
/// record predicates hanging off the reserved record subject (each via an OPTIONAL-free
/// `DELETE WHERE`, idempotent on an absent record), then `INSERT DATA`s the new record. The
/// `DELETE WHERE`s + the `INSERT DATA` are submitted as ONE SPARQL update (`;`-separated), so they
/// commit atomically as a single generation on the server.
///
/// `pss:modified` is treated like the other single-valued predicates: its stale value is ALWAYS
/// cleared (so writing `last_modified: None` faithfully leaves NO modification time — a later read
/// returns `None`, never a stale `Some(...)` that could drive a wrong 304); the fresh value is
/// INSERTed only when `modified` is `Some`.
pub(crate) fn update_put_meta_with_append_seq(
    resource: &str,
    content_type: &str,
    blob_key: &str,
    etag: &str,
    modified: Option<&str>,
    append_seq: u64,
) -> Result<String, BuildError> {
    let g = iri(resource)?;
    let s = iri_const(&s_record());
    let pct = iri_const(&p_content_type());
    let pbk = iri_const(&p_blob_key());
    let pet = iri_const(&p_etag());
    let pmod = iri_const(&p_modified());
    let pseq = iri_const(&p_append_seq());
    // `pss:modified` is single-valued exactly like the other record predicates, so its stale value is
    // ALWAYS DELETE-cleared — even when `modified` is `None`. This makes the written state faithful to
    // the `ResourceMeta`: writing `last_modified: None` genuinely clears any prior timestamp, so a
    // later read returns `None` (never a stale `Some(...)` that could drive a wrong 304). Only the
    // INSERT of a fresh value is conditional on `Some`.
    let mod_insert = match modified {
        Some(m) => format!(" ; {pmod} {dt}", dt = datetime_literal(m)),
        None => String::new(),
    };
    Ok(format!(
        "DELETE WHERE {{ GRAPH {g} {{ {s} {pseq} ?oldSeq }} }} ; \
         DELETE WHERE {{ GRAPH {g} {{ {s} {pmod} ?oldMod }} }} ; \
         DELETE WHERE {{ GRAPH {g} {{ {s} {pct} ?oldCt }} }} ; \
         DELETE WHERE {{ GRAPH {g} {{ {s} {pbk} ?oldBk }} }} ; \
         DELETE WHERE {{ GRAPH {g} {{ {s} {pet} ?oldEt }} }} ; \
         INSERT DATA {{ GRAPH {g} {{ {s} {pct} {ct} ; {pbk} {bk} ; {pet} {et} ; {pseq} {append_seq}{mod_insert} . }} }}",
        g = g,
        s = s,
        pmod = pmod,
        pct = pct,
        ct = literal(content_type),
        pbk = pbk,
        bk = literal(blob_key),
        pet = pet,
        et = literal(etag),
        mod_insert = mod_insert,
    ))
}

/// Upsert metadata in one modify, guarded by a nonce fence and the writer epoch.
/// Only reserved record predicates are replaced; body triples are preserved.
/// The completion marker survives later changes to the resource graph.
pub fn update_put_meta_fenced(
    resource: &str,
    content_type: &str,
    blob_key: &str,
    etag: &str,
    modified: Option<&str>,
    nonce: &str,
    epoch: u64,
) -> Result<String, BuildError> {
    update_put_meta_snapshot_fenced(
        resource,
        content_type,
        blob_key,
        etag,
        modified,
        nonce,
        epoch,
        0,
    )
}

/// Fenced metadata upsert preserving a complete append snapshot.
#[allow(clippy::too_many_arguments)]
pub(crate) fn update_put_meta_snapshot_fenced(
    resource: &str,
    content_type: &str,
    blob_key: &str,
    etag: &str,
    modified: Option<&str>,
    nonce: &str,
    epoch: u64,
    append_seq: u64,
) -> Result<String, BuildError> {
    let g = iri(resource)?;
    let rec = iri_const(&s_record());
    let predicates = [
        p_content_type(),
        p_blob_key(),
        p_etag(),
        p_modified(),
        p_append_seq(),
    ]
    .iter()
    .map(|p| iri_const(p))
    .collect::<Vec<_>>()
    .join(", ");
    let modified = modified
        .map(|m| format!(" ; {} {}", iri_const(&p_modified()), datetime_literal(m)))
        .unwrap_or_default();
    Ok(format!(
        "DELETE {{ GRAPH {g} {{ {rec} ?oldP ?oldO }} }} \
         INSERT {{ GRAPH {g} {{ {rec} {pct} {ct} ; {pbk} {bk} ; {pet} {et} ; {pseq} {append_seq}{modified} }} {marker} }} \
         WHERE {{ OPTIONAL {{ GRAPH {g} {{ {rec} ?oldP ?oldO }} FILTER (?oldP IN ({predicates})) }} {guard} }}",
        pct = iri_const(&p_content_type()), ct = literal(content_type),
        pbk = iri_const(&p_blob_key()), bk = literal(blob_key),
        pet = iri_const(&p_etag()), et = literal(etag),
        pseq = iri_const(&p_append_seq()),
        marker = completion_marker(resource, nonce)?, guard = fence_guard(Some(epoch), nonce),
    ))
}

/// UPDATE: ATOMICALLY create-or-replace a child record + its container-containment edge, guarded by a
/// `container`-EXISTS check — in ONE SPARQL `DELETE/INSERT … WHERE` update.
///
/// `DELETE/INSERT … WHERE` is a single atomic operation whose `WHERE` clause requires the container's
/// index record to be present; if it is absent the pattern yields no solution and **neither the
/// DELETE nor the INSERT runs** (the LDP layer maps "edge not present afterwards ⇒ container missing"
/// to a 404). When it matches:
/// - the DELETE clause first removes any PRE-EXISTING reserved record predicates of the child (so a
///   re-create — or a same-IRI race that already landed a record — leaves EXACTLY ONE
///   `contentType`/`blobKey`/`etag` triple, never an accumulation that would make
///   `select_meta(... LIMIT 1)` nondeterministic — the round-3 finding), and
/// - the INSERT clause writes the child's fresh record AND the `container ldp:contains child` edge
///   together — there is no window in which the edge exists without the child's metadata, mirroring
///   the in-memory atomic impl.
///
/// Because DELETE + INSERT share the SAME `WHERE` (container-EXISTS), the stale-record cleanup only
/// happens when the container actually exists — a create against a missing container touches nothing.
/// `OPTIONAL` binds the old values so the DELETE is a no-op when the child is genuinely new.
///
/// It ALSO writes a per-operation **create marker** (`createMarker <nonce>`) into the child graph in
/// the same atomic INSERT. The caller confirms success by ASKing for THAT exact nonce (via
/// [`ask_create_marker`]) — a signal no concurrent `remove_child`/sibling create ever touches — so the
/// confirm is immune to a containment-edge race between the update and the check.
///
/// Concurrent creates retain each other's markers. After confirming its own nonce, a client may
/// remove that nonce alone; markers for operations still awaiting confirmation remain available.
/// Only the single-valued record predicates are DELETE-replaced by this update.
pub fn update_create_child(
    container: &str,
    child: &str,
    content_type: &str,
    blob_key: &str,
    etag: &str,
    modified: Option<&str>,
    nonce: &str,
) -> Result<String, BuildError> {
    create_child_guarded(
        container,
        child,
        content_type,
        blob_key,
        etag,
        modified,
        nonce,
        None,
        0,
    )
}

/// [`update_create_child`], which does nothing once its `nonce` is fenced ([`update_fence`]): the
/// form a client sends when a failed update can still commit after it has answered (an HTTP
/// update that timed out), so that a fence can make its outcome final.
/// The writer epoch must also match; zero permits an index with no epoch record yet.
#[allow(clippy::too_many_arguments)]
pub fn update_create_child_fenced(
    container: &str,
    child: &str,
    content_type: &str,
    blob_key: &str,
    etag: &str,
    modified: Option<&str>,
    nonce: &str,
    epoch: u64,
) -> Result<String, BuildError> {
    create_child_guarded(
        container,
        child,
        content_type,
        blob_key,
        etag,
        modified,
        nonce,
        Some(epoch),
        0,
    )
}

/// Build a child transaction from a complete stored metadata snapshot.
#[cfg(any(feature = "http-sparq", feature = "embedded-sparq"))]
pub(crate) fn update_create_child_meta(
    container: &str,
    child: &str,
    meta: &super::ResourceMeta,
    nonce: &str,
    epoch: Option<u64>,
) -> Result<String, BuildError> {
    let modified = meta
        .last_modified
        .and_then(super::timestamp::to_xsd_datetime);
    create_child_guarded(
        container,
        child,
        &meta.content_type,
        &meta.blob_key,
        &meta.etag,
        modified.as_deref(),
        nonce,
        epoch,
        meta.append_seq,
    )
}

#[allow(clippy::too_many_arguments)]
fn create_child_guarded(
    container: &str,
    child: &str,
    content_type: &str,
    blob_key: &str,
    etag: &str,
    modified: Option<&str>,
    nonce: &str,
    epoch: Option<u64>,
    append_seq: u64,
) -> Result<String, BuildError> {
    let cg = iri(child)?;
    let crec = iri_const(&s_record());
    let pct = iri_const(&p_content_type());
    let pbk = iri_const(&p_blob_key());
    let pet = iri_const(&p_etag());
    let pmark = iri_const(&p_create_marker());
    let pmod = iri_const(&p_modified());
    let pseq = iri_const(&p_append_seq());
    let pg = iri(container)?;
    let prec = iri_const(&s_record());
    let contains = iri_const(LDP_CONTAINS);
    let childi = iri(child)?;
    // `pss:modified` is threaded into the SAME atomic modify as the other single-valued record
    // predicates (so a re-create replaces it, mirroring `update_put_meta`). The stale `?oldMod` is
    // ALWAYS DELETE-cleared (bound by the WHERE `OPTIONAL`) — even when `modified` is `None` — so a
    // re-create with no timestamp genuinely clears any prior one (a later read returns `None`, never a
    // stale `Some(...)`). Only the INSERT of a fresh value is conditional on `Some`.
    let mod_del = format!(" ; {pmod} ?oldMod");
    let mod_opt = format!("OPTIONAL {{ GRAPH {cg} {{ {crec} {pmod} ?oldMod }} }} ");
    let mod_ins = match modified {
        Some(m) => format!(" ; {pmod} {dt}", dt = datetime_literal(m)),
        None => String::new(),
    };
    Ok(format!(
        "DELETE {{ \
            GRAPH {cg} {{ {crec} {pct} ?oldCt ; {pbk} ?oldBk ; {pet} ?oldEt ; {pseq} ?oldSeq{mod_del} . }} \
         }} INSERT {{ \
            GRAPH {cg} {{ {crec} {pct} {ct} ; {pbk} {bk} ; {pet} {et} ; {pseq} {append_seq}{mod_ins} ; {pmark} {nce} . }} \
            GRAPH {pg} {{ {prec} {contains} {childi} . }} \
         }} WHERE {{ \
            GRAPH {pg} {{ {prec} {pct} ?anyCt }} \
            OPTIONAL {{ GRAPH {cg} {{ {crec} {pct} ?oldCt }} }} \
            OPTIONAL {{ GRAPH {cg} {{ {crec} {pbk} ?oldBk }} }} \
            OPTIONAL {{ GRAPH {cg} {{ {crec} {pet} ?oldEt }} }} \
            OPTIONAL {{ GRAPH {cg} {{ {crec} {pseq} ?oldSeq }} }} \
            {mod_opt}{fence}\
         }}",
        fence = fence_guard(epoch, nonce),
        mod_del = mod_del,
        mod_ins = mod_ins,
        mod_opt = mod_opt,
        cg = cg,
        crec = crec,
        pct = pct,
        ct = literal(content_type),
        pbk = pbk,
        bk = literal(blob_key),
        pet = pet,
        et = literal(etag),
        pmark = pmark,
        nce = literal(nonce),
        pg = pg,
        prec = prec,
        contains = contains,
        childi = childi,
    ))
}

/// Whether an IRI is a RESERVED metadata term this server owns within a resource's graph: the index
/// record subject (`urn:pss:index#record`) or any `urn:pss:index#` predicate. Body RDF must never
/// write these, else untrusted data could corrupt/spoof the index records that share the graph.
fn is_reserved_term(value: &str) -> bool {
    value == s_record() || value.starts_with(PSS_NS)
}

/// UPDATE: insert a resource's parsed body triples (already validated RDF) into its graph, as one
/// `INSERT DATA` block. Each term is rendered through the safe (fallible) builders — an
/// IRIREF-invalid subject/predicate/object IRI is rejected. Empty input ⇒ `Ok("")` (the caller skips
/// the request).
///
/// A body triple whose subject/predicate (or an IRI object) touches a RESERVED metadata term — the
/// `urn:pss:index#record` subject or a `urn:pss:index#` predicate — is REJECTED with
/// [`BuildError::ReservedTermCollision`]. The resource's RDF and its index records share ONE named
/// graph (the single-graph model), so without this guard untrusted body data could overwrite/spoof
/// the index record (or be silently deleted by `put_meta` / hidden from `construct_resource`). Fail
/// closed: a colliding triple aborts the whole insert.
pub fn insert_body_data(
    resource: &str,
    ntriples_lines: &[(String, String, BodyObject)],
) -> Result<String, BuildError> {
    if ntriples_lines.is_empty() {
        return Ok(String::new());
    }
    let g = iri(resource)?;
    let mut body = String::new();
    for (s, p, o) in ntriples_lines {
        // Reject any triple touching the reserved metadata terms (subject, predicate, or an IRI
        // object that names a reserved term).
        if is_reserved_term(s) || is_reserved_term(p) || o.touches_reserved_term() {
            return Err(BuildError::ReservedTermCollision);
        }
        body.push_str(&iri(s)?);
        body.push(' ');
        body.push_str(&iri(p)?);
        body.push(' ');
        body.push_str(&o.render()?);
        body.push_str(" . ");
    }
    Ok(format!("INSERT DATA {{ GRAPH {g} {{ {body} }} }}", g = g))
}

/// UPDATE: drop a resource's whole graph (its index record + RDF) — the DELETE byte-pointer lookup
/// happens first via [`select_meta`]. `DROP SILENT` is idempotent on an absent graph.
///
/// For a CONTAINER target this is also what clears the container's `ldp:contains` set: the container's
/// own named graph holds both its index record and its containment edges, so dropping the graph
/// removes the (by-then-empty, per the handler's empty-container precondition) membership too. The
/// container's edge in its PARENT's graph is detached separately by [`update_remove_child`].
pub fn update_delete_resource(resource: &str) -> Result<String, BuildError> {
    Ok(format!("DROP SILENT GRAPH {g}", g = iri(resource)?))
}

/// Build a resource deletion and parent detach for one captured transaction.
///
/// The embedded client captures the entire request and commits its effects once.
///
/// # Errors
///
/// Rejects invalid resource or parent IRIs before any mutation.
#[cfg(any(all(feature = "embedded-sparq", not(target_arch = "wasm32")), test))]
pub(super) fn update_delete_resource_with_parent(
    resource: &str,
    parent: Option<&str>,
) -> Result<String, BuildError> {
    let delete = update_delete_resource(resource)?;
    match parent {
        Some(parent) => Ok(format!(
            "{}; {delete}",
            update_remove_child(parent, resource)?
        )),
        None => Ok(delete),
    }
}

/// Delete a resource graph in one modify under its nonce fence and writer epoch.
/// An absent resource still records completion (the operation is idempotent).
pub fn update_delete_resource_fenced(
    resource: &str,
    nonce: &str,
    epoch: u64,
) -> Result<String, BuildError> {
    update_delete_resource_with_parent_fenced(resource, None, nonce, epoch)
}

/// Delete a resource graph and detach its parent edge in the same fenced modify.
/// The completion marker is independent of the resource and parent graphs.
pub fn update_delete_resource_with_parent_fenced(
    resource: &str,
    parent: Option<&str>,
    nonce: &str,
    epoch: u64,
) -> Result<String, BuildError> {
    let g = iri(resource)?;
    let parent = parent
        .map(|p| -> Result<String, BuildError> {
            Ok(format!(
                "GRAPH {} {{ {} {} {g} }}",
                iri(p)?,
                iri_const(&s_record()),
                iri_const(LDP_CONTAINS)
            ))
        })
        .transpose()?
        .unwrap_or_default();
    Ok(format!(
        "DELETE {{ GRAPH {g} {{ ?s ?p ?o }} {parent} }} INSERT {{ {marker} \
         GRAPH {mg} {{ {receipt} <{PSS_NS}deletedBlobKey> ?oldKey }} }} \
         WHERE {{ OPTIONAL {{ GRAPH {g} {{ ?s ?p ?o }} }} \
         OPTIONAL {{ GRAPH {g} {{ {rec} {pbk} ?oldKey }} }} {guard} }}",
        mg = iri_const(&g_delete_markers()),
        receipt = delete_receipt(nonce)?,
        rec = iri_const(&s_record()),
        pbk = iri_const(&p_blob_key()),
        marker = completion_marker(resource, nonce)?,
        guard = fence_guard(Some(epoch), nonce),
    ))
}

/// Read the blob key captured atomically by a fenced resource delete.
/// No row means the resource had no metadata pointer when the delete committed.
pub fn select_deleted_blob_key(nonce: &str) -> Result<String, BuildError> {
    Ok(format!(
        "SELECT ?bk WHERE {{ GRAPH {} {{ {} {} ?bk }} }}",
        iri_const(&g_delete_markers()),
        delete_receipt(nonce)?,
        iri_const(&format!("{PSS_NS}deletedBlobKey"))
    ))
}

fn delete_receipt(nonce: &str) -> Result<String, BuildError> {
    iri(&format!("{PSS_NS}deleted/{nonce}"))
}

/// Detach one containment edge under its nonce fence and writer epoch.
/// An absent edge still records completion.
pub fn update_remove_child_fenced(
    container: &str,
    child: &str,
    nonce: &str,
    epoch: u64,
) -> Result<String, BuildError> {
    Ok(format!(
        "DELETE {{ GRAPH {g} {{ {rec} {contains} {child} }} }} INSERT {{ {marker} }} WHERE {{ {guard} }}",
        g = iri(container)?, rec = iri_const(&s_record()), contains = iri_const(LDP_CONTAINS),
        child = iri(child)?, marker = completion_marker(child, nonce)?, guard = fence_guard(Some(epoch), nonce),
    ))
}

fn completion_marker(resource: &str, nonce: &str) -> Result<String, BuildError> {
    Ok(format!(
        "GRAPH {} {{ {} {} {} }}",
        iri_const(&g_delete_markers()),
        iri(resource)?,
        iri_const(&p_delete_marker()),
        literal(nonce)
    ))
}

/// Remove only this operation's completion evidence after its confirmation and receipt reads.
/// A failed cleanup may be left for later; it does not change the completed write's outcome.
#[cfg(feature = "http-sparq")]
pub(super) fn update_clear_completion(resource: &str, nonce: &str) -> Result<String, BuildError> {
    Ok(format!(
        "DELETE {{ GRAPH {mg} {{ {resource} {deleted} {nonce_literal} . {receipt} {key} ?key }} \
         GRAPH {resource} {{ {record} {created} {nonce_literal} }} }} \
         WHERE {{ OPTIONAL {{ GRAPH {mg} {{ {receipt} {key} ?key }} }} }}",
        mg = iri_const(&g_delete_markers()),
        resource = iri(resource)?,
        deleted = iri_const(&p_delete_marker()),
        nonce_literal = literal(nonce),
        receipt = delete_receipt(nonce)?,
        key = iri_const(&format!("{PSS_NS}deletedBlobKey")),
        record = iri_const(&s_record()),
        created = iri_const(&p_create_marker()),
    ))
}

/// The reserved graph holding the storage index's writer epoch.
/// Generic resource mutations refuse this IRI; only a lifecycle start raises it.
pub const WRITER_EPOCH: &str = "urn:x-sparq-lws:writer-epoch";

/// Read the writer epoch. No row means that no lifecycle start has raised it yet.
pub fn select_writer_epoch() -> String {
    format!("SELECT ?epoch WHERE {{ GRAPH <{WRITER_EPOCH}> {{ <{WRITER_EPOCH}> <{PSS_NS}epoch> ?epoch }} }}")
}

/// Raise the writer epoch to `epoch` in one atomic modify, recording this start's `owner` nonce.
/// A delayed raise cannot lower a subsequent epoch or claim another start's equal epoch.
pub fn update_begin_epoch(epoch: u64, owner: &str) -> String {
    format!(
        "DELETE {{ GRAPH <{WRITER_EPOCH}> {{ <{WRITER_EPOCH}> <{PSS_NS}epoch> ?old ; <{PSS_NS}epochOwner> ?owner }} }} \
        INSERT {{ GRAPH <{WRITER_EPOCH}> {{ <{WRITER_EPOCH}> <{PSS_NS}epoch> {epoch} ; <{PSS_NS}epochOwner> {owner} }} }} \
        WHERE {{ OPTIONAL {{ GRAPH <{WRITER_EPOCH}> {{ <{WRITER_EPOCH}> <{PSS_NS}epoch> ?old }} }} \
        OPTIONAL {{ GRAPH <{WRITER_EPOCH}> {{ <{WRITER_EPOCH}> <{PSS_NS}epochOwner> ?owner }} }} \
        FILTER (!BOUND(?old) || ?old < {epoch}) }}",
        owner = literal(owner),
    )
}

/// Confirm the start that established this epoch, including its unique owner nonce.
#[cfg(feature = "http-sparq")]
pub(super) fn ask_writer_epoch_owner(epoch: u64, owner: &str) -> String {
    format!("ASK {{ GRAPH <{WRITER_EPOCH}> {{ <{WRITER_EPOCH}> <{PSS_NS}epoch> {epoch} ; <{PSS_NS}epochOwner> {} }} }}", literal(owner))
}

fn epoch_guard(epoch: u64) -> String {
    if epoch == 0 {
        // Clients used without a lifecycle start retain ordinary LDP semantics until a start.
        format!("FILTER NOT EXISTS {{ GRAPH <{WRITER_EPOCH}> {{ <{WRITER_EPOCH}> <{PSS_NS}epoch> ?epoch }} FILTER (?epoch != 0) }} ")
    } else {
        format!("GRAPH <{WRITER_EPOCH}> {{ <{WRITER_EPOCH}> <{PSS_NS}epoch> {epoch} }} ")
    }
}

/// UPDATE: ATOMICALLY delete a container's whole graph ONLY IF it exists AND is empty (no
/// `ldp:contains` member), AND detach it from its parent's containment, AND write a per-operation
/// **delete marker** — ALL in ONE operation so the caller can confirm the outcome race-free.
///
/// This is ONE SPARQL 1.1 `DELETE/INSERT … WHERE` **modify** operation (NOT two `;`-joined
/// statements). That distinction is load-bearing:
///
/// > Under SPARQL 1.1 Update, a `;`-joined sequence evaluates each operation's WHERE against the graph
/// > AS LEFT BY the previous operation. So a `DELETE …` THEN a separate `INSERT … WHERE {exists+empty
/// > guard}` would evaluate the marker INSERT's guard AFTER the DELETE already emptied the container —
/// > the guard no longer matches, the marker is NEVER written, and the caller mis-reports a genuine
/// > delete as `NotFound`. (This was a real bug in an earlier two-statement form.)
///
/// A SINGLE `DELETE { … } INSERT { … } WHERE { … }` instead evaluates its WHERE ONCE against the
/// pre-modification graph, then applies the deletes and inserts atomically against THAT one solution
/// set. So the marker INSERT and every DELETE clause share the SAME pre-delete guard match — the
/// marker is written IFF the delete ran, with no sequencing hazard.
///
/// The ONE operation:
/// - **WHERE (the guard, evaluated once, pre-modification):** the record EXISTS (`<record> contentType
///   ?anyCt` in the container's graph — the container-exists guard) AND there is NO `ldp:contains`
///   member (`FILTER NOT EXISTS` — the empty guard). A non-empty or absent container ⇒ no solution ⇒
///   nothing deletes or inserts (the safety invariant: a non-empty container is NEVER deleted). It
///   binds `?s ?p ?o` over the container's graph so the DELETE can drop every triple.
/// - **DELETE (atomic, multi-graph):** removes EVERY triple of the container's own graph
///   (`GRAPH <g> { ?s ?p ?o }` — its index record; an empty container holds no other triples) AND, when
///   a `parent` is given, the `<parent-record> ldp:contains <container>` edge that lives in the
///   PARENT's graph (`GRAPH <pg> { <prec> contains <g> }`). A SPARQL `DELETE` clause may span multiple
///   graphs, so the parent-edge detach is folded into the SAME guarded operation — closing the window
///   in which a concurrent POST recreates the child under the parent and a separate, later detach
///   orphaned it.
/// - **INSERT (atomic):** writes `<g> <deleteMarker> "nonce"` into the reserved `deleteMarkers` scratch
///   graph. The caller ASKs for THIS nonce ([`ask_delete_marker`]) to learn `Deleted`; its absence + an
///   [`ask_exists`] check splits `NotEmpty` (record still present) from `NotFound` (record absent). The
///   marker is in a distinct graph, so the DELETE never removes it.
///
/// `parent` is `None` for a root/parentless container (no edge to detach). It is rendered through the
/// injection-safe `iri()` builder. The marker graph is NEVER a resource graph, so this can't collide
/// with user data; the IRI builders reject any IRIREF-invalid container/parent, fail-closed.
pub fn update_delete_container_if_empty(
    container: &str,
    parent: Option<&str>,
    nonce: &str,
) -> Result<String, BuildError> {
    delete_container_guarded(container, parent, nonce, None)
}

/// [`update_delete_container_if_empty`], which does nothing once its `nonce` is fenced
/// ([`update_fence`]); see [`update_create_child_fenced`].
pub fn update_delete_container_if_empty_fenced(
    container: &str,
    parent: Option<&str>,
    nonce: &str,
    epoch: u64,
) -> Result<String, BuildError> {
    delete_container_guarded(container, parent, nonce, Some(epoch))
}

fn delete_container_guarded(
    container: &str,
    parent: Option<&str>,
    nonce: &str,
    epoch: Option<u64>,
) -> Result<String, BuildError> {
    let g = iri(container)?;
    let rec = iri_const(&s_record());
    let pct = iri_const(&p_content_type());
    let contains = iri_const(LDP_CONTAINS);
    let mg = iri_const(&g_delete_markers());
    let pmark = iri_const(&p_delete_marker());
    // The parent-edge DELETE clause: `GRAPH <parent> { <record> ldp:contains <container> }`, folded
    // into the SAME operation so the detach is atomic with the graph delete. Empty when parentless.
    let parent_delete = match parent {
        Some(p) => format!(
            "GRAPH {pg} {{ {prec} {contains} {childi} }} ",
            pg = iri(p)?,
            prec = rec,
            contains = contains,
            childi = g,
        ),
        None => String::new(),
    };
    // ONE SPARQL 1.1 `DELETE { } INSERT { } WHERE { }` modify — its WHERE is evaluated ONCE against the
    // pre-modification graph, so the multi-graph DELETE (container graph + parent edge) and the marker
    // INSERT share the SAME exists+empty solution set. NOT `;`-joined (that would re-evaluate the
    // INSERT guard after the DELETE emptied the graph). NO `//` line comments here — this string is
    // sent verbatim to the SPARQL endpoint.
    Ok(format!(
        "DELETE {{ \
            GRAPH {g} {{ ?s ?p ?o }} \
            {parent_delete}\
         }} INSERT {{ \
            GRAPH {mg} {{ {g} {pmark} {nce} }} \
         }} WHERE {{ \
            GRAPH {g} {{ ?s ?p ?o }} \
            GRAPH {g} {{ {rec} {pct} ?anyCt }} \
            FILTER NOT EXISTS {{ GRAPH {g} {{ {rec} {contains} ?anyChild }} }} \
            {fence}\
         }}",
        fence = fence_guard(epoch, nonce),
        g = g,
        rec = rec,
        pct = pct,
        contains = contains,
        mg = mg,
        pmark = pmark,
        nce = literal(nonce),
        parent_delete = parent_delete,
    ))
}

/// The `WHERE` clause that makes a fenced update ([`update_create_child_fenced`],
/// [`update_delete_container_if_empty_fenced`]) require their writer epoch and do nothing once
/// their `nonce` is fenced; empty for an unfenced one.
fn fence_guard(epoch: Option<u64>, nonce: &str) -> String {
    let Some(epoch) = epoch else {
        return String::new();
    };
    format!(
        "{epoch_guard}FILTER NOT EXISTS {{ GRAPH {fg} {{ {rec} {p} {n} }} }} ",
        epoch_guard = epoch_guard(epoch),
        fg = iri_const(&g_fences()),
        rec = iri_const(&s_record()),
        p = iri_const(&p_fenced()),
        n = literal(nonce),
    )
}

/// UPDATE: fence the fenced update whose nonce is `nonce`, so that it does nothing if it has not
/// been applied yet, whenever it is.
///
/// This is what makes the outcome of an update that failed but may still commit final. An HTTP
/// update that timed out (or whose connection failed, or that `sparq-server` answered `503` for
/// because its own wait elapsed) can still be queued or running on the server, and commit after
/// the client has read that it did not. `sparq-server` applies each update atomically, one at a
/// time, on its one writer, and answers a successful update only once a later query sees what it
/// committed. So once this fence has succeeded, the fenced update has either been applied before
/// it (and every later read sees it) or will find the fence when it is applied (and do nothing):
/// a read after the fence tells its outcome, and that outcome no longer changes. No ordering of
/// requests is relied on (the fenced update may still be in a socket, or waiting for a worker,
/// when the fence is applied), only the atomicity of each update and read-your-writes.
///
/// A fence is idempotent (re-sending it is harmless) and stays: it must outlive the update it
/// fences, which may still arrive at any time. Each is one triple, written only for an update that
/// failed this way.
pub fn update_fence(nonce: &str) -> String {
    format!(
        "INSERT DATA {{ GRAPH {fg} {{ {rec} {p} {n} }} }}",
        fg = iri_const(&g_fences()),
        rec = iri_const(&s_record()),
        p = iri_const(&p_fenced()),
        n = literal(nonce),
    )
}

/// ASK whether the completion marker for `container` and `nonce` is present in the reserved
/// scratch graph. Used by conditional deletes and other guarded index writes to confirm that
/// their modify ran. No other operation writes or removes this nonce.
pub fn ask_delete_marker(container: &str, nonce: &str) -> Result<String, BuildError> {
    Ok(format!(
        "ASK {{ GRAPH {mg} {{ {g} {p} {n} }} }}",
        mg = iri_const(&g_delete_markers()),
        g = iri(container)?,
        p = iri_const(&p_delete_marker()),
        n = literal(nonce),
    ))
}

/// UPDATE: remove a `container ldp:contains child` edge (idempotent — a `DELETE WHERE` of an absent
/// edge is a no-op).
pub fn update_remove_child(container: &str, child: &str) -> Result<String, BuildError> {
    Ok(format!(
        "DELETE WHERE {{ GRAPH {pg} {{ {prec} {contains} {childi} }} }}",
        pg = iri(container)?,
        prec = iri_const(&s_record()),
        contains = iri_const(LDP_CONTAINS),
        childi = iri(child)?,
    ))
}

/// An object position for [`insert_body_data`]: either an IRI/bnode node or a literal (with its
/// datatype/lang preserved). Rendered through the safe builders so untrusted RDF cannot inject.
#[derive(Debug, Clone)]
pub enum BodyObject {
    /// A named node (IRI).
    Iri(String),
    /// A blank node, by label (rendered `_:label` with the label sanitised to bnode-safe chars).
    Blank(String),
    /// A plain `xsd:string` (or implicitly-typed) literal.
    PlainLiteral(String),
    /// A language-tagged literal: (lexical, lang).
    LangLiteral(String, String),
    /// A typed literal: (lexical, datatype IRI).
    TypedLiteral(String, String),
}

impl BodyObject {
    /// Whether this object (when an IRI / datatype-IRI) names a reserved metadata term.
    fn touches_reserved_term(&self) -> bool {
        match self {
            BodyObject::Iri(v) => is_reserved_term(v),
            BodyObject::TypedLiteral(_, dt) => is_reserved_term(dt),
            _ => false,
        }
    }

    /// Render this object term safely. Fallible because an IRI / datatype-IRI that is not a valid
    /// IRIREF is rejected (never escaped-and-aliased), and a malformed language tag is rejected
    /// (never silently rewritten — fail-closed).
    pub fn render(&self) -> Result<String, BuildError> {
        Ok(match self {
            BodyObject::Iri(v) => iri(v)?,
            BodyObject::Blank(label) => render_bnode(label),
            BodyObject::PlainLiteral(v) => literal(v),
            BodyObject::LangLiteral(v, lang) => {
                format!("{}@{}", literal(v), validate_lang(lang)?)
            }
            BodyObject::TypedLiteral(v, dt) => format!("{}^^{}", literal(v), iri(dt)?),
        })
    }
}

/// Render a blank-node label as a SPARQL `BLANK_NODE_LABEL` using an INJECTIVE, grammar-safe
/// encoding: ONLY `[A-Za-z0-9]` pass through; EVERY other byte (including `-`, `.`, `_`) is hex-escaped
/// as `_xXX_`. Escaping `-`/`.`/`_` too (not just the unsafe ones) keeps the encoding both injective
/// (distinct labels never collapse — the round-3 finding) AND always-grammar-valid in EVERY position,
/// so it can never emit an illegal leading/trailing `-`/`.` (the round-4 finding: a SPARQL bnode label
/// may not end with `.`). The fixed `b` prefix keeps the first char legal. The output is always a
/// single well-formed `_:...` token, so it cannot inject.
fn render_bnode(label: &str) -> String {
    let mut out = String::with_capacity(label.len() + 3);
    out.push_str("_:b"); // fixed prefix: a grammar-legal leading label char
    for &byte in label.as_bytes() {
        if byte.is_ascii_alphanumeric() {
            out.push(byte as char);
        } else {
            // `_xXX_` — the `_` introducer + 2 hex + `_` closer keeps the run grammar-internal (the
            // trailing `_` is a legal interior PN_CHARS char and never a `.`/`-`), so no position is
            // ever invalid.
            out.push_str("_x");
            out.push(nibble_hex(byte >> 4));
            out.push(nibble_hex(byte & 0x0f));
            out.push('_');
        }
    }
    out
}

/// VALIDATE a language tag against the SPARQL `LANGTAG` grammar
/// (`'@' [a-zA-Z]+ ('-' [a-zA-Z0-9]+)*`) and return it unchanged, or [`BuildError::InvalidLangTag`].
///
/// The shape (NOT merely "alphanumeric + hyphen"): an ALPHABETIC primary subtag of length ≥ 1,
/// followed by zero or more `-`-separated subtags each of length ≥ 1 and alphanumeric. So `en`,
/// `en-US`, `zh-Hans-CN` pass; `1`/`1x` (non-alpha primary), `en-` (empty trailing subtag),
/// `en--US` (empty interior subtag), `` (empty) are REJECTED. Fail-CLOSED: a malformed tag is
/// refused, never silently rewritten (the round-6 finding) and never emits invalid query text.
fn validate_lang(lang: &str) -> Result<&str, BuildError> {
    let mut subtags = lang.split('-');
    // Primary subtag: alphabetic, non-empty.
    match subtags.next() {
        Some(primary)
            if !primary.is_empty() && primary.chars().all(|c| c.is_ascii_alphabetic()) => {}
        _ => return Err(BuildError::InvalidLangTag),
    }
    // Each remaining subtag: alphanumeric, non-empty.
    for sub in subtags {
        if sub.is_empty() || !sub.chars().all(|c| c.is_ascii_alphanumeric()) {
            return Err(BuildError::InvalidLangTag);
        }
    }
    Ok(lang)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delete_with_parent_validates_both_iris_and_reuses_the_delete_builders() {
        let child = "https://pod.example/c/item";
        let parent = "https://pod.example/c/";
        assert_eq!(
            update_delete_resource_with_parent(child, None).unwrap(),
            update_delete_resource(child).unwrap()
        );
        assert_eq!(
            update_delete_resource_with_parent(child, Some(parent)).unwrap(),
            format!(
                "{}; {}",
                update_remove_child(parent, child).unwrap(),
                update_delete_resource(child).unwrap()
            )
        );
        assert_eq!(
            update_delete_resource_with_parent("bad>iri", None),
            Err(BuildError::InvalidIri)
        );
        assert_eq!(
            update_delete_resource_with_parent(child, Some("bad>iri")),
            Err(BuildError::InvalidIri)
        );
    }

    #[test]
    fn iri_rejects_injection_delimiters() {
        // A crafted IRI trying to break out of the `<...>` term must be REJECTED (not escaped), so it
        // can never alias another IRI nor inject.
        let attack = "http://e/x> } ; DROP GRAPH <http://victim> ; INSERT DATA { GRAPH <http://e/x";
        assert_eq!(iri(attack), Err(BuildError::InvalidIri));
        // A clean IRI is wrapped verbatim.
        assert_eq!(iri("http://e/a").unwrap(), "<http://e/a>");
    }

    #[test]
    fn iri_rejects_control_and_space() {
        assert_eq!(iri("http://e/a b"), Err(BuildError::InvalidIri)); // space
        assert_eq!(iri("http://e/a\nc"), Err(BuildError::InvalidIri)); // control
        assert_eq!(iri("http://e/a\\b"), Err(BuildError::InvalidIri)); // backslash
    }

    #[test]
    fn iri_is_injective_no_aliasing() {
        // A raw `>` is rejected; a literal `%3E` is a DISTINCT, valid IRI that passes through verbatim
        // — so the two can never alias (the round-4 finding: percent-escaping would collapse them).
        assert_eq!(iri("http://e/a>b"), Err(BuildError::InvalidIri));
        assert_eq!(iri("http://e/a%3Eb").unwrap(), "<http://e/a%3Eb>");
    }

    #[test]
    fn literal_escapes_quote_and_backslash_and_newlines() {
        let attack = "x\" . } ; DROP GRAPH <http://victim> ; INSERT { \"";
        let rendered = literal(attack);
        assert!(rendered.starts_with('"') && rendered.ends_with('"'));
        assert!(rendered.contains("\\\""), "inner quote escaped: {rendered}");
        // The breakout invariant: in the INTERIOR (between the delimiting quotes), every `"` is
        // escaped — i.e. there is NO unescaped `"` that could close the literal early.
        let interior = &rendered[1..rendered.len() - 1];
        let bytes = interior.as_bytes();
        for (i, &b) in bytes.iter().enumerate() {
            if b == b'"' {
                assert!(
                    i > 0 && bytes[i - 1] == b'\\',
                    "an unescaped quote at interior byte {i} could break out: {rendered}"
                );
            }
        }
        let nl = literal("a\nb\\c");
        assert!(nl.contains("\\n") && nl.contains("\\\\"));
        assert!(!nl.contains('\n'));
    }

    #[test]
    fn builders_wrap_every_untrusted_value() {
        // The container-exists-guarded create: the child IRI + literals are all bracketed/quoted.
        let q = update_create_child(
            "http://pod/c/",
            "http://pod/c/note",
            "text/turtle",
            "blob-key",
            "\"etag\"",
            None,
            "op-1",
        )
        .unwrap();
        assert!(q.contains("INSERT {"));
        assert!(q.contains("WHERE {"));
        assert!(q.contains(&iri_const(LDP_CONTAINS)));
        // The user-controlled etag literal "\"etag\"" must be escaped inside one literal.
        assert!(q.contains("\\\"etag\\\""), "etag escaped: {q}");
    }

    #[test]
    fn create_child_rejects_an_invalid_child_iri() {
        // An IRIREF-invalid child IRI is rejected, fail-closed (never escaped-and-aliased).
        let r = update_create_child(
            "http://pod/c/",
            "http://pod/c/no te", // a space — invalid IRIREF
            "text/turtle",
            "bk",
            "\"e\"",
            None,
            "op-1",
        );
        assert_eq!(r, Err(BuildError::InvalidIri));
    }

    #[test]
    fn put_meta_does_not_drop_the_whole_graph() {
        // The metadata re-write must DELETE only the three reserved record predicates, never the
        // whole graph (which would erase containment edges + user RDF — the roborev finding).
        let q = update_put_meta("http://pod/c/", "text/turtle", "bk", "\"e\"", None).unwrap();
        assert!(!q.contains("DROP"), "must not DROP the graph: {q}");
        // It targets exactly the five single-valued reserved predicates for deletion (modified is
        // ALWAYS cleared, even with None), then re-inserts.
        assert_eq!(
            q.matches("DELETE WHERE").count(),
            5,
            "five targeted deletes (incl. modified and appendSeq): {q}"
        );
        assert!(q.contains("INSERT DATA"));
        assert!(q.contains(&iri_const(&p_content_type())));
        assert!(q.contains(&iri_const(&p_blob_key())));
        assert!(q.contains(&iri_const(&p_etag())));
        // It must NOT mention ldp:contains (it leaves containment untouched).
        assert!(
            !q.contains(LDP_CONTAINS),
            "put_meta must not touch containment: {q}"
        );
        // With no modification time, the modified predicate is CLEARED (present in a DELETE) but no
        // fresh timestamp is INSERTed — so no typed dateTime literal appears.
        assert!(
            q.contains(&iri_const(&p_modified())),
            "modified is still delete-cleared when None: {q}"
        );
        assert!(
            !q.contains("^^<http://www.w3.org/2001/XMLSchema#dateTime>"),
            "no dateTime INSERTed when None: {q}"
        );
    }

    #[test]
    fn put_meta_with_modified_writes_a_typed_datetime_and_a_fourth_delete() {
        // A put carrying a modification time DELETE-replaces `pss:modified` (single-valued, like the
        // other record predicates) and INSERTs it as an `xsd:dateTime`-typed literal.
        let q = update_put_meta(
            "http://pod/c/note",
            "text/turtle",
            "bk",
            "\"e\"",
            Some("2026-07-05T12:34:56Z"),
        )
        .unwrap();
        // Five targeted deletes: the record predicates, modified and appendSeq.
        assert_eq!(
            q.matches("DELETE WHERE").count(),
            5,
            "three record deletes + modified and appendSeq deletes: {q}"
        );
        assert!(
            q.contains(&iri_const(&p_modified())),
            "modified predicate: {q}"
        );
        // The timestamp is a DATATYPED literal, not a plain string — so a consumer reads it as a
        // dateTime. The lexical value is escaped inside one literal (injection-safe).
        assert!(
            q.contains("\"2026-07-05T12:34:56Z\"^^<http://www.w3.org/2001/XMLSchema#dateTime>"),
            "typed xsd:dateTime literal: {q}"
        );
        assert!(!q.contains("DROP"), "still never DROPs the graph: {q}");
    }

    #[test]
    fn create_child_with_modified_includes_the_typed_timestamp() {
        let q = update_create_child(
            "http://pod/c/",
            "http://pod/c/note",
            "text/turtle",
            "bk",
            "\"e\"",
            Some("2026-07-05T12:34:56Z"),
            "op-1",
        )
        .unwrap();
        assert!(
            q.contains(&iri_const(&p_modified())),
            "modified predicate: {q}"
        );
        assert!(
            q.contains("\"2026-07-05T12:34:56Z\"^^<http://www.w3.org/2001/XMLSchema#dateTime>"),
            "typed xsd:dateTime literal in the INSERT: {q}"
        );
        // The old modified is bound via an OPTIONAL (so the DELETE replaces it on a re-create).
        assert!(
            q.contains("?oldMod"),
            "old modified bound for single-valued replace: {q}"
        );
        // Symmetry: without a modification time the stale value is still CLEARED (predicate present in
        // the DELETE) but no fresh typed timestamp is INSERTed.
        let none = update_create_child(
            "http://pod/c/",
            "http://pod/c/note",
            "text/turtle",
            "bk",
            "\"e\"",
            None,
            "op-1",
        )
        .unwrap();
        assert!(
            none.contains(&iri_const(&p_modified())),
            "modified is still delete-cleared when None: {none}"
        );
        assert!(
            !none.contains("^^<http://www.w3.org/2001/XMLSchema#dateTime>"),
            "no dateTime INSERTed when None: {none}"
        );
    }

    #[test]
    fn select_meta_binds_modified_optionally() {
        let q = select_meta("http://pod/c/note").unwrap();
        assert!(q.contains("?mod"), "selects the modified var: {q}");
        assert!(q.contains("OPTIONAL"), "modified is OPTIONAL: {q}");
        assert!(
            q.contains(&iri_const(&p_modified())),
            "modified predicate: {q}"
        );
    }

    #[test]
    fn select_read_plan_binds_modified_optionally() {
        let q =
            select_read_plan("http://pod/c/note", &["http://pod/c/note.acl".to_string()]).unwrap();
        assert!(q.contains("?mod"), "selects the modified var: {q}");
        assert!(q.contains("OPTIONAL"), "modified is OPTIONAL: {q}");
        assert!(
            q.contains(&iri_const(&p_modified())),
            "modified predicate: {q}"
        );
    }

    #[test]
    fn ask_contains_edge_targets_the_exact_edge() {
        let q = ask_contains_edge("http://pod/c/", "http://pod/c/note").unwrap();
        assert!(q.starts_with("ASK"));
        assert!(q.contains(&iri_const("http://pod/c/")));
        assert!(q.contains(&iri_const(LDP_CONTAINS)));
        assert!(q.contains(&iri_const("http://pod/c/note")));
    }

    #[test]
    fn ask_create_marker_targets_the_nonce() {
        let q = ask_create_marker("http://pod/c/note", "op-abc").unwrap();
        assert!(q.starts_with("ASK"));
        assert!(q.contains(&iri_const(&p_create_marker())));
        assert!(q.contains("\"op-abc\""), "nonce as a literal: {q}");
    }

    #[test]
    fn body_object_renders_each_term_safely() {
        assert_eq!(
            BodyObject::Iri("http://e/a".into()).render().unwrap(),
            "<http://e/a>"
        );
        assert_eq!(
            BodyObject::PlainLiteral("a\"b".into()).render().unwrap(),
            "\"a\\\"b\""
        );
        assert_eq!(
            BodyObject::LangLiteral("hi".into(), "en-US".into())
                .render()
                .unwrap(),
            "\"hi\"@en-US"
        );
        assert_eq!(
            BodyObject::TypedLiteral(
                "5".into(),
                "http://www.w3.org/2001/XMLSchema#integer".into()
            )
            .render()
            .unwrap(),
            "\"5\"^^<http://www.w3.org/2001/XMLSchema#integer>"
        );
        // A crafted lang tag is REJECTED fail-closed (never silently rewritten).
        assert_eq!(
            BodyObject::LangLiteral("x".into(), "en> } DROP".into()).render(),
            Err(BuildError::InvalidLangTag)
        );
        // An empty lang tag is rejected too (would otherwise emit invalid `"x"@`).
        assert_eq!(
            BodyObject::LangLiteral("x".into(), "".into()).render(),
            Err(BuildError::InvalidLangTag)
        );
        // An IRIREF-invalid object IRI / datatype IRI is rejected.
        assert_eq!(
            BodyObject::Iri("http://e/a>b".into()).render(),
            Err(BuildError::InvalidIri)
        );
        // A crafted bnode label is INJECTIVELY encoded (every non-alphanumeric byte hex-escaped
        // `_xXX_`), never lossily stripped + always grammar-valid (no trailing `.`/`-`).
        // `a b> .` => b + a + _x20_ (space) + b + _x3E_ ('>') + _x20_ (space) + _x2E_ ('.').
        assert_eq!(
            BodyObject::Blank("a b> .".into()).render().unwrap(),
            "_:ba_x20_b_x3E__x20__x2E_"
        );
    }

    #[test]
    fn render_bnode_is_injective_and_always_valid() {
        // Distinct labels the OLD lossy filter would have collapsed must now differ.
        let a = BodyObject::Blank("a b".into()).render().unwrap();
        let b = BodyObject::Blank("ab".into()).render().unwrap();
        assert_ne!(a, b, "distinct labels must not collapse: {a} vs {b}");
        // A label ending in `.` must NOT produce an invalid trailing-dot bnode (round-4 finding):
        // the `.` is hex-escaped, so the last char is the `_` closer (a legal interior char).
        let trailing_dot = BodyObject::Blank("foo.".into()).render().unwrap();
        assert!(
            !trailing_dot.ends_with('.'),
            "no invalid trailing dot: {trailing_dot}"
        );
        assert_eq!(trailing_dot, "_:bfoo_x2E_");
        // An all-unsafe label still yields a well-formed, non-empty bnode.
        let weird = BodyObject::Blank("> } .".into()).render().unwrap();
        assert!(weird.starts_with("_:b"));
        assert!(!weird.contains(' ') && !weird.contains('>') && !weird.contains('}'));
    }

    #[test]
    fn create_child_atomically_replaces_a_stale_child_record() {
        // The guarded create must DELETE any pre-existing child reserved-record predicates (incl. the
        // marker) before INSERTing, so a re-create can't accumulate duplicate metadata.
        let q = update_create_child(
            "http://pod/c/",
            "http://pod/c/note",
            "text/turtle",
            "bk",
            "\"e\"",
            None,
            "op-1",
        )
        .unwrap();
        assert!(
            q.starts_with("DELETE {"),
            "must DELETE stale child record first: {q}"
        );
        assert!(q.contains("INSERT {"));
        assert!(q.contains("WHERE {"));
        assert!(q.contains("OPTIONAL"), "old values bound via OPTIONAL: {q}");
        assert!(q.contains(&iri_const(LDP_CONTAINS)));
        // The per-operation marker (nonce) is written + its predicate referenced.
        assert!(q.contains(&iri_const(&p_create_marker())));
        assert!(q.contains("\"op-1\""), "marker nonce literal present: {q}");
        // The marker is APPEND-ONLY: it must NOT appear in the DELETE block (round-5 finding — a
        // concurrent same-child create deleting markers could clear another op's nonce). Split at
        // INSERT and assert the marker predicate is only in the INSERT half, never the DELETE half.
        let delete_block = q.split("INSERT {").next().unwrap();
        assert!(
            !delete_block.contains(&iri_const(&p_create_marker())),
            "marker must not be in the DELETE block (append-only): {q}"
        );
    }

    #[test]
    fn delete_container_if_empty_is_one_single_modify_not_a_sequence() {
        // MUTATION-CHECK (the regression for the round's HIGH): the conditional delete MUST be a
        // SINGLE `DELETE { } INSERT { } WHERE { }` modify — its WHERE evaluated ONCE pre-modification,
        // so the marker INSERT and the graph DELETE share one guard match. It must NOT be the old
        // two-statement `DELETE …WHERE… ; INSERT …WHERE…` form, whose second WHERE re-evaluates AFTER
        // the DELETE emptied the graph (⇒ the marker is never written ⇒ a real delete mis-reports
        // NotFound). So we assert the SHAPE: exactly one WHERE, no `;`-join, DELETE+INSERT before it.
        let q = update_delete_container_if_empty("http://pod/c/", None, "op-7").unwrap();
        assert!(
            q.starts_with("DELETE {"),
            "starts with the guarded delete: {q}"
        );
        // It is a single DELETE/INSERT/WHERE modify: ONE `INSERT {`, ONE `WHERE {`, ZERO operation
        // separators (`;`). The two-statement pre-fix form would have TWO `WHERE {` and a ` ; `.
        assert_eq!(
            q.matches("WHERE {").count(),
            1,
            "exactly ONE WHERE (a single modify, not a `;`-joined pair each with its own WHERE): {q}"
        );
        assert_eq!(
            q.matches("INSERT {").count(),
            1,
            "exactly ONE INSERT clause: {q}"
        );
        assert!(
            !q.contains(';'),
            "no `;` operation separator — a single modify, not a two-statement sequence: {q}"
        );
        // The single WHERE carries BOTH the exists guard and the empty guard; the INSERT is the marker.
        assert!(q.contains("FILTER NOT EXISTS"), "empty guard present: {q}");
        assert!(
            q.contains(LDP_CONTAINS),
            "empty guard targets ldp:contains: {q}"
        );
        assert!(
            q.contains(&iri_const(&p_content_type())),
            "exists guard present: {q}"
        );
        assert!(
            q.contains(&iri_const(&p_delete_marker())),
            "marker predicate present: {q}"
        );
        assert!(
            q.contains(&iri_const(&g_delete_markers())),
            "marker scratch graph present: {q}"
        );
        assert!(q.contains("\"op-7\""), "the op nonce is a literal: {q}");
        // It must NOT use a blanket DROP (which would unconditionally erase a non-empty container).
        assert!(!q.contains("DROP"), "must not blanket-DROP the graph: {q}");
        // No `//`-style line comment leaked into the query text (the IRIs legitimately contain `//`,
        // so check for a comment introducer — `//` followed by a space — which only a stray comment
        // would produce; an IRI's `//` is always followed by a host character, never a space).
        assert!(
            !q.contains("// "),
            "no `//`-comment markers in the query text: {q}"
        );
    }

    #[test]
    fn delete_container_if_empty_folds_the_parent_edge_into_the_same_modify() {
        // FINDING 2: the parent containment detach (`<parent> ldp:contains <container>`, in the
        // PARENT's graph) must be folded INTO the same single modify's DELETE — NOT a separate later
        // step that races a concurrent recreate. Assert the parent graph + its `ldp:contains` edge
        // appear inside the ONE operation, and there is still exactly one WHERE / no `;`.
        let parent = "http://pod/";
        let container = "http://pod/c/";
        let q = update_delete_container_if_empty(container, Some(parent), "op-9").unwrap();
        assert!(
            q.contains(&iri_const(parent)),
            "the parent graph IRI must appear in the modify (the detach is folded in): {q}"
        );
        // Still a single modify (the parent fold must not reintroduce a sequence).
        assert_eq!(
            q.matches("WHERE {").count(),
            1,
            "still exactly ONE WHERE after folding the parent edge: {q}"
        );
        assert!(
            !q.contains(';'),
            "still no `;` separator after folding the parent edge: {q}"
        );
        // The parent-edge DELETE clause sits BEFORE the WHERE (in the DELETE template), and the
        // container IRI is the OBJECT of the `ldp:contains` edge in the parent graph.
        let where_at = q.find("WHERE {").unwrap();
        let parent_edge = format!(
            "GRAPH {pg} {{ {prec} {contains} {childi} }}",
            pg = iri_const(parent),
            prec = iri_const(&s_record()),
            contains = iri_const(LDP_CONTAINS),
            childi = iri_const(container),
        );
        let edge_at = q
            .find(&parent_edge)
            .unwrap_or_else(|| panic!("parent-edge DELETE clause present: {q}"));
        assert!(
            edge_at < where_at,
            "the parent-edge detach is in the DELETE template (before WHERE), so it is part of the \
             atomic modify: {q}"
        );
    }

    #[test]
    fn delete_container_if_empty_omits_the_parent_clause_when_parentless() {
        // A root/parentless container has no edge to detach: the modify carries only the container
        // graph + marker, no second GRAPH in the DELETE.
        let q = update_delete_container_if_empty("http://pod/c/", None, "op-1").unwrap();
        // The DELETE template (before WHERE) names the container graph exactly once and no parent.
        let where_at = q.find("WHERE {").unwrap();
        let delete_block = &q[..where_at];
        assert_eq!(
            delete_block.matches("GRAPH <http://pod/c/>").count(),
            1,
            "only the container graph in the DELETE template when parentless: {q}"
        );
    }

    #[test]
    fn delete_container_if_empty_rejects_an_invalid_container_iri() {
        // An IRIREF-invalid container IRI is rejected fail-closed (never escaped-and-aliased), so an
        // injection can never reach the endpoint.
        assert_eq!(
            update_delete_container_if_empty("http://pod/c d/", None, "op-1"),
            Err(BuildError::InvalidIri)
        );
    }

    #[test]
    fn delete_container_if_empty_rejects_an_invalid_parent_iri() {
        // An IRIREF-invalid PARENT IRI is likewise rejected fail-closed (the parent flows through the
        // same injection-safe `iri()` builder).
        assert_eq!(
            update_delete_container_if_empty("http://pod/c/", Some("http://pod/ bad/"), "op-1"),
            Err(BuildError::InvalidIri)
        );
    }

    #[test]
    fn ask_delete_marker_targets_the_scratch_graph_subject_and_nonce() {
        let q = ask_delete_marker("http://pod/c/", "op-abc").unwrap();
        assert!(q.starts_with("ASK"));
        assert!(
            q.contains(&iri_const(&g_delete_markers())),
            "scratch graph: {q}"
        );
        assert!(
            q.contains(&iri_const("http://pod/c/")),
            "container subject: {q}"
        );
        assert!(
            q.contains(&iri_const(&p_delete_marker())),
            "marker predicate: {q}"
        );
        assert!(q.contains("\"op-abc\""), "nonce as a literal: {q}");
    }

    // Require explicit binary groups before counting their UNION depth: adjacent
    // UNION tokens in a group would otherwise associate implicitly.
    fn read_plan_union_depth(query: &str) -> usize {
        let mut groups: Vec<(usize, usize)> = Vec::new();
        let mut depth = 0;
        let bytes = query.as_bytes();
        for (i, byte) in bytes.iter().enumerate() {
            match byte {
                b'{' => groups.push((0, 0)),
                b'}' => {
                    let (unions, children) = groups.pop().unwrap();
                    assert!(unions <= 1, "read-plan UNION groups must be binary");
                    let nested = children + usize::from(unions == 1);
                    if let Some((_, maximum)) = groups.last_mut() {
                        *maximum = (*maximum).max(nested);
                    } else {
                        depth = depth.max(nested);
                    }
                }
                _ if bytes[i..].starts_with(b"UNION ") => groups.last_mut().unwrap().0 += 1,
                _ => {}
            }
        }
        assert!(groups.is_empty(), "read-plan groups must be balanced");
        depth
    }

    #[test]
    fn select_read_plan_bounds_union_nesting_and_retains_values() {
        let boundary = READ_PLAN_CONSTANT_CANDIDATES;
        for count in (0..=boundary + 1).chain([4096]) {
            let candidates: Vec<_> = (0..count)
                .map(|i| format!("https://pod.example/acl-{i}"))
                .collect();
            let query = select_read_plan("https://pod.example/doc", &candidates).unwrap();
            let depth = read_plan_union_depth(&query);
            if count <= boundary {
                let names = count + 1;
                let expected_depth = (usize::BITS - (names - 1).leading_zeros()) as usize;
                assert_eq!(
                    depth, expected_depth,
                    "logarithmic UNION nesting at candidate count {count}"
                );
                assert_eq!(
                    query.matches("GRAPH <").count(),
                    names,
                    "constant branches below the boundary"
                );
                assert!(
                    !query.contains("VALUES ?g"),
                    "constant form below the boundary"
                );
            } else {
                assert!(
                    query.contains("VALUES ?g {"),
                    "VALUES form above the boundary"
                );
                assert!(
                    query.contains("GRAPH ?g {"),
                    "variable graph in the VALUES form"
                );
                assert_eq!(depth, 0, "VALUES form has no UNION nesting");
            }
        }
        let target = "https://pod.example/.acl";
        let duplicates = vec![target.to_string(); boundary + 1];
        let query = select_read_plan(target, &duplicates).unwrap();
        assert!(
            query.contains("VALUES ?g {"),
            "walk length selects the query form"
        );
        assert_eq!(
            query.matches(&iri_const(target)).count(),
            1,
            "VALUES names remain deduplicated"
        );
    }

    #[test]
    fn select_read_plan_is_one_query_over_all_candidates() {
        // ONE SELECT carrying constant GRAPH branches for the whole per-read
        // metadata chain in one protocol request (§3.1).
        let candidates = vec![
            "http://pod/a/b/doc.acl".to_string(),
            "http://pod/a/b/.acl".to_string(),
            "http://pod/a/.acl".to_string(),
            "http://pod/.acl".to_string(),
        ];
        let q = select_read_plan("http://pod/a/b/doc", &candidates).unwrap();
        assert!(q.starts_with("SELECT ?g ?ct ?bk ?etag"), "{q}");
        assert!(!q.contains("GRAPH ?g"), "only constant graph branches: {q}");
        assert_eq!(
            q.matches("GRAPH <").count(),
            1 + candidates.len(),
            "one branch per name: {q}"
        );
        assert!(q.contains(&iri_const("http://pod/a/b/doc")), "target: {q}");
        for c in &candidates {
            assert!(q.contains(&iri_const(c)), "candidate {c}: {q}");
        }
        // Every branch binds the required reserved predicates.
        assert!(q.contains(&iri_const(&p_content_type())));
        assert!(q.contains(&iri_const(&p_blob_key())));
        assert!(q.contains(&iri_const(&p_etag())));
    }

    #[test]
    fn select_read_plan_dedups_a_target_that_is_its_own_candidate() {
        // A `.acl` target IS its own first candidate: evaluate its graph only once.
        let acl = "http://pod/a/doc.acl";
        let candidates = vec![acl.to_string(), "http://pod/a/.acl".to_string()];
        let q = select_read_plan(acl, &candidates).unwrap();
        assert_eq!(
            q.matches(&format!("GRAPH {}", iri_const(acl))).count(),
            1,
            "the duplicate graph is evaluated once: {q}"
        );
    }

    #[test]
    fn select_read_plan_rejects_an_invalid_iri_fail_closed() {
        // An IRIREF-invalid target OR candidate rejects the WHOLE build — never escaped-and-aliased
        // (the injection-safe builder discipline).
        assert_eq!(
            select_read_plan("http://pod/a b", &[]),
            Err(BuildError::InvalidIri)
        );
        assert_eq!(
            select_read_plan("http://pod/a", &["http://pod/> DROP".to_string()]),
            Err(BuildError::InvalidIri)
        );
    }

    #[test]
    fn insert_body_data_is_empty_for_no_triples() {
        assert_eq!(insert_body_data("http://e/r", &[]).unwrap(), "");
    }

    #[test]
    fn insert_body_data_rejects_an_invalid_iri() {
        let triples = vec![(
            "http://e/s".to_string(),
            "http://e/p".to_string(),
            BodyObject::Iri("http://e/o>x".into()),
        )];
        assert_eq!(
            insert_body_data("http://e/r", &triples),
            Err(BuildError::InvalidIri)
        );
    }

    #[test]
    fn insert_body_data_rejects_reserved_term_collisions() {
        // A body triple touching the reserved record subject, a `pss:` predicate, or a `pss:` IRI
        // object is rejected — untrusted RDF must not corrupt/spoof the index records that share the
        // resource's graph (the round-6 finding).
        let res = "http://e/r";
        // reserved subject
        let s = vec![(
            s_record(),
            "http://e/p".to_string(),
            BodyObject::PlainLiteral("x".into()),
        )];
        assert_eq!(
            insert_body_data(res, &s),
            Err(BuildError::ReservedTermCollision)
        );
        // reserved predicate
        let p = vec![(
            "http://e/s".to_string(),
            p_content_type(),
            BodyObject::PlainLiteral("x".into()),
        )];
        assert_eq!(
            insert_body_data(res, &p),
            Err(BuildError::ReservedTermCollision)
        );
        // reserved IRI object
        let o = vec![(
            "http://e/s".to_string(),
            "http://e/p".to_string(),
            BodyObject::Iri(s_record()),
        )];
        assert_eq!(
            insert_body_data(res, &o),
            Err(BuildError::ReservedTermCollision)
        );
        // a clean triple is fine.
        let ok = vec![(
            "http://e/s".to_string(),
            "http://e/p".to_string(),
            BodyObject::PlainLiteral("x".into()),
        )];
        assert!(insert_body_data(res, &ok).is_ok());
    }

    #[test]
    fn lang_tag_is_validated_fail_closed() {
        // Valid per the SPARQL LANGTAG grammar.
        assert_eq!(validate_lang("en"), Ok("en"));
        assert_eq!(validate_lang("en-US"), Ok("en-US"));
        assert_eq!(validate_lang("zh-Hans-CN"), Ok("zh-Hans-CN"));
        // Invalid: empty, whitespace/forbidden char, non-alpha primary, empty trailing/interior
        // subtag — all rejected fail-closed (round-7 finding).
        assert_eq!(validate_lang(""), Err(BuildError::InvalidLangTag));
        assert_eq!(validate_lang("en US"), Err(BuildError::InvalidLangTag));
        assert_eq!(validate_lang("en>"), Err(BuildError::InvalidLangTag));
        assert_eq!(validate_lang("1"), Err(BuildError::InvalidLangTag));
        assert_eq!(validate_lang("1x"), Err(BuildError::InvalidLangTag));
        assert_eq!(validate_lang("en-"), Err(BuildError::InvalidLangTag));
        assert_eq!(validate_lang("en--US"), Err(BuildError::InvalidLangTag));
        assert_eq!(validate_lang("-en"), Err(BuildError::InvalidLangTag));
    }

    /// A reclaim is named only while no record names the bytes, a hold's record included, on the
    /// engine `sparq-server` runs.
    #[cfg(all(feature = "embedded-sparq", not(target_arch = "wasm32")))]
    #[test]
    fn a_reclaim_begins_only_on_unreferenced_bytes_on_the_engine() {
        let mut g = sparq_core::Graph::load_str("", "ntriples").unwrap();
        let run = |g: &mut sparq_core::Graph, u: &str| sparq_engine::update_in_place_atomic(g, u);
        let ask = |g: &sparq_core::Graph, q: &str| sparq_engine::ask(g, q).unwrap();
        run(
            &mut g,
            &update_put_meta("urn:x-sparq-lws:held/a/b", "x", "k1", "\"h\"", None).unwrap(),
        )
        .unwrap();
        assert!(ask(&g, &ask_blob_referenced("k1")));
        run(&mut g, &update_begin_reclaim("k1")).unwrap();
        assert!(!ask(&g, &ask_reclaiming("k1")), "held bytes");
        run(&mut g, &update_begin_reclaim("k2")).unwrap();
        assert!(ask(&g, &ask_reclaiming("k2")));
        let rows = sparq_engine::query(&g, &select_reclaiming())
            .unwrap()
            .rows
            .len();
        assert_eq!(rows, 1);
        run(&mut g, &update_end_reclaim("k2")).unwrap();
        assert!(!ask(&g, &ask_reclaiming("k2")));
    }

    #[cfg(all(
        feature = "http-sparq",
        feature = "embedded-sparq",
        not(target_arch = "wasm32")
    ))]
    #[test]
    fn append_and_fold_builders_observe_nonce_fences_and_writer_epochs() {
        let child = "https://pod.example/c/d";
        let expected = super::super::ResourceMeta {
            content_type: "application/n-triples".into(),
            blob_key: "original".into(),
            etag: "old".into(),
            last_modified: None,
            append_seq: 3,
        };
        let receipt = "urn:x-sparq-lws:append-results/op-table";
        for protection in ["nonce", "epoch", "pre-start", "active"] {
            let epoch = if protection == "pre-start" { 0 } else { 1 };
            let writes = [
                (
                    "append",
                    update_publish_append(
                        child,
                        "original",
                        "insert",
                        "op-table",
                        "2020-01-01T00:00:00Z",
                        Some(epoch),
                    )
                    .unwrap(),
                ),
                (
                    "fold",
                    update_fold_appends(child, &expected, "folded", receipt, "op-table", epoch)
                        .unwrap(),
                ),
            ];
            for (name, update) in writes {
                let mut graph = sparq_core::Graph::load_str("", "ntriples").unwrap();
                let run = |g: &mut sparq_core::Graph, u: &str| {
                    sparq_engine::update_in_place_atomic(g, u).unwrap()
                };
                run(
                    &mut graph,
                    &update_put_meta_with_append_seq(
                        child,
                        &expected.content_type,
                        "original",
                        "old",
                        None,
                        3,
                    )
                    .unwrap(),
                );
                if epoch > 0 {
                    run(&mut graph, &update_begin_epoch(epoch, "start"));
                }
                match protection {
                    "nonce" => run(&mut graph, &update_fence("op-table")),
                    "epoch" | "pre-start" => run(&mut graph, &update_begin_epoch(2, "later-start")),
                    _ => {}
                }
                let snapshot = |g: &sparq_core::Graph| {
                    sparq_engine::query_json(
                        g,
                        "SELECT ?g ?s ?p ?o WHERE { GRAPH ?g { ?s ?p ?o } } ORDER BY ?g ?s ?p ?o",
                    )
                    .unwrap()
                };
                let before = snapshot(&graph);
                run(&mut graph, &update);
                let result =
                    sparq_engine::query(&graph, &select_append_receipt(receipt).unwrap()).unwrap();
                if protection == "active" {
                    assert_eq!(result.rows.len(), 1, "{name}: active publication receipt");
                    assert_ne!(snapshot(&graph), before, "{name}: active publication");
                } else {
                    assert!(result.rows.is_empty(), "{name}: {protection}");
                    assert_eq!(snapshot(&graph), before, "{name}: {protection}");
                }
            }
        }
    }

    fn fenced_writes(epoch: u64) -> Vec<(&'static str, String)> {
        let (parent, child, nonce) = (
            "https://pod.example/c/",
            "https://pod.example/c/d",
            "op-table",
        );
        vec![
            (
                "put",
                update_put_meta_fenced(child, "text/plain", "candidate", "new", None, nonce, epoch)
                    .unwrap(),
            ),
            (
                "delete",
                update_delete_resource_fenced(child, nonce, epoch).unwrap(),
            ),
            (
                "delete_parent",
                update_delete_resource_with_parent_fenced(child, Some(parent), nonce, epoch)
                    .unwrap(),
            ),
            (
                "detach",
                update_remove_child_fenced(parent, child, nonce, epoch).unwrap(),
            ),
            (
                "create",
                update_create_child_fenced(
                    parent,
                    child,
                    "text/plain",
                    "candidate",
                    "new",
                    None,
                    nonce,
                    epoch,
                )
                .unwrap(),
            ),
            (
                "empty_delete",
                update_delete_container_if_empty_fenced(child, Some(parent), nonce, epoch).unwrap(),
            ),
        ]
    }

    #[test]
    fn every_fenced_builder_carries_the_epoch_and_nonce_guards() {
        for (name, update) in fenced_writes(7) {
            let guard = update.split("WHERE {").last().unwrap();
            assert!(guard.contains("GRAPH <urn:x-sparq-lws:writer-epoch> { <urn:x-sparq-lws:writer-epoch> <urn:pss:index#epoch> 7 }"), "{name}: {update}");
            assert!(
                guard.contains("FILTER NOT EXISTS { GRAPH <urn:pss:index#fences>"),
                "{name}: {update}"
            );
            assert!(
                guard.contains("<urn:pss:index#fenced> \"op-table\""),
                "{name}: {update}"
            );
        }
    }

    #[cfg(all(feature = "embedded-sparq", not(target_arch = "wasm32")))]
    #[test]
    fn every_fenced_write_is_final_after_a_nonce_fence_or_a_new_epoch() {
        let (parent, child) = ("https://pod.example/c/", "https://pod.example/c/d");
        for protection in ["nonce", "epoch", "pre-start", "active"] {
            let epoch = if protection == "pre-start" { 0 } else { 1 };
            for (name, update) in fenced_writes(epoch) {
                let mut graph = sparq_core::Graph::load_str("", "ntriples").unwrap();
                let run = |g: &mut sparq_core::Graph, u: &str| {
                    sparq_engine::update_in_place_atomic(g, u).unwrap()
                };
                let ask = |g: &sparq_core::Graph, q: &str| sparq_engine::ask(g, q).unwrap();
                run(
                    &mut graph,
                    &update_put_meta(parent, "text/plain", "parent", "p", None).unwrap(),
                );
                run(
                    &mut graph,
                    &update_create_child(
                        parent,
                        child,
                        "text/plain",
                        "original",
                        "old",
                        Some("2020-01-01T00:00:00Z"),
                        "op-initial",
                    )
                    .unwrap(),
                );
                run(
                    &mut graph,
                    &format!("INSERT DATA {{ GRAPH <{child}> {{ <urn:s> <urn:p> \"body\" }} }}"),
                );
                if epoch > 0 {
                    run(&mut graph, &update_begin_epoch(1, "start-one"));
                }
                match protection {
                    "nonce" => run(&mut graph, &update_fence("op-table")),
                    "epoch" | "pre-start" => run(&mut graph, &update_begin_epoch(2, "start-two")),
                    _ => {}
                }
                run(&mut graph, &update);
                let marker = match name {
                    "create" => ask_create_marker(child, "op-table").unwrap(),
                    _ => ask_delete_marker(child, "op-table").unwrap(),
                };
                if protection == "active" {
                    assert!(ask(&graph, &marker), "{name}: active write did not commit");
                    match name {
                        "put" | "create" => {
                            let q = format!(
                                "ASK {{ GRAPH <{child}> {{ <{}> <{}> \"candidate\" }} }}",
                                s_record(),
                                p_blob_key()
                            );
                            assert!(ask(&graph, &q), "{name}");
                            assert!(
                                ask(
                                    &graph,
                                    &format!(
                                        "ASK {{ GRAPH <{child}> {{ <urn:s> <urn:p> \"body\" }} }}"
                                    )
                                ),
                                "body preserved"
                            );
                            assert!(
                                !ask(
                                    &graph,
                                    &format!(
                                        "ASK {{ GRAPH <{child}> {{ <{}> <{}> ?modified }} }}",
                                        s_record(),
                                        p_modified()
                                    )
                                ),
                                "stale timestamp cleared"
                            );
                        }
                        "detach" => {
                            assert!(ask(&graph, &ask_exists(child).unwrap()));
                            assert!(!ask(&graph, &ask_contains_edge(parent, child).unwrap()));
                        }
                        _ => {
                            assert!(!ask(&graph, &ask_exists(child).unwrap()), "{name}");
                            if name != "delete" {
                                assert!(!ask(&graph, &ask_contains_edge(parent, child).unwrap()));
                            }
                        }
                    }
                    if matches!(name, "delete" | "delete_parent") {
                        let result = sparq_engine::query(
                            &graph,
                            &select_deleted_blob_key("op-table").unwrap(),
                        )
                        .unwrap();
                        assert_eq!(
                            result.rows,
                            vec![vec![Some(
                                oxrdf::Literal::new_simple_literal("original").into()
                            )]],
                            "{name}: receipt captures the deleted record's exact key"
                        );
                        assert!(
                            !ask(&graph, &ask_blob_referenced("original")),
                            "receipt is not a live pointer"
                        );
                    }
                } else {
                    assert!(!ask(&graph, &marker), "{protection} {name}: marker");
                    assert!(
                        ask(&graph, &ask_exists(child).unwrap()),
                        "{protection} {name}: record"
                    );
                    assert!(
                        ask(&graph, &ask_contains_edge(parent, child).unwrap()),
                        "{protection} {name}: edge"
                    );
                    let q = format!(
                        "ASK {{ GRAPH <{child}> {{ <{}> <{}> \"original\" }} }}",
                        s_record(),
                        p_blob_key()
                    );
                    assert!(ask(&graph, &q), "{protection} {name}: original key");
                }
            }
        }
    }

    #[cfg(all(feature = "embedded-sparq", not(target_arch = "wasm32")))]
    #[test]
    fn epoch_raises_do_not_lower_a_later_start() {
        let mut graph = sparq_core::Graph::load_str("", "ntriples").unwrap();
        for epoch in [1, 2, 2, 1] {
            sparq_engine::update_in_place_atomic(&mut graph, &update_begin_epoch(epoch, "start"))
                .unwrap();
        }
        assert!(sparq_engine::ask(&graph,
            "ASK { GRAPH <urn:x-sparq-lws:writer-epoch> { <urn:x-sparq-lws:writer-epoch> <urn:pss:index#epoch> 2 } }").unwrap());
        assert!(!sparq_engine::ask(&graph,
            "ASK { GRAPH <urn:x-sparq-lws:writer-epoch> { <urn:x-sparq-lws:writer-epoch> <urn:pss:index#epoch> 1 } }").unwrap());
    }

    /// Issue #6836: the fenced create and delete do nothing once fenced, and as before otherwise,
    /// on the engine `sparq-server` runs (so the endpoint parses and applies them as written).
    #[cfg(all(feature = "embedded-sparq", not(target_arch = "wasm32")))]
    #[test]
    fn a_fenced_update_does_nothing_once_fenced_on_the_engine() {
        let mut g = sparq_core::Graph::load_str("", "ntriples").unwrap();
        let run = |g: &mut sparq_core::Graph, u: &str| sparq_engine::update_in_place_atomic(g, u);
        let ask = |g: &sparq_core::Graph, q: &str| sparq_engine::ask(g, q).unwrap();
        let (c, a, b) = ("http://pod/c/", "http://pod/c/a", "http://pod/c/b");
        run(
            &mut g,
            &format!(
                "INSERT DATA {{ GRAPH <{c}> {{ <{r}> <{ct}> \"text/turtle\" }} }}",
                r = s_record(),
                ct = p_content_type()
            ),
        )
        .unwrap();
        let create = |child: &str, nonce: &str| {
            update_create_child_fenced(c, child, "text/turtle", "bk", "\"e\"", None, nonce, 0)
                .unwrap()
        };
        // Not fenced: applied as the unfenced form is.
        run(&mut g, &create(a, "op-a")).unwrap();
        assert!(ask(&g, &ask_create_marker(a, "op-a").unwrap()));
        assert!(ask(&g, &ask_contains_edge(c, a).unwrap()));
        // Fenced first: nothing, whenever it arrives; the fence is idempotent.
        run(&mut g, &update_fence("op-b")).unwrap();
        run(&mut g, &update_fence("op-b")).unwrap();
        run(&mut g, &create(b, "op-b")).unwrap();
        assert!(!ask(&g, &ask_create_marker(b, "op-b").unwrap()));
        assert!(!ask(&g, &ask_exists(b).unwrap()));
        assert!(!ask(&g, &ask_contains_edge(c, b).unwrap()));
        // Another write's fence does not touch this one.
        run(&mut g, &create(b, "op-c")).unwrap();
        assert!(ask(&g, &ask_contains_edge(c, b).unwrap()));
        // A fenced delete, fenced: nothing; not fenced: as before.
        run(&mut g, &update_fence("op-d")).unwrap();
        run(
            &mut g,
            &update_delete_container_if_empty_fenced(a, Some(c), "op-d", 0).unwrap(),
        )
        .unwrap();
        assert!(ask(&g, &ask_exists(a).unwrap()));
        assert!(!ask(&g, &ask_delete_marker(a, "op-d").unwrap()));
        run(
            &mut g,
            &update_delete_container_if_empty_fenced(a, Some(c), "op-e", 0).unwrap(),
        )
        .unwrap();
        assert!(!ask(&g, &ask_exists(a).unwrap()));
        assert!(ask(&g, &ask_delete_marker(a, "op-e").unwrap()));
        assert!(!ask(&g, &ask_contains_edge(c, a).unwrap()));
    }
}
