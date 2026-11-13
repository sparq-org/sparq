//! Draft credential statement gadgets. Research grade; not externally audited.
//! No production descriptor or answers-data-model presentation is defined here.
use crate::suite_constants::{DT_DATETIME, DT_INTEGER, DT_STRING, SUITE_ID};
use compose::hashes::{commit_fold, h2, h3, h5};
use std::collections::bounded_vec::BoundedVec;
pub use compose::issuer::Point;
use crate::table::{append, canonicalize, check_cell, empty, equal_cell, same_term, Table, unbound};
use compose::issuer::schnorr_verify;

// Query-table representation; authenticated suite cells cross the adapter below.
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
global STATUS_LEAF: Field = 0x5643515f45534c46;
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
/// Copy authenticated suite observations into the query-table representation.
/// A bound suite default graph becomes the table's absent graph-name cell.
pub fn from_suite_cell(cell: crate::term::TermCell) -> TermCell {
    if !cell.bound() | (cell.kind() == 0) {
        if !cell.bound() {
            cell.assert_padding();
        }
        unbound()
    } else {
        TermCell {
            bound: cell.bound(),
            kind: cell.kind(),
            enc: cell.enc(),
            key_class: cell.key_class(),
            key: cell.key(),
            dt: cell.datatype(),
            lang: cell.language(),
            scope: cell.credential(),
        }
    }
}
/// Authenticated lexical constructor, using the suite's term encoding.
pub fn iri_bytes<let L: u32>(bytes: [u8; L]) -> TermCell {
    from_suite_cell(crate::term::open_term(
        1,
        BoundedVec::<u8, L>::from_array(bytes),
        BoundedVec::<u8, 0>::new(),
        BoundedVec::<u8, 0>::new(),
        [0; 31],
        0,
    ))
}
pub fn vocabulary() -> Vocabulary {
    comptime {
        Vocabulary {
            rdf_type: iri_bytes("http://www.w3.org/1999/02/22-rdf-syntax-ns#type".as_bytes()).enc,
            vc_type: iri_bytes("https://www.w3.org/2018/credentials#VerifiableCredential".as_bytes())
                .enc,
            issuer: iri_bytes("https://www.w3.org/2018/credentials#issuer".as_bytes()).enc,
            subject: iri_bytes(
                "https://www.w3.org/2018/credentials#credentialSubject".as_bytes(),
            )
                .enc,
            confidence: iri_bytes(
                "https://www.w3.org/2018/credentials#confidenceMethod".as_bytes(),
            )
                .enc,
            multikey: iri_bytes("https://w3id.org/security#Multikey".as_bytes()).enc,
            public_key: iri_bytes("https://w3id.org/security#publicKeyMultibase".as_bytes()).enc,
            credential_status: iri_bytes("https://www.w3.org/2018/credentials#credentialStatus"
                .as_bytes())
                .enc,
            status_entry: iri_bytes(
                "https://www.w3.org/ns/credentials/status#BitstringStatusListEntry".as_bytes(),
            )
                .enc,
            status_list: iri_bytes("https://www.w3.org/ns/credentials/status#statusListCredential"
                .as_bytes())
                .enc,
            status_index: iri_bytes("https://www.w3.org/ns/credentials/status#statusListIndex"
                .as_bytes())
                .enc,
            status_purpose: iri_bytes("https://www.w3.org/ns/credentials/status#statusPurpose"
                .as_bytes())
                .enc,
            status_size: iri_bytes(
                "https://www.w3.org/ns/credentials/status#statusSize".as_bytes(),
            )
                .enc,
            // Metadata literals below remain fixture values until their openers arrive.
            revocation: 114,
            suspension: 115,
            valid_from: iri_bytes("https://www.w3.org/2018/credentials#validFrom".as_bytes()).enc,
            valid_until: iri_bytes("https://www.w3.org/2018/credentials#validUntil".as_bytes()).enc,
            verified_issuer: iri_bytes("https://w3id.org/sparq/vcq#verifiedIssuer".as_bytes()).enc,
            integer_dt: DT_INTEGER,
            time_dt: DT_DATETIME,
            string_dt: DT_STRING,
            key_dt: iri_bytes("https://w3id.org/security#multibase".as_bytes()).enc,
            assertion_method: iri_bytes("https://w3id.org/security#assertionMethod".as_bytes()).enc,
            suite: SUITE_ID,
        }
    }
}
/// Reference to an already encoded IRI in a query table. Source cells must
/// originate in the suite opening; this helper authenticates no supplied bytes.
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
        dt: vocabulary().key_dt,
        lang: 0,
        scope: 0,
    }
}
// STAND-IN (lane A): replaced in step 2. W3C statusPurpose is a string literal.
pub fn stand_in_purpose(suspended: bool) -> TermCell {
    let v = vocabulary();
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
        enc: h3(0x45494e44, vocabulary().string_dt, value as Field),
        key_class: 0,
        key: 0,
        dt: vocabulary().string_dt,
        lang: 0,
        scope: 0,
    }
}
// STAND-IN (lane A): replaced in step 2. Metadata transport for fixture callers.
// Suite callers use open_suite_credential; root/method/creation metadata awaits openers.
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
    let v = vocabulary();
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
    let v = vocabulary();
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
    issuer_root: Field,
    issuer_depth: u32,
    sig: Signature,
    membership: Membership<K>,
) -> TermCell {
    stand_in_open_credential(c);
    let (_, issuer) = required_property(c, c.root, vocabulary().issuer);
    assert(
        issuer.bound
            & (issuer.kind == 1)
            & c.verification_method.bound
            & (c.verification_method.kind == 1),
        "issuer identity kind",
    );
    assert(
        set_merkle_root(
            trusted_key_leaf(issuer, c.verification_method, c.issuer_key),
            membership,
            issuer_depth,
        )
            == issuer_root,
        "issuer authorization set",
    );
    verify_signature(c.issuer_key, sig, c.signed_message);
    issuer
}
fn digest_halves(digest: [u8; 32]) -> [Field; 2] {
    let mut high: Field = 0;
    let mut low: Field = 0;
    for i in 0..16 {
        high = high * 256 + digest[i] as Field;
        low = low * 256 + digest[i + 16] as Field;
    }
    [high, low]
}
pub fn holder_message(request_digest: [u8; 32]) -> Field {
    let limbs = digest_halves(request_digest);
    h3(HOLDER_REQUEST, limbs[0], limbs[1])
}
// Scalar multiplication dominates the holder check.
pub fn check_holder<let N: u32>(
    c: Credential<N>,
    request_digest: [u8; 32],
    pk: Point,
    sig: Signature,
) {
    check_holder_key(c, pk);
    verify_signature(pk, sig, holder_message(request_digest));
}
pub fn check_holder_key<let N: u32>(c: Credential<N>, pk: Point) {
    stand_in_open_credential(c);
    let v = vocabulary();
    let (_, subject) = required_property(c, c.root, v.subject);
    assert(subject.kind != 3, "subject node kind");
    let (_, key_node) = required_property(c, subject, v.confidence);
    assert(key_node.kind != 3, "key node kind");
    assert(contains(c, key_node, v.rdf_type, iri(v.multikey)), "holder key type");
    let (_, key) = required_property(c, key_node, v.public_key);
    assert(equal_cell(key, stand_in_key_cell(pk)), "holder key binding");
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
pub fn snapshot_digest<let D: u32>(snapshot: Snapshot) -> Field {
    commit_fold(
        [
            SNAPSHOT,
            snapshot.list.enc,
            snapshot.purpose.enc,
            snapshot.bit_len as Field,
            D as Field,
            snapshot.root,
        ],
        6,
    )
}
// Dedicated tag and arity distinguish status leaves from two-input Merkle nodes.
pub fn status_leaf(bit: bool) -> Field {
    h3(STATUS_LEAF, bit as Field, 0)
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
// and curates snapshots; the leaf at the credential's W3C index hashes its tagged bit.
pub fn check_status<let N: u32, let D: u32, let A: u32>(
    c: Credential<N>,
    snapshot_root: Field,
    snapshot_depth: u32,
    allow_missing: bool,
    evidence: [StatusEvidence<D, A>; N],
) {
    stand_in_open_credential(c);
    let v = vocabulary();
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
                set_merkle_root(snapshot_digest::<D>(e.snapshot), e.accepted, snapshot_depth)
                    == snapshot_root,
                "snapshot policy",
            );
            assert(e.path.index == index as Field, "status index binding");
            assert(!e.bit, "status bit set");
            assert(merkle_root(status_leaf(e.bit), e.path) == e.snapshot.root, "status path");
        }
    }
    assert((count > 0) | allow_missing, "status entry absent");
}
pub fn check_validity<let N: u32>(c: Credential<N>, now: u64) {
    stand_in_open_credential(c);
    let v = vocabulary();
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
/// Public policy values, reconstructed by the verifier from its stored request.
/// Field order matches the authenticated layout's agreed policy group.
pub struct PublicPolicy {
    pub issuer_root: Field,
    pub issuer_depth: u32,
    pub holder_required: bool,
    pub holder_issuers_present: bool,
    pub holder_issuer_count: u32,
    pub holder_issuers: [Field; 4],
    pub status_required: bool,
    pub allow_missing_status: bool,
    pub snapshot_root: Field,
    pub snapshot_depth: u32,
    pub validity_required: bool,
    pub now: u64,
    pub freshness_required: bool,
    pub earliest_created: u64,
}
/// Disabled conditions have one canonical public encoding. The verifier emits
/// these words; this check derives neither the policy nor the request digest.
pub fn check_policy<let K: u32, let A: u32>(policy: PublicPolicy) {
    assert(policy.issuer_root != 0, "issuer root absent");
    assert(policy.issuer_depth <= K, "issuer depth capacity");
    assert(policy.snapshot_depth <= A, "snapshot depth capacity");
    assert(
        policy.holder_required
            | (
                !policy.holder_issuers_present
                    & (policy.holder_issuer_count == 0)
                    & (policy.holder_issuers == [0; 4])
            ),
        "disabled holder policy",
    );
    assert(
        policy.status_required
            | (
                !policy.allow_missing_status
                    & (policy.snapshot_root == 0)
                    & (policy.snapshot_depth == 0)
            ),
        "disabled status policy",
    );
    assert(!policy.status_required | (policy.snapshot_root != 0), "snapshot root absent");
    assert(policy.validity_required | (policy.now == 0), "disabled validity policy");
    assert(policy.freshness_required | (policy.earliest_created == 0), "disabled freshness policy");
    check_holder_policy(policy);
}
fn check_holder_policy(policy: PublicPolicy) {
    assert(policy.holder_issuer_count <= 4, "holder issuer capacity");
    assert(
        policy.holder_issuers_present | (policy.holder_issuer_count == 0),
        "holder issuer presence",
    );
    for i in 0..4 {
        let item = policy.holder_issuers[i];
        if i < policy.holder_issuer_count {
            assert(item != 0, "holder issuer encoding");
            if i > 0 {
                assert(
                    crate::term::field_lt(policy.holder_issuers[i - 1], item),
                    "holder issuer order",
                );
            }
        } else {
            assert(item == 0, "holder issuer padding");
        }
    }
}

/// A compiled maximum path opens a member at the public set-tree depth only.
/// Unused siblings and high index bits are canonical zero. Hashing dominates.
pub fn set_merkle_root<let M: u32>(leaf: Field, member: Membership<M>, depth: u32) -> Field {
    assert(depth <= M, "set depth capacity");
    let bits: [bool; M] = member.index.to_le_bits();
    let mut node = leaf;
    for i in 0..M {
        if i < depth {
            node = h2(
                if bits[i] { member.siblings[i] } else { node },
                if bits[i] { node } else { member.siblings[i] },
            );
        } else {
            assert(!bits[i] & (member.siblings[i] == 0), "set path padding");
        }
    }
    node
}

pub fn check_envelope<let N: u32, let K: u32, let D: u32, let A: u32>(
    c: Credential<N>,
    digest: [u8; 32],
    policy: PublicPolicy,
    issuer_sig: Signature,
    issuer_path: Membership<K>,
    holder_pk: Point,
    holder_sig: Signature,
    status: [StatusEvidence<D, A>; N],
) -> TermCell {
    check_policy::<K, A>(policy);
    let issuer = check_issuer(
        c,
        policy.issuer_root,
        policy.issuer_depth,
        issuer_sig,
        issuer_path,
    );
    if holder_applies(issuer, policy) {
        check_holder(c, digest, holder_pk, holder_sig);
    }
    if policy.status_required {
        check_status(
            c,
            policy.snapshot_root,
            policy.snapshot_depth,
            policy.allow_missing_status,
            status,
        );
    }
    if policy.validity_required {
        check_validity(c, policy.now);
    }
    check_freshness(c, policy.freshness_required, policy.earliest_created);
    issuer
}
// Exact per-credential layout; merge_datasets supplies the cross-credential set merge.
// STAND-IN (lane B): replaced in step 2. Generated wiring passes check_issuer's
// return directly here; this isolated layout helper is not issuer authentication.
pub fn credential_dataset<let N: u32, let O: u32>(
    c: Credential<N>,
    issuer: TermCell,
    presentation_scope: Field,
) -> Table<O, 4> {
    stand_in_open_credential(c);
    let (_, claimed_issuer) = required_property(c, c.root, vocabulary().issuer);
    assert(equal_cell(issuer, claimed_issuer), "dataset issuer binding");
    let graph = stand_in_graph_name(presentation_scope, c.credential_index);
    let mut out = empty::<O, 4>();
    for i in 0..N {
        let q = c.quads[i];
        out = append(out, q, i < c.len);
        out = append(out, [q[0], q[1], q[2], graph], i < c.len);
    }
    out = append(
        out,
        [graph, iri(vocabulary().verified_issuer), issuer, unbound()],
        true,
    );
    canonicalize(out)
}

// STAND-IN (lane A): replaced in step 2. Use the suite's scoped blank-node encoding.
pub fn stand_in_graph_name(presentation_scope: Field, index: u32) -> TermCell {
    TermCell {
        bound: true,
        kind: 2,
        enc: h3(GRAPH_NAME, presentation_scope, index as Field),
        key_class: 0,
        key: 0,
        dt: 0,
        lang: 0,
        scope: index + 1,
    }
}

/// Freshness is separate from validity; the verifier supplies the inclusive bound.
/// STAND-IN (lane A): replaced in step 2. created_at awaits its authenticated opener.
pub fn check_freshness<let N: u32>(c: Credential<N>, required: bool, earliest: u64) {
    stand_in_open_credential(c);
    if required {
        assert(c.created_at.bound, "creation time unavailable");
        assert(
            stand_in_checked_number(c.created_at, vocabulary().time_dt) >= earliest,
            "before earliestCreated",
        );
    }
}

pub fn holder_applies(issuer: TermCell, policy: PublicPolicy) -> bool {
    check_holder_policy(policy);
    let mut listed = false;
    for i in 0..4 {
        listed =
            listed | ((i < policy.holder_issuer_count) & (issuer.enc == policy.holder_issuers[i]));
    }
    policy.holder_required & (!policy.holder_issuers_present | listed)
}

pub struct HolderEvidence {
    pub pk: Point,
    pub sig: Signature,
}
/// Require one evidence slot for each distinct required key, with no spare slots.
/// Metadata binding still uses the explicitly marked Multikey opener above.
pub fn check_holders<let C: u32, let N: u32>(
    credentials: [Credential<N>; C],
    policy: PublicPolicy,
    digest: [u8; 32],
    slots: [u32; C],
    evidence: [HolderEvidence; C],
) {
    let mut used = [false; C];
    for i in 0..C {
        let (_, issuer) =
            required_property(credentials[i], credentials[i].root, vocabulary().issuer);
        if holder_applies(issuer, policy) {
            assert(slots[i] < C, "holder evidence slot");
            check_holder_key(credentials[i], evidence[slots[i]].pk);
            used[slots[i]] = true;
        }
    }
    for i in 0..C {
        assert(evidence[i].sig.present == used[i], "holder evidence coverage");
        if used[i] {
            for j in 0..i {
                assert(
                    !used[j]
                        | (evidence[i].pk.x != evidence[j].pk.x)
                        | (evidence[i].pk.y != evidence[j].pk.y),
                    "duplicate holder key",
                );
            }
            verify_signature(evidence[i].pk, evidence[i].sig, holder_message(digest));
        }
    }
}

/// Check every credential against public policy words. The public request digest
/// comes from the verifier; it is used here only for holder signatures.
pub fn check_public_envelope<let C: u32, let N: u32, let K: u32, let D: u32, let A: u32>(
    credentials: [Credential<N>; C],
    digest: [u8; 32],
    policy: PublicPolicy,
    signatures: [Signature; C],
    issuers: [Membership<K>; C],
    holders: [HolderEvidence; C],
    slots: [u32; C],
    status: [[StatusEvidence<D, A>; N]; C],
) -> [TermCell; C] {
    std::static_assert(C > 0, "credential capacity");
    check_policy::<K, A>(policy);
    let mut verified = [unbound(); C];
    for i in 0..C {
        let mut individual = policy;
        individual.holder_required = false;
        individual.holder_issuers_present = false;
        individual.holder_issuer_count = 0;
        individual.holder_issuers = [0; 4];
        verified[i] = check_envelope(
            credentials[i],
            digest,
            individual,
            signatures[i],
            issuers[i],
            holders[0].pk,
            holders[0].sig,
            status[i],
        );
    }
    check_holders(credentials, policy, digest, slots, holders);
    verified
}

/// Complete RDF set merge: collapse shared default triples while preserving
/// each credential's named graph and its one verified-issuer assertion.
pub fn merge_datasets<let C: u32, let I: u32, let O: u32>(
    datasets: [Table<I, 4>; C],
) -> Table<O, 4> {
    std::static_assert((C > 0) & (I > 0), "dataset merge capacity");
    for c in 0..C {
        crate::table::check_table(datasets[c]);
    }
    let mut out = empty::<O, 4>();
    let mut cursors = [0; C];
    let mut previous = [unbound(); 4];
    let mut has_previous = false;
    // Multiway merge over sorted complete inputs. Every round consumes one row;
    // adjacent equality collapses duplicates without re-sorting the combined table.
    for _ in 0..C * I {
        let mut found = false;
        let mut chosen = [unbound(); 4];
        let mut source: u32 = 0;
        for c in 0..C {
            if cursors[c] < datasets[c].len {
                let row = datasets[c].rows[cursors[c]];
                if !found | crate::table::row_less(row, chosen) {
                    chosen = row;
                    source = c;
                }
                found = true;
            }
        }
        if found {
            let duplicate = has_previous & crate::table::same_row(previous, chosen);
            out = append(out, chosen, !duplicate);
            previous = chosen;
            has_previous = true;
            cursors[source] = cursors[source] + 1;
        }
    }
    for c in 0..C {
        assert(cursors[c] == datasets[c].len, "dataset merge coverage");
    }
    out
}

/// Recompute the complete salted suite message and copy only opened terms into
/// the table. Metadata remains the marked transport until its openers arrive.
/// STAND-IN (lane A): replaced in step 2. Bind root, method and created_at metadata.
pub fn open_suite_credential<let N: u32>(
    quads: [[crate::term::TermCell; 4]; N],
    salts: [Field; N],
    salt: Field,
    count: u32,
    config: [u128; 2],
    mut metadata: Credential<N>,
) -> Credential<N> {
    metadata.signed_message =
        crate::suite::open_credential_revealed(quads, salts, salt, count, config);
    metadata.len = count;
    for i in 0..N {
        for j in 0..4 {
            metadata.quads[i][j] = from_suite_cell(quads[i][j]);
        }
    }
    stand_in_open_credential(metadata);
    metadata
}
