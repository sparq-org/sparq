mod native;
mod fixtures;
use compose_core::hashes::h2;
use envelope_core::envelope::{
    check_freshness, check_holder, check_holders, check_issuer, check_public_envelope, check_status,
    check_validity, credential_dataset, CredentialOpening, from_suite_cell, holder_message,
    HolderEvidence, iri_bytes, Membership, merge_datasets, merkle_root, open_suite_credential,
    OpenedCredential, Point, PublicPolicy, required_property, Signature, Snapshot, snapshot_digest,
    status_leaf, StatusEvidence, trusted_key_leaf, ValidityOpening,
};
use envelope_core::table::{equal_cell, Table};
use envelope_core::term::{same_term, TermCell};
use std::collections::bounded_vec::BoundedVec;
fn main() {}
fn key(n: u8) -> Point {
    if n == 1 {
        fixtures::key_1()
    } else if n == 2 {
        fixtures::key_2()
    } else if n == 3 {
        fixtures::key_3()
    } else if n == 11 {
        fixtures::key_11()
    } else {
        fixtures::key_12()
    }
}
fn no_sig() -> Signature {
    Signature { present: false, r: key(11), s: 0, e: 0, e_k: 0 }
}
fn digest() -> [u8; 32] {
    [7; 32]
}
fn open<let N: u32>(c: CredentialOpening<N, 160>) -> OpenedCredential<N> {
    open_suite_credential(c, 0)
}
fn base() -> OpenedCredential<16> {
    open(fixtures::base::<16>())
}
fn holder() -> OpenedCredential<16> {
    open(fixtures::holder::<16>())
}
fn validity() -> OpenedCredential<16> {
    open(fixtures::validity::<16>())
}
fn status() -> OpenedCredential<16> {
    open(fixtures::status::<16>())
}
fn ir<let N: u32>(text: [u8; N]) -> TermCell {
    iri_bytes(text)
}
fn issuer<let N: u32>(c: OpenedCredential<N>) -> TermCell {
    required_property(c, c.credential_subject(), envelope_core::term::iri_issuer())
}
fn issuer_path() -> Membership<2> {
    Membership { index: 0, siblings: [777, 0] }
}
fn issuer_root<let N: u32>(c: OpenedCredential<N>, pk: Point) -> Field {
    h2(
        trusted_key_leaf(issuer(c), c.verification_method(), pk),
        777,
    )
}
fn check_i<let N: u32>(c: OpenedCredential<N>, root: Field, pk: Point) -> TermCell {
    check_issuer(c, root, 1, pk, issuer_path())
}
fn holder_sig() -> Signature {
    fixtures::holder_signature_11()
}
fn holder_evidence(n: u8) -> HolderEvidence {
    HolderEvidence {
        pk: key(n),
        sig: if n == 11 {
            holder_sig()
        } else {
            fixtures::holder_signature_12()
        },
    }
}
fn empty_holder() -> HolderEvidence {
    HolderEvidence { pk: key(11), sig: no_sig() }
}
fn path() -> Membership<2> {
    Membership {
        index: 1,
        siblings: [status_leaf(true), h2(status_leaf(false), status_leaf(true))],
    }
}
fn acceptance() -> Membership<2> {
    Membership { index: 0, siblings: [999, 0] }
}
fn snapshot(suspended: bool) -> Snapshot {
    Snapshot {
        list: if suspended {
            ir("urn:list:suspension".as_bytes())
        } else {
            ir("urn:list:revocation".as_bytes())
        },
        purpose: if suspended {
            fixtures::purpose_suspension()
        } else {
            fixtures::purpose_revocation()
        },
        bit_len: 4,
        root: merkle_root(status_leaf(false), path()),
    }
}
fn no_status<let N: u32>() -> [StatusEvidence<2, 2>; N] {
    [
        StatusEvidence {
            present: false,
            snapshot: snapshot(false),
            accepted: acceptance(),
            bit: false,
            path: path(),
            purpose: BoundedVec::new(),
            index: BoundedVec::new(),
            size: BoundedVec::new(),
        }; N
    ]
}
fn evidence_for<let N: u32>(c: OpenedCredential<N>) -> [StatusEvidence<2, 2>; N] {
    let mut es = no_status();
    let qs = c.quads();
    for i in 0..N {
        if (i < c.count())
            & same_term(qs[i][0], c.credential_subject())
            & same_term(qs[i][1], envelope_core::term::iri_credential_status()) {
            let purpose = c.property(qs[i][2], envelope_core::term::iri_status_purpose());
            let suspended = same_term(purpose, fixtures::purpose_suspension());
            es[i].present = true;
            es[i].snapshot = snapshot(suspended);
            es[i].purpose = if suspended {
                BoundedVec::<u8, 16>::from_array("suspension".as_bytes())
            } else {
                BoundedVec::<u8, 16>::from_array("revocation".as_bytes())
            };
            es[i].index = BoundedVec::<u8, 20>::from_array("0001".as_bytes());
            es[i].size = BoundedVec::<u8, 20>::from_array("1".as_bytes());
        }
    }
    es
}

fn status_row<let N: u32>(c: OpenedCredential<N>) -> u32 {
    let mut at: u32 = 0;
    for i in 0..N {
        if (i < c.count())
            & same_term(
                c.quads()[i][1],
                envelope_core::term::iri_credential_status(),
            ) {
            at = i;
        }
    }
    at
}
fn policy_root() -> Field {
    h2(snapshot_digest::<2>(snapshot(false)), 999)
}
fn root_for(s: Snapshot) -> Field {
    h2(snapshot_digest::<2>(s), 999)
}
fn period() -> ValidityOpening {
    bounds(
        "1970-01-01T00:00:00.101Z".as_bytes(),
        "1970-01-01T00:00:00.201Z".as_bytes(),
    )
}
fn bounds<let F: u32, let U: u32>(from: [u8; F], until: [u8; U]) -> ValidityOpening {
    ValidityOpening {
        from: BoundedVec::<u8, 32>::from_array(from),
        until: BoundedVec::<u8, 32>::from_array(until),
    }
}
fn no_bounds() -> ValidityOpening {
    bounds("".as_bytes(), "".as_bytes())
}
fn policy() -> PublicPolicy {
    PublicPolicy {
        issuer_root: 1,
        issuer_depth: 1,
        holder_required: false,
        holder_issuers_present: false,
        holder_issuer_count: 0,
        holder_issuers: [0; 4],
        status_required: false,
        allow_missing_status: false,
        snapshot_root: 0,
        snapshot_depth: 0,
        validity_required: false,
        now: 0,
        freshness_required: false,
        earliest_created: 0,
    }
}
fn scoped_policy(who: TermCell) -> PublicPolicy {
    let mut p = policy();
    p.holder_required = true;
    p.holder_issuers_present = true;
    p.holder_issuer_count = 1;
    p.holder_issuers = [who.enc(), 0, 0, 0];
    p
}
fn dataset<let O: u32>(c: OpenedCredential<16>, index: u32) -> Table<O, 4> {
    credential_dataset::<16, O, 6>(c, issuer(c), digest(), index)
}
fn check_all<let N: u32>(
    c: OpenedCredential<N>,
    pk: Point,
    p: PublicPolicy,
    es: [StatusEvidence<2, 2>; N],
    v: ValidityOpening,
    hs: HolderEvidence,
) {
    let _ = check_public_envelope(
        [c],
        digest(),
        p,
        [pk],
        [issuer_path()],
        [hs],
        [0],
        [es],
        [v],
    );
}
fn deeper_evidence_for(c: OpenedCredential<16>) -> [StatusEvidence<3, 2>; 16] {
    let base = evidence_for(c);
    let one = status_leaf(true);
    let member = Membership {
        index: 1,
        siblings: [path().siblings[0], path().siblings[1], h2(h2(one, one), h2(one, one))],
    };
    let mut es = [
        StatusEvidence {
            present: false,
            snapshot: snapshot(false),
            accepted: acceptance(),
            bit: false,
            path: member,
            purpose: BoundedVec::new(),
            index: BoundedVec::new(),
            size: BoundedVec::new(),
        }; 16
    ];
    for i in 0..16 {
        es[i].present = base[i].present;
        es[i].purpose = base[i].purpose;
        es[i].index = base[i].index;
        es[i].size = base[i].size;
        es[i].snapshot.root = merkle_root(status_leaf(false), member);
    }
    es
}
fn copies<let N: u32>(d: Table<N, 4>, s: TermCell, p: TermCell, o: TermCell) -> u32 {
    let mut n: u32 = 0;
    for i in 0..N {
        if (i < d.len)
            & equal_cell(d.rows[i][0], from_suite_cell(s))
            & equal_cell(d.rows[i][1], from_suite_cell(p))
            & equal_cell(d.rows[i][2], from_suite_cell(o)) {
            n = n + 1;
        }
    }
    n
}
fn pinned<let N: u32>(d: Table<N, 4>, fact: TermCell, who: TermCell) -> bool {
    let mut found = false;
    for i in 0..N {
        for j in 0..N {
            let assertion = (i < d.len)
                & equal_cell(
                    d.rows[i][1],
                    from_suite_cell(envelope_core::envelope::verified_issuer_predicate()),
                )
                & equal_cell(d.rows[i][2], from_suite_cell(who));
            let statement = (j < d.len)
                & equal_cell(
                    d.rows[j][1],
                    from_suite_cell(ir("urn:fact:issuer".as_bytes())),
                )
                & equal_cell(d.rows[j][2], from_suite_cell(fact));
            found = found | (assertion & statement & equal_cell(d.rows[i][0], d.rows[j][3]));
        }
    }
    found
}

