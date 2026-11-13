mod native;
use compose_core::hashes::{commit_fold, h2};
use compose_core::issuer::{BJJ_GX, BJJ_GY, scalar_mul};
use envelope_core::envelope::{
    check_envelope, check_holder, check_issuer, check_status, check_validity, Credential,
    credential_dataset, holder_message, iri, Membership, merkle_root, Point, property, PublicPolicy,
    Signature, Snapshot, snapshot_digest, stand_in_index, stand_in_key_cell, stand_in_number,
    stand_in_open_credential, stand_in_purpose, status_leaf, StatusEvidence, TermCell,
    trusted_key_leaf, vocabulary,
};
use envelope_core::table::{check_table, equal_cell, same_term, Table, unbound};

fn main() {}
fn request_digest(high: u128, low: u128) -> [u8; 32] {
    let hi: [u8; 16] = (high as Field).to_be_bytes();
    let lo: [u8; 16] = (low as Field).to_be_bytes();
    let mut bytes = [0; 32];
    for i in 0..16 {
        bytes[i] = hi[i];
        bytes[16 + i] = lo[i];
    }
    bytes
}

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
    let v = vocabulary();
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
    let v = vocabulary();
    c.quads[2] = q(1, v.subject, iri(2));
    c.quads[3] = q(2, v.confidence, iri(3));
    c.quads[4] = q(3, v.rdf_type, iri(v.multikey));
    c.quads[5] = q(3, v.public_key, stand_in_key_cell(key(1)));
    c.len = 6;
    c
}
fn validity() -> Credential<16> {
    let mut c = base();
    let v = vocabulary();
    c.quads[2] = q(1, v.valid_from, stand_in_number(100, v.time_dt));
    c.quads[3] = q(1, v.valid_until, stand_in_number(200, v.time_dt));
    c.len = 4;
    c
}
fn status() -> Credential<16> {
    let mut c = base();
    let v = vocabulary();
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
fn policy(c: Credential<16>) -> PublicPolicy {
    PublicPolicy {
        issuer_root: issuer_root(c),
        issuer_depth: 1,
        holder_required: true,
        holder_issuers_present: false,
        holder_issuers: [0; 4],
        holder_issuer_count: 0,
        status_required: true,
        allow_missing_status: false,
        snapshot_root: policy_root(),
        snapshot_depth: 1,
        validity_required: true,
        now: 150,
        freshness_required: false,
        earliest_created: 0,
    }
}

#[test]
fn issuer_accepted_and_identity_returned() {
    let c = base();
    assert(
        equal_cell(
            check_issuer(c, issuer_root(c), 1, sign(1, 42), issuer_path()),
            iri(900),
        ),
    );
}
#[test(should_fail_with = "required property absent")]
fn issuer_missing_refused() {
    let mut c = base();
    c.len = 1;
    c.quads[1] = [unbound(); 4];
    let _ = check_issuer(c, 1, 1, no_sig(), issuer_path());
}
#[test(should_fail_with = "signature evidence absent")]
fn issuer_evidence_omitted_refused() {
    let c = base();
    let _ = check_issuer(c, issuer_root(c), 1, no_sig(), issuer_path());
}
#[test(should_fail_with = "issuer authorization set")]
fn another_key_set_refused() {
    let _ = check_issuer(
        base(),
        issuer_root(base()) + 1,
        1,
        sign(1, 42),
        issuer_path(),
    );
}
#[test(should_fail_with = "issuer authorization set")]
fn another_issuer_label_refused() {
    let mut c = base();
    let root = issuer_root(c);
    c.quads[1][2] = iri(902);
    let _ = check_issuer(c, root, 1, sign(1, 42), issuer_path());
}
#[test(should_fail_with = "issuer authorization set")]
fn another_verification_method_refused() {
    let mut c = base();
    let root = issuer_root(c);
    c.verification_method = iri(903);
    let _ = check_issuer(c, root, 1, sign(1, 42), issuer_path());
}
#[test(should_fail_with = "challenge reduction not bound to its preimage")]
fn another_issuer_message_refused() {
    let mut c = base();
    c.signed_message = 43;
    let _ = check_issuer(c, issuer_root(c), 1, sign(1, 42), issuer_path());
}
#[test(should_fail_with = "issuer authorization set")]
fn different_issuer_key_refused() {
    let mut c = base();
    let root = issuer_root(c);
    c.issuer_key = key(2);
    let _ = check_issuer(c, root, 1, sign(2, 42), issuer_path());
}

#[test]
fn holder_request_signature_accepted() {
    check_holder(
        holder(),
        request_digest(10, 20),
        key(1),
        sign(1, holder_message(request_digest(10, 20))),
    );
}
#[test(should_fail_with = "required property absent")]
fn holder_key_absent_refused() {
    check_holder(base(), request_digest(10, 20), key(1), no_sig());
}
#[test(should_fail_with = "signature evidence absent")]
fn holder_signature_omitted_refused() {
    check_holder(holder(), request_digest(10, 20), key(1), no_sig());
}
#[test(should_fail_with = "holder key binding")]
fn holder_other_key_refused() {
    check_holder(
        holder(),
        request_digest(10, 20),
        key(2),
        sign(2, holder_message(request_digest(10, 20))),
    );
}
#[test(should_fail_with = "challenge reduction not bound to its preimage")]
fn holder_other_request_refused() {
    check_holder(
        holder(),
        request_digest(10, 21),
        key(1),
        sign(1, holder_message(request_digest(10, 20))),
    );
}
#[test(should_fail_with = "challenge reduction not bound to its preimage")]
fn holder_other_high_digest_limb_refused() {
    check_holder(
        holder(),
        request_digest(11, 20),
        key(1),
        sign(1, holder_message(request_digest(10, 20))),
    );
}
#[test(should_fail_with = "property cardinality")]
fn holder_ambiguous_key_refused() {
    let mut c = holder();
    c.quads[6] = q(3, vocabulary().public_key, stand_in_key_cell(key(2)));
    c.len = 7;
    check_holder(c, request_digest(10, 20), key(1), no_sig());
}

#[test]
fn status_active_accepted() {
    check_status(status(), policy_root(), 1, false, evidence());
}
#[test]
fn suspension_active_accepted() {
    let mut c = status();
    c.quads[5][2] = stand_in_purpose(true);
    let mut e = evidence();
    e[2].snapshot.purpose = stand_in_purpose(true);
    let root = merkle_root(snapshot_digest::<2>(e[2].snapshot), acceptance());
    check_status(c, root, 1, false, e);
}
#[test(should_fail_with = "status entry absent")]
fn status_absent_required_refused() {
    check_status(base(), policy_root(), 1, false, no_status());
}
#[test]
fn status_absence_explicitly_permitted() {
    check_status(base(), policy_root(), 1, true, no_status());
}
#[test(should_fail_with = "status evidence coverage")]
fn status_evidence_omitted_refused() {
    check_status(status(), policy_root(), 1, false, no_status());
}
#[test(should_fail_with = "snapshot policy")]
fn status_other_policy_refused() {
    check_status(status(), policy_root() + 1, 1, false, evidence());
}
#[test(should_fail_with = "snapshot policy")]
fn status_other_snapshot_refused() {
    let mut e = evidence();
    e[2].snapshot.root = e[2].snapshot.root + 1;
    check_status(status(), policy_root(), 1, false, e);
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
    check_status(status(), root, 1, false, deeper_evidence());
}
#[test(should_fail_with = "snapshot policy")]
fn status_same_bits_at_another_depth_refused() {
    check_status(status(), policy_root(), 1, false, deeper_evidence());
}
#[test(should_fail_with = "snapshot policy")]
fn status_policy_with_another_depth_for_the_same_root_refused() {
    let root = merkle_root(snapshot_digest::<3>(snapshot()), acceptance());
    check_status(status(), root, 1, false, evidence());
}
#[test(should_fail_with = "status path")]
fn status_untagged_leaves_refused() {
    let mut e = evidence();
    let member = Membership { index: 1, siblings: [1, h2(0, 1)] };
    e[2].path = member;
    e[2].snapshot.root = merkle_root(0, member);
    let root = merkle_root(snapshot_digest::<2>(e[2].snapshot), acceptance());
    check_status(status(), root, 1, false, e);
}
#[test(should_fail_with = "status path")]
fn status_leaf_using_another_envelope_tag_refused() {
    let mut e = evidence();
    e[2].snapshot.root = merkle_root(holder_message(request_digest(0, 0)), path());
    let root = merkle_root(snapshot_digest::<2>(e[2].snapshot), acceptance());
    check_status(status(), root, 1, false, e);
}
#[test(should_fail_with = "snapshot identity")]
fn status_other_list_refused() {
    let mut e = evidence();
    e[2].snapshot.list = iri(801);
    check_status(status(), policy_root(), 1, false, e);
}
#[test(should_fail_with = "snapshot identity")]
fn status_other_purpose_refused() {
    let mut e = evidence();
    e[2].snapshot.purpose = stand_in_purpose(true);
    check_status(status(), policy_root(), 1, false, e);
}
#[test(should_fail_with = "status index binding")]
fn status_other_index_refused() {
    let mut e = evidence();
    e[2].path.index = 0;
    check_status(status(), policy_root(), 1, false, e);
}
#[test(should_fail_with = "status bit set")]
fn status_set_bit_refused() {
    let mut e = evidence();
    e[2].bit = true;
    e[2].snapshot.root = merkle_root(status_leaf(true), path());
    let root = merkle_root(snapshot_digest::<2>(e[2].snapshot), acceptance());
    check_status(status(), root, 1, false, e);
}
#[test(should_fail_with = "status path")]
fn status_other_path_refused() {
    let mut e = evidence();
    e[2].path.siblings[0] = 2;
    check_status(status(), policy_root(), 1, false, e);
}
#[test(should_fail_with = "status index length")]
fn status_outside_snapshot_length_refused() {
    let mut e = evidence();
    e[2].snapshot.bit_len = 1;
    let root = merkle_root(snapshot_digest::<2>(e[2].snapshot), acceptance());
    check_status(status(), root, 1, false, e);
}
#[test(should_fail_with = "snapshot capacity")]
fn status_snapshot_overflow_refused() {
    let mut e = evidence();
    e[2].snapshot.bit_len = 5;
    check_status(status(), policy_root(), 1, false, e);
}
#[test(should_fail_with = "status size")]
fn status_multibit_refused() {
    let mut c = status();
    c.quads[7] = q(
        4,
        vocabulary().status_size,
        stand_in_number(2, vocabulary().integer_dt),
    );
    c.len = 8;
    check_status(c, policy_root(), 1, false, evidence());
}
#[test]
fn status_explicit_single_bit_accepted() {
    let mut c = status();
    c.quads[7] = q(
        4,
        vocabulary().status_size,
        stand_in_number(1, vocabulary().integer_dt),
    );
    c.len = 8;
    check_status(c, policy_root(), 1, false, evidence());
}
#[test(should_fail_with = "status purpose")]
fn unsupported_status_purpose_refused() {
    let mut c = status();
    c.quads[5][2] = iri(700);
    check_status(c, policy_root(), 1, false, evidence());
}
#[test(should_fail_with = "required property absent")]
fn incomplete_status_entry_refused() {
    let mut c = status();
    c.quads[6] = [unbound(); 4];
    c.len = 6;
    check_status(c, policy_root(), 1, true, evidence());
}
#[test(should_fail_with = "status evidence coverage")]
fn missing_status_option_does_not_omit_present_entries() {
    check_status(status(), policy_root(), 1, true, no_status());
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
    check_status(c, policy_root(), 1, true, no_status());
}
#[test(should_fail_with = "credential padding")]
fn shortening_credential_cannot_claim_status_absent() {
    let mut c = status();
    c.len = 2;
    check_status(c, policy_root(), 1, true, no_status());
}
#[test(should_fail_with = "status evidence coverage")]
fn status_extra_evidence_refused() {
    let mut e = evidence();
    e[15].present = true;
    check_status(status(), policy_root(), 1, false, e);
}
#[test]
fn all_status_entries_checked() {
    let mut c = status();
    let v = vocabulary();
    c.quads[7] = q(1, v.credential_status, iri(5));
    c.quads[8] = q(5, v.rdf_type, iri(v.status_entry));
    c.quads[9] = q(5, v.status_list, iri(800));
    c.quads[10] = q(5, v.status_purpose, stand_in_purpose(false));
    c.quads[11] = q(5, v.status_index, stand_in_index(1));
    c.len = 12;
    let mut e = evidence();
    e[7].present = true;
    check_status(c, policy_root(), 1, false, e);
}
#[test(should_fail_with = "status evidence coverage")]
fn second_status_entry_cannot_be_omitted() {
    let mut c = status();
    c.quads[7] = q(1, vocabulary().credential_status, iri(5));
    c.len = 8;
    check_status(c, policy_root(), 1, false, evidence());
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
    c.quads[3][2] = stand_in_number(100, vocabulary().time_dt);
    check_validity(c, 100);
}
#[test(should_fail_with = "validity bounds reversed")]
fn reversed_bounds_refused() {
    let mut c = validity();
    c.quads[3][2] = stand_in_number(99, vocabulary().time_dt);
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
        vocabulary().valid_until,
        stand_in_number(300, vocabulary().time_dt),
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

#[test(should_fail_with = "challenge reduction not bound to its preimage")]
fn holder_signature_for_other_domain_refused() {
    let digest = request_digest(10, 20);
    check_holder(
        holder(),
        request_digest(10, 21),
        key(1),
        sign(1, holder_message(digest)),
    );
}
#[test]
fn disabled_optional_checks_are_explicit() {
    let c = base();
    let mut p = policy(c);
    p.holder_required = false;
    p.holder_issuers_present = false;
    p.holder_issuer_count = 0;
    p.holder_issuers = [0; 4];
    p.status_required = false;
    p.snapshot_root = 0;
    p.snapshot_depth = 0;
    p.allow_missing_status = false;
    p.validity_required = false;
    p.now = 0;
    assert(equal_cell(
        check_envelope(
            c,
            request_digest(10, 20),
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
        request_digest(10, 20),
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
        request_digest(10, 20),
        policy(c),
        sign(1, 42),
        issuer_path(),
        key(1),
        sign(1, holder_message(request_digest(10, 20))),
        no_status(),
    );
}
#[test(should_fail_with = "after validUntil")]
fn envelope_required_validity_cannot_be_omitted() {
    let c = validity();
    let mut p = policy(c);
    p.holder_required = false;
    p.holder_issuers_present = false;
    p.holder_issuer_count = 0;
    p.holder_issuers = [0; 4];
    p.status_required = false;
    p.snapshot_root = 0;
    p.snapshot_depth = 0;
    p.allow_missing_status = false;
    p.now = 201;
    let _ = check_envelope(
        c,
        request_digest(10, 20),
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
    c.quads[2] = q(1, vocabulary().verified_issuer, iri(900));
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
    let (present, cell) = property(c, c.root, vocabulary().valid_until);
    assert(!present);
    assert(equal_cell(cell, unbound()));
}

// Hand-built GRAPH + verifiedIssuer pin at the dataset boundary, before query integration.
fn pinned_fact<let R: u32>(d: Table<R, 4>, fact: Field, issuer: Field) -> bool {
    let mut found = false;
    let v = vocabulary();
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
        1,
        sign(1, 42),
        Membership { index: 0, siblings: [leaf_b] },
    );
    let bi = check_issuer(
        b,
        root,
        1,
        sign(2, 42),
        Membership { index: 1, siblings: [leaf_a] },
    );
    let ad = credential_dataset::<16, 7>(a, ai, 20);
    let bd = credential_dataset::<16, 7>(b, bi, 20);
    check_table(ad);
    check_table(bd);
    assert(ad.len == 7);
    assert(bd.len == 7);
    assert(pinned_fact(ad, 700, 900) & pinned_fact(bd, 701, 902));
    assert(!pinned_fact(ad, 700, 902) & !pinned_fact(bd, 701, 900));
}
#[test(should_fail_with = "dataset issuer binding")]
fn dataset_swapped_issuer_refused() {
    let _ = credential_dataset::<16, 5>(base(), iri(902), 20);
}
#[test(should_fail_with = "operator capacity")]
fn dataset_capacity_refused() {
    let _ = credential_dataset::<16, 4>(base(), iri(900), 20);
}

#[test]
fn envelope_all_requirements_accepted() {
    let mut c = holder();
    let v = vocabulary();
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
    let digest = request_digest(10, 20);
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
    c.quads[4] = q(1, vocabulary().rdf_type, iri(555));
    c.len = 5;
    check_validity(c, 150);
}
#[test]
fn unrelated_subject_bounds_do_not_set_credential_period() {
    let mut c = base();
    c.quads[2] = q(
        2,
        vocabulary().valid_until,
        stand_in_number(100, vocabulary().time_dt),
    );
    c.len = 3;
    check_validity(c, 201);
}
#[test(should_fail_with = "status index binding")]
fn status_non_decimal_index_cell_refused() {
    let mut c = status();
    c.quads[6][2] = stand_in_number(1, vocabulary().integer_dt);
    check_status(c, policy_root(), 1, false, evidence());
}
#[test(should_fail_with = "typed number opening")]
fn validity_wrong_datatype_refused() {
    let mut c = validity();
    c.quads[3][2] = stand_in_number(200, vocabulary().integer_dt);
    check_validity(c, 150);
}
#[test(should_fail_with = "holder key type")]
fn holder_unsupported_method_refused() {
    let mut c = holder();
    c.quads[4][2] = iri(500);
    check_holder(c, request_digest(10, 20), key(1), no_sig());
}
#[test(should_fail_with = "status entry type")]
fn status_unsupported_entry_refused() {
    let mut c = status();
    c.quads[3][2] = iri(500);
    check_status(c, policy_root(), 1, false, evidence());
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
    p.snapshot_root = 0;
    p.snapshot_depth = 0;
    p.allow_missing_status = false;
    let _ = check_envelope(
        c,
        request_digest(10, 20),
        p,
        sign(1, 42),
        issuer_path(),
        key(1),
        no_sig(),
        no_status(),
    );
}
#[test]
fn dataset_graph_names_are_presentation_local() {
    let a = credential_dataset::<16, 5>(base(), iri(900), 20);
    let b = credential_dataset::<16, 5>(base(), iri(900), 21);
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
    c.created_at = stand_in_number(100, vocabulary().time_dt);
    check_validity(c, 99);
}
#[test]
fn absent_bounds_include_creation_and_have_no_end() {
    let mut c = base();
    c.created_at = stand_in_number(100, vocabulary().time_dt);
    check_validity(c, 100);
    check_validity(c, 0xffffffffffffffff);
}
#[test(should_fail_with = "creation time unavailable")]
fn absent_bounds_with_unknown_creation_refused() {
    let mut c = base();
    c.created_at = unbound();
    check_validity(c, 150);
}

#[test]
fn freshness_includes_boundary_second() {
    let mut c = validity();
    c.created_at = stand_in_number(100, vocabulary().time_dt);
    envelope_core::envelope::check_freshness(c, true, 100);
}
#[test(should_fail_with = "before earliestCreated")]
fn freshness_one_second_earlier_refused() {
    let mut c = validity();
    c.created_at = stand_in_number(99, vocabulary().time_dt);
    envelope_core::envelope::check_freshness(c, true, 100);
}
#[test(should_fail_with = "creation time unavailable")]
fn freshness_unknown_creation_refused() {
    let mut c = validity();
    c.created_at = unbound();
    envelope_core::envelope::check_freshness(c, true, 100);
}
#[test]
fn absent_freshness_imposes_nothing() {
    let mut c = validity();
    c.created_at = unbound();
    envelope_core::envelope::check_freshness(c, false, 100);
    check_validity(c, 150);
}
#[test(should_fail_with = "before earliestCreated")]
fn envelope_freshness_runs_with_explicit_validity_bounds() {
    let c = validity();
    let mut p = policy(c);
    p.holder_required = false;
    p.holder_issuers_present = false;
    p.holder_issuer_count = 0;
    p.holder_issuers = [0; 4];
    p.status_required = false;
    p.snapshot_root = 0;
    p.snapshot_depth = 0;
    p.allow_missing_status = false;
    p.freshness_required = true;
    p.earliest_created = 1;
    let _ = check_envelope(
        c,
        request_digest(10, 20),
        p,
        sign(1, 42),
        issuer_path(),
        key(1),
        no_sig(),
        no_status(),
    );
}
fn scoped_policy(issuer: TermCell) -> PublicPolicy {
    let mut p = policy(base());
    p.holder_issuers_present = true;
    p.holder_issuer_count = 1;
    p.holder_issuers[0] = issuer.enc;
    p.status_required = false;
    p.snapshot_root = 0;
    p.snapshot_depth = 0;
    p.allow_missing_status = false;
    p
}
#[test]
fn unlisted_issuer_without_holder_key_accepted() {
    let c = base();
    let p = scoped_policy(iri(902));
    let _ = check_envelope(
        c,
        request_digest(10, 20),
        p,
        sign(1, 42),
        issuer_path(),
        key(1),
        no_sig(),
        no_status(),
    );
}
#[test(should_fail_with = "required property absent")]
fn listed_issuer_without_holder_key_refused() {
    let c = base();
    let p = scoped_policy(iri(900));
    let _ = check_envelope(
        c,
        request_digest(10, 20),
        p,
        sign(1, 42),
        issuer_path(),
        key(1),
        no_sig(),
        no_status(),
    );
}
fn holder_evidence(secret: Field, digest: [u8; 32]) -> envelope_core::envelope::HolderEvidence {
    envelope_core::envelope::HolderEvidence {
        pk: key(secret),
        sig: sign(secret, holder_message(digest)),
    }
}
fn empty_holder_evidence() -> envelope_core::envelope::HolderEvidence {
    envelope_core::envelope::HolderEvidence { pk: key(1), sig: no_sig() }
}
#[test]
fn two_credentials_share_one_holder_signature() {
    envelope_core::envelope::check_holders(
        [holder(), holder()],
        policy(holder()),
        request_digest(10, 20),
        [0, 0],
        [holder_evidence(1, request_digest(10, 20)), empty_holder_evidence()],
    );
}
#[test]
fn two_distinct_holder_keys_use_two_signatures() {
    let mut b = holder();
    b.quads[5][2] = stand_in_key_cell(key(2));
    envelope_core::envelope::check_holders(
        [holder(), b],
        policy(holder()),
        request_digest(10, 20),
        [0, 1],
        [holder_evidence(1, request_digest(10, 20)), holder_evidence(2, request_digest(10, 20))],
    );
}
#[test(should_fail_with = "holder evidence coverage")]
fn two_distinct_holder_keys_with_one_signature_refused() {
    let mut b = holder();
    b.quads[5][2] = stand_in_key_cell(key(2));
    let mut absent = empty_holder_evidence();
    absent.pk = key(2);
    envelope_core::envelope::check_holders(
        [holder(), b],
        policy(holder()),
        request_digest(10, 20),
        [0, 1],
        [holder_evidence(1, request_digest(10, 20)), absent],
    );
}
#[test(should_fail_with = "duplicate holder key")]
fn repeated_signature_for_the_same_holder_key_refused() {
    envelope_core::envelope::check_holders(
        [holder(), holder()],
        policy(holder()),
        request_digest(10, 20),
        [0, 1],
        [holder_evidence(1, request_digest(10, 20)), holder_evidence(1, request_digest(10, 20))],
    );
}
#[test(should_fail_with = "holder key binding")]
fn distinct_holder_keys_cannot_share_a_slot() {
    let mut b = holder();
    b.quads[5][2] = stand_in_key_cell(key(2));
    envelope_core::envelope::check_holders(
        [holder(), b],
        policy(holder()),
        request_digest(10, 20),
        [0, 0],
        [holder_evidence(1, request_digest(10, 20)), empty_holder_evidence()],
    );
}
#[test(should_fail_with = "empty holder issuer list")]
fn explicit_empty_holder_issuer_list_refused() {
    let mut p = scoped_policy(iri(900));
    p.holder_issuer_count = 0;
    p.holder_issuers = [0; 4];
    envelope_core::envelope::check_holders(
        [base()],
        p,
        request_digest(10, 20),
        [0],
        [empty_holder_evidence()],
    );
}
#[test(should_fail_with = "holder issuer padding")]
fn holder_issuer_list_requires_zero_padding() {
    let mut p = scoped_policy(iri(900));
    p.holder_issuers[1] = 902;
    let _ = envelope_core::envelope::holder_applies(iri(900), p);
}
#[test]
fn canonical_roots_ignore_member_order_and_duplicates() {
    let a = trusted_key_leaf(iri(900), iri(901), key(1));
    let b = trusted_key_leaf(iri(902), iri(903), key(2));
    let (r, d, n) = native::canonical_set_root([a, b, a, 0], 3);
    let (s, e, m) = native::canonical_set_root([b, a], 2);
    assert((r == s) & (d == e) & (n == m) & (d == 1) & (n == 2));
    let x = snapshot_digest::<2>(snapshot());
    let mut other = snapshot();
    other.root = other.root + 1;
    let y = snapshot_digest::<2>(other);
    let (r, _, _) = native::canonical_set_root([x, y], 2);
    let (s, _, _) = native::canonical_set_root([y, x], 2);
    assert(r == s);
}
#[test]
fn canonical_root_has_minimal_depth_zero_padding_and_unsigned_order() {
    let (root, d, n) = native::canonical_set_root([-1, 2, 1, 0], 3);
    assert((d == 2) & (n == 3) & (root == h2(h2(1, 2), h2(-1, 0))));
    let (root, d, n) = native::canonical_set_root([2, 2, 0, 0], 2);
    assert((d == 0) & (n == 1) & (root == 2));
}
#[test(should_fail_with = "zero set member")]
fn canonical_set_zero_member_refused() {
    let _ = native::canonical_set_root([0, 1], 2);
}
#[test(should_fail_with = "set count")]
fn canonical_set_empty_refused() {
    let _ = native::canonical_set_root([0, 0], 0);
}
#[test(should_fail_with = "set input padding")]
fn canonical_set_nonzero_padding_refused() {
    let _ = native::canonical_set_root([1, 2], 1);
}
#[test]
fn same_triple_has_one_default_copy_and_each_named_copy() {
    let a = base();
    let mut b = base();
    b.credential_index = 1;
    let ad = credential_dataset::<16, 5>(a, iri(900), 20);
    let bd = credential_dataset::<16, 5>(b, iri(900), 20);
    let merged = envelope_core::envelope::merge_datasets::<2, 5, 10>([ad, bd]);
    check_table(merged);
    let mut defaults: u32 = 0;
    let mut named: u32 = 0;
    for i in 0..10 {
        if (i < merged.len)
            & same_term(merged.rows[i][0], a.quads[0][0])
            & same_term(merged.rows[i][1], a.quads[0][1])
            & same_term(merged.rows[i][2], a.quads[0][2]) {
            if merged.rows[i][3].bound {
                named = named + 1;
            } else {
                defaults = defaults + 1;
            }
        }
    }
    assert((defaults == 1) & (named == 2) & (merged.len == 8));
}
fn complete() -> Credential<16> {
    let mut c = holder();
    for i in 0..5 {
        c.quads[6 + i] = status().quads[2 + i];
    }
    c.quads[11] = validity().quads[2];
    c.quads[12] = validity().quads[3];
    c.len = 13;
    c.created_at = stand_in_number(100, vocabulary().time_dt);
    c
}
#[test]
fn four_credentials_enforce_all_conditions_and_merge() {
    let mut credentials = [complete(); 4];
    for i in 0..4 {
        credentials[i].credential_index = i;
    }
    let mut p = policy(credentials[0]);
    p.freshness_required = true;
    p.earliest_created = 100;
    let digest = request_digest(10, 20);
    let mut e = no_status();
    e[6].present = true;
    let verified = envelope_core::envelope::check_public_envelope(
        credentials,
        digest,
        p,
        [sign(1, 42); 4],
        [issuer_path(); 4],
        [
            holder_evidence(1, digest),
            empty_holder_evidence(),
            empty_holder_evidence(),
            empty_holder_evidence(),
        ],
        [0; 4],
        [e; 4],
    );
    let mut datasets = [envelope_core::table::empty::<27, 4>(); 4];
    for i in 0..4 {
        datasets[i] = credential_dataset(credentials[i], verified[i], 20);
    }
    let merged = envelope_core::envelope::merge_datasets::<4, 27, 69>(datasets);
    check_table(merged);
    // Thirteen shared default triples, their four named copies, four assertions.
    assert(merged.len == 69);
}
fn run_four(last: Credential<16>) {
    let mut credentials = [complete(); 4];
    credentials[3] = last;
    for i in 0..4 {
        credentials[i].credential_index = i;
    }
    let mut p = policy(credentials[0]);
    p.freshness_required = true;
    p.earliest_created = 100;
    let digest = request_digest(10, 20);
    let mut e = no_status();
    e[6].present = true;
    let _ = envelope_core::envelope::check_public_envelope(
        credentials,
        digest,
        p,
        [sign(1, 42); 4],
        [issuer_path(); 4],
        [
            holder_evidence(1, digest),
            empty_holder_evidence(),
            empty_holder_evidence(),
            empty_holder_evidence(),
        ],
        [0; 4],
        [e; 4],
    );
}
#[test(should_fail_with = "issuer authorization set")]
fn fourth_credential_issuer_is_checked() {
    let mut c = complete();
    c.quads[1][2] = iri(902);
    run_four(c);
}
#[test(should_fail_with = "status evidence coverage")]
fn fourth_credential_status_is_checked() {
    let mut c = complete();
    c.quads[6] = q(4, 600, iri(20));
    run_four(c);
}
#[test(should_fail_with = "after validUntil")]
fn fourth_credential_validity_is_checked() {
    let mut c = complete();
    c.quads[12][2] = stand_in_number(149, vocabulary().time_dt);
    run_four(c);
}
#[test(should_fail_with = "before earliestCreated")]
fn fourth_credential_freshness_is_checked() {
    let mut c = complete();
    c.created_at = stand_in_number(99, vocabulary().time_dt);
    run_four(c);
}
#[test(should_fail_with = "holder key binding")]
fn fourth_credential_holder_is_checked() {
    let mut c = complete();
    c.quads[5][2] = stand_in_key_cell(key(2));
    run_four(c);
}

fn suite_iri<let L: u32>(text: [u8; L]) -> envelope_core::term::TermCell {
    envelope_core::term::open_term(
        1,
        std::collections::bounded_vec::BoundedVec::<u8, L>::from_array(text),
        std::collections::bounded_vec::BoundedVec::<u8, 0>::new(),
        std::collections::bounded_vec::BoundedVec::<u8, 0>::new(),
        [0; 31],
        0,
    )
}
#[test]
fn suite_cells_and_complete_signed_message_reach_the_envelope() {
    let root = suite_iri("urn:test:credential".as_bytes());
    let issuer = suite_iri("urn:test:issuer".as_bytes());
    let method = suite_iri("urn:test:key".as_bytes());
    let graph = envelope_core::term::open_term(
        0,
        std::collections::bounded_vec::BoundedVec::<u8, 0>::new(),
        std::collections::bounded_vec::BoundedVec::<u8, 0>::new(),
        std::collections::bounded_vec::BoundedVec::<u8, 0>::new(),
        [0; 31],
        0,
    );
    let mut quads = [
        [
            root,
            suite_iri("http://www.w3.org/1999/02/22-rdf-syntax-ns#type".as_bytes()),
            suite_iri(
                "https://www.w3.org/2018/credentials#VerifiableCredential".as_bytes(),
            ),
            graph,
        ],
        [root, suite_iri("https://www.w3.org/2018/credentials#issuer".as_bytes()), issuer, graph],
    ];
    let a = envelope_core::suite::quad_hash([
        quads[0][0].enc(),
        quads[0][1].enc(),
        quads[0][2].enc(),
        quads[0][3].enc(),
    ]);
    let b = envelope_core::suite::quad_hash([
        quads[1][0].enc(),
        quads[1][1].enc(),
        quads[1][2].enc(),
        quads[1][3].enc(),
    ]);
    if envelope_core::term::field_lt(b, a) {
        let tmp = quads[0];
        quads[0] = quads[1];
        quads[1] = tmp;
    }
    let mut metadata = Credential {
        quads: [[unbound(); 4]; 2],
        len: 0,
        root: envelope_core::envelope::from_suite_cell(root),
        credential_index: 0,
        verification_method: envelope_core::envelope::from_suite_cell(method),
        issuer_key: key(1),
        signed_message: 0,
        created_at: unbound(),
    };
    let salts = [envelope_core::suite::leaf_salt(7, 0), envelope_core::suite::leaf_salt(7, 1)];
    metadata =
        envelope_core::envelope::open_suite_credential(quads, salts, 7, 2, [10, 20], metadata);
    let leaf = trusted_key_leaf(
        envelope_core::envelope::from_suite_cell(issuer),
        metadata.verification_method,
        key(1),
    );
    let verified = check_issuer(
        metadata,
        leaf,
        0,
        sign(1, metadata.signed_message),
        Membership { index: 0, siblings: [] },
    );
    assert(equal_cell(verified, envelope_core::envelope::from_suite_cell(issuer)));
    assert(
        equal_cell(
            envelope_core::envelope::iri_bytes("urn:test:issuer".as_bytes()),
            verified,
        ),
    );
}

#[test(should_fail_with = "holder evidence coverage")]
fn unused_holder_signature_refused() {
    let mut p = scoped_policy(iri(902));
    p.validity_required = false;
    p.now = 0;
    envelope_core::envelope::check_holders(
        [base()],
        p,
        request_digest(10, 20),
        [0],
        [holder_evidence(1, request_digest(10, 20))],
    );
}

fn inactive_policy() -> PublicPolicy {
    let mut p = policy(base());
    p.holder_required = false;
    p.status_required = false;
    p.snapshot_root = 0;
    p.snapshot_depth = 0;
    p.validity_required = false;
    p.now = 0;
    p
}
#[test]
fn inactive_public_policy_has_zero_values() {
    envelope_core::envelope::check_policy::<3, 3>(inactive_policy());
}
#[test(should_fail_with = "disabled freshness policy")]
fn inactive_freshness_with_nonzero_bound_refused() {
    let mut p = inactive_policy();
    p.earliest_created = 1;
    envelope_core::envelope::check_policy::<3, 3>(p);
}
#[test(should_fail_with = "disabled validity policy")]
fn inactive_validity_with_nonzero_time_refused() {
    let mut p = inactive_policy();
    p.now = 1;
    envelope_core::envelope::check_policy::<3, 3>(p);
}
#[test(should_fail_with = "disabled status policy")]
fn inactive_status_with_nonzero_root_refused() {
    let mut p = inactive_policy();
    p.snapshot_root = 1;
    envelope_core::envelope::check_policy::<3, 3>(p);
}
#[test(should_fail_with = "disabled status policy")]
fn inactive_status_with_nonzero_depth_refused() {
    let mut p = inactive_policy();
    p.snapshot_depth = 1;
    envelope_core::envelope::check_policy::<3, 3>(p);
}
#[test(should_fail_with = "disabled status policy")]
fn inactive_status_with_missing_option_refused() {
    let mut p = inactive_policy();
    p.allow_missing_status = true;
    envelope_core::envelope::check_policy::<3, 3>(p);
}
#[test(should_fail_with = "disabled holder policy")]
fn inactive_holder_with_present_scope_refused() {
    let mut p = inactive_policy();
    p.holder_issuers_present = true;
    envelope_core::envelope::check_policy::<3, 3>(p);
}
#[test(should_fail_with = "disabled holder policy")]
fn inactive_holder_with_nonzero_count_refused() {
    let mut p = inactive_policy();
    p.holder_issuer_count = 1;
    p.holder_issuers[0] = 900;
    envelope_core::envelope::check_policy::<3, 3>(p);
}
#[test(should_fail_with = "disabled holder policy")]
fn inactive_holder_with_nonzero_word_refused() {
    let mut p = inactive_policy();
    p.holder_issuers[3] = 900;
    envelope_core::envelope::check_policy::<3, 3>(p);
}
#[test(should_fail_with = "holder issuer presence")]
fn absent_holder_scope_requires_zero_count() {
    let mut p = policy(holder());
    p.holder_issuer_count = 1;
    p.holder_issuers[0] = 900;
    envelope_core::envelope::check_policy::<3, 3>(p);
}
#[test(should_fail_with = "holder issuer order")]
fn duplicate_holder_issuer_words_refused() {
    let mut p = scoped_policy(iri(900));
    p.holder_issuer_count = 2;
    p.holder_issuers[1] = 900;
    envelope_core::envelope::check_policy::<3, 3>(p);
}
#[test]
fn set_member_at_public_depth_below_compiled_maximum_accepted() {
    let path = Membership { index: 0, siblings: [777, 0, 0] };
    assert(envelope_core::envelope::set_merkle_root(42, path, 1) == h2(42, 777));
    assert(
        envelope_core::envelope::set_merkle_root(42, Membership { index: 0, siblings: [0; 3] }, 0)
            == 42,
    );
    let _ = check_issuer(base(), issuer_root(base()), 1, sign(1, 42), path);
}
#[test(should_fail_with = "issuer authorization set")]
fn issuer_member_one_level_too_shallow_refused() {
    let _ = check_issuer(
        base(),
        issuer_root(base()),
        0,
        sign(1, 42),
        Membership { index: 0, siblings: [0; 3] },
    );
}
#[test(should_fail_with = "issuer authorization set")]
fn issuer_member_one_level_too_deep_refused() {
    let _ = check_issuer(
        base(),
        issuer_root(base()),
        2,
        sign(1, 42),
        Membership { index: 0, siblings: [777, 0, 0] },
    );
}
#[test(should_fail_with = "set path padding")]
fn unused_set_sibling_refused() {
    let _ = envelope_core::envelope::set_merkle_root(
        42,
        Membership { index: 0, siblings: [777, 5, 0] },
        1,
    );
}
#[test(should_fail_with = "set path padding")]
fn index_above_public_depth_refused() {
    let _ = envelope_core::envelope::set_merkle_root(
        42,
        Membership { index: 2, siblings: [777, 0, 0] },
        1,
    );
}
#[test(should_fail_with = "set depth capacity")]
fn public_set_depth_above_compiled_maximum_refused() {
    let _ =
        envelope_core::envelope::set_merkle_root(42, Membership { index: 0, siblings: [0; 3] }, 4);
}
#[test]
fn snapshot_member_at_public_depth_below_compiled_maximum_accepted() {
    let short = evidence();
    let mut e = [
        StatusEvidence {
            present: false,
            snapshot: snapshot(),
            accepted: Membership { index: 0, siblings: [999, 0] },
            bit: false,
            path: path(),
        }; 16
    ];
    e[2].present = short[2].present;
    check_status(status(), policy_root(), 1, false, e);
}
#[test(should_fail_with = "snapshot policy")]
fn snapshot_member_one_level_too_shallow_refused() {
    let mut e = evidence();
    e[2].accepted.siblings = [0];
    check_status(status(), policy_root(), 0, false, e);
}
#[test(should_fail_with = "snapshot policy")]
fn snapshot_member_one_level_too_deep_refused() {
    let mut e = [
        StatusEvidence {
            present: false,
            snapshot: snapshot(),
            accepted: Membership { index: 0, siblings: [999, 0] },
            bit: false,
            path: path(),
        }; 16
    ];
    e[2].present = true;
    check_status(status(), policy_root(), 2, false, e);
}
#[test(should_fail_with = "status entry absent")]
fn public_policy_a_refuses_evidence_satisfying_only_policy_b() {
    let mut a = inactive_policy();
    a.status_required = true;
    a.snapshot_root = policy_root();
    a.snapshot_depth = 1;
    let _ = envelope_core::envelope::check_public_envelope(
        [base()],
        request_digest(10, 20),
        a,
        [sign(1, 42)],
        [issuer_path()],
        [empty_holder_evidence()],
        [0],
        [no_status()],
    );
}
#[test]
fn public_policy_b_accepts_its_evidence() {
    let _ = envelope_core::envelope::check_public_envelope(
        [base()],
        request_digest(10, 20),
        inactive_policy(),
        [sign(1, 42)],
        [issuer_path()],
        [empty_holder_evidence()],
        [0],
        [no_status()],
    );
}

#[test]
fn native_policy_root_vectors() {
    let (pair, d, n) = native::canonical_set_root([1, 2], 2);
    let (reverse, rd, rn) = native::canonical_set_root([2, 1], 2);
    let (duplicates, dd, dn) = native::canonical_set_root([2, 1, 2, 0], 3);
    let (singleton, sd, sn) = native::canonical_set_root([2, 0, 0, 0], 1);
    let (padded, pd, pn) = native::canonical_set_root([3, 1, 2, 0], 3);
    let (larger, ld, ln) = native::canonical_set_root([2, 1, 0, 0, 0, 0, 0, 0], 2);
    let (unsigned, ud, un) = native::canonical_set_root([-1, 2, 1, 0], 3);
    assert((pair == reverse) & (pair == duplicates) & (pair == larger));
    assert(
        (d == 1) & (rd == 1) & (dd == 1) & (ld == 1) & (n == 2) & (rn == 2) & (dn == 2) & (ln == 2),
    );
    assert(
        (singleton == 2) & (sd == 0) & (sn == 1) & (pd == 2) & (pn == 3) & (ud == 2) & (un == 3),
    );
    let trust = trusted_key_leaf(
        envelope_core::envelope::iri_bytes("urn:test:issuer".as_bytes()),
        envelope_core::envelope::iri_bytes("urn:test:key".as_bytes()),
        key(1),
    );
    assert(pair == 0x038682aa1cb5ae4e0a3f13da432a95c77c5c111f6f030faf9cad641ce1ed7383);
    assert(padded == 0x23e40150eaf2a79630776edb762a0525705f0206f8e3da4cbbe2fba8ae3d7b87);
    assert(unsigned == 0x13d34888a5f10181210c333c7cec7169f39265a0f2664be7b5bdaa417a3ca537);
    assert(trust == 0x0634ae948a9551ef686ae6afd30505a0ab76509e71b977393210dc7e764dd62c);
    println([pair, singleton, padded, unsigned, trust]);
}
