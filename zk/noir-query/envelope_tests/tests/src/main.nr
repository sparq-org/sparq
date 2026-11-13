use compose_core::hashes::{commit_fold, h2};
use compose_core::issuer::{BJJ_GX, BJJ_GY, scalar_mul};
use envelope_core::envelope::{
    check_envelope, check_holder, check_issuer, check_status, check_validity, Credential,
    credential_dataset, holder_message, iri, Membership, merkle_root, Point, property,
    RequestPolicy, Signature, Snapshot, snapshot_digest, stand_in_index, stand_in_key_cell,
    stand_in_number, stand_in_open_credential, stand_in_purpose, stand_in_request_digest,
    stand_in_vocabulary, status_leaf, StatusEvidence, TermCell, trusted_key_leaf,
};
use envelope_core::table::{check_table, equal_cell, same_term, Table, unbound};

fn main() {}

global L: Field = 2736030358979909402780800718157159386076813972158567259200215660948447373041;
fn key(secret: Field) -> Point {
    scalar_mul(secret, Point { x: BJJ_GX, y: BJJ_GY })
}
// Synthetic fixture signer only, for secret scalars one and two, nonce one.
// This is test witness generation; all checks use the library signature relation.
fn sign(secret: Field, message: Field) -> Signature {
    assert((secret == 1) | (secret == 2));
    let pk = key(secret);
    let r = key(1);
    let base = commit_fold([0x5a4b5349475f4331, r.x, r.y, pk.x, pk.y, message], 6);
    let bits: [bool; 254] = base.to_le_bits();
    let mut e: Field = 0;
    let mut quotient: Field = 0;
    for i in 0..254 {
        let next = e * 2 + bits[253 - i] as Field;
        let carry = !next.lt(L);
        e = next - (carry as Field) * L;
        quotient = quotient * 2 + carry as Field;
    }
    let response = e * secret + 1;
    let s = response - ((!response.lt(L)) as Field) * L;
    Signature { present: true, r, s, e, e_k: quotient }
}
fn no_sig() -> Signature {
    Signature { present: false, r: key(1), s: 0, e: 0, e_k: 0 }
}
fn q(s: u32, p: Field, o: TermCell) -> [TermCell; 4] {
    [iri(s as Field), iri(p), o, unbound()]
}
fn base() -> Credential<16> {
    let v = stand_in_vocabulary();
    let mut quads = [[unbound(); 4]; 16];
    quads[0] = q(1, v.rdf_type, iri(v.vc_type));
    quads[1] = q(1, v.issuer, iri(900));
    Credential {
        quads,
        len: 2,
        root: iri(1),
        credential_index: 0,
        verification_method: iri(901),
        issuer_key: key(1),
        signed_message: 42,
        created_at: stand_in_number(0, v.time_dt),
    }
}
fn holder() -> Credential<16> {
    let mut c = base();
    let v = stand_in_vocabulary();
    c.quads[2] = q(1, v.subject, iri(2));
    c.quads[3] = q(2, v.confidence, iri(3));
    c.quads[4] = q(3, v.rdf_type, iri(v.multikey));
    c.quads[5] = q(3, v.public_key, stand_in_key_cell(key(1)));
    c.len = 6;
    c
}
fn validity() -> Credential<16> {
    let mut c = base();
    let v = stand_in_vocabulary();
    c.quads[2] = q(1, v.valid_from, stand_in_number(100, v.time_dt));
    c.quads[3] = q(1, v.valid_until, stand_in_number(200, v.time_dt));
    c.len = 4;
    c
}
fn status() -> Credential<16> {
    let mut c = base();
    let v = stand_in_vocabulary();
    c.quads[2] = q(1, v.credential_status, iri(4));
    c.quads[3] = q(4, v.rdf_type, iri(v.status_entry));
    c.quads[4] = q(4, v.status_list, iri(800));
    c.quads[5] = q(4, v.status_purpose, stand_in_purpose(false));
    c.quads[6] = q(4, v.status_index, stand_in_index(1));
    c.len = 7;
    c
}
fn path() -> Membership<2> {
    Membership {
        index: 1,
        siblings: [status_leaf(true), h2(status_leaf(false), status_leaf(true))],
    }
}
fn acceptance() -> Membership<1> {
    Membership { index: 0, siblings: [999] }
}
fn snapshot() -> Snapshot {
    Snapshot {
        list: iri(800),
        purpose: stand_in_purpose(false),
        bit_len: 4,
        root: merkle_root(status_leaf(false), path()),
    }
}
fn no_status() -> [StatusEvidence<2, 1>; 16] {
    [
        StatusEvidence {
            present: false,
            snapshot: snapshot(),
            accepted: acceptance(),
            bit: false,
            path: path(),
        }; 16
    ]
}
fn evidence() -> [StatusEvidence<2, 1>; 16] {
    let mut e = no_status();
    e[2].present = true;
    e
}
fn deeper_evidence() -> [StatusEvidence<3, 1>; 16] {
    let one = status_leaf(true);
    let member = Membership {
        index: 1,
        siblings: [path().siblings[0], path().siblings[1], h2(h2(one, one), h2(one, one))],
    };
    let mut deeper = snapshot();
    deeper.root = merkle_root(status_leaf(false), member);
    let mut e = [
        StatusEvidence {
            present: false,
            snapshot: deeper,
            accepted: acceptance(),
            bit: false,
            path: member,
        }; 16
    ];
    e[2].present = true;
    e
}
fn policy_root() -> Field {
    merkle_root(snapshot_digest::<2>(snapshot()), acceptance())
}
fn issuer_path() -> Membership<1> {
    Membership { index: 0, siblings: [777] }
}
fn issuer_root(c: Credential<16>) -> Field {
    merkle_root(
        trusted_key_leaf(c.quads[1][2], c.verification_method, c.issuer_key),
        issuer_path(),
    )
}
fn policy(c: Credential<16>) -> RequestPolicy {
    RequestPolicy {
        trusted_root: issuer_root(c),
        holder_required: true,
        status_required: true,
        allow_missing_status: false,
        accepted_snapshots: policy_root(),
        validity_required: true,
        now: 150,
    }
}