// Every component case authenticates the same signed Solid vectors at full capacity.
fn four_inputs(last: OpenedCredential<64>) -> [OpenedCredential<64>; 4] {
    [
        open_suite_credential(fixtures::complete::<64>(), 0),
        open_suite_credential(fixtures::complete_shared::<64>(), 1),
        open_suite_credential(fixtures::complete_distinct::<64>(), 2),
        last,
    ]
}
fn four_normal() -> [OpenedCredential<64>; 4] {
    four_inputs(
        open_suite_credential(fixtures::complete_unlisted::<64>(), 3),
    )
}
fn four_policy(cs: [OpenedCredential<64>; 4]) -> (PublicPolicy, [Membership<2>; 4]) {
    let leaves = [
        trusted_key_leaf(issuer(cs[0]), cs[0].verification_method(), key(1)),
        trusted_key_leaf(issuer(cs[1]), cs[1].verification_method(), key(2)),
        trusted_key_leaf(
            ir("urn:issuer:3".as_bytes()),
            ir("urn:method:3".as_bytes()),
            key(3),
        ),
        0,
    ];
    let mut sorted = leaves;
    for _ in 0..3 {
        for i in 0..2 {
            if envelope_core::term::field_lt(sorted[i + 1], sorted[i]) {
                let x = sorted[i];
                sorted[i] = sorted[i + 1];
                sorted[i + 1] = x;
            }
        }
    }
    let root = h2(h2(sorted[0], sorted[1]), h2(sorted[2], 0));
    let (canonical, depth, unique) = native::canonical_set_root(leaves, 3);
    assert(root == canonical);
    assert((depth == 2) & (unique == 3));
    let mut paths = [Membership { index: 0, siblings: [0; 2] }; 4];
    for j in 0..4 {
        let leaf = if j == 0 {
            leaves[0]
        } else if j == 1 {
            leaves[1]
        } else if j == 2 {
            leaves[0]
        } else {
            leaves[2]
        };
        for k in 0..3 {
            if sorted[k] == leaf {
                paths[j] = Membership {
                    index: k as Field,
                    siblings: if k == 0 {
                        [sorted[1], h2(sorted[2], 0)]
                    } else if k == 1 {
                        [sorted[0], h2(sorted[2], 0)]
                    } else {
                        [0, h2(sorted[0], sorted[1])]
                    },
                };
            }
        }
    }
    let rev = snapshot_digest::<2>(snapshot(false));
    let susp = snapshot_digest::<2>(snapshot(true));
    let mut p = scoped_policy(issuer(cs[0]));
    p.issuer_root = root;
    p.issuer_depth = 2;
    p.holder_issuer_count = 2;
    let i1 = issuer(cs[0]).enc();
    let i2 = issuer(cs[1]).enc();
    p.holder_issuers = if envelope_core::term::field_lt(i1, i2) {
        [i1, i2, 0, 0]
    } else {
        [i2, i1, 0, 0]
    };
    let rev_first = envelope_core::term::field_lt(rev, susp);
    p.status_required = true;
    p.snapshot_root = if rev_first {
        h2(rev, susp)
    } else {
        h2(susp, rev)
    };
    p.snapshot_depth = 1;
    p.validity_required = true;
    p.now = 150;
    p.freshness_required = true;
    p.earliest_created = 101;
    (p, paths)
}
fn four_issuer(cs: [OpenedCredential<64>; 4]) {
    let (p, paths) = four_policy(cs);
    envelope_core::envelope::check_policy::<2, 2>(p);
    let keys = [key(1), key(2), key(1), key(3)];
    for i in 0..4 {
        let actual = check_issuer(cs[i], p.issuer_root, p.issuer_depth, keys[i], paths[i]);
        assert(same_term(actual, issuer(cs[i])));
    }
}
fn four_holder(cs: [OpenedCredential<64>; 4], slots: [u32; 4]) {
    let (p, _) = four_policy(cs);
    check_holders(
        cs,
        p,
        digest(),
        slots,
        [holder_evidence(11), holder_evidence(12), empty_holder(), empty_holder()],
    );
}
fn four_status(cs: [OpenedCredential<64>; 4]) {
    let (p, _) = four_policy(cs);
    let rev = snapshot_digest::<2>(snapshot(false));
    let susp = snapshot_digest::<2>(snapshot(true));
    let rev_first = envelope_core::term::field_lt(rev, susp);
    // Complete per-credential evidence, with the same accepted snapshot set.
    for j in 0..4 {
        let mut es = evidence_for(cs[j]);
        for i in 0..64 {
            if es[i].present {
                let suspended = same_term(es[i].snapshot.purpose, fixtures::purpose_suspension());
                es[i].accepted = Membership {
                    index: if suspended == rev_first { 1 } else { 0 },
                    siblings: [if suspended { rev } else { susp }, 0],
                };
            }
        }
        check_status(cs[j], p.snapshot_root, p.snapshot_depth, false, es);
    }
}
fn four_validity(cs: [OpenedCredential<64>; 4], last_period: ValidityOpening) {
    let (p, _) = four_policy(cs);
    for i in 0..4 {
        check_validity(cs[i], p.now, if i == 3 { last_period } else { period() });
    }
}
fn four_freshness(cs: [OpenedCredential<64>; 4]) {
    let (p, _) = four_policy(cs);
    for i in 0..4 {
        check_freshness(cs[i], p.freshness_required, p.earliest_created);
    }
}
fn count_quad<let R: u32>(d: Table<R, 4>, row: [envelope_core::table::TermCell; 4]) -> u32 {
    let mut count: u32 = 0;
    for i in 0..R {
        count = count + ((i < d.len) & envelope_core::table::same_row(d.rows[i], row)) as u32;
    }
    count
}
fn four_dataset_view(cs: [OpenedCredential<64>; 4], index: u32) {
    let (p, paths) = four_policy(cs);
    let keys = [key(1), key(2), key(1), key(3)];
    let actual = check_issuer(
        cs[index],
        p.issuer_root,
        p.issuer_depth,
        keys[index],
        paths[index],
    );
    let d = credential_dataset::<64, 129, 8>(cs[index], actual, digest(), index);
    envelope_core::table::check_table(d);
    assert(d.len == 2 * cs[index].count() + 1);
    let graph = from_suite_cell(envelope_core::term::credential_graph(
        [0x07070707070707070707070707070707, 0x07070707070707070707070707070707],
        index,
    ));
    let rows = cs[index].quads();
    for i in 0..64 {
        if i < cs[index].count() {
            let q = [
                from_suite_cell(rows[i][0]),
                from_suite_cell(rows[i][1]),
                from_suite_cell(rows[i][2]),
                envelope_core::table::unbound(),
            ];
            assert(count_quad(d, q) == 1);
            assert(count_quad(d, [q[0], q[1], q[2], graph]) == 1);
        }
    }
    assert(
        count_quad(
            d,
            [
                graph,
                from_suite_cell(envelope_core::envelope::verified_issuer_predicate()),
                from_suite_cell(actual),
                envelope_core::table::unbound(),
            ],
        )
            == 1,
    );
}
fn four_shared_merge(cs: [OpenedCredential<64>; 4]) {
    let (p, paths) = four_policy(cs);
    let keys = [key(1), key(2), key(1), key(3)];
    let s = ir("urn:subject".as_bytes());
    let pred = ir("urn:shared".as_bytes());
    let mut inputs = [envelope_core::table::empty::<3, 4>(); 4];
    for i in 0..4 {
        let actual = check_issuer(cs[i], p.issuer_root, p.issuer_depth, keys[i], paths[i]);
        let object = required_property(cs[i], s, pred);
        let graph = from_suite_cell(envelope_core::term::credential_graph(
            [0x07070707070707070707070707070707, 0x07070707070707070707070707070707],
            i,
        ));
        let q = [
            from_suite_cell(s),
            from_suite_cell(pred),
            from_suite_cell(object),
            envelope_core::table::unbound(),
        ];
        inputs[i] = envelope_core::table::canonicalize(
            Table {
                rows: [
                    q,
                    [q[0], q[1], q[2], graph],
                    [
                        graph,
                        from_suite_cell(envelope_core::envelope::verified_issuer_predicate()),
                        from_suite_cell(actual),
                        envelope_core::table::unbound(),
                    ],
                ],
                len: 3,
            },
        );
    }
    let merged = merge_datasets::<4, 3, 12>(inputs);
    assert(merged.len == 9);
    assert(copies(merged, s, pred, ir("urn:value".as_bytes())) == 5);
}
fn run_four(last: OpenedCredential<64>) {
    run_four_with_validity(last, period());
}
fn run_four_with_validity(last: OpenedCredential<64>, last_period: ValidityOpening) {
    let a = open_suite_credential(fixtures::complete::<64>(), 0);
    let b = open_suite_credential(fixtures::complete_shared::<64>(), 1);
    let c = open_suite_credential(fixtures::complete_distinct::<64>(), 2);
    let cs = [a, b, c, last];
    let keys = [key(1), key(2), key(1), key(3)];
    let leaves = [
        trusted_key_leaf(issuer(a), a.verification_method(), key(1)),
        trusted_key_leaf(issuer(b), b.verification_method(), key(2)),
        trusted_key_leaf(
            ir("urn:issuer:3".as_bytes()),
            ir("urn:method:3".as_bytes()),
            key(3),
        ),
        0,
    ];
    let mut sorted = leaves;
    for _ in 0..3 {
        for i in 0..2 {
            if envelope_core::term::field_lt(sorted[i + 1], sorted[i]) {
                let x = sorted[i];
                sorted[i] = sorted[i + 1];
                sorted[i + 1] = x;
            }
        }
    }
    let root = h2(h2(sorted[0], sorted[1]), h2(sorted[2], 0));
    let (canonical, depth, unique) = native::canonical_set_root(leaves, 3);
    assert(root == canonical);
    assert((depth == 2) & (unique == 3));
    let mut paths = [Membership { index: 0, siblings: [0; 2] }; 4];
    for j in 0..4 {
        let leaf = if j == 0 {
            leaves[0]
        } else if j == 1 {
            leaves[1]
        } else if j == 2 {
            leaves[0]
        } else {
            leaves[2]
        };
        for k in 0..3 {
            if sorted[k] == leaf {
                paths[j] = Membership {
                    index: k as Field,
                    siblings: if k == 0 {
                        [sorted[1], h2(sorted[2], 0)]
                    } else if k == 1 {
                        [sorted[0], h2(sorted[2], 0)]
                    } else {
                        [0, h2(sorted[0], sorted[1])]
                    },
                };
            }
        }
    }
    let rev = snapshot_digest::<2>(snapshot(false));
    let susp = snapshot_digest::<2>(snapshot(true));
    let mut p = scoped_policy(issuer(a));
    p.issuer_root = root;
    p.issuer_depth = 2;
    p.holder_issuer_count = 2;
    let i1 = issuer(a).enc();
    let i2 = issuer(b).enc();
    p.holder_issuers = if envelope_core::term::field_lt(i1, i2) {
        [i1, i2, 0, 0]
    } else {
        [i2, i1, 0, 0]
    };
    let rev_first = envelope_core::term::field_lt(rev, susp);
    p.status_required = true;
    p.snapshot_root = if rev_first {
        h2(rev, susp)
    } else {
        h2(susp, rev)
    };
    p.snapshot_depth = 1;
    p.validity_required = true;
    p.now = 150;
    p.freshness_required = true;
    p.earliest_created = 101;
    let mut es = [evidence_for(a), evidence_for(b), evidence_for(c), evidence_for(last)];
    for j in 0..4 {
        for i in 0..64 {
            if es[j][i].present {
                let suspended =
                    same_term(es[j][i].snapshot.purpose, fixtures::purpose_suspension());
                es[j][i].accepted = Membership {
                    index: if suspended == rev_first { 1 } else { 0 },
                    siblings: [if suspended { rev } else { susp }, 0],
                };
            }
        }
    }
    println([
        p.issuer_root,
        p.issuer_depth as Field,
        p.holder_required as Field,
        p.holder_issuers_present as Field,
        p.holder_issuer_count as Field,
        p.holder_issuers[0],
        p.holder_issuers[1],
        p.holder_issuers[2],
        p.holder_issuers[3],
        p.status_required as Field,
        p.allow_missing_status as Field,
        p.snapshot_root,
        p.snapshot_depth as Field,
        p.validity_required as Field,
        p.now as Field,
        p.freshness_required as Field,
        p.earliest_created as Field,
    ]);
    let verified = check_public_envelope(
        cs,
        digest(),
        p,
        keys,
        paths,
        [holder_evidence(11), holder_evidence(12), empty_holder(), empty_holder()],
        [0, 0, 1, 0],
        es,
        [period(), period(), period(), last_period],
    );
    let ds = [
        credential_dataset::<64, 129, 8>(a, verified[0], digest(), 0),
        credential_dataset::<64, 129, 8>(b, verified[1], digest(), 1),
        credential_dataset::<64, 129, 8>(c, verified[2], digest(), 2),
        credential_dataset::<64, 129, 8>(last, verified[3], digest(), 3),
    ];
    let merged = merge_datasets::<4, 129, 516>(ds);
    assert(merged.len > 0);
    assert(
        copies(
            merged,
            ir("urn:subject".as_bytes()),
            ir("urn:shared".as_bytes()),
            ir("urn:value".as_bytes()),
        )
            == 5,
    );
}
#[test]
fn issuer_accepted_and_identity_returned() {
    let c = base();
    assert(
        same_term(
            check_i(c, issuer_root(c, key(1)), key(1)),
            ir("urn:issuer:1".as_bytes()),
        ),
    );
}

