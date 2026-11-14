//! Authenticated credential statement gadgets. Research grade; not externally audited.
//! Public policy words and request digest are reconstructed by the verifier.
use compose::hashes::{commit_fold, h2, h3, h5};
use compose::issuer::schnorr_verify;
use std::collections::bounded_vec::BoundedVec;
pub use crate::suite_metadata::OpenedCredential;
pub use compose::issuer::Point;
use crate::suite_entry::TermOpening;
use crate::suite_metadata::{multikey_cell, open_status_index, open_status_purpose, open_time};
use crate::table::{empty, Table, unbound};
use crate::term::{same_term, TermCell};

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
// Distinct domains for trust members, snapshots, status leaves and holder messages.
global KEY_SET_LEAF: Field = 0x5643515f454b4559;
global SNAPSHOT: Field = 0x5643515f45534e50;
global STATUS_LEAF: Field = 0x5643515f45534c46;
global HOLDER_REQUEST: Field = 0x5643515f45484f4c;

/// Private raw suite evidence. Every observation is derived by the suite opener.
pub struct CredentialOpening<let N: u32, let L: u32> {
    pub raw: [[TermOpening<L>; 4]; N],
    pub salts: [Field; N],
    pub salt: Field,
    pub count: u32,
    pub configuration: [u128; 2],
    pub created: TermOpening<32>,
    pub method: TermOpening<128>,
    pub pk: Point,
    pub signature: Signature,
}
pub fn open_suite_credential<let N: u32, let L: u32>(
    proof: CredentialOpening<N, L>,
    credential_index: u32,
) -> OpenedCredential<N> {
    assert(proof.signature.present, "signature evidence absent");
    let sig = proof.signature;
    let c = crate::suite::open_credential_hidden(
        proof.raw,
        proof.salts,
        proof.salt,
        proof.count,
        proof.configuration,
        proof.created,
        proof.method,
        proof.pk,
        credential_index,
        sig.r,
        sig.s,
        sig.e,
        sig.e_k,
    );
    check_credential(c);
    c
}
/// Structural envelope restrictions over the complete authenticated table.
pub fn check_credential<let N: u32>(c: OpenedCredential<N>) {
    let rows = c.quads();
    for i in 0..N {
        if i < c.count() {
            assert(rows[i][3].kind() == 0, "credential named graph");
            assert(
                !same_term(rows[i][1], verified_issuer_predicate()),
                "reserved issuer predicate",
            );
        }
    }
    let subject = c.credential_subject();
    assert(subject.bound() & (subject.kind() != 3), "credential root");
}
pub fn from_suite_cell(cell: TermCell) -> crate::table::TermCell {
    if !cell.bound() | (cell.kind() == 0) {
        unbound()
    } else {
        crate::table::TermCell {
            bound: cell.bound(),
            kind: cell.kind(),
            enc: cell.identity(),
            key_class: cell.key_class(),
            key: cell.key(),
            dt: cell.datatype(),
            lang: cell.language(),
            scope: cell.credential(),
        }
    }
}
pub fn iri_bytes<let L: u32>(bytes: [u8; L]) -> TermCell {
    crate::term::open_term(
        1,
        BoundedVec::<u8, L>::from_array(bytes),
        BoundedVec::<u8, 0>::new(),
        BoundedVec::<u8, 0>::new(),
        [0; 31],
        0,
    )
}
pub fn confidence_predicate() -> TermCell {
    crate::term::iri_confidence_method()
}
pub fn verified_issuer_predicate() -> TermCell {
    crate::term::iri_verified_issuer()
}
pub fn required_property<let N: u32>(c: OpenedCredential<N>, s: TermCell, p: TermCell) -> TermCell {
    let value = c.property(s, p);
    assert(value.bound(), "required property absent");
    value
}
pub fn contains<let N: u32>(c: OpenedCredential<N>, s: TermCell, p: TermCell, o: TermCell) -> bool {
    let mut found = false;
    let rows = c.quads();
    for i in 0..N {
        found = found
            | (
                (i < c.count())
                    & same_term(rows[i][0], s)
                    & same_term(rows[i][1], p)
                    & same_term(rows[i][2], o)
            );
    }
    found
}
fn verify_signature(pk: Point, sig: Signature, message: Field) {
    assert(sig.present, "signature evidence absent");
    schnorr_verify(pk, sig.r, sig.s, sig.e, sig.e_k, message);
}
// Directions and range come from one index; hashing dominates the path.
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
/// Same fields and hash arities as envelope_policy::trust_member_digest.
/// The method and Multikey come from the signed suite metadata.
pub fn trusted_key_leaf(issuer: TermCell, method: TermCell, pk: Point) -> Field {
    h5(
        KEY_SET_LEAF,
        issuer.enc(),
        method.enc(),
        h3(crate::suite_constants::SUITE_ID, pk.x, pk.y),
        crate::term::iri_assertion_method().enc(),
    )
}
pub fn check_issuer<let N: u32, let K: u32>(
    c: OpenedCredential<N>,
    issuer_root: Field,
    issuer_depth: u32,
    pk: Point,
    membership: Membership<K>,
) -> TermCell {
    check_credential(c);
    let issuer = required_property(c, c.credential_subject(), crate::term::iri_issuer());
    let method = c.verification_method();
    assert((issuer.kind() == 1) & method.bound() & (method.kind() == 1), "issuer identity kind");
    checked_multikey(c.verification_key(), pk);
    assert(
        set_merkle_root(
            trusted_key_leaf(issuer, method, pk),
            membership,
            issuer_depth,
        )
            == issuer_root,
        "issuer authorization set",
    );
    issuer
}
fn digest_halves(digest: [u8; 32]) -> [u128; 2] {
    let mut limbs: [u128; 2] = [0; 2];
    for i in 0..32 {
        limbs[i / 16] = limbs[i / 16] * 256 + digest[i] as u128;
    }
    limbs
}
pub fn holder_message(request_digest: [u8; 32]) -> Field {
    let limbs = digest_halves(request_digest);
    h3(HOLDER_REQUEST, limbs[0] as Field, limbs[1] as Field)
}
// Key-independent RDF equality does not check the signed value key. The
// canonical constructor supplies both identity and the complete keyed encoding.
fn checked_multikey(cell: TermCell, pk: Point) {
    let expected = multikey_cell(pk);
    assert(
        same_term(cell, expected) & (cell.enc() == expected.enc()),
        "verification key opening differs",
    );
}
pub fn check_holder_key<let N: u32>(c: OpenedCredential<N>, pk: Point) {
    check_credential(c);
    let subject = required_property(c, c.credential_subject(), crate::term::iri_subject());
    assert(subject.kind() != 3, "subject node kind");
    let node = required_property(c, subject, confidence_predicate());
    assert(node.kind() != 3, "key node kind");
    assert(
        contains(
            c,
            node,
            crate::term::iri_rdf_type(),
            crate::term::iri_multikey(),
        ),
        "holder key type",
    );
    let key = required_property(c, node, crate::term::iri_public_key());
    checked_multikey(key, pk);
}
pub fn check_holder<let N: u32>(
    c: OpenedCredential<N>,
    digest: [u8; 32],
    pk: Point,
    sig: Signature,
) {
    check_holder_key(c, pk);
    verify_signature(pk, sig, holder_message(digest));
}
pub struct Snapshot {
    pub list: TermCell,
    pub purpose: TermCell,
    pub bit_len: u32,
    pub root: Field,
}
pub fn snapshot_digest<let D: u32>(s: Snapshot) -> Field {
    snapshot_digest_at_depth(s, D)
}
/// Encode the actual bit-tree depth; the status consumer constrains its bound.
pub fn snapshot_digest_at_depth(s: Snapshot, depth: u32) -> Field {
    commit_fold(
        [SNAPSHOT, s.list.enc(), s.purpose.enc(), s.bit_len as Field, depth as Field, s.root],
        6,
    )
}
pub fn status_leaf(bit: bool) -> Field {
    h3(STATUS_LEAF, bit as Field, 0)
}
// Protocol digests use keyed encodings even when RDF equality uses identity.
// The credential purpose has already passed its checked lexical constructor.
fn check_snapshot_identity(snapshot: Snapshot, list: TermCell, purpose: TermCell) {
    assert(
        same_term(snapshot.list, list) & same_term(snapshot.purpose, purpose)
            & (snapshot.list.enc() == list.enc()) & (snapshot.purpose.enc() == purpose.enc()),
        "snapshot identity",
    );
}
pub struct StatusEvidence<let D: u32, let A: u32> {
    pub present: bool,
    pub snapshot: Snapshot,
    pub accepted: Membership<A>,
    pub bit: bool,
    pub path: Membership<D>,
    pub purpose: BoundedVec<u8, 16>,
    pub index: BoundedVec<u8, 20>,
    pub size: BoundedVec<u8, 20>,
}
/// Private status evidence with a bounded actual bit-tree depth.
/// Accepted-set depth remains a separate public policy value.
pub struct BoundedStatusEvidence<let MAX: u32, let A: u32> {
    pub depth: u32,
    pub present: bool,
    pub snapshot: Snapshot,
    pub accepted: Membership<A>,
    pub bit: bool,
    pub path: Membership<MAX>,
    pub purpose: BoundedVec<u8, 16>,
    pub index: BoundedVec<u8, 20>,
    pub size: BoundedVec<u8, 20>,
}
/// Reduce only actual bit-path levels; unused siblings and index bits are zero.
/// Hashing dominates the bounded path. The maximum also bounds decomposition.
pub fn status_merkle_root<let MAX: u32>(leaf: Field, member: Membership<MAX>, depth: u32) -> Field {
    assert(depth <= MAX, "status depth capacity");
    let bits: [bool; MAX] = member.index.to_le_bits();
    let mut node = leaf;
    for level in 0..MAX {
        if level < depth {
            node = h2(
                if bits[level] {
                    member.siblings[level]
                } else {
                    node
                },
                if bits[level] {
                    node
                } else {
                    member.siblings[level]
                },
            );
        } else {
            assert(!bits[level], "status index depth");
            assert(member.siblings[level] == 0, "status path padding");
        }
    }
    node
}
fn fixed_status_evidence<let D: u32, let A: u32>(
    e: StatusEvidence<D, A>,
) -> BoundedStatusEvidence<D, A> {
    BoundedStatusEvidence {
        depth: D,
        present: e.present,
        snapshot: e.snapshot,
        accepted: e.accepted,
        bit: e.bit,
        path: e.path,
        purpose: e.purpose,
        index: e.index,
        size: e.size,
    }
}
/// Fixed-depth compatibility entry; all status constraints live in the bounded relation.
pub fn check_status<let N: u32, let D: u32, let A: u32>(
    c: OpenedCredential<N>,
    snapshot_root: Field,
    snapshot_depth: u32,
    allow_missing: bool,
    evidence: [StatusEvidence<D, A>; N],
) {
    let mut bounded = [fixed_status_evidence(evidence[0]); N];
    for i in 0..N {
        bounded[i] = fixed_status_evidence(evidence[i]);
    }
    check_status_bounded(c, snapshot_root, snapshot_depth, allow_missing, bounded);
}
/// One authenticated status entry: metadata openings, snapshot policy and clear bit.
fn check_status_entry<let N: u32, let MAX: u32, let A: u32>(
    c: OpenedCredential<N>,
    entry: TermCell,
    snapshot_root: Field,
    snapshot_set_depth: u32,
    e: BoundedStatusEvidence<MAX, A>,
) {
    assert(e.depth <= MAX, "status depth capacity");
    let mut capacity: u64 = 1;
    for level in 0..MAX {
        if level < e.depth {
            capacity = capacity * 2;
        }
    }
    assert(entry.kind() != 3, "status entry kind");
    assert(
        contains(
            c,
            entry,
            crate::term::iri_rdf_type(),
            crate::term::iri_status_entry(),
        ),
        "status entry type",
    );
    let list = required_property(c, entry, crate::term::iri_status_list());
    assert(list.kind() == 1, "status list kind");
    let purpose = required_property(c, entry, crate::term::iri_status_purpose());
    let _suspended = open_status_purpose(purpose, e.purpose);
    let index_cell = required_property(c, entry, crate::term::iri_status_index());
    let index = open_status_index(index_cell, e.index);
    let size = c.property(entry, crate::term::iri_status_size());
    if size.bound() {
        let checked = crate::term::recompute_key_for(
            size,
            e.size,
            BoundedVec::<u8, 64>::from_array("http://www.w3.org/2001/XMLSchema#integer"
                .as_bytes()),
            BoundedVec::<u8, 0>::new(),
            crate::suite_constants::DT_INTEGER,
        )
            .cell();
        // Compare against the suite's exact key for integer one.
        let (_, one) = crate::term_key::recompute(
            BoundedVec::<u8, 1>::from_array([49]),
            crate::suite_constants::DT_INTEGER,
        );
        assert(checked.key() == one, "status size");
    }
    check_snapshot_identity(e.snapshot, list, purpose);
    assert(index < e.snapshot.bit_len as u64, "status index length");
    assert(e.path.index == index as Field, "status index binding");
    let path_root = status_merkle_root(status_leaf(e.bit), e.path, e.depth);
    assert(e.snapshot.bit_len as u64 <= capacity, "snapshot capacity");
    assert(
        set_merkle_root(
            snapshot_digest_at_depth(e.snapshot, e.depth),
            e.accepted,
            snapshot_set_depth,
        )
            == snapshot_root,
        "snapshot policy",
    );
    assert(!e.bit, "status bit set");
    assert(path_root == e.snapshot.root, "status path");
}
/// Cover every signed status row, with no witnessed absence or selected subset.
pub fn check_status_bounded<let N: u32, let MAX: u32, let A: u32>(
    c: OpenedCredential<N>,
    snapshot_root: Field,
    snapshot_set_depth: u32,
    allow_missing: bool,
    evidence: [BoundedStatusEvidence<MAX, A>; N],
) {
    check_credential(c);
    assert(MAX <= 32, "status tree depth");
    let mut count: u32 = 0;
    let rows = c.quads();
    let subject = c.credential_subject();
    for i in 0..N {
        let hit = (i < c.count())
            & same_term(rows[i][0], subject)
            & same_term(rows[i][1], crate::term::iri_credential_status());
        assert(evidence[i].present == hit, "status evidence coverage");
        count = count + hit as u32;
        if hit {
            check_status_entry(c, rows[i][2], snapshot_root, snapshot_set_depth, evidence[i]);
        }
    }
    assert((count > 0) | allow_missing, "status entry absent");
}
/// Status evidence for one selected signed row.
pub struct SlotStatusEvidence<let MAX: u32, let A: u32> {
    pub row: u32,
    pub status: BoundedStatusEvidence<MAX, A>,
}
/// Slot form for a verifier-selected entry capacity S. One identity pass counts every
/// signed status row; present slots name distinct rows in increasing order, so the
/// count equality covers every row. More rows than S cannot be proved.
pub fn check_status_slots<let N: u32, let S: u32, let MAX: u32, let A: u32>(
    c: OpenedCredential<N>,
    snapshot_root: Field,
    snapshot_set_depth: u32,
    allow_missing: bool,
    slots: [SlotStatusEvidence<MAX, A>; S],
) {
    check_credential(c);
    assert(MAX <= 32, "status tree depth");
    let rows = c.quads();
    let subject = c.credential_subject();
    let mut hits = [false; N];
    let mut count: u32 = 0;
    for i in 0..N {
        let hit = (i < c.count())
            & same_term(rows[i][0], subject)
            & same_term(rows[i][1], crate::term::iri_credential_status());
        hits[i] = hit;
        count = count + hit as u32;
    }
    let mut present: u32 = 0;
    let mut open = true;
    let mut last: u32 = 0;
    for s in 0..S {
        let slot = slots[s];
        let used = slot.status.present;
        assert(!used | open, "status slot order");
        if used {
            assert(slot.row < N, "status row range");
            assert((present == 0) | (slot.row > last), "status row order");
            assert(hits[slot.row], "status row");
            check_status_entry(
                c,
                rows[slot.row][2],
                snapshot_root,
                snapshot_set_depth,
                slot.status,
            );
            last = slot.row;
            present = present + 1;
        } else {
            assert((slot.row == 0) & (slot.status.depth == 0), "status slot padding");
        }
        open = used;
    }
    assert(present == count, "status evidence coverage");
    assert((count > 0) | allow_missing, "status entry absent");
}
/// Lexical witnesses open the authenticated bound cells; absence comes from all quads.
pub struct ValidityOpening {
    pub from: BoundedVec<u8, 32>,
    pub until: BoundedVec<u8, 32>,
}
/// Public instants are exact nonnegative Unix epoch milliseconds. Signed metadata
/// remains i64, so pre-epoch bounds compare without an unsigned cast or rounding.
pub fn check_validity<let N: u32>(c: OpenedCredential<N>, now: u64, lexical: ValidityOpening) {
    check_credential(c);
    assert(now <= 0x7fffffffffffffff, "time range");
    let now = now as i64;
    let from_cell = c.valid_from();
    let until_cell = c.valid_until();
    let from = open_time(from_cell, lexical.from);
    let until = open_time(until_cell, lexical.until);
    let mut start: i64 = -9223372036854775808;
    let mut end: i64 = 9223372036854775807;
    if from_cell.bound() {
        assert(from.known(), "validFrom unavailable");
        start = from.millis();
    }
    if until_cell.bound() {
        assert(until.known(), "validUntil unavailable");
        end = until.millis();
    }
    if !from_cell.bound() & !until_cell.bound() {
        assert(c.created().known(), "creation time unavailable");
        start = c.created().millis();
    }
    assert(start <= end, "validity bounds reversed");
    assert(start <= now, "before validFrom");
    assert(now <= end, "after validUntil");
}
pub fn check_freshness<let N: u32>(c: OpenedCredential<N>, required: bool, earliest: u64) {
    check_credential(c);
    if required {
        assert(earliest <= 0x7fffffffffffffff, "time range");
        assert(c.created().known(), "creation time unavailable");
        assert(c.created().millis() >= earliest as i64, "before earliestCreated");
    }
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
        !policy.holder_issuers_present | (policy.holder_issuer_count > 0),
        "empty holder issuer list",
    );
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