#[test]
fn issuer_accepted_and_identity_returned() {
    let c = base();
    assert(
        equal_cell(
            check_issuer(c, issuer_root(c), sign(1, 42), issuer_path()),
            iri(900),
        ),
    );
}
#[test(should_fail_with = "required property absent")]
fn issuer_missing_refused() {
    let mut c = base();
    c.len = 1;
    c.quads[1] = [unbound(); 4];
    let _ = check_issuer(c, 1, no_sig(), issuer_path());
}
#[test(should_fail_with = "signature evidence absent")]
fn issuer_evidence_omitted_refused() {
    let c = base();
    let _ = check_issuer(c, issuer_root(c), no_sig(), issuer_path());
}
#[test(should_fail_with = "issuer authorization set")]
fn another_key_set_refused() {
    let _ = check_issuer(base(), issuer_root(base()) + 1, sign(1, 42), issuer_path());
}
#[test(should_fail_with = "issuer authorization set")]
fn another_issuer_label_refused() {
    let mut c = base();
    let root = issuer_root(c);
    c.quads[1][2] = iri(902);
    let _ = check_issuer(c, root, sign(1, 42), issuer_path());
}
#[test(should_fail_with = "issuer authorization set")]
fn another_verification_method_refused() {
    let mut c = base();
    let root = issuer_root(c);
    c.verification_method = iri(903);
    let _ = check_issuer(c, root, sign(1, 42), issuer_path());
}
#[test(should_fail_with = "challenge reduction not bound to its preimage")]
fn another_issuer_message_refused() {
    let mut c = base();
    c.signed_message = 43;
    let _ = check_issuer(c, issuer_root(c), sign(1, 42), issuer_path());
}
#[test(should_fail_with = "issuer authorization set")]
fn different_issuer_key_refused() {
    let mut c = base();
    let root = issuer_root(c);
    c.issuer_key = key(2);
    let _ = check_issuer(c, root, sign(2, 42), issuer_path());
}