#[test(should_fail_with = "required property absent")]
fn issuer_missing_refused() {
    let c = open(fixtures::missing_issuer::<16>());
    let _ = check_i(c, 1, key(1));
}

#[test(should_fail_with = "signature evidence absent")]
fn issuer_evidence_omitted_refused() {
    let mut c = fixtures::base::<16>();
    c.signature.present = false;
    let _ = open(c);
}

#[test(should_fail_with = "issuer authorization set")]
fn another_key_set_refused() {
    let c = base();
    let _ = check_i(c, issuer_root(c, key(1)) + 1, key(1));
}

#[test(should_fail_with = "issuer authorization set")]
fn another_issuer_label_refused() {
    let c = open(fixtures::complete_other_issuer::<64>());
    let root = h2(
        trusted_key_leaf(
            ir("urn:issuer:3".as_bytes()),
            c.verification_method(),
            key(3),
        ),
        777,
    );
    let _ = check_i(c, root, key(3));
}

#[test(should_fail_with = "issuer authorization set")]
fn another_verification_method_refused() {
    let c = base();
    let root = h2(
        trusted_key_leaf(issuer(c), ir("urn:other-method".as_bytes()), key(1)),
        777,
    );
    let _ = check_i(c, root, key(1));
}

#[test(should_fail_with = "challenge reduction not bound to its preimage")]
fn another_issuer_message_refused() {
    let mut c = fixtures::base::<16>();
    c.configuration[0] = c.configuration[0] + 1;
    let _ = open(c);
}

