//! Generated wiring between raw witnesses and the owned envelope consumer.
use crate::envelope::{PublicPolicy, HolderEvidence, Membership, BoundedStatusEvidence, Snapshot, ValidityOpening};
use crate::ops_core_profile::QuadOpening;
use crate::ops_core_suite::{OpenedCredential, BoundedCredentialProof};
use std::collections::bounded_vec::BoundedVec;

pub struct RawStatus<let D:u32,let A:u32> {
    pub depth:u32, pub present:bool, pub bit_len:u32, pub root:Field,
    pub accepted:Membership<A>, pub bit:bool, pub path:Membership<D>,
    pub purpose:[u8;16], pub purpose_len:u32,
    pub index:[u8;20], pub index_len:u32,
    pub size:[u8;20], pub size_len:u32,
}
pub struct RawValidity {
    pub from:[u8;32], pub from_len:u32, pub until:[u8;32], pub until_len:u32,
}
pub struct CredentialEvidence<let N:u32> {
    pub holder_slot:u32, pub status:[RawStatus<32,4>;N], pub validity:RawValidity,
}
fn status_evidence<let N:u32,let D:u32,let A:u32>(
    c:crate::suite_metadata::OpenedCredential<N>,raw:[RawStatus<D,A>;N]) -> [BoundedStatusEvidence<D,A>;N] {
    let absent=BoundedStatusEvidence {depth:0,present:false,
        snapshot:Snapshot {list:crate::term::padding(),purpose:crate::term::padding(),bit_len:0,root:0},
        accepted:Membership {index:0,siblings:[0;A]},bit:false,path:Membership {index:0,siblings:[0;D]},
        purpose:BoundedVec::new(),index:BoundedVec::new(),size:BoundedVec::new()};
    let mut result=[absent;N];let quads=c.quads();
    for i in 0..N {
        let e=raw[i];let mut list=crate::term::padding();let mut purpose=crate::term::padding();
        // The consumer independently checks presence against every signed row.
        if e.present {
            list=c.property(quads[i][2],crate::term::iri_status_list());
            purpose=c.property(quads[i][2],crate::term::iri_status_purpose());
        }
        result[i]=BoundedStatusEvidence {depth:e.depth,present:e.present,snapshot:Snapshot {list,purpose,bit_len:e.bit_len,root:e.root},
            accepted:e.accepted,bit:e.bit,path:e.path,
            purpose:BoundedVec::from_parts_unchecked(e.purpose,e.purpose_len),
            index:BoundedVec::from_parts_unchecked(e.index,e.index_len),
            size:BoundedVec::from_parts_unchecked(e.size,e.size_len)};
    }
    result
}

/// Each slot opens once. All policy decisions delegate to the envelope consumer.
pub fn open_slot<let N:u32,let R:u32,let L:u32,let M:u32,let C:u32>(
    raw:[QuadOpening<R,L>;N],proof:BoundedCredentialProof<N>,
    count:u32,active:bool,quad_bound:u32,credential_index:u32,
    digest:[u8;32],mode:u8,revealed_message:Field,revealed_key:Field,
    policy:PublicPolicy,evidence:CredentialEvidence<N>,holders:[HolderEvidence;C],
) -> (OpenedCredential<N>,crate::term::TermCell,[bool;C]) {
    assert(mode<=1,"suite signature mode");
    let mut view=crate::ops_core_suite::inactive_bounded(raw,proof,count,quad_bound,credential_index,active,
        revealed_message,revealed_key);
    let mut issuer=crate::term::padding();let mut used=[false;C];
    if active {
        let c=crate::ops_core_suite::authenticate_profile_bounded_mode::<N,R,L,M>(
            raw,proof,count,quad_bound,credential_index,mode,revealed_message,revealed_key);
        issuer=crate::envelope::check_issuer(c,policy.issuer_root,policy.issuer_depth,proof.pk,
            Membership{index:proof.issuer_index,siblings:proof.issuer_siblings});
        if crate::envelope::holder_applies(issuer,policy) {
            assert(evidence.holder_slot<C,"holder evidence slot");
            crate::envelope::check_holder_key(c,holders[evidence.holder_slot].pk);
            used[evidence.holder_slot]=true;
        }
        if policy.status_required {
            crate::envelope::check_status_bounded(c,policy.snapshot_root,policy.snapshot_depth,policy.allow_missing_status,
                status_evidence(c,evidence.status));
        }
        if policy.validity_required {
            crate::envelope::check_validity(c,policy.now,ValidityOpening{
                from:BoundedVec::from_parts_unchecked(evidence.validity.from,evidence.validity.from_len),
                until:BoundedVec::from_parts_unchecked(evidence.validity.until,evidence.validity.until_len)});
        }
        crate::envelope::check_freshness(c,policy.freshness_required,policy.earliest_created);
        view=crate::ops_core_suite::from_authenticated_bounded::<N,R,L,0>(c,raw,credential_index,digest);
    }
    (view,issuer,used)
}


pub fn merge_coverage<let C:u32>(a:[bool;C],b:[bool;C]) -> [bool;C] {
    let mut used=[false;C];for i in 0..C {used[i]=a[i]|b[i];}used
}