#[test]
fn holder_request_signature_accepted() {
    check_holder(
        holder(),
        [10, 20],
        key(1),
        sign(1, holder_message([10, 20])),
    );
}
#[test(should_fail_with = "required property absent")]
fn holder_key_absent_refused() {
    check_holder(base(), [10, 20], key(1), no_sig());
}
#[test(should_fail_with = "signature evidence absent")]
fn holder_signature_omitted_refused() {
    check_holder(holder(), [10, 20], key(1), no_sig());
}
#[test(should_fail_with = "holder key binding")]
fn holder_other_key_refused() {
    check_holder(
        holder(),
        [10, 20],
        key(2),
        sign(2, holder_message([10, 20])),
    );
}
#[test(should_fail_with = "challenge reduction not bound to its preimage")]
fn holder_other_request_refused() {
    check_holder(
        holder(),
        [10, 21],
        key(1),
        sign(1, holder_message([10, 20])),
    );
}
#[test(should_fail_with = "challenge reduction not bound to its preimage")]
fn holder_other_high_digest_limb_refused() {
    check_holder(
        holder(),
        [11, 20],
        key(1),
        sign(1, holder_message([10, 20])),
    );
}
#[test(should_fail_with = "property cardinality")]
fn holder_ambiguous_key_refused() {
    let mut c = holder();
    c.quads[6] = q(
        3,
        stand_in_vocabulary().public_key,
        stand_in_key_cell(key(2)),
    );
    c.len = 7;
    check_holder(c, [10, 20], key(1), no_sig());
}