#[test(should_fail_with = "issuer authorization set")]
fn different_issuer_key_refused() {
    let c = base();
    let root = h2(
        trusted_key_leaf(issuer(c), c.verification_method(), key(2)),
        777,
    );
    let _ = check_i(c, root, key(1));
}
#[test(should_fail_with = "verification key opening differs")]
fn issuer_point_must_open_authenticated_multikey() {
    let c = base();
    let _ = check_i(c, issuer_root(c, key(1)), key(2));
}

#[test]
fn holder_request_signature_accepted() {
    check_holder(holder(), digest(), key(11), holder_sig());
}

#[test(should_fail_with = "required property absent")]
fn holder_key_absent_refused() {
    check_holder(base(), digest(), key(11), no_sig());
}

#[test(should_fail_with = "signature evidence absent")]
fn holder_signature_omitted_refused() {
    check_holder(holder(), digest(), key(11), no_sig());
}

#[test(should_fail_with = "verification key opening differs")]
fn holder_other_key_refused() {
    check_holder(holder(), digest(), key(12), fixtures::holder_signature_12());
}

#[test(should_fail_with = "challenge reduction not bound to its preimage")]
fn holder_other_request_refused() {
    let mut d = digest();
    d[31] = 8;
    check_holder(holder(), d, key(11), holder_sig());
}

#[test(should_fail_with = "challenge reduction not bound to its preimage")]
fn holder_other_high_digest_limb_refused() {
    let mut d = digest();
    d[0] = 8;
    check_holder(holder(), d, key(11), holder_sig());
}

#[test(should_fail_with = "property cardinality")]
fn holder_ambiguous_key_refused() {
    check_holder(
        open(fixtures::ambiguous_holder::<16>()),
        digest(),
        key(11),
        no_sig(),
    );
}

#[test]
fn status_active_accepted() {
    let opened_status = status();
    check_status(
        opened_status,
        policy_root(),
        1,
        false,
        evidence_for(opened_status),
    );
}

#[test]
fn suspension_active_accepted() {
    let c = open(fixtures::suspension::<16>());
    check_status(c, root_for(snapshot(true)), 1, false, evidence_for(c));
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
    let opened_status = status();
    check_status(opened_status, policy_root(), 1, false, no_status());
}

#[test(should_fail_with = "snapshot policy")]
fn status_other_policy_refused() {
    let opened_status = status();
    check_status(
        opened_status,
        policy_root() + 1,
        1,
        false,
        evidence_for(opened_status),
    );
}

#[test(should_fail_with = "snapshot policy")]
fn status_other_snapshot_refused() {
    let c = status();
    let i = status_row(c);
    let mut es = evidence_for(c);
    es[i].snapshot.root = es[i].snapshot.root + 1;
    check_status(c, policy_root(), 1, false, es);
}

#[test(should_fail_with = "snapshot identity")]
fn status_other_list_refused() {
    let c = status();
    let i = status_row(c);
    let mut es = evidence_for(c);
    es[i].snapshot.list = ir("urn:other-list".as_bytes());
    check_status(c, policy_root(), 1, false, es);
}

#[test(should_fail_with = "snapshot identity")]
fn status_other_purpose_refused() {
    let c = status();
    let i = status_row(c);
    let mut es = evidence_for(c);
    es[i].snapshot.purpose = fixtures::purpose_suspension();
    check_status(c, policy_root(), 1, false, es);
}

#[test(should_fail_with = "status index binding")]
fn status_other_index_refused() {
    let c = status();
    let i = status_row(c);
    let mut es = evidence_for(c);
    es[i].path.index = 0;
    check_status(c, policy_root(), 1, false, es);
}

#[test(should_fail_with = "status bit set")]
fn status_set_bit_refused() {
    let c = status();
    let i = status_row(c);
    let mut es = evidence_for(c);
    es[i].bit = true;
    es[i].snapshot.root = merkle_root(status_leaf(true), path());
    check_status(c, root_for(es[i].snapshot), 1, false, es);
}

#[test(should_fail_with = "status path")]
fn status_other_path_refused() {
    let c = status();
    let i = status_row(c);
    let mut es = evidence_for(c);
    es[i].path.siblings[0] = 2;
    check_status(c, policy_root(), 1, false, es);
}

#[test(should_fail_with = "status index length")]
fn status_outside_snapshot_length_refused() {
    let c = status();
    let i = status_row(c);
    let mut es = evidence_for(c);
    es[i].snapshot.bit_len = 1;
    check_status(c, root_for(es[i].snapshot), 1, false, es);
}

#[test(should_fail_with = "snapshot capacity")]
fn status_snapshot_overflow_refused() {
    let c = status();
    let i = status_row(c);
    let mut es = evidence_for(c);
    es[i].snapshot.bit_len = 5;
    check_status(c, policy_root(), 1, false, es);
}

#[test(should_fail_with = "status size")]
fn status_multibit_refused() {
    let c = open(fixtures::size_two::<16>());
    let mut es = evidence_for(c);
    es[status_row(c)].size = BoundedVec::from_array("2".as_bytes());
    check_status(c, policy_root(), 1, false, es);
}

#[test(should_fail_with = "unknown status purpose")]
fn unsupported_status_purpose_refused() {
    let c = open(fixtures::bad_purpose::<16>());
    let mut es = evidence_for(c);
    es[status_row(c)].purpose = BoundedVec::from_array("other".as_bytes());
    check_status(c, policy_root(), 1, false, es);
}

#[test(should_fail_with = "required property absent")]
fn incomplete_status_entry_refused() {
    let c = open(fixtures::missing_index::<16>());
    check_status(c, policy_root(), 1, true, evidence_for(c));
}

#[test(should_fail_with = "status evidence coverage")]
fn missing_status_option_does_not_omit_present_entries() {
    let opened_status = status();
    check_status(opened_status, policy_root(), 1, true, no_status());
}

#[test(should_fail_with = "status evidence coverage")]
fn absence_claim_checks_last_quad() {
    let c = open(fixtures::last_status::<64>());
    assert(status_row(c) + 1 == c.count());
    check_status(c, policy_root(), 1, true, no_status());
}

#[test(should_fail_with = "non-zero padding")]
fn shortening_credential_cannot_claim_status_absent() {
    let mut c = fixtures::status::<16>();
    c.count = 2;
    let _ = open(c);
}

