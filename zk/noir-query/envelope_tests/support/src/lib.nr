//! Test and probe adapters; authentication and conversion use the core consumer API.
use envelope_core::envelope::{
    CredentialOpening, Membership, OpenedCredential, Snapshot, StatusEvidence,
};
use envelope_core::ops_core_profile::{ObjectOpening, QuadOpening, ResourceOpening};
use envelope_core::ops_core_suite::{
    authenticate_profile_bounded, BoundedCredentialProof, from_authenticated_bounded,
};
use envelope_core::suite_entry::{BoundedTermOpening, TermOpening};
use std::collections::bounded_vec::BoundedVec;

fn resource<let L: u32>(raw: TermOpening<L>) -> ResourceOpening<L> {
    assert((raw.datatype_len == 0) & (raw.language_len == 0), "resource metadata");
    assert(
        (raw.datatype == [0; 128]) & (raw.language == [0; 64]) & (raw.key == [0; 31]),
        "resource padding",
    );
    ResourceOpening { kind: raw.kind, lexical: raw.lexical, lexical_len: raw.lexical_len }
}
fn bounded<let L: u32>(raw: TermOpening<L>) -> BoundedTermOpening<L, 41, 1> {
    assert((raw.datatype_len <= 41) & (raw.language_len <= 1), "fixture metadata capacity");
    let mut datatype = [0; 41];
    for i in 0..128 {
        if i < 41 {
            datatype[i] = raw.datatype[i];
        } else {
            assert(raw.datatype[i] == 0, "fixture datatype padding");
        }
    }
    for i in 1..64 {
        assert(raw.language[i] == 0, "fixture language padding");
    }
    BoundedTermOpening {
        bound: raw.bound,
        kind: raw.kind,
        lexical: raw.lexical,
        lexical_len: raw.lexical_len,
        datatype,
        datatype_len: raw.datatype_len,
        language: [raw.language[0]],
        language_len: raw.language_len,
        key: raw.key,
    }
}
fn quad<let L: u32>(raw: [TermOpening<L>; 4]) -> QuadOpening<L, L> {
    let object = bounded(raw[2]);
    assert(raw[3].kind == 0, "credential named graph");
    assert((raw[3].lexical == [0; L]) & (raw[3].lexical_len == 0), "graph padding");
    let _graph = resource(raw[3]);
    QuadOpening {
        subject: resource(raw[0]),
        predicate: resource(raw[1]),
        object: ObjectOpening {
            kind: object.kind,
            lexical: object.lexical,
            lexical_len: object.lexical_len,
            datatype: object.datatype,
            datatype_len: object.datatype_len,
            language: object.language,
            language_len: object.language_len,
            key: object.key,
        },
    }
}
fn raw_quads<let N: u32, let L: u32>(proof: CredentialOpening<N, L>) -> [QuadOpening<L, L>; N] {
    let mut raw = [quad(proof.raw[0]); N];
    for i in 0..N {
        raw[i] = quad(proof.raw[i]);
    }
    raw
}
fn proof_fields<let N: u32, let L: u32>(
    proof: CredentialOpening<N, L>,
) -> BoundedCredentialProof<N> {
    assert(proof.signature.present, "signature evidence absent");
    BoundedCredentialProof {
        salt: proof.salt,
        salts: proof.salts,
        config: proof.configuration,
        created: bounded(proof.created),
        method: resource(proof.method),
        pk: proof.pk,
        r: proof.signature.r,
        s: proof.signature.s,
        e: proof.signature.e,
        e_k: proof.signature.e_k,
        issuer_index: 0,
        issuer_siblings: [0; 4],
    }
}
pub fn open<let N: u32, let L: u32>(
    proof: CredentialOpening<N, L>,
    credential_index: u32,
) -> OpenedCredential<N> {
    let c = authenticate_profile_bounded::<N, L, L, 128>(
        raw_quads(proof),
        proof_fields(proof),
        proof.count,
        N,
        credential_index,
    );
    envelope_core::envelope::check_credential(c);
    c
}
/// Bind opening and conversion to the same index; the converted view is never a witness.
pub fn open_and_convert<let N: u32, let L: u32>(
    proof: CredentialOpening<N, L>,
    credential_index: u32,
    request_digest: [u8; 32],
) -> (OpenedCredential<N>, envelope_core::ops_core_suite::OpenedCredential<N>) {
    let raw = raw_quads(proof);
    let c = authenticate_profile_bounded::<N, L, L, 128>(
        raw,
        proof_fields(proof),
        proof.count,
        N,
        credential_index,
    );
    envelope_core::envelope::check_credential(c);
    // Value consumers perform their own checked reads; this conversion makes no global key claim.
    let view = from_authenticated_bounded::<N, L, L, 0>(c, raw, credential_index, request_digest);
    (c, view)
}

/// ABI evidence uses raw openings; sealed cells are constructed inside the program.
pub struct StatusOpening<let D: u32, let A: u32> {
    pub present: bool,
    pub list: TermOpening<160>,
    pub snapshot_purpose: TermOpening<16>,
    pub bit_len: u32,
    pub root: Field,
    pub accepted: Membership<A>,
    pub bit: bool,
    pub path: Membership<D>,
    pub purpose: BoundedVec<u8, 16>,
    pub index: BoundedVec<u8, 20>,
    pub size: BoundedVec<u8, 20>,
}
pub fn open_status<let D: u32, let A: u32>(raw: StatusOpening<D, A>) -> StatusEvidence<D, A> {
    StatusEvidence {
        present: raw.present,
        snapshot: Snapshot {
            list: envelope_core::suite_entry::open_cell(raw.list, 0),
            purpose: envelope_core::suite_entry::open_cell(raw.snapshot_purpose, 0),
            bit_len: raw.bit_len,
            root: raw.root,
        },
        accepted: raw.accepted,
        bit: raw.bit,
        path: raw.path,
        purpose: raw.purpose,
        index: raw.index,
        size: raw.size,
    }
}
pub fn open_status_rows<let N: u32, let D: u32, let A: u32>(
    raw: [StatusOpening<D, A>; N],
) -> [StatusEvidence<D, A>; N] {
    let mut evidence = [open_status(raw[0]); N];
    for i in 0..N {
        evidence[i] = open_status(raw[i]);
    }
    evidence
}