#[test]
fn status_active_accepted() {
    check_status(status(), policy_root(), false, evidence());
}
#[test]
fn suspension_active_accepted() {
    let mut c = status();
    c.quads[5][2] = stand_in_purpose(true);
    let mut e = evidence();
    e[2].snapshot.purpose = stand_in_purpose(true);
    let root = merkle_root(snapshot_digest::<2>(e[2].snapshot), acceptance());
    check_status(c, root, false, e);
}
#[test(should_fail_with = "status entry absent")]
fn status_absent_required_refused() {
    check_status(base(), policy_root(), false, no_status());
}
#[test]
fn status_absence_explicitly_permitted() {
    check_status(base(), policy_root(), true, no_status());
}
#[test(should_fail_with = "status evidence coverage")]
fn status_evidence_omitted_refused() {
    check_status(status(), policy_root(), false, no_status());
}
#[test(should_fail_with = "snapshot policy")]
fn status_other_policy_refused() {
    check_status(status(), policy_root() + 1, false, evidence());
}
#[test(should_fail_with = "snapshot policy")]
fn status_other_snapshot_refused() {
    let mut e = evidence();
    e[2].snapshot.root = e[2].snapshot.root + 1;
    check_status(status(), policy_root(), false, e);
}
#[test]
fn snapshot_digest_binds_depth_even_with_identical_root() {
    let s = snapshot();
    assert(snapshot_digest::<2>(s) != snapshot_digest::<3>(s));
}
#[test]
fn same_status_bits_at_another_depth_have_another_digest() {
    // Both trees contain [set, clear, clear, set]; the deeper tree pads with set bits.
    let deeper = deeper_evidence()[2].snapshot;
    assert(snapshot_digest::<2>(snapshot()) != snapshot_digest::<3>(deeper));
    let root = merkle_root(snapshot_digest::<3>(deeper), acceptance());
    check_status(status(), root, false, deeper_evidence());
}
#[test(should_fail_with = "snapshot policy")]
fn status_same_bits_at_another_depth_refused() {
    check_status(status(), policy_root(), false, deeper_evidence());
}
#[test(should_fail_with = "snapshot policy")]
fn status_policy_with_another_depth_for_the_same_root_refused() {
    let root = merkle_root(snapshot_digest::<3>(snapshot()), acceptance());
    check_status(status(), root, false, evidence());
}
#[test(should_fail_with = "status path")]
fn status_untagged_leaves_refused() {
    let mut e = evidence();
    let member = Membership { index: 1, siblings: [1, h2(0, 1)] };
    e[2].path = member;
    e[2].snapshot.root = merkle_root(0, member);
    let root = merkle_root(snapshot_digest::<2>(e[2].snapshot), acceptance());
    check_status(status(), root, false, e);
}
#[test(should_fail_with = "status path")]
fn status_leaf_using_another_envelope_tag_refused() {
    let mut e = evidence();
    e[2].snapshot.root = merkle_root(holder_message([0, 0]), path());
    let root = merkle_root(snapshot_digest::<2>(e[2].snapshot), acceptance());
    check_status(status(), root, false, e);
}
#[test(should_fail_with = "snapshot identity")]
fn status_other_list_refused() {
    let mut e = evidence();
    e[2].snapshot.list = iri(801);
    check_status(status(), policy_root(), false, e);
}
#[test(should_fail_with = "snapshot identity")]
fn status_other_purpose_refused() {
    let mut e = evidence();
    e[2].snapshot.purpose = stand_in_purpose(true);
    check_status(status(), policy_root(), false, e);
}
#[test(should_fail_with = "status index binding")]
fn status_other_index_refused() {
    let mut e = evidence();
    e[2].path.index = 0;
    check_status(status(), policy_root(), false, e);
}
#[test(should_fail_with = "status bit set")]
fn status_set_bit_refused() {
    let mut e = evidence();
    e[2].bit = true;
    e[2].snapshot.root = merkle_root(status_leaf(true), path());
    let root = merkle_root(snapshot_digest::<2>(e[2].snapshot), acceptance());
    check_status(status(), root, false, e);
}
#[test(should_fail_with = "status path")]
fn status_other_path_refused() {
    let mut e = evidence();
    e[2].path.siblings[0] = 2;
    check_status(status(), policy_root(), false, e);
}
#[test(should_fail_with = "status index length")]
fn status_outside_snapshot_length_refused() {
    let mut e = evidence();
    e[2].snapshot.bit_len = 1;
    let root = merkle_root(snapshot_digest::<2>(e[2].snapshot), acceptance());
    check_status(status(), root, false, e);
}
#[test(should_fail_with = "snapshot capacity")]
fn status_snapshot_overflow_refused() {
    let mut e = evidence();
    e[2].snapshot.bit_len = 5;
    check_status(status(), policy_root(), false, e);
}
#[test(should_fail_with = "status size")]
fn status_multibit_refused() {
    let mut c = status();
    c.quads[7] = q(
        4,
        stand_in_vocabulary().status_size,
        stand_in_number(2, stand_in_vocabulary().integer_dt),
    );
    c.len = 8;
    check_status(c, policy_root(), false, evidence());
}
#[test]
fn status_explicit_single_bit_accepted() {
    let mut c = status();
    c.quads[7] = q(
        4,
        stand_in_vocabulary().status_size,
        stand_in_number(1, stand_in_vocabulary().integer_dt),
    );
    c.len = 8;
    check_status(c, policy_root(), false, evidence());
}
#[test(should_fail_with = "status purpose")]
fn unsupported_status_purpose_refused() {
    let mut c = status();
    c.quads[5][2] = iri(700);
    check_status(c, policy_root(), false, evidence());
}
#[test(should_fail_with = "required property absent")]
fn incomplete_status_entry_refused() {
    let mut c = status();
    c.quads[6] = [unbound(); 4];
    c.len = 6;
    check_status(c, policy_root(), true, evidence());
}
#[test(should_fail_with = "status evidence coverage")]
fn missing_status_option_does_not_omit_present_entries() {
    check_status(status(), policy_root(), true, no_status());
}
#[test(should_fail_with = "status evidence coverage")]
fn absence_claim_checks_last_quad() {
    let mut c = status();
    for i in 7..15 {
        c.quads[i] = q(50 + i, 600, iri(20));
    }
    c.quads[15] = c.quads[2];
    c.quads[2] = q(40, 600, iri(21));
    c.len = 16;
    check_status(c, policy_root(), true, no_status());
}
#[test(should_fail_with = "credential padding")]
fn shortening_credential_cannot_claim_status_absent() {
    let mut c = status();
    c.len = 2;
    check_status(c, policy_root(), true, no_status());
}
#[test(should_fail_with = "status evidence coverage")]
fn status_extra_evidence_refused() {
    let mut e = evidence();
    e[15].present = true;
    check_status(status(), policy_root(), false, e);
}
#[test]
fn all_status_entries_checked() {
    let mut c = status();
    let v = stand_in_vocabulary();
    c.quads[7] = q(1, v.credential_status, iri(5));
    c.quads[8] = q(5, v.rdf_type, iri(v.status_entry));
    c.quads[9] = q(5, v.status_list, iri(800));
    c.quads[10] = q(5, v.status_purpose, stand_in_purpose(false));
    c.quads[11] = q(5, v.status_index, stand_in_index(1));
    c.len = 12;
    let mut e = evidence();
    e[7].present = true;
    check_status(c, policy_root(), false, e);
}
#[test(should_fail_with = "status evidence coverage")]
fn second_status_entry_cannot_be_omitted() {
    let mut c = status();
    c.quads[7] = q(1, stand_in_vocabulary().credential_status, iri(5));
    c.len = 8;
    check_status(c, policy_root(), false, evidence());
}