#[test(should_fail_with = "status evidence coverage")]
fn status_extra_evidence_refused() {
    let opened_status = status();
    let mut es = evidence_for(opened_status);
    es[15].present = true;
    check_status(opened_status, policy_root(), 1, false, es);
}

#[test(should_fail_with = "status evidence coverage")]
fn second_status_entry_cannot_be_omitted() {
    let c = open(fixtures::two_status::<16>());
    let mut es = evidence_for(c);
    es[status_row(c)].present = false;
    check_status(c, policy_root(), 1, false, es);
}

#[test]
fn status_explicit_single_bit_accepted() {
    let c = open(fixtures::size_one::<16>());
    check_status(c, policy_root(), 1, false, evidence_for(c));
}

#[test]
fn all_status_entries_checked() {
    let c = open(fixtures::two_status::<16>());
    check_status(c, policy_root(), 1, false, evidence_for(c));
}

#[test]
fn snapshot_digest_binds_depth_even_with_identical_root() {
    assert(snapshot_digest::<2>(snapshot(false)) != snapshot_digest::<3>(snapshot(false)));
}

#[test]
fn same_status_bits_at_another_depth_have_another_digest() {
    let opened_status = status();
    let es = deeper_evidence_for(opened_status);
    let s = es[status_row(opened_status)].snapshot;
    assert(snapshot_digest::<2>(snapshot(false)) != snapshot_digest::<3>(s));
    check_status(
        opened_status,
        h2(snapshot_digest::<3>(s), 999),
        1,
        false,
        es,
    );
}

#[test(should_fail_with = "snapshot policy")]
fn status_same_bits_at_another_depth_refused() {
    let opened_status = status();
    check_status(
        opened_status,
        policy_root(),
        1,
        false,
        deeper_evidence_for(opened_status),
    );
}

#[test(should_fail_with = "snapshot policy")]
fn status_policy_with_another_depth_for_the_same_root_refused() {
    let opened_status = status();
    check_status(
        opened_status,
        h2(snapshot_digest::<3>(snapshot(false)), 999),
        1,
        false,
        evidence_for(opened_status),
    );
}

#[test(should_fail_with = "status path")]
fn status_untagged_leaves_refused() {
    let c = status();
    let mut es = evidence_for(c);
    let i = status_row(c);
    let member = Membership { index: 1, siblings: [1, h2(0, 1)] };
    es[i].path = member;
    es[i].snapshot.root = merkle_root(0, member);
    check_status(c, root_for(es[i].snapshot), 1, false, es);
}

#[test(should_fail_with = "status path")]
fn status_leaf_using_another_envelope_tag_refused() {
    let c = status();
    let mut es = evidence_for(c);
    let i = status_row(c);
    es[i].snapshot.root = merkle_root(holder_message(digest()), path());
    check_status(c, root_for(es[i].snapshot), 1, false, es);
}

#[test]
fn validity_inclusive_endpoints() {
    let opened_validity = validity();
    check_validity(opened_validity, 101, period());
    check_validity(opened_validity, 150, period());
    check_validity(opened_validity, 201, period());
}

#[test(should_fail_with = "before validFrom")]
fn validity_second_before_refused() {
    check_validity(
        open(fixtures::second_period::<16>()),
        101,
        bounds(
            "1970-01-01T00:00:01.101Z".as_bytes(),
            "1970-01-01T00:00:02.201Z".as_bytes(),
        ),
    );
}

#[test(should_fail_with = "after validUntil")]
fn validity_second_after_refused() {
    check_validity(
        open(fixtures::second_period::<16>()),
        3201,
        bounds(
            "1970-01-01T00:00:01.101Z".as_bytes(),
            "1970-01-01T00:00:02.201Z".as_bytes(),
        ),
    );
}

#[test(should_fail_with = "validity bounds reversed")]
fn reversed_bounds_refused() {
    check_validity(
        open(fixtures::reversed::<16>()),
        101,
        bounds(
            "1970-01-01T00:00:00.101Z".as_bytes(),
            "1970-01-01T00:00:00.100Z".as_bytes(),
        ),
    );
}

#[test(should_fail_with = "validUntil unavailable")]
fn validity_present_unusable_bound_refused() {
    check_validity(open(fixtures::bad_until::<16>()), 150, period());
}

#[test(should_fail_with = "quads not strictly increasing")]
fn validity_changed_key_refused() {
    let mut c = fixtures::validity::<16>();
    for i in 0..16 {
        if c.raw[i][2].kind == 3 {
            c.raw[i][2].key[8] = 3;
        }
    }
    let _ = open(c);
}

#[test(should_fail_with = "property cardinality")]
fn validity_duplicate_upper_bound_refused() {
    check_validity(open(fixtures::duplicate_until::<16>()), 150, period());
}

#[test(should_fail_with = "after validUntil")]
fn upper_bound_in_last_quad_cannot_be_omitted() {
    let c = open(fixtures::last_until::<64>());
    assert(
        same_term(
            c.quads()[c.count() - 1][1],
            envelope_core::term::iri_valid_until(),
        ),
    );
    check_validity(
        c,
        202,
        bounds("".as_bytes(), "1970-01-01T00:00:00.201Z".as_bytes()),
    );
}

#[test(should_fail_with = "non-zero padding")]
fn shortening_credential_cannot_claim_no_valid_until() {
    let mut c = fixtures::validity::<16>();
    c.count = 3;
    let _ = open(c);
}

#[test(should_fail_with = "challenge reduction not bound to its preimage")]
fn holder_signature_for_other_domain_refused() {
    let mut d = digest();
    d[10] = 9;
    check_holder(holder(), d, key(11), holder_sig());
}

#[test(should_fail_with = "validUntil unavailable")]
fn validity_wrong_datatype_refused() {
    check_validity(open(fixtures::wrong_datatype::<16>()), 150, period());
}

#[test(should_fail_with = "holder key type")]
fn holder_unsupported_method_refused() {
    check_holder(
        open(fixtures::bad_holder_type::<16>()),
        digest(),
        key(11),
        no_sig(),
    );
}

#[test(should_fail_with = "status entry type")]
fn status_unsupported_entry_refused() {
    let c = open(fixtures::bad_status_type::<16>());
    check_status(c, policy_root(), 1, false, evidence_for(c));
}

#[test(should_fail_with = "status index is not decimal")]
fn status_non_decimal_index_cell_refused() {
    let c = open(fixtures::bad_index::<16>());
    let mut es = evidence_for(c);
    es[status_row(c)].index = BoundedVec::from_array("one".as_bytes());
    check_status(c, policy_root(), 1, false, es);
}

#[test]
fn absent_bounds_are_indefinite() {
    let opened_base = base();
    check_validity(opened_base, 0, no_bounds());
    check_validity(opened_base, 9223372036854775807, no_bounds());
}

#[test]
fn absent_upper_bound_accepted() {
    check_validity(
        open(fixtures::from_only::<16>()),
        10000,
        bounds("1970-01-01T00:00:00.101Z".as_bytes(), "".as_bytes()),
    );
}

#[test]
fn absent_lower_bound_accepted() {
    check_validity(
        open(fixtures::until_only::<16>()),
        0,
        bounds("".as_bytes(), "1970-01-01T00:00:00.201Z".as_bytes()),
    );
}

#[test]
fn equal_bounds_valid_at_that_instant() {
    check_validity(
        open(fixtures::equal_bounds::<16>()),
        101,
        bounds(
            "1970-01-01T00:00:00.101Z".as_bytes(),
            "1970-01-01T00:00:00.101Z".as_bytes(),
        ),
    );
}

#[test(should_fail_with = "credential named graph")]
fn input_named_graph_refused() {
    let _ = open(fixtures::named::<16>());
}

#[test(should_fail_with = "reserved issuer predicate")]
fn input_method_assertion_refused() {
    let _ = open(fixtures::reserved::<16>());
}