pub fn holder_applies(issuer: TermCell, policy: PublicPolicy) -> bool {
    check_holder_policy(policy);
    let mut listed = false;
    for i in 0..4 {
        listed = listed
            | ((i < policy.holder_issuer_count) & (issuer.enc() == policy.holder_issuers[i]));
    }
    policy.holder_required & (!policy.holder_issuers_present | listed)
}
pub struct HolderEvidence {
    pub pk: Point,
    pub sig: Signature,
}
/// One signature over the public request digest for each distinct required key.
pub fn check_holders<let C: u32, let N: u32>(
    credentials: [OpenedCredential<N>; C],
    policy: PublicPolicy,
    digest: [u8; 32],
    slots: [u32; C],
    evidence: [HolderEvidence; C],
) {
    let mut used = [false; C];
    for i in 0..C {
        let issuer = required_property(
            credentials[i],
            credentials[i].credential_subject(),
            crate::term::iri_issuer(),
        );
        if holder_applies(issuer, policy) {
            assert(slots[i] < C, "holder evidence slot");
            check_holder_key(credentials[i], evidence[slots[i]].pk);
            used[slots[i]] = true;
        }
    }
    check_holder_signatures(digest, used, evidence);
}
/// Generated callers can accumulate required slots while opening active
/// credentials, then enforce the same distinct-key signature coverage here.
pub fn check_holder_signatures<let C: u32>(
    digest: [u8; 32],
    used: [bool; C],
    evidence: [HolderEvidence; C],
) {
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
pub fn check_public_envelope<let C: u32, let N: u32, let K: u32, let D: u32, let A: u32>(
    credentials: [OpenedCredential<N>; C],
    digest: [u8; 32],
    policy: PublicPolicy,
    keys: [Point; C],
    issuers: [Membership<K>; C],
    holders: [HolderEvidence; C],
    slots: [u32; C],
    status: [[StatusEvidence<D, A>; N]; C],
    validity: [ValidityOpening; C],
) -> [TermCell; C] {
    check_policy::<K, A>(policy);
    let mut verified = [crate::term::padding(); C];
    for i in 0..C {
        check_credential(credentials[i]);
        verified[i] = check_issuer(
            credentials[i],
            policy.issuer_root,
            policy.issuer_depth,
            keys[i],
            issuers[i],
        );
        if policy.status_required {
            check_status(
                credentials[i],
                policy.snapshot_root,
                policy.snapshot_depth,
                policy.allow_missing_status,
                status[i],
            );
        }
        if policy.validity_required {
            check_validity(credentials[i], policy.now, validity[i]);
        }
        check_freshness(
            credentials[i],
            policy.freshness_required,
            policy.earliest_created,
        );
    }
    check_holders(credentials, policy, digest, slots, holders);
    verified
}
// Field-wise selections keep conditional row updates local to their cells.
fn choose_cell(
    take: bool,
    a: crate::table::TermCell,
    b: crate::table::TermCell,
) -> crate::table::TermCell {
    crate::table::TermCell {
        bound: if take { a.bound } else { b.bound },
        kind: if take { a.kind } else { b.kind },
        enc: if take { a.enc } else { b.enc },
        key_class: if take { a.key_class } else { b.key_class },
        key: if take { a.key } else { b.key },
        dt: if take { a.dt } else { b.dt },
        lang: if take { a.lang } else { b.lang },
        scope: if take { a.scope } else { b.scope },
    }
}
fn choose_row(
    take: bool,
    a: [crate::table::TermCell; 4],
    b: [crate::table::TermCell; 4],
) -> [crate::table::TermCell; 4] {
    [
        choose_cell(take, a[0], b[0]),
        choose_cell(take, a[1], b[1]),
        choose_cell(take, a[2], b[2]),
        choose_cell(take, a[3], b[3]),
    ]
}
fn dataset_row_before(a: [crate::table::TermCell; 4], b: [crate::table::TermCell; 4]) -> bool {
    (a[0].bound & !b[0].bound) | ((a[0].bound == b[0].bound) & crate::table::row_less(a, b))
}
/// Bitonic network over the smallest power-of-two prefix. Padding sorts last;
/// comparisons use exactly the operator table order. Hash-field comparisons
/// dominate; each exchange selects two rows without selecting the whole array.
fn sort_dataset<let M: u32, let O: u32, let G: u32>(
    input: [[crate::table::TermCell; 4]; M],
    count: u32,
) -> Table<O, 4> {
    std::static_assert(M > 0, "dataset sort capacity");
    assert(count <= O, "operator capacity");
    assert(count <= M, "dataset sort count");
    std::static_assert(
        (G < 31) & (M <= (1 << G)) & ((1 << G) < 2 * M),
        "dataset sorting depth",
    );
    let width: u32 = 1 << G;
    let mut rows = [[unbound(); 4]; 2 * M];
    for i in 0..M {
        rows[i] = input[i];
    }
    for level in 0..G {
        let size = 1 << (level + 1);
        for step in 0..G {
            let active_step = step <= level;
            let stride = 1 << (if active_step { level - step } else { 0 });
            for i in 0..2 * M {
                let j = i ^ stride;
                if active_step & (i < width) & (j < width) & (j > i) {
                    let a = rows[i];
                    let b = rows[j];
                    let swap = if (i & size) == 0 {
                        dataset_row_before(b, a)
                    } else {
                        dataset_row_before(a, b)
                    };
                    rows[i] = choose_row(swap, b, a);
                    rows[j] = choose_row(swap, a, b);
                }
            }
        }
    }
    let mut out = empty::<O, 4>();
    out.len = count;
    for i in 0..O {
        let source = if i < count { i } else { 0 };
        out.rows[i] = choose_row(i < count, rows[source], [unbound(); 4]);
    }
    out
}
/// Consume B's conversion made with the opener's generated index binding.
/// Its graph already binds that index to the public request digest. Callers must
/// obtain this view from from_authenticated_bounded, never from a witness.
/// No independent index or graph-name computation enters this consumer.
pub fn credential_dataset<let N: u32, let O: u32, let G: u32>(
    c: OpenedCredential<N>,
    issuer: TermCell,
    view: crate::ops_core_suite::OpenedCredential<N>,
) -> Table<O, 4> {
    check_credential(c);
    let claimed = required_property(c, c.credential_subject(), crate::term::iri_issuer());
    assert(same_term(issuer, claimed), "dataset issuer binding");
    assert(view.message() == c.message(), "dataset message binding");
    let graph = view.graph();
    assert(graph.bound & (graph.kind == 2), "dataset graph binding");
    let rows = c.quads();
    let mut candidates = [[unbound(); 4]; 2 * N + 1];
    for i in 0..N {
        assert(view.active()[i] == (i < c.count()), "dataset active binding");
        for p in 0..3 {
            assert(
                crate::table::equal_cell(view.rows()[i][p], from_suite_cell(rows[i][p])),
                "dataset row binding",
            );
        }
        let q = [view.rows()[i][0], view.rows()[i][1], view.rows()[i][2], unbound()];
        candidates[2 * i] = choose_row(view.active()[i], q, [unbound(); 4]);
        candidates[2 * i + 1] =
            choose_row(view.active()[i], [q[0], q[1], q[2], graph], [unbound(); 4]);
    }
    candidates[2 * N] =
        [graph, from_suite_cell(verified_issuer_predicate()), from_suite_cell(issuer), unbound()];
    sort_dataset::<2 * N + 1, O, G>(candidates, 2 * c.count() + 1)
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
    let mut storage = [[unbound(); 4]; O + 1];
    let mut len: u32 = 0;
    let mut cursors = [0; C];
    let mut previous = [unbound(); 4];
    let mut has_previous = false;
    // One input row is consumed each round. The extra output slot absorbs
    // inactive writes, so a conditional never selects the whole output array.
    for _ in 0..C * I {
        let mut found = false;
        let mut chosen = [unbound(); 4];
        let mut source: u32 = 0;
        for c in 0..C {
            let active = cursors[c] < datasets[c].len;
            let index = if active { cursors[c] } else { 0 };
            let row = datasets[c].rows[index];
            let take = active & (!found | crate::table::row_less(row, chosen));
            chosen = choose_row(take, row, chosen);
            source = if take { c } else { source };
            found = found | active;
        }
        let duplicate = has_previous & crate::table::same_row(previous, chosen);
        let emit = found & !duplicate;
        assert(!emit | (len < O), "operator capacity");
        let destination = if emit { len } else { O };
        storage[destination] = choose_row(emit, chosen, [unbound(); 4]);
        len = len + emit as u32;
        previous = choose_row(found, chosen, previous);
        has_previous = has_previous | found;
        cursors[source] = cursors[source] + found as u32;
    }
    for c in 0..C {
        assert(cursors[c] == datasets[c].len, "dataset merge coverage");
    }
    let mut out = empty::<O, 4>();
    out.len = len;
    for i in 0..O {
        out.rows[i] = storage[i];
    }
    out
}

#[test]
fn dataset_sort_matches_the_table_order_with_padding() {
    let a = from_suite_cell(iri_bytes("urn:sort:a".as_bytes()));
    let b = from_suite_cell(iri_bytes("urn:sort:b".as_bytes()));
    let c = from_suite_cell(iri_bytes("urn:sort:c".as_bytes()));
    let ra = [a, a, a, unbound()];
    let rb = [b, a, b, unbound()];
    let rc = [c, a, c, unbound()];
    let expected = crate::table::canonicalize(Table { rows: [ra, rb, rc], len: 3 });
    for order in [
        [ra, rb, rc],
        [ra, rc, rb],
        [rb, ra, rc],
        [rb, rc, ra],
        [rc, ra, rb],
        [rc, rb, ra],
    ] {
        let sorted = sort_dataset::<5, 3, 3>(
            [order[0], [unbound(); 4], order[1], [unbound(); 4], order[2]],
            3,
        );
        crate::table::assert_table(expected, sorted);
    }
    let one = sort_dataset::<3, 3, 2>([[unbound(); 4], ra, [unbound(); 4]], 1);
    crate::table::assert_table(
        Table { rows: [ra, [unbound(); 4], [unbound(); 4]], len: 1 },
        one,
    );
}
#[test]
fn dataset_merge_keeps_capacity_padding_and_exact_duplicates() {
    let a = from_suite_cell(iri_bytes("urn:merge:a".as_bytes()));
    let row = [a, a, a, unbound()];
    let input = Table { rows: [row, [unbound(); 4]], len: 1 };
    let merged = merge_datasets::<2, 2, 3>([input, input]);
    crate::table::assert_table(
        Table { rows: [row, [unbound(); 4], [unbound(); 4]], len: 1 },
        merged,
    );
}
#[test(should_fail_with = "operator capacity")]
fn dataset_merge_refuses_an_output_below_the_distinct_count() {
    let a = from_suite_cell(iri_bytes("urn:merge:a".as_bytes()));
    let b = from_suite_cell(iri_bytes("urn:merge:b".as_bytes()));
    let row_a = [a, a, a, unbound()];
    let row_b = [b, b, b, unbound()];
    let _ = merge_datasets::<2, 1, 1>([
        Table { rows: [row_a], len: 1 },
        Table { rows: [row_b], len: 1 },
    ]);
}

fn checked_key_test_point() -> Point {
    Point {
        x: 0x09110e7c3e843e302bee8946301ca5755dd78b330d56b8799f147bb81e01046f,
        y: 0x0de711eb49a82d54b6ddd2a10e277e118e7bc4b716a6783973db6b6fb912d7ca,
    }
}
#[test]
fn multikey_read_accepts_the_canonical_checked_constructor() {
    let pk = checked_key_test_point();
    checked_multikey(multikey_cell(pk), pk);
}
#[test(should_fail_with = "verification key opening differs")]
fn multikey_read_refuses_a_different_signed_value_key() {
    let pk = checked_key_test_point();
    let expected = multikey_cell(pk);
    let bytes = expected.key().to_be_bytes::<32>();
    let mut key = [0; 31];
    for i in 0..31 { key[i] = bytes[i + 1]; }
    key[30] = key[30] ^ 1;
    let cell = crate::term::open_term(
        3,
        BoundedVec::<u8, 137>::from_array("f8180c00109110e7c3e843e302bee8946301ca5755dd78b330d56b8799f147bb81e01046f0de711eb49a82d54b6ddd2a10e277e118e7bc4b716a6783973db6b6fb912d7ca".as_bytes()),
        BoundedVec::<u8, 64>::from_array("http://www.w3.org/2001/XMLSchema#string".as_bytes()),
        BoundedVec::<u8, 0>::new(),
        key,
        0,
    );
    assert(same_term(cell, expected));
    assert(cell.enc() != expected.enc());
    checked_multikey(cell, pk);
}
#[test]
fn table_conversion_keeps_identity_and_authenticated_value_metadata() {
    let cell = multikey_cell(checked_key_test_point());
    let converted = from_suite_cell(cell);
    assert(converted.enc == cell.identity());
    assert(converted.enc != cell.enc());
    assert((converted.key_class == cell.key_class()) & (converted.key == cell.key()));
    assert((converted.dt == cell.datatype()) & (converted.lang == cell.language()));
}
#[test]
fn fixed_iri_helpers_match_the_native_suite_encodings() {
    assert(confidence_predicate().enc() == 0x1f303d9c55335e92efbec3b7bbb23da4290305eaf6a61e041b25cc5cf65a1ac0);
    assert(verified_issuer_predicate().enc() == 0x09eec572ceda200a642295e68e09118d7e1d26ea1a082cc560e303164907f5e9);
}

fn snapshot_test_purpose(change_key: bool) -> TermCell {
    let lexical = BoundedVec::<u8, 16>::from_array("revocation".as_bytes());
    let (_, value) = crate::term_key::recompute(lexical, crate::suite_constants::DT_STRING);
    let bytes = value.to_be_bytes::<32>();
    let mut key = [0; 31];
    for i in 0..31 { key[i] = bytes[i + 1]; }
    if change_key { key[1] = key[1] ^ 1; }
    crate::term::open_term(
        3, lexical,
        BoundedVec::<u8, 64>::from_array("http://www.w3.org/2001/XMLSchema#string".as_bytes()),
        BoundedVec::<u8, 0>::new(), key, 0,
    )
}
#[test]
fn snapshot_metadata_matches_checked_protocol_encodings() {
    let list = iri_bytes("urn:snapshot".as_bytes());
    let purpose = snapshot_test_purpose(false);
    check_snapshot_identity(Snapshot { list, purpose, bit_len: 1, root: 1 }, list, purpose);
}
#[test(should_fail_with = "snapshot identity")]
fn snapshot_metadata_refuses_a_different_key_with_the_same_identity() {
    let list = iri_bytes("urn:snapshot".as_bytes());
    let purpose = snapshot_test_purpose(false);
    let changed = snapshot_test_purpose(true);
    assert(same_term(purpose, changed));
    assert(purpose.enc() != changed.enc());
    check_snapshot_identity(
        Snapshot { list, purpose: changed, bit_len: 1, root: 1 }, list, purpose,
    );
}

mod bounded_status_tests {
    use crate::suite_entry::TermOpening;
    use crate::term::same_term;
    use super::{
        BoundedStatusEvidence, check_status, check_status_bounded, check_status_slots,
        CredentialOpening, iri_bytes, SlotStatusEvidence,
        Membership, merkle_root, open_suite_credential, OpenedCredential, Point, required_property,
        Signature, Snapshot, snapshot_digest, snapshot_digest_at_depth, snapshot_test_purpose,
        status_leaf, status_merkle_root, StatusEvidence,
    };
    use compose::hashes::h2;
    use std::collections::bounded_vec::BoundedVec;
    // The signed fixture is the existing envelope status vector from the issuance example.
    fn absent<let L: u32>() -> TermOpening<L> {
        TermOpening {
            bound: false,
            kind: 0,
            lexical: [0; L],
            lexical_len: 0,
            datatype: [0; 128],
            datatype_len: 0,
            language: [0; 64],
            language_len: 0,
            key: [0; 31],
        }
    }
    fn raw<let L: u32, let T: u32, let D: u32, let G: u32>(
        kind: u8,
        text: [u8; T],
        dt: [u8; D],
        lang: [u8; G],
        key: [u8; 31],
    ) -> TermOpening<L> {
        let mut r = absent::<L>();
        r.bound = true;
        r.kind = kind;
        r.lexical_len = T;
        r.datatype_len = D;
        r.language_len = G;
        r.key = key;
        for i in 0..T {
            r.lexical[i] = text[i];
        }
        for i in 0..D {
            r.datatype[i] = dt[i];
        }
        for i in 0..G {
            r.language[i] = lang[i];
        }
        r
    }
    fn credential_proof<let N: u32>() -> CredentialOpening<N, 160> {
        let mut rows = [[absent::<160>(); 4]; N];
        let mut salts = [0; N];
        rows[0] = [
            raw(
                1,
                "urn:credential".as_bytes(),
                "".as_bytes(),
                "".as_bytes(),
                [0; 31],
            ),
            raw(
                1,
                "http://www.w3.org/1999/02/22-rdf-syntax-ns#type".as_bytes(),
                "".as_bytes(),
                "".as_bytes(),
                [0; 31],
            ),
            raw(
                1,
                "https://www.w3.org/2018/credentials#VerifiableCredential".as_bytes(),
                "".as_bytes(),
                "".as_bytes(),
                [0; 31],
            ),
            raw(0, "".as_bytes(), "".as_bytes(), "".as_bytes(), [0; 31]),
        ];
        salts[0] = 0x0a845faa4ea64651b38b59b526c347b3d2bdea2b334ca9987a154d12bb524dbd;
        rows[1] = [
            raw(
                1,
                "urn:status:0".as_bytes(),
                "".as_bytes(),
                "".as_bytes(),
                [0; 31],
            ),
            raw(
                1,
                "https://www.w3.org/ns/credentials/status#statusListCredential".as_bytes(),
                "".as_bytes(),
                "".as_bytes(),
                [0; 31],
            ),
            raw(
                1,
                "urn:list:revocation".as_bytes(),
                "".as_bytes(),
                "".as_bytes(),
                [0; 31],
            ),
            raw(0, "".as_bytes(), "".as_bytes(), "".as_bytes(), [0; 31]),
        ];
        salts[1] = 0x141da10894efebf23d64d7057749fd50806335ba04bb7c24c8dd6673f848888b;
        rows[2] = [
            raw(
                1,
                "urn:subject".as_bytes(),
                "".as_bytes(),
                "".as_bytes(),
                [0; 31],
            ),
            raw(
                1,
                "urn:fact:issuer".as_bytes(),
                "".as_bytes(),
                "".as_bytes(),
                [0; 31],
            ),
            raw(
                1,
                "urn:fact:1".as_bytes(),
                "".as_bytes(),
                "".as_bytes(),
                [0; 31],
            ),
            raw(0, "".as_bytes(), "".as_bytes(), "".as_bytes(), [0; 31]),
        ];
        salts[2] = 0x1536b237355f7230bdb01d3a05b8cfc78cf3ef772a4cfbfc0f56649734853935;
        rows[3] = [
            raw(
                1,
                "urn:status:0".as_bytes(),
                "".as_bytes(),
                "".as_bytes(),
                [0; 31],
            ),
            raw(
                1,
                "http://www.w3.org/1999/02/22-rdf-syntax-ns#type".as_bytes(),
                "".as_bytes(),
                "".as_bytes(),
                [0; 31],
            ),
            raw(
                1,
                "https://www.w3.org/ns/credentials/status#BitstringStatusListEntry".as_bytes(),
                "".as_bytes(),
                "".as_bytes(),
                [0; 31],
            ),
            raw(0, "".as_bytes(), "".as_bytes(), "".as_bytes(), [0; 31]),
        ];
        salts[3] = 0x15e469943c9c1c82fd66018353dc543b33a23076177ad35c4c267e7795b4cb0a;
        rows[4] = [
            raw(
                1,
                "urn:subject".as_bytes(),
                "".as_bytes(),
                "".as_bytes(),
                [0; 31],
            ),
            raw(
                1,
                "urn:shared".as_bytes(),
                "".as_bytes(),
                "".as_bytes(),
                [0; 31],
            ),
            raw(
                1,
                "urn:value".as_bytes(),
                "".as_bytes(),
                "".as_bytes(),
                [0; 31],
            ),
            raw(0, "".as_bytes(), "".as_bytes(), "".as_bytes(), [0; 31]),
        ];
        salts[4] = 0x03bff45413336a47a4ace38258984c0dd09bb3f4e77b87357dc8952ecfae337a;
        rows[5] = [
            raw(
                1,
                "urn:credential".as_bytes(),
                "".as_bytes(),
                "".as_bytes(),
                [0; 31],
            ),
            raw(
                1,
                "https://www.w3.org/2018/credentials#credentialStatus".as_bytes(),
                "".as_bytes(),
                "".as_bytes(),
                [0; 31],
            ),
            raw(
                1,
                "urn:status:0".as_bytes(),
                "".as_bytes(),
                "".as_bytes(),
                [0; 31],
            ),
            raw(0, "".as_bytes(), "".as_bytes(), "".as_bytes(), [0; 31]),
        ];
        salts[5] = 0x2d6ac64036592573c76df594cf54d2cabf7a2d69679a8eb7236147b74189c555;
        rows[6] = [
            raw(
                1,
                "urn:credential".as_bytes(),
                "".as_bytes(),
                "".as_bytes(),
                [0; 31],
            ),
            raw(
                1,
                "https://www.w3.org/2018/credentials#issuer".as_bytes(),
                "".as_bytes(),
                "".as_bytes(),
                [0; 31],
            ),
            raw(
                1,
                "urn:issuer:1".as_bytes(),
                "".as_bytes(),
                "".as_bytes(),
                [0; 31],
            ),
            raw(0, "".as_bytes(), "".as_bytes(), "".as_bytes(), [0; 31]),
        ];
        salts[6] = 0x0c3d6e496122a8ac066ea6f128ec4c11f8b8f15687868d2f4e1c4ecdd9025a7a;
        rows[7] = [
            raw(
                1,
                "urn:status:0".as_bytes(),
                "".as_bytes(),
                "".as_bytes(),
                [0; 31],
            ),
            raw(
                1,
                "https://www.w3.org/ns/credentials/status#statusPurpose".as_bytes(),
                "".as_bytes(),
                "".as_bytes(),
                [0; 31],
            ),
            raw(
                3,
                "revocation".as_bytes(),
                "http://www.w3.org/2001/XMLSchema#string".as_bytes(),
                "".as_bytes(),
                [
                    6, 114, 101, 118, 111, 99, 97, 116, 105, 111, 110, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
                    0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
                ],
            ),
            raw(0, "".as_bytes(), "".as_bytes(), "".as_bytes(), [0; 31]),
        ];
        salts[7] = 0x03f6578610d053c4c3ec246fdb1ea631d1c540eec29522bc5f9537e052a7cc35;
        rows[8] = [
            raw(
                1,
                "urn:status:0".as_bytes(),
                "".as_bytes(),
                "".as_bytes(),
                [0; 31],
            ),
            raw(
                1,
                "https://www.w3.org/ns/credentials/status#statusListIndex".as_bytes(),
                "".as_bytes(),
                "".as_bytes(),
                [0; 31],
            ),
            raw(
                3,
                "0001".as_bytes(),
                "http://www.w3.org/2001/XMLSchema#string".as_bytes(),
                "".as_bytes(),
                [
                    6, 48, 48, 48, 49, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
                    0, 0, 0, 0, 0, 0,
                ],
            ),
            raw(0, "".as_bytes(), "".as_bytes(), "".as_bytes(), [0; 31]),
        ];
        salts[8] = 0x0c3dfb14cda6e1e3915d1a8c4136a80ad63e07daf0b38982d3d68d2929e98f41;
        CredentialOpening {
            raw: rows,
            salts,
            salt: 0x297e6ca1cd19c915b7d424aa73c4542f7581c011d74bf672222f8c230973170f,
            count: 9,
            configuration: [
                286021852831425289605033915724327055018,
                109278919710069188030686208432707155326,
            ],
            created: absent(),
            method: raw(
                1,
                "urn:method:1".as_bytes(),
                "".as_bytes(),
                "".as_bytes(),
                [0; 31],
            ),
            pk: Point {
                x: 0x1932c73aa883c1bb2df0890c2e5eaf9f4ba79582867ed1bd906d863b87b5fe78,
                y: 0x21a0ff4c5132dc1cf212fb066ed462850c724ecf6726149b97a7a1cb5433ebb9,
            },
            signature: Signature {
                present: true,
                r: Point {
                    x: 0x0bcad2a85197bfbff9f6d94a5750b35bc5b908832a923dbde52676ba63d3f52e,
                    y: 0x189dafb1a82257906d64863a0f97578acacc91920583155fcd1911b80ccd88a0,
                },
                s: 0x04ba587da90a7adcf35a30748a9a9e17e8cb174e698cc258029af324e4d33370,
                e: 0x04549acda5b418b06b8820aa9a8ce74d7e76249e63799e58cbae499dec4652a5,
                e_k: 0x0000000000000000000000000000000000000000000000000000000000000005,
            },
        }
    }
    fn credential() -> OpenedCredential<16> {
        open_suite_credential(credential_proof::<16>(), 0)
    }
    fn row(c: OpenedCredential<16>) -> u32 {
        let mut found = 0;
        let rows = c.quads();
        for i in 0..16 {
            if (i < c.count())
                & same_term(rows[i][0], c.credential_subject())
                & same_term(rows[i][1], crate::term::iri_credential_status()) {
                found = i;
            }
        }
        found
    }
    fn evidence<let MAX: u32>(
        c: OpenedCredential<16>,
        depth: u32,
    ) -> [BoundedStatusEvidence<MAX, 1>; 16] {
        let mut siblings = [0; MAX];
        for level in 0..MAX {
            if level < depth {
                siblings[level] = 9;
            }
        }
        let path = Membership { index: 1, siblings };
        let entry = c.quads()[row(c)][2];
        let snapshot = Snapshot {
            list: required_property(c, entry, crate::term::iri_status_list()),
            purpose: required_property(c, entry, crate::term::iri_status_purpose()),
            bit_len: 2,
            root: status_merkle_root(status_leaf(false), path, depth),
        };
        let mut out = [
            BoundedStatusEvidence {
                depth,
                present: false,
                snapshot,
                accepted: Membership { index: 0, siblings: [0] },
                bit: false,
                path,
                purpose: BoundedVec::from_array("revocation".as_bytes()),
                index: BoundedVec::from_array("0001".as_bytes()),
                size: BoundedVec::from_array("1".as_bytes()),
            }; 16
        ];
        out[row(c)].present = true;
        out
    }
    fn policy<let MAX: u32>(
        c: OpenedCredential<16>,
        es: [BoundedStatusEvidence<MAX, 1>; 16],
    ) -> Field {
        let entry = es[row(c)];
        snapshot_digest_at_depth(entry.snapshot, entry.depth)
    }
    #[test]
    fn bounded_status_shallow_depth_is_accepted() {
        let c = credential();
        let es = evidence::<32>(c, 1);
        check_status_bounded(c, policy(c, es), 0, false, es);
    }
    #[test]
    fn bounded_status_maximum_depth_is_accepted() {
        let c = credential();
        let es = evidence::<32>(c, 32);
        check_status_bounded(c, policy(c, es), 0, false, es);
    }
    #[test(should_fail_with = "status depth capacity")]
    fn bounded_status_depth_above_maximum_is_refused() {
        let c = credential();
        let mut es = evidence::<4>(c, 1);
        es[row(c)].depth = 5;
        check_status_bounded(c, 1, 0, false, es);
    }
    #[test(should_fail_with = "status path padding")]
    fn bounded_status_unused_sibling_is_refused() {
        let c = credential();
        let mut es = evidence::<32>(c, 1);
        es[row(c)].path.siblings[1] = 7;
        check_status_bounded(c, policy(c, es), 0, false, es);
    }
    #[test(should_fail_with = "status index depth")]
    fn bounded_status_index_at_depth_limit_is_refused() {
        let c = credential();
        let mut es = evidence::<32>(c, 1);
        es[row(c)].depth = 0;
        check_status_bounded(c, policy(c, es), 0, false, es);
    }
    #[test(should_fail_with = "snapshot policy")]
    fn bounded_status_snapshot_depth_relabelling_is_refused() {
        let c = credential();
        let mut es = evidence::<32>(c, 1);
        let root = policy(c, es);
        es[row(c)].depth = 2;
        check_status_bounded(c, root, 0, false, es);
    }
    #[test(should_fail_with = "snapshot capacity")]
    fn bounded_status_bit_length_above_capacity_is_refused() {
        let c = credential();
        let mut es = evidence::<32>(c, 1);
        es[row(c)].snapshot.bit_len = 3;
        check_status_bounded(c, policy(c, es), 0, false, es);
    }
    #[test(should_fail_with = "status bit set")]
    fn bounded_status_set_bit_is_refused() {
        let c = credential();
        let mut es = evidence::<32>(c, 1);
        es[row(c)].bit = true;
        check_status_bounded(c, policy(c, es), 0, false, es);
    }
    #[test]
    fn bounded_status_path_stops_at_the_actual_depth() {
        let leaf = status_leaf(false);
        let path = Membership { index: 1, siblings: [9, 0, 0, 0] };
        assert(status_merkle_root(leaf, path, 1) == h2(9, leaf));
        assert(status_merkle_root(leaf, path, 1) != merkle_root(leaf, path));
        assert(status_merkle_root(leaf, Membership { index: 0, siblings: [0; 4] }, 0) == leaf);
    }
    #[test(should_fail_with = "status index depth")]
    fn bounded_status_path_refuses_a_high_index_bit() {
        let _root = status_merkle_root(
            status_leaf(false),
            Membership { index: 2, siblings: [0; 4] },
            1,
        );
    }
    #[test(should_fail)]
    fn bounded_status_path_refuses_an_index_above_the_compiled_range() {
        let _root = status_merkle_root(
            status_leaf(false),
            Membership { index: 0x100000000, siblings: [0; 32] },
            32,
        );
    }
    #[test]
    fn fixed_status_api_and_bounded_status_share_the_relation() {
        let c = credential();
        let es = evidence::<1>(c, 1);
        let mut fixed = [
            StatusEvidence {
                present: false,
                snapshot: es[0].snapshot,
                accepted: es[0].accepted,
                bit: es[0].bit,
                path: es[0].path,
                purpose: es[0].purpose,
                index: es[0].index,
                size: es[0].size,
            }; 16
        ];
        for i in 0..16 {
            let e = es[i];
            fixed[i] = StatusEvidence {
                present: e.present,
                snapshot: e.snapshot,
                accepted: e.accepted,
                bit: e.bit,
                path: e.path,
                purpose: e.purpose,
                index: e.index,
                size: e.size,
            };
        }
        let root = policy(c, es);
        check_status(c, root, 0, false, fixed);
        check_status_bounded(c, root, 0, false, es);
    }
    #[test]
    fn bounded_status_snapshot_vectors_match_rust() {
        let s = Snapshot {
            list: iri_bytes("urn:snapshot".as_bytes()),
            purpose: snapshot_test_purpose(false),
            bit_len: 2,
            root: 17,
        };
        assert(
            snapshot_digest_at_depth(s, 1)
                == 0x21fee3d80bb93e4729b47a9b29e2de1401eff4941174578f9eec2c2fe1a10e5a,
        );
        assert(
            snapshot_digest_at_depth(s, 32)
                == 0x20c9c57f3dda8ce0d2de515ed2f11ab5085a0a3fa720970a293424f706b56d57,
        );
        assert(snapshot_digest::<1>(s) == snapshot_digest_at_depth(s, 1));
        assert(snapshot_digest::<32>(s) == snapshot_digest_at_depth(s, 32));
    }
    fn slots<let S: u32>(
        c: OpenedCredential<16>,
        es: [BoundedStatusEvidence<32, 1>; 16],
    ) -> [SlotStatusEvidence<32, 1>; S] {
        let mut blank = es[0];
        blank.present = false;
        blank.depth = 0;
        let mut out = [SlotStatusEvidence { row: 0, status: blank }; S];
        out[0] = SlotStatusEvidence { row: row(c), status: es[row(c)] };
        out
    }
    #[test]
    fn status_slots_accept_one_entry() {
        let c = credential();
        let es = evidence::<32>(c, 1);
        check_status_slots(c, policy(c, es), 0, false, slots::<1>(c, es));
    }
    #[test]
    fn status_slots_accept_padding_after_the_entry() {
        let c = credential();
        let es = evidence::<32>(c, 32);
        check_status_slots(c, policy(c, es), 0, false, slots::<3>(c, es));
    }
    #[test(should_fail_with = "status evidence coverage")]
    fn status_slots_refuse_an_omitted_entry() {
        let c = credential();
        let es = evidence::<32>(c, 1);
        let mut ss = slots::<1>(c, es);
        ss[0].status.present = false;
        ss[0].row = 0;
        ss[0].status.depth = 0;
        check_status_slots(c, policy(c, es), 0, true, ss);
    }
    #[test(should_fail_with = "status row")]
    fn status_slots_refuse_a_row_that_is_not_a_status_entry() {
        let c = credential();
        let es = evidence::<32>(c, 1);
        let mut ss = slots::<1>(c, es);
        ss[0].row = (row(c) + 1) % 16;
        check_status_slots(c, policy(c, es), 0, false, ss);
    }
    #[test(should_fail_with = "status row order")]
    fn status_slots_refuse_a_repeated_row() {
        let c = credential();
        let es = evidence::<32>(c, 1);
        let mut ss = slots::<2>(c, es);
        ss[1] = ss[0];
        check_status_slots(c, policy(c, es), 0, false, ss);
    }
    #[test(should_fail_with = "status slot order")]
    fn status_slots_refuse_a_gap_before_an_entry() {
        let c = credential();
        let es = evidence::<32>(c, 1);
        let mut ss = slots::<2>(c, es);
        let first = ss[0];
        ss[0] = ss[1];
        ss[1] = first;
        check_status_slots(c, policy(c, es), 0, false, ss);
    }
    #[test(should_fail_with = "status slot padding")]
    fn status_slots_refuse_unzeroed_padding() {
        let c = credential();
        let es = evidence::<32>(c, 1);
        let mut ss = slots::<2>(c, es);
        ss[1].row = row(c);
        check_status_slots(c, policy(c, es), 0, false, ss);
    }
    #[test(should_fail_with = "status bit set")]
    fn status_slots_run_the_entry_relation() {
        let c = credential();
        let mut es = evidence::<32>(c, 1);
        es[row(c)].bit = true;
        check_status_slots(c, policy(c, es), 0, false, slots::<1>(c, es));
    }
}