#[test]
fn validity_inclusive_endpoints() {
    check_validity(validity(), 100);
    check_validity(validity(), 150);
    check_validity(validity(), 200);
}
#[test(should_fail_with = "before validFrom")]
fn validity_second_before_refused() {
    check_validity(validity(), 99);
}
#[test(should_fail_with = "after validUntil")]
fn validity_second_after_refused() {
    check_validity(validity(), 201);
}
#[test]
fn absent_bounds_are_indefinite() {
    check_validity(base(), 0);
    check_validity(base(), 0xffffffffffffffff);
}
#[test]
fn absent_upper_bound_accepted() {
    let mut c = validity();
    c.len = 3;
    c.quads[3] = [unbound(); 4];
    check_validity(c, 201);
}
#[test]
fn absent_lower_bound_accepted() {
    let mut c = base();
    c.quads[2] = validity().quads[3];
    c.len = 3;
    check_validity(c, 0);
}
#[test]
fn equal_bounds_valid_at_that_instant() {
    let mut c = validity();
    c.quads[3][2] = stand_in_number(100, stand_in_vocabulary().time_dt);
    check_validity(c, 100);
}
#[test(should_fail_with = "validity bounds reversed")]
fn reversed_bounds_refused() {
    let mut c = validity();
    c.quads[3][2] = stand_in_number(99, stand_in_vocabulary().time_dt);
    check_validity(c, 100);
}
#[test(should_fail_with = "typed number opening")]
fn validity_present_unusable_bound_refused() {
    let mut c = validity();
    c.quads[3][2] = iri(700);
    check_validity(c, 201);
}
#[test(should_fail_with = "typed number opening")]
fn validity_changed_key_refused() {
    let mut c = validity();
    c.quads[3][2].key = 300;
    check_validity(c, 201);
}
#[test(should_fail_with = "property cardinality")]
fn validity_duplicate_upper_bound_refused() {
    let mut c = validity();
    c.quads[4] = q(
        1,
        stand_in_vocabulary().valid_until,
        stand_in_number(300, stand_in_vocabulary().time_dt),
    );
    c.len = 5;
    check_validity(c, 150);
}
#[test(should_fail_with = "after validUntil")]
fn upper_bound_in_last_quad_cannot_be_omitted() {
    let mut c = base();
    for i in 2..15 {
        c.quads[i] = q(50 + i, 600, iri(20));
    }
    c.quads[15] = validity().quads[3];
    c.len = 16;
    check_validity(c, 201);
}
#[test(should_fail_with = "credential padding")]
fn shortening_credential_cannot_claim_no_valid_until() {
    let mut c = validity();
    c.len = 3;
    check_validity(c, 201);
}