#[test(should_fail_with = "wrong count")]
fn credential_overflow_refused() {
    let mut c = fixtures::base::<16>();
    c.count = 65;
    let _ = open(c);
}

#[test(should_fail_with = "blank scope")]
fn another_credential_blank_scope_refused() {
    let a = open_suite_credential(fixtures::blank_scope::<16>(), 0);
    let b = open_suite_credential(fixtures::blank_scope::<16>(), 1);
    let sa = required_property(
        a,
        a.credential_subject(),
        envelope_core::term::iri_subject(),
    );
    let sb = required_property(
        b,
        b.credential_subject(),
        envelope_core::term::iri_subject(),
    );
    assert(sa.credential() == 1);
    assert(sb.credential() == 2);
    assert(same_term(sa, sb), "blank scope");
}

#[test(should_fail_with = "quads not strictly increasing")]
fn duplicate_input_quad_refused() {
    let mut c = fixtures::base::<16>();
    c.raw[1] = c.raw[0];
    let _ = open(c);
}

#[test]
fn exact_absence_read_covers_all_rows() {
    let c = open(fixtures::last_until::<64>());
    assert(c.valid_until().bound());
    assert(!c.valid_from().bound());
}

#[test]
fn additional_credential_types_accepted() {
    let c = open(fixtures::additional_type::<16>());
    assert(c.credential_subject().bound());
}

#[test]
fn unrelated_subject_bounds_do_not_set_credential_period() {
    check_validity(open(fixtures::unrelated_bounds::<16>()), 200, no_bounds());
}

#[test(should_fail_with = "before validFrom")]
fn absent_bounds_cannot_precede_creation() {
    check_validity(open(fixtures::created::<16>()), 100, no_bounds());
}

#[test(should_fail_with = "creation time unavailable")]
fn absent_bounds_with_unknown_creation_refused() {
    check_validity(open(fixtures::unknown_created::<16>()), 150, no_bounds());
}

#[test(should_fail_with = "before earliestCreated")]
fn freshness_one_second_earlier_refused() {
    check_freshness(open(fixtures::earlier_created::<16>()), true, 1100);
}

#[test(should_fail_with = "creation time unavailable")]
fn freshness_unknown_creation_refused() {
    check_freshness(open(fixtures::unknown_created::<16>()), true, 100);
}

#[test(should_fail_with = "before earliestCreated")]
fn freshness_one_millisecond_earlier_refused() {
    check_freshness(open(fixtures::earlier_created::<16>()), true, 101);
}

#[test(should_fail_with = "before validFrom")]
fn validity_millisecond_before_refused() {
    check_validity(validity(), 100, period());
}

#[test(should_fail_with = "after validUntil")]
fn validity_millisecond_after_refused() {
    check_validity(validity(), 202, period());
}

#[test]
fn absent_bounds_include_creation_and_have_no_end() {
    let c = open(fixtures::created::<16>());
    check_validity(c, 101, no_bounds());
    check_validity(c, 999999, no_bounds());
}

#[test]
fn freshness_includes_boundary_second() {
    let c = open(fixtures::created::<16>());
    assert(c.created().known());
    assert(c.created().millis() == 101);
    check_freshness(c, true, 101);
}

#[test]
fn absent_freshness_imposes_nothing() {
    check_freshness(open(fixtures::unknown_created::<16>()), false, 0);
}

#[test]
fn disabled_optional_checks_are_explicit() {
    let c = base();
    let mut p = policy();
    p.issuer_root = issuer_root(c, key(1));
    check_all(c, key(1), p, no_status(), no_bounds(), empty_holder());
}

#[test(should_fail_with = "required property absent")]
fn envelope_required_holder_cannot_be_omitted() {
    let c = base();
    let mut p = policy();
    p.issuer_root = issuer_root(c, key(1));
    p.holder_required = true;
    check_all(c, key(1), p, no_status(), no_bounds(), empty_holder());
}

#[test(should_fail_with = "status entry absent")]
fn envelope_required_status_cannot_be_omitted() {
    let c = base();
    let mut p = policy();
    p.issuer_root = issuer_root(c, key(1));
    p.status_required = true;
    p.snapshot_root = policy_root();
    p.snapshot_depth = 1;
    check_all(c, key(1), p, no_status(), no_bounds(), empty_holder());
}

#[test(should_fail_with = "after validUntil")]
fn envelope_required_validity_cannot_be_omitted() {
    let c = validity();
    let mut p = policy();
    p.issuer_root = issuer_root(c, key(1));
    p.validity_required = true;
    p.now = 202;
    check_all(c, key(1), p, no_status(), period(), empty_holder());
}

#[test(should_fail_with = "before earliestCreated")]
fn envelope_freshness_runs_with_explicit_validity_bounds() {
    let c = validity();
    let mut p = policy();
    p.issuer_root = issuer_root(c, key(1));
    p.validity_required = true;
    p.now = 150;
    p.freshness_required = true;
    p.earliest_created = 101;
    check_all(c, key(1), p, no_status(), period(), empty_holder());
}

#[test(should_fail_with = "holder evidence coverage")]
fn required_holder_signature_cannot_be_omitted_in_envelope() {
    let opened_holder = holder();
    let c = opened_holder;
    let mut p = scoped_policy(issuer(c));
    p.issuer_root = issuer_root(c, key(1));
    check_all(c, key(1), p, no_status(), no_bounds(), empty_holder());
}

#[test]
fn envelope_all_requirements_accepted() {
    let c = open(fixtures::complete::<64>());
    let mut p = scoped_policy(issuer(c));
    p.issuer_root = issuer_root(c, key(1));
    p.status_required = true;
    p.snapshot_root = policy_root();
    p.snapshot_depth = 1;
    p.validity_required = true;
    p.now = 150;
    p.freshness_required = true;
    p.earliest_created = 101;
    check_all(c, key(1), p, evidence_for(c), period(), holder_evidence(11));
}

#[test]
fn unlisted_issuer_without_holder_key_accepted() {
    let p = scoped_policy(ir("urn:issuer:2".as_bytes()));
    check_holders([base()], p, digest(), [0], [empty_holder()]);
}

#[test(should_fail_with = "required property absent")]
fn listed_issuer_without_holder_key_refused() {
    let opened_base = base();
    let p = scoped_policy(issuer(opened_base));
    check_holders([opened_base], p, digest(), [0], [empty_holder()]);
}

#[test]
fn two_credentials_share_one_holder_signature() {
    let opened_holder = holder();
    let p = scoped_policy(issuer(opened_holder));
    check_holders(
        [opened_holder, opened_holder],
        p,
        digest(),
        [0, 0],
        [holder_evidence(11), empty_holder()],
    );
}

#[test]
fn two_distinct_holder_keys_use_two_signatures() {
    let opened_holder = holder();
    let p = scoped_policy(issuer(opened_holder));
    check_holders(
        [opened_holder, open(fixtures::holder_two::<16>())],
        p,
        digest(),
        [0, 1],
        [holder_evidence(11), holder_evidence(12)],
    );
}

#[test(should_fail_with = "holder evidence coverage")]
fn two_distinct_holder_keys_with_one_signature_refused() {
    let opened_holder = holder();
    let p = scoped_policy(issuer(opened_holder));
    let mut missing = holder_evidence(12);
    missing.sig.present = false;
    check_holders(
        [opened_holder, open(fixtures::holder_two::<16>())],
        p,
        digest(),
        [0, 1],
        [holder_evidence(11), missing],
    );
}

#[test(should_fail_with = "duplicate holder key")]
fn repeated_signature_for_the_same_holder_key_refused() {
    let opened_holder = holder();
    let p = scoped_policy(issuer(opened_holder));
    check_holders(
        [opened_holder, opened_holder],
        p,
        digest(),
        [0, 1],
        [holder_evidence(11), holder_evidence(11)],
    );
}

