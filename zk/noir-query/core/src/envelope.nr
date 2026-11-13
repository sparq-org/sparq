//! Draft credential statement gadgets. Research grade; not externally audited.
//! No production descriptor or answers-data-model presentation is defined here.
use compose_core::hashes::{commit_fold, h2, h3, h4, h5};
pub use compose_core::issuer::Point;
use crate::table::{append, canonicalize, check_cell, empty, equal_cell, same_term, Table, unbound};
use compose_core::issuer::schnorr_verify;

// STAND-IN (lane A): replaced in step 2. Import the authenticated suite cell then.
pub use crate::table::TermCell;

pub struct Signature {
    pub present: bool,
    pub r: Point,
    pub s: Field,
    pub e: Field,
    pub e_k: Field,
}
pub struct Membership<let D: u32> {
    pub index: Field,
    pub siblings: [Field; D],
}
// Lane E domain separators for distinct draft profile commitments.
global KEY_SET_LEAF: Field = 0x5643515f454b4559;
global SNAPSHOT: Field = 0x5643515f45534e50;
global HOLDER_REQUEST: Field = 0x5643515f45484f4c;
global GRAPH_NAME: Field = 0x5643515f45475241;

pub struct Vocabulary {
    pub rdf_type: Field,
    pub vc_type: Field,
    pub issuer: Field,
    pub subject: Field,
    pub confidence: Field,
    pub multikey: Field,
    pub public_key: Field,
    pub credential_status: Field,
    pub status_entry: Field,
    pub status_list: Field,
    pub status_index: Field,
    pub status_purpose: Field,
    pub status_size: Field,
    pub revocation: Field,
    pub suspension: Field,
    pub valid_from: Field,
    pub valid_until: Field,
    pub verified_issuer: Field,
    pub integer_dt: Field,
    pub time_dt: Field,
    pub string_dt: Field,
    pub key_dt: Field,
    pub assertion_method: Field,
    pub suite: Field,
}
// STAND-IN (lane A): replaced in step 2. Synthetic encodings, not W3C term hashes.
pub fn stand_in_vocabulary() -> Vocabulary {
    Vocabulary {
        rdf_type: 101,
        vc_type: 102,
        issuer: 103,
        subject: 104,
        confidence: 105,
        multikey: 106,
        public_key: 107,
        credential_status: 108,
        status_entry: 109,
        status_list: 110,
        status_index: 111,
        status_purpose: 112,
        status_size: 113,
        revocation: 114,
        suspension: 115,
        valid_from: 116,
        valid_until: 117,
        verified_issuer: 118,
        integer_dt: 119,
        time_dt: 120,
        key_dt: 121,
        assertion_method: 122,
        suite: 123,
        string_dt: 124,
    }
}
// STAND-IN (lane A): replaced in step 2. A supplied field identifier, not an IRI opening.
pub fn iri(enc: Field) -> TermCell {
    TermCell { bound: true, kind: 1, enc, key_class: 0, key: 0, dt: 0, lang: 0, scope: 0 }
}
// STAND-IN (lane A): replaced in step 2. Whole-second offset/integer fixture cells.
// The suite must open lexical bytes and recompute exact typed keys without rounding.
pub fn stand_in_number(value: u64, datatype: Field) -> TermCell {
    TermCell {
        bound: true,
        kind: 3,
        enc: h3(0x454e554d, datatype, value as Field),
        key_class: 1,
        key: value as Field,
        dt: datatype,
        lang: 0,
        scope: 0,
    }
}
// STAND-IN (lane A): replaced in step 2. The final Multikey byte codec is lane A's.
pub fn stand_in_key_cell(pk: Point) -> TermCell {
    TermCell {
        bound: true,
        kind: 3,
        enc: h3(0x454b4559, pk.x, pk.y),
        key_class: 0,
        key: 0,
        dt: stand_in_vocabulary().key_dt,
        lang: 0,
        scope: 0,
    }
}
// STAND-IN (lane A): replaced in step 2. W3C statusPurpose is a string literal.
pub fn stand_in_purpose(suspended: bool) -> TermCell {
    let v = stand_in_vocabulary();
    TermCell {
        bound: true,
        kind: 3,
        enc: if suspended {
            v.suspension
        } else {
            v.revocation
        },
        key_class: 0,
        key: 0,
        dt: v.string_dt,
        lang: 0,
        scope: 0,
    }
}
// STAND-IN (lane A): replaced in step 2. Parse the W3C decimal index string opening.
pub fn stand_in_index(value: u64) -> TermCell {
    TermCell {
        bound: true,
        kind: 3,
        enc: h3(0x45494e44, stand_in_vocabulary().string_dt, value as Field),
        key_class: 0,
        key: 0,
        dt: stand_in_vocabulary().string_dt,
        lang: 0,
        scope: 0,
    }
}
// STAND-IN (lane A): replaced in step 2. No authenticated relation from these
// cells/root/key to signed_message exists in this step.
pub struct Credential<let N: u32> {
    pub quads: [[TermCell; 4]; N],
    pub len: u32,
    pub root: TermCell,
    pub credential_index: u32,
    pub verification_method: TermCell,
    pub issuer_key: Point,
    pub signed_message: Field,
    pub created_at: TermCell,
}
// STAND-IN (lane A): replaced in step 2. Structural opening, not authentication.
// The actual opening must authenticate every cell, count, root, key and message.
pub fn stand_in_open_credential<let N: u32>(c: Credential<N>) {
    let v = stand_in_vocabulary();
    assert(c.len <= N, "credential capacity");
    check_cell(c.root);
    check_cell(c.created_at);
    assert(c.root.bound & (c.root.kind != 3), "credential root");
    assert(c.credential_index < 0xffffffff, "credential scope capacity");
    for i in 0..N {
        for p in 0..4 {
            check_cell(c.quads[i][p]);
            assert((i < c.len) | equal_cell(c.quads[i][p], unbound()), "credential padding");
        }
        if i < c.len {
            assert(c.quads[i][0].bound & c.quads[i][1].bound & c.quads[i][2].bound, "quad bound");
            assert((c.quads[i][0].kind != 3) & (c.quads[i][1].kind == 1), "quad positions");
            assert(!c.quads[i][3].bound, "credential named graph");
            for j in 0..N {
                if j < i {
                    assert(
                        !(
                            same_term(c.quads[i][0], c.quads[j][0])
                                & same_term(c.quads[i][1], c.quads[j][1])
                                & same_term(c.quads[i][2], c.quads[j][2])
                        ),
                        "duplicate credential quad",
                    );
                }
            }
            assert(c.quads[i][1].enc != v.verified_issuer, "reserved issuer predicate");
            for p in 0..3 {
                assert(
                    (c.quads[i][p].kind != 2) | (c.quads[i][p].scope == c.credential_index + 1),
                    "blank scope",
                );
            }
        }
    }
    assert(contains(c, c.root, v.rdf_type, iri(v.vc_type)), "credential type");
}
pub fn contains<let N: u32>(c: Credential<N>, s: TermCell, p: Field, o: TermCell) -> bool {
    let mut found = false;
    for i in 0..N {
        found = found
            | (
                (i < c.len)
                    & same_term(c.quads[i][0], s)
                    & same_term(c.quads[i][1], iri(p))
                    & same_term(c.quads[i][2], o)
            );
    }
    found
}
// Complete singleton read; no witnessed presence flag, index or selected subset.
pub fn property<let N: u32>(
    c: Credential<N>,
    subject: TermCell,
    predicate: Field,
) -> (bool, TermCell) {
    let mut count: u32 = 0;
    let mut value = unbound();
    for i in 0..N {
        let hit = (i < c.len)
            & same_term(c.quads[i][0], subject)
            & same_term(c.quads[i][1], iri(predicate));
        count = count + hit as u32;
        if hit {
            value = c.quads[i][2];
        }
    }
    assert(count <= 1, "property cardinality");
    (count == 1, value)
}
pub fn required_property<let N: u32>(c: Credential<N>, s: TermCell, p: Field) -> (bool, TermCell) {
    let (present, value) = property(c, s, p);
    assert(present, "required property absent");
    (present, value)
}
fn verify_signature(pk: Point, sig: Signature, message: Field) {
    assert(sig.present, "signature evidence absent");
    schnorr_verify(pk, sig.r, sig.s, sig.e, sig.e_k, message);
}
// Directions and range come from the same index; hashing dominates this path.
pub fn merkle_root<let D: u32>(leaf: Field, member: Membership<D>) -> Field {
    let bits: [bool; D] = member.index.to_le_bits();
    let mut node = leaf;
    for i in 0..D {
        node = h2(
            if bits[i] { member.siblings[i] } else { node },
            if bits[i] { node } else { member.siblings[i] },
        );
    }
    node
}
// Binds issuer identity, verification method, suite, assertion authorization and key.
pub fn trusted_key_leaf(issuer: TermCell, method: TermCell, pk: Point) -> Field {
    let v = stand_in_vocabulary();
    h5(
        KEY_SET_LEAF,
        issuer.enc,
        method.enc,
        h3(v.suite, pk.x, pk.y),
        v.assertion_method,
    )
}
pub fn check_issuer<let N: u32, let K: u32>(
    c: Credential<N>,
    trusted_root: Field,
    sig: Signature,
    membership: Membership<K>,
) -> TermCell {
    stand_in_open_credential(c);
    let (_, issuer) = required_property(c, c.root, stand_in_vocabulary().issuer);
    assert(
        issuer.bound
            & (issuer.kind == 1)
            & c.verification_method.bound
            & (c.verification_method.kind == 1),
        "issuer identity kind",
    );
    assert(
        merkle_root(
            trusted_key_leaf(issuer, c.verification_method, c.issuer_key),
            membership,
        )
            == trusted_root,
        "issuer authorization set",
    );
    verify_signature(c.issuer_key, sig, c.signed_message);
    issuer
}
pub fn holder_message(request_digest: [u128; 2]) -> Field {
    h3(
        HOLDER_REQUEST,
        request_digest[0] as Field,
        request_digest[1] as Field,
    )
}
// Scalar multiplication dominates the holder check.
pub fn check_holder<let N: u32>(
    c: Credential<N>,
    request_digest: [u128; 2],
    pk: Point,
    sig: Signature,
) {
    stand_in_open_credential(c);
    let v = stand_in_vocabulary();
    let (_, subject) = required_property(c, c.root, v.subject);
    assert(subject.kind != 3, "subject node kind");
    let (_, key_node) = required_property(c, subject, v.confidence);
    assert(key_node.kind != 3, "key node kind");
    assert(contains(c, key_node, v.rdf_type, iri(v.multikey)), "holder key type");
    let (_, key) = required_property(c, key_node, v.public_key);
    assert(equal_cell(key, stand_in_key_cell(pk)), "holder key binding");
    verify_signature(pk, sig, holder_message(request_digest));
}
// STAND-IN (lane A): replaced in step 2. Bind a value to its synthetic typed term.
fn stand_in_checked_number(cell: TermCell, dt: Field) -> u64 {
    let value = cell.key as u64;
    assert(equal_cell(cell, stand_in_number(value, dt)), "typed number opening");
    value
}
pub struct Snapshot {
    pub list: TermCell,
    pub purpose: TermCell,
    pub bit_len: u32,
    pub root: Field,
}
pub fn snapshot_digest(snapshot: Snapshot) -> Field {
    h5(
        SNAPSHOT,
        snapshot.list.enc,
        snapshot.purpose.enc,
        snapshot.bit_len as Field,
        snapshot.root,
    )
}
pub struct StatusEvidence<let D: u32, let A: u32> {
    pub present: bool,
    pub snapshot: Snapshot,
    pub accepted: Membership<A>,
    pub bit: bool,
    pub path: Membership<D>,
}
// Each credential-root status row must be covered, including rows unused by the query.
// Snapshot construction uses W3C left-to-right bit order. The verifier authenticates
// and curates snapshots; the leaf at the credential's W3C index is its status bit.
pub fn check_status<let N: u32, let D: u32, let A: u32>(
    c: Credential<N>,
    accepted_root: Field,
    allow_missing: bool,
    evidence: [StatusEvidence<D, A>; N],
) {
    stand_in_open_credential(c);
    let v = stand_in_vocabulary();
    assert(D <= 32, "status tree depth");
    let mut tree_capacity: Field = 1;
    for _ in 0..D {
        tree_capacity = tree_capacity * 2;
    }
    let mut count: u32 = 0;
    for i in 0..N {
        let hit = (i < c.len)
            & same_term(c.quads[i][0], c.root)
            & same_term(c.quads[i][1], iri(v.credential_status));
        assert(evidence[i].present == hit, "status evidence coverage");
        count = count + hit as u32;
        if hit {
            let entry = c.quads[i][2];
            assert(entry.kind != 3, "status entry kind");
            assert(contains(c, entry, v.rdf_type, iri(v.status_entry)), "status entry type");
            let (_, list) = required_property(c, entry, v.status_list);
            assert(list.kind == 1, "status list kind");
            let (_, purpose) = required_property(c, entry, v.status_purpose);
            assert(
                equal_cell(purpose, stand_in_purpose(false))
                    | equal_cell(purpose, stand_in_purpose(true)),
                "status purpose",
            );
            let (_, index_cell) = required_property(c, entry, v.status_index);
            let index = evidence[i].path.index as u64;
            assert(equal_cell(index_cell, stand_in_index(index)), "status index binding");
            let (size_present, size) = property(c, entry, v.status_size);
            if size_present {
                assert(stand_in_checked_number(size, v.integer_dt) == 1, "status size");
            }
            let e = evidence[i];
            assert(
                equal_cell(e.snapshot.list, list) & equal_cell(e.snapshot.purpose, purpose),
                "snapshot identity",
            );
            assert(index < e.snapshot.bit_len as u64, "status index length");
            assert((e.snapshot.bit_len as Field).lt(tree_capacity + 1), "snapshot capacity");
            assert(
                merkle_root(snapshot_digest(e.snapshot), e.accepted) == accepted_root,
                "snapshot policy",
            );
            assert(e.path.index == index as Field, "status index binding");
            assert(!e.bit, "status bit set");
            assert(merkle_root(e.bit as Field, e.path) == e.snapshot.root, "status path");
        }
    }
    assert((count > 0) | allow_missing, "status entry absent");
}
pub fn check_validity<let N: u32>(c: Credential<N>, now: u64) {
    stand_in_open_credential(c);
    let v = stand_in_vocabulary();
    let (has_from, from) = property(c, c.root, v.valid_from);
    let (has_until, until) = property(c, c.root, v.valid_until);
    let mut start: u64 = 0;
    let mut end: u64 = 0xffffffffffffffff;
    if has_from {
        start = stand_in_checked_number(from, v.time_dt);
    }
    if has_until {
        end = stand_in_checked_number(until, v.time_dt);
    }
    // VC DM assumes creation as the start when both explicit bounds are absent.
    // The suite opening must authenticate this metadata; unknown creation refuses.
    if !has_from & !has_until {
        assert(c.created_at.bound, "creation time unavailable");
        start = stand_in_checked_number(c.created_at, v.time_dt);
    }
    assert(start <= end, "validity bounds reversed");
    assert(start <= now, "before validFrom");
    assert(now <= end, "after validUntil");
}
// STAND-IN (lane B): replaced in step 2. Derive this from the stored request.
pub struct RequestPolicy {
    pub trusted_root: Field,
    pub holder_required: bool,
    pub status_required: bool,
    pub allow_missing_status: bool,
    pub accepted_snapshots: Field,
    pub validity_required: bool,
    pub now: u64,
}
// STAND-IN (lane B): replaced in step 2. Local fixture, NOT RDFC/SHA-256.
// Query, domain, challenge and every policy field enter; both digest limbs are kept.
pub fn stand_in_request_digest(
    query: Field,
    domain: Field,
    challenge: Field,
    policy: RequestPolicy,
) -> [u128; 2] {
    let digest = commit_fold(
        [
            0x5643515f45524551,
            query,
            domain,
            challenge,
            policy.trusted_root,
            policy.holder_required as Field,
            policy.status_required as Field,
            policy.allow_missing_status as Field,
            policy.accepted_snapshots,
            policy.validity_required as Field,
            policy.now as Field,
        ],
        11,
    );
    let bits: [bool; 254] = digest.to_le_bits();
    let mut low: u128 = 0;
    let mut high: u128 = 0;
    for i in 0..128 {
        low = low + (bits[i] as u128) * (1_u128 << (i as u128));
    }
    for i in 0..126 {
        high = high + (bits[128 + i] as u128) * (1_u128 << (i as u128));
    }
    [high, low]
}
pub fn check_envelope<let N: u32, let K: u32, let D: u32, let A: u32>(
    c: Credential<N>,
    digest: [u128; 2],
    policy: RequestPolicy,
    issuer_sig: Signature,
    issuer_path: Membership<K>,
    holder_pk: Point,
    holder_sig: Signature,
    status: [StatusEvidence<D, A>; N],
) -> TermCell {
    let issuer = check_issuer(c, policy.trusted_root, issuer_sig, issuer_path);
    if policy.holder_required {
        check_holder(c, digest, holder_pk, holder_sig);
    }
    if policy.status_required {
        check_status(
            c,
            policy.accepted_snapshots,
            policy.allow_missing_status,
            status,
        );
    }
    if policy.validity_required {
        check_validity(c, policy.now);
    }
    issuer
}
// Exact per-credential layout; cross-credential merge/dedup is operator integration.
// STAND-IN (lane B): replaced in step 2. Generated wiring passes check_issuer's
// return directly here; this isolated layout helper is not issuer authentication.
pub fn credential_dataset<let N: u32, let O: u32>(
    c: Credential<N>,
    issuer: TermCell,
    digest: [u128; 2],
) -> Table<O, 4> {
    stand_in_open_credential(c);
    let (_, claimed_issuer) = required_property(c, c.root, stand_in_vocabulary().issuer);
    assert(equal_cell(issuer, claimed_issuer), "dataset issuer binding");
    let graph = stand_in_graph_name(digest, c.credential_index);
    let mut out = empty::<O, 4>();
    for i in 0..N {
        let q = c.quads[i];
        out = append(out, q, i < c.len);
        out = append(out, [q[0], q[1], q[2], graph], i < c.len);
    }
    out = append(
        out,
        [graph, iri(stand_in_vocabulary().verified_issuer), issuer, unbound()],
        true,
    );
    canonicalize(out)
}

// STAND-IN (lane A): replaced in step 2. Use the suite's scoped blank-node encoding.
pub fn stand_in_graph_name(digest: [u128; 2], index: u32) -> TermCell {
    TermCell {
        bound: true,
        kind: 2,
        enc: h4(
            GRAPH_NAME,
            digest[0] as Field,
            digest[1] as Field,
            index as Field,
        ),
        key_class: 0,
        key: 0,
        dt: 0,
        lang: 0,
        scope: index + 1,
    }
}