#[test]
fn request_digest_covers_every_policy_field() {
    let p = policy(base());
    let expected = stand_in_request_digest(10, 20, 30, p);
    assert(expected != stand_in_request_digest(11, 20, 30, p));
    assert(expected != stand_in_request_digest(10, 21, 30, p));
    assert(expected != stand_in_request_digest(10, 20, 31, p));
    let mut changed = p;
    changed.trusted_root = changed.trusted_root + 1;
    assert(expected != stand_in_request_digest(10, 20, 30, changed));
    changed = p;
    changed.holder_required = false;
    assert(expected != stand_in_request_digest(10, 20, 30, changed));
    changed = p;
    changed.status_required = false;
    assert(expected != stand_in_request_digest(10, 20, 30, changed));
    changed = p;
    changed.allow_missing_status = true;
    assert(expected != stand_in_request_digest(10, 20, 30, changed));
    changed = p;
    changed.accepted_snapshots = changed.accepted_snapshots + 1;
    assert(expected != stand_in_request_digest(10, 20, 30, changed));
    changed = p;
    changed.validity_required = false;
    assert(expected != stand_in_request_digest(10, 20, 30, changed));
    changed = p;
    changed.now = 151;
    assert(expected != stand_in_request_digest(10, 20, 30, changed));
}
#[test(should_fail_with = "challenge reduction not bound to its preimage")]
fn holder_signature_for_other_domain_refused() {
    let p = policy(holder());
    let digest = stand_in_request_digest(10, 20, 30, p);
    check_holder(
        holder(),
        stand_in_request_digest(10, 21, 30, p),
        key(1),
        sign(1, holder_message(digest)),
    );
}
#[test]
fn disabled_optional_checks_are_explicit() {
    let c = base();
    let mut p = policy(c);
    p.holder_required = false;
    p.status_required = false;
    p.validity_required = false;
    assert(equal_cell(
        check_envelope(
            c,
            [10, 20],
            p,
            sign(1, 42),
            issuer_path(),
            key(1),
            no_sig(),
            no_status(),
        ),
        iri(900),
    ));
}
#[test(should_fail_with = "required property absent")]
fn envelope_required_holder_cannot_be_omitted() {
    let c = base();
    let _ = check_envelope(
        c,
        [10, 20],
        policy(c),
        sign(1, 42),
        issuer_path(),
        key(1),
        no_sig(),
        no_status(),
    );
}
#[test(should_fail_with = "status entry absent")]
fn envelope_required_status_cannot_be_omitted() {
    let c = holder();
    let _ = check_envelope(
        c,
        [10, 20],
        policy(c),
        sign(1, 42),
        issuer_path(),
        key(1),
        sign(1, holder_message([10, 20])),
        no_status(),
    );
}
#[test(should_fail_with = "after validUntil")]
fn envelope_required_validity_cannot_be_omitted() {
    let c = validity();
    let mut p = policy(c);
    p.holder_required = false;
    p.status_required = false;
    p.now = 201;
    let _ = check_envelope(
        c,
        [10, 20],
        p,
        sign(1, 42),
        issuer_path(),
        key(1),
        no_sig(),
        no_status(),
    );
}