#[test(should_fail_with = "verification key opening differs")]
fn distinct_holder_keys_cannot_share_a_slot() {
    let opened_holder = holder();
    let p = scoped_policy(issuer(opened_holder));
    check_holders(
        [opened_holder, open(fixtures::holder_two::<16>())],
        p,
        digest(),
        [0, 0],
        [holder_evidence(11), empty_holder()],
    );
}

#[test(should_fail_with = "holder evidence coverage")]
fn unused_holder_signature_refused() {
    let p = scoped_policy(ir("urn:unlisted".as_bytes()));
    check_holders([base()], p, digest(), [0], [holder_evidence(11)]);
}

#[test(should_fail_with = "empty holder issuer list")]
fn explicit_empty_holder_issuer_list_refused() {
    let opened_base = base();
    let mut p = scoped_policy(issuer(opened_base));
    p.holder_issuer_count = 0;
    p.holder_issuers = [0; 4];
    check_holders([opened_base], p, digest(), [0], [empty_holder()]);
}

#[test(should_fail_with = "holder issuer padding")]
fn holder_issuer_list_requires_zero_padding() {
    let opened_base = base();
    let mut p = scoped_policy(issuer(opened_base));
    p.holder_issuers[3] = 9;
    let _ = envelope_core::envelope::holder_applies(issuer(opened_base), p);
}

