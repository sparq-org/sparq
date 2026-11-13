//! Authenticated credential statement gadgets. Research grade; not externally audited.
//! Public policy words and request digest are reconstructed by the verifier.
use compose::hashes::{commit_fold, h2, h3, h5};
use compose::issuer::schnorr_verify;
use std::collections::bounded_vec::BoundedVec;
pub use crate::suite_metadata::OpenedCredential;
pub use compose::issuer::Point;
use crate::suite_entry::TermOpening;
use crate::suite_metadata::{open_multikey, open_status_index, open_status_purpose, open_time};
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
            enc: cell.enc(),
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
    comptime {
        iri_bytes(
            "https://www.w3.org/2018/credentials#confidenceMethod".as_bytes(),
        )
    }
}
pub fn verified_issuer_predicate() -> TermCell {
    comptime {
        iri_bytes("https://w3id.org/sparq/vcq#verifiedIssuer".as_bytes())
    }
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
    open_multikey(c.verification_key(), pk);
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
    open_multikey(key, pk);
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
    commit_fold(
        [SNAPSHOT, s.list.enc(), s.purpose.enc(), s.bit_len as Field, D as Field, s.root],
        6,
    )
}
pub fn status_leaf(bit: bool) -> Field {
    h3(STATUS_LEAF, bit as Field, 0)
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
/// Cover every signed status row, with no witnessed absence or selected subset.
pub fn check_status<let N: u32, let D: u32, let A: u32>(
    c: OpenedCredential<N>,
    snapshot_root: Field,
    snapshot_depth: u32,
    allow_missing: bool,
    evidence: [StatusEvidence<D, A>; N],
) {
    check_credential(c);
    assert(D <= 32, "status tree depth");
    let mut capacity: Field = 1;
    for _ in 0..D {
        capacity = capacity * 2;
    }
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
            let e = evidence[i];
            let entry = rows[i][2];
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
            assert(
                same_term(e.snapshot.list, list) & same_term(e.snapshot.purpose, purpose),
                "snapshot identity",
            );
            assert(index < e.snapshot.bit_len as u64, "status index length");
            assert((e.snapshot.bit_len as Field).lt(capacity + 1), "snapshot capacity");
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
/// STAND-IN (lane B): replaced in step 2. Generated wiring must pass the public
/// request digest here and check_issuer's result, then evaluate the resulting table.
pub fn credential_dataset<let N: u32, let O: u32, let G: u32>(
    c: OpenedCredential<N>,
    issuer: TermCell,
    request_digest: [u8; 32],
    credential_index: u32,
) -> Table<O, 4> {
    check_credential(c);
    let claimed = required_property(c, c.credential_subject(), crate::term::iri_issuer());
    assert(same_term(issuer, claimed), "dataset issuer binding");
    let graph = from_suite_cell(crate::term::credential_graph(
        digest_halves(request_digest),
        credential_index,
    ));
    let rows = c.quads();
    let mut candidates = [[unbound(); 4]; 2 * N + 1];
    for i in 0..N {
        let q = [
            from_suite_cell(rows[i][0]),
            from_suite_cell(rows[i][1]),
            from_suite_cell(rows[i][2]),
            unbound(),
        ];
        candidates[2 * i] = choose_row(i < c.count(), q, [unbound(); 4]);
        candidates[2 * i + 1] =
            choose_row(i < c.count(), [q[0], q[1], q[2], graph], [unbound(); 4]);
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