#[test(should_fail_with = "credential named graph")]
fn input_named_graph_refused() {
    let mut c = base();
    c.quads[0][3] = iri(1000);
    stand_in_open_credential(c);
}
#[test(should_fail_with = "reserved issuer predicate")]
fn input_method_assertion_refused() {
    let mut c = base();
    c.quads[2] = q(1, stand_in_vocabulary().verified_issuer, iri(900));
    c.len = 3;
    stand_in_open_credential(c);
}
#[test(should_fail_with = "credential capacity")]
fn credential_overflow_refused() {
    let mut c = base();
    c.len = 17;
    stand_in_open_credential(c);
}
#[test(should_fail_with = "blank scope")]
fn another_credential_blank_scope_refused() {
    let mut c = base();
    c.quads[1][2].kind = 2;
    c.quads[1][2].scope = 2;
    stand_in_open_credential(c);
}
#[test]
fn exact_absence_read_covers_all_rows() {
    let c = base();
    let (present, cell) = property(c, c.root, stand_in_vocabulary().valid_until);
    assert(!present);
    assert(equal_cell(cell, unbound()));
}

// Hand-built GRAPH + verifiedIssuer pin at the dataset boundary, before query integration.
fn pinned_fact<let R: u32>(d: Table<R, 4>, fact: Field, issuer: Field) -> bool {
    let mut found = false;
    let v = stand_in_vocabulary();
    for i in 0..R {
        for j in 0..R {
            found = found
                | (
                    (i < d.len)
                        & (j < d.len)
                        & same_term(d.rows[i][1], iri(600))
                        & same_term(d.rows[i][2], iri(fact))
                        & d.rows[i][3].bound
                        & same_term(d.rows[j][0], d.rows[i][3])
                        & same_term(d.rows[j][1], iri(v.verified_issuer))
                        & same_term(d.rows[j][2], iri(issuer))
                        & !d.rows[j][3].bound
                );
        }
    }
    found
}
#[test]
fn two_issuer_fact_pins_and_swapped_pins() {
    let mut a = base();
    a.quads[2] = q(10, 600, iri(700));
    a.len = 3;
    let mut b = base();
    b.quads[1][2] = iri(902);
    b.issuer_key = key(2);
    b.verification_method = iri(903);
    b.credential_index = 1;
    b.quads[2] = q(11, 600, iri(701));
    b.len = 3;
    let leaf_a = trusted_key_leaf(iri(900), a.verification_method, a.issuer_key);
    let leaf_b = trusted_key_leaf(iri(902), b.verification_method, b.issuer_key);
    let root = h2(leaf_a, leaf_b);
    let ai = check_issuer(
        a,
        root,
        sign(1, 42),
        Membership { index: 0, siblings: [leaf_b] },
    );
    let bi = check_issuer(
        b,
        root,
        sign(2, 42),
        Membership { index: 1, siblings: [leaf_a] },
    );
    let ad = credential_dataset::<16, 7>(a, ai, [10, 20]);
    let bd = credential_dataset::<16, 7>(b, bi, [10, 20]);
    check_table(ad);
    check_table(bd);
    assert(ad.len == 7);
    assert(bd.len == 7);
    assert(pinned_fact(ad, 700, 900) & pinned_fact(bd, 701, 902));
    assert(!pinned_fact(ad, 700, 902) & !pinned_fact(bd, 701, 900));
}
#[test(should_fail_with = "dataset issuer binding")]
fn dataset_swapped_issuer_refused() {
    let _ = credential_dataset::<16, 5>(base(), iri(902), [10, 20]);
}
#[test(should_fail_with = "operator capacity")]
fn dataset_capacity_refused() {
    let _ = credential_dataset::<16, 4>(base(), iri(900), [10, 20]);
}