#[test]
fn canonical_roots_ignore_member_order_and_duplicates() {
    let opened_base = base();
    let a = trusted_key_leaf(
        issuer(opened_base),
        opened_base.verification_method(),
        key(1),
    );
    let b = trusted_key_leaf(
        issuer(open(fixtures::issuer_two::<16>())),
        open(fixtures::issuer_two::<16>()).verification_method(),
        key(2),
    );
    let (r, d, n) = native::canonical_set_root([a, b, a, 0], 3);
    let (s, e, m) = native::canonical_set_root([b, a], 2);
    assert((r == s) & (d == e) & (n == m) & (d == 1) & (n == 2));
    let x = snapshot_digest::<2>(snapshot(false));
    let mut other = snapshot(false);
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
fn inactive_public_policy_has_zero_values() {
    envelope_core::envelope::check_policy::<3, 3>(policy());
}

#[test(should_fail_with = "disabled freshness policy")]
fn inactive_freshness_with_nonzero_bound_refused() {
    let mut p = policy();
    p.earliest_created = 1;
    envelope_core::envelope::check_policy::<3, 3>(p);
}

#[test(should_fail_with = "disabled validity policy")]
fn inactive_validity_with_nonzero_time_refused() {
    let mut p = policy();
    p.now = 1;
    envelope_core::envelope::check_policy::<3, 3>(p);
}

#[test(should_fail_with = "disabled status policy")]
fn inactive_status_with_nonzero_root_refused() {
    let mut p = policy();
    p.snapshot_root = 1;
    envelope_core::envelope::check_policy::<3, 3>(p);
}

#[test(should_fail_with = "disabled status policy")]
fn inactive_status_with_nonzero_depth_refused() {
    let mut p = policy();
    p.snapshot_depth = 1;
    envelope_core::envelope::check_policy::<3, 3>(p);
}

#[test(should_fail_with = "disabled status policy")]
fn inactive_status_with_missing_option_refused() {
    let mut p = policy();
    p.allow_missing_status = true;
    envelope_core::envelope::check_policy::<3, 3>(p);
}

#[test(should_fail_with = "disabled holder policy")]
fn inactive_holder_with_present_scope_refused() {
    let mut p = policy();
    p.holder_issuers_present = true;
    envelope_core::envelope::check_policy::<3, 3>(p);
}

#[test(should_fail_with = "disabled holder policy")]
fn inactive_holder_with_nonzero_count_refused() {
    let mut p = policy();
    p.holder_issuer_count = 1;
    p.holder_issuers[0] = 900;
    envelope_core::envelope::check_policy::<3, 3>(p);
}

#[test(should_fail_with = "disabled holder policy")]
fn inactive_holder_with_nonzero_word_refused() {
    let mut p = policy();
    p.holder_issuers[3] = 900;
    envelope_core::envelope::check_policy::<3, 3>(p);
}

#[test(should_fail_with = "holder issuer presence")]
fn absent_holder_scope_requires_zero_count() {
    let mut p = policy();
    p.holder_required = true;
    p.holder_issuer_count = 1;
    p.holder_issuers[0] = 900;
    envelope_core::envelope::check_policy::<3, 3>(p);
}

#[test(should_fail_with = "holder issuer order")]
fn duplicate_holder_issuer_words_refused() {
    let mut p = scoped_policy(issuer(base()));
    p.holder_issuer_count = 2;
    p.holder_issuers[1] = p.holder_issuers[0];
    envelope_core::envelope::check_policy::<3, 3>(p);
}

#[test]
fn two_issuer_fact_pins_and_swapped_pins() {
    let a = base();
    let b = open_suite_credential(fixtures::issuer_two::<16>(), 1);
    let la = trusted_key_leaf(issuer(a), a.verification_method(), key(1));
    let lb = trusted_key_leaf(issuer(b), b.verification_method(), key(2));
    let first = envelope_core::term::field_lt(la, lb);
    let root = if first { h2(la, lb) } else { h2(lb, la) };
    let va = check_issuer(
        a,
        root,
        1,
        key(1),
        Membership { index: if first { 0 } else { 1 }, siblings: [lb, 0] },
    );
    let vb = check_issuer(
        b,
        root,
        1,
        key(2),
        Membership { index: if first { 1 } else { 0 }, siblings: [la, 0] },
    );
    let d = merge_datasets::<2, 9, 32>([
        credential_dataset::<16, 9, 6>(a, va, digest(), 0),
        credential_dataset::<16, 9, 6>(b, vb, digest(), 1),
    ]);
    let f1 = ir("urn:fact:1".as_bytes());
    let f2 = ir("urn:fact:2".as_bytes());
    assert(pinned(d, f1, issuer(a)));
    assert(pinned(d, f2, issuer(b)));
    assert(!pinned(d, f1, issuer(b)));
    assert(!pinned(d, f2, issuer(a)));
}

#[test(should_fail_with = "dataset issuer binding")]
fn dataset_swapped_issuer_refused() {
    let _ = credential_dataset::<16, 9, 6>(base(), ir("urn:issuer:2".as_bytes()), digest(), 0);
}

#[test(should_fail_with = "operator capacity")]
fn dataset_capacity_refused() {
    let _ = dataset::<1>(base(), 0);
}

#[test]
fn dataset_graph_names_are_presentation_local() {
    let c = base();
    let d = dataset::<9>(c, 0);
    let mut other = digest();
    other[0] = 8;
    let e = credential_dataset::<16, 9, 6>(c, issuer(c), other, 0);
    let mut different = false;
    for i in 0..9 {
        different = different | !envelope_core::table::same_row(d.rows[i], e.rows[i]);
    }
    assert(different);
}

#[test]
fn graph_scope_public_digest_changes_graph_names() {
    let a = envelope_core::term::credential_graph(
        [0x07070707070707070707070707070707, 0x07070707070707070707070707070707],
        0,
    );
    let expected = from_suite_cell(a);
    let ds = dataset::<9>(base(), 0);
    let mut found = false;
    for i in 0..9 {
        found = found | ((i < ds.len) & equal_cell(ds.rows[i][0], expected));
    }
    assert(found);
}

#[test(should_fail_with = "public graph scope")]
fn other_public_graph_scope_refused() {
    let opened_base = base();
    let mut other = digest();
    other[31] = 8;
    let a = credential_dataset::<16, 9, 6>(opened_base, issuer(opened_base), digest(), 0);
    let b = credential_dataset::<16, 9, 6>(opened_base, issuer(opened_base), other, 0);
    for i in 0..9 {
        assert(envelope_core::table::same_row(a.rows[i], b.rows[i]), "public graph scope");
    }
}

#[test]
fn same_triple_has_one_default_copy_and_each_named_copy() {
    let a = base();
    let b = open_suite_credential(fixtures::base::<16>(), 1);
    let d = merge_datasets::<2, 9, 20>([dataset::<9>(a, 0), dataset::<9>(b, 1)]);
    assert(
        copies(
            d,
            ir("urn:subject".as_bytes()),
            ir("urn:shared".as_bytes()),
            ir("urn:value".as_bytes()),
        )
            == 3,
    );
    assert(d.len == 14);
}

#[test]
fn four_credentials_enforce_all_conditions_and_merge() {
    run_four(
        open_suite_credential(fixtures::complete_unlisted::<64>(), 3),
    );
}

#[test(should_fail_with = "issuer authorization set")]
fn fourth_credential_issuer_is_checked() {
    four_issuer(
        four_inputs(
            open_suite_credential(fixtures::complete_other_issuer::<64>(), 3),
        ),
    );
}
#[test(should_fail_with = "status entry absent")]
fn fourth_credential_status_is_checked() {
    four_status(
        four_inputs(
            open_suite_credential(fixtures::complete_no_status::<64>(), 3),
        ),
    );
}
#[test(should_fail_with = "after validUntil")]
fn fourth_credential_validity_is_checked() {
    four_validity(
        four_inputs(open_suite_credential(fixtures::complete_expired::<64>(), 3)),
        bounds(
            "1970-01-01T00:00:00.101Z".as_bytes(),
            "1970-01-01T00:00:00.149Z".as_bytes(),
        ),
    );
}
#[test(should_fail_with = "before earliestCreated")]
fn fourth_credential_freshness_is_checked() {
    four_freshness(
        four_inputs(open_suite_credential(fixtures::complete_old::<64>(), 3)),
    );
}
#[test(should_fail_with = "verification key opening differs")]
fn fourth_credential_holder_is_checked() {
    let cs = [
        open_suite_credential(fixtures::complete::<64>(), 0),
        open_suite_credential(fixtures::complete_shared::<64>(), 1),
        open_suite_credential(fixtures::complete_unlisted::<64>(), 2),
        open_suite_credential(fixtures::complete_distinct::<64>(), 3),
    ];
    four_holder(cs, [0, 0, 0, 0]);
}
#[test]
fn four_credentials_issuer_component() {
    four_issuer(four_normal());
}
#[test]
fn four_credentials_holder_component() {
    four_holder(four_normal(), [0, 0, 1, 0]);
}
#[test]
fn four_credentials_status_component() {
    four_status(four_normal());
}
#[test]
fn four_credentials_holder_status_together() {
    let cs = four_normal();
    four_issuer(cs);
    four_holder(cs, [0, 0, 1, 0]);
    four_status(cs);
}
#[test]
fn envelope_protocol_vectors_match_rust() {
    assert(holder_message(digest()) == fixtures::holder_message_vector());
    assert(status_leaf(false) == fixtures::status_leaf_false_vector());
    assert(status_leaf(true) == fixtures::status_leaf_true_vector());
    assert(snapshot_digest::<2>(snapshot(false)) == fixtures::snapshot_revocation_vector());
    assert(snapshot_digest::<2>(snapshot(true)) == fixtures::snapshot_suspension_vector());
}
#[test]
fn four_credentials_validity_component() {
    four_validity(four_normal(), period());
}
#[test]
fn four_credentials_freshness_component() {
    four_freshness(four_normal());
}
#[test]
fn four_credentials_dataset_first_view() {
    four_dataset_view(four_normal(), 0);
}
#[test]
fn four_credentials_dataset_second_view() {
    four_dataset_view(four_normal(), 1);
}
#[test]
fn four_credentials_dataset_third_view() {
    four_dataset_view(four_normal(), 2);
}
#[test]
fn four_credentials_dataset_fourth_view() {
    four_dataset_view(four_normal(), 3);
}
#[test]
fn four_credentials_shared_merge_component() {
    four_shared_merge(four_normal());
}

#[test]
fn suite_cells_and_complete_signed_message_reach_the_envelope() {
    let raw = fixtures::complete::<64>();
    let c = open(raw);
    assert(c.created_at().bound());
    assert(c.created().millis() == 101);
    assert(c.root() != 0);
    assert(c.message() != 0);
    assert(
        trusted_key_leaf(issuer(c), c.verification_method(), key(1))
            == fixtures::issued_trust_digest(),
    );
    let _ = check_i(c, issuer_root(c, key(1)), key(1));
}

#[test]
fn set_member_at_public_depth_below_compiled_maximum_accepted() {
    let c = base();
    let _ = check_i(c, issuer_root(c, key(1)), key(1));
}

#[test(should_fail_with = "issuer authorization set")]
fn issuer_member_one_level_too_shallow_refused() {
    let c = base();
    let leaf = trusted_key_leaf(issuer(c), c.verification_method(), key(1));
    let _ = check_i(c, leaf, key(1));
}

#[test(should_fail_with = "issuer authorization set")]
fn issuer_member_one_level_too_deep_refused() {
    let c = base();
    let _ = check_i(c, h2(issuer_root(c, key(1)), 888), key(1));
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
    let opened_status = status();
    check_status(
        opened_status,
        policy_root(),
        1,
        false,
        evidence_for(opened_status),
    );
}

#[test(should_fail_with = "snapshot policy")]
fn snapshot_member_one_level_too_shallow_refused() {
    let opened_status = status();
    check_status(
        opened_status,
        snapshot_digest::<2>(snapshot(false)),
        1,
        false,
        evidence_for(opened_status),
    );
}

#[test(should_fail_with = "snapshot policy")]
fn snapshot_member_one_level_too_deep_refused() {
    let opened_status = status();
    check_status(
        opened_status,
        h2(policy_root(), 888),
        1,
        false,
        evidence_for(opened_status),
    );
}

#[test(should_fail_with = "status entry absent")]
fn public_policy_a_refuses_evidence_satisfying_only_policy_b() {
    let c = base();
    let mut p = policy();
    p.issuer_root = issuer_root(c, key(1));
    p.status_required = true;
    p.snapshot_root = policy_root();
    p.snapshot_depth = 1;
    check_all(c, key(1), p, no_status(), no_bounds(), empty_holder());
}

#[test]
fn public_policy_b_accepts_its_evidence() {
    let c = base();
    let mut p = policy();
    p.issuer_root = issuer_root(c, key(1));
    check_all(c, key(1), p, no_status(), no_bounds(), empty_holder());
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
    let c = open(fixtures::complete::<64>());
    let trust = trusted_key_leaf(issuer(c), c.verification_method(), key(1));
    assert(pair == 0x038682aa1cb5ae4e0a3f13da432a95c77c5c111f6f030faf9cad641ce1ed7383);
    assert(padded == 0x23e40150eaf2a79630776edb762a0525705f0206f8e3da4cbbe2fba8ae3d7b87);
    assert(unsigned == 0x13d34888a5f10181210c333c7cec7169f39265a0f2664be7b5bdaa417a3ca537);
    assert(trust == fixtures::issued_trust_digest());
    let original = trusted_key_leaf(
        ir("urn:test:issuer".as_bytes()),
        ir("urn:test:key".as_bytes()),
        Point { x: compose_core::issuer::BJJ_GX, y: compose_core::issuer::BJJ_GY },
    );
    assert(original == 0x0634ae948a9551ef686ae6afd30505a0ab76509e71b977393210dc7e764dd62c);

    println([pair, singleton, padded, unsigned, trust]);
}

#[test(should_fail_with = "creation time unavailable")]
fn submillisecond_creation_is_not_rounded() {
    check_freshness(open(fixtures::created_submillis::<16>()), true, 100);
}