#[test]
fn envelope_all_requirements_accepted() {
    let mut c = holder();
    let v = stand_in_vocabulary();
    let st = status();
    for i in 0..5 {
        c.quads[6 + i] = st.quads[2 + i];
    }
    c.quads[11] = validity().quads[2];
    c.quads[12] = validity().quads[3];
    c.len = 13;
    let mut e = no_status();
    e[6].present = true;
    let p = policy(c);
    let digest = stand_in_request_digest(10, 20, 30, p);
    assert(equal_cell(
        check_envelope(
            c,
            digest,
            p,
            sign(1, 42),
            issuer_path(),
            key(1),
            sign(1, holder_message(digest)),
            e,
        ),
        iri(900),
    ));
    assert(v.valid_from != v.valid_until);
}
#[test]
fn additional_credential_types_accepted() {
    let mut c = validity();
    c.quads[4] = q(1, stand_in_vocabulary().rdf_type, iri(555));
    c.len = 5;
    check_validity(c, 150);
}
#[test]
fn unrelated_subject_bounds_do_not_set_credential_period() {
    let mut c = base();
    c.quads[2] = q(
        2,
        stand_in_vocabulary().valid_until,
        stand_in_number(100, stand_in_vocabulary().time_dt),
    );
    c.len = 3;
    check_validity(c, 201);
}
#[test(should_fail_with = "status index binding")]
fn status_non_decimal_index_cell_refused() {
    let mut c = status();
    c.quads[6][2] = stand_in_number(1, stand_in_vocabulary().integer_dt);
    check_status(c, policy_root(), false, evidence());
}
#[test(should_fail_with = "typed number opening")]
fn validity_wrong_datatype_refused() {
    let mut c = validity();
    c.quads[3][2] = stand_in_number(200, stand_in_vocabulary().integer_dt);
    check_validity(c, 150);
}
#[test(should_fail_with = "holder key type")]
fn holder_unsupported_method_refused() {
    let mut c = holder();
    c.quads[4][2] = iri(500);
    check_holder(c, [10, 20], key(1), no_sig());
}
#[test(should_fail_with = "status entry type")]
fn status_unsupported_entry_refused() {
    let mut c = status();
    c.quads[3][2] = iri(500);
    check_status(c, policy_root(), false, evidence());
}

#[test(should_fail_with = "duplicate credential quad")]
fn duplicate_input_quad_refused() {
    let mut c = base();
    c.quads[2] = c.quads[0];
    c.len = 3;
    stand_in_open_credential(c);
}
#[test(should_fail_with = "signature evidence absent")]
fn required_holder_signature_cannot_be_omitted_in_envelope() {
    let c = holder();
    let mut p = policy(c);
    p.status_required = false;
    let _ = check_envelope(
        c,
        [10, 20],
        p,
        sign(1, 42),
        issuer_path(),
        key(1),
        no_sig(),
        no_status(),
    );
}
#[test]
fn dataset_graph_names_are_request_local() {
    let a = credential_dataset::<16, 5>(base(), iri(900), [10, 20]);
    let b = credential_dataset::<16, 5>(base(), iri(900), [10, 21]);
    let mut a_graph = unbound();
    let mut b_graph = unbound();
    for i in 0..5 {
        if a.rows[i][3].bound {
            a_graph = a.rows[i][3];
        }
        if b.rows[i][3].bound {
            b_graph = b.rows[i][3];
        }
    }
    assert(a_graph.bound & b_graph.bound);
    assert(!same_term(a_graph, b_graph));
}

#[test(should_fail_with = "before validFrom")]
fn absent_bounds_cannot_precede_creation() {
    let mut c = base();
    c.created_at = stand_in_number(100, stand_in_vocabulary().time_dt);
    check_validity(c, 99);
}
#[test]
fn absent_bounds_include_creation_and_have_no_end() {
    let mut c = base();
    c.created_at = stand_in_number(100, stand_in_vocabulary().time_dt);
    check_validity(c, 100);
    check_validity(c, 0xffffffffffffffff);
}
#[test(should_fail_with = "creation time unavailable")]
fn absent_bounds_with_unknown_creation_refused() {
    let mut c = base();
    c.created_at = unbound();
    check_validity(c, 150);
}
