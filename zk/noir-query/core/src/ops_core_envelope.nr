//! Generated wiring between raw witnesses and the owned envelope consumer.
use crate::envelope::{PublicPolicy, HolderEvidence, Membership, BoundedStatusEvidence, Snapshot, ValidityOpening};
use crate::ops_core_profile::QuadOpening;
use crate::ops_core_suite::{OpenedCredential, BoundedCredentialProof};
use std::collections::bounded_vec::BoundedVec;

pub struct RawStatus<let D:u32,let A:u32> {
    pub list_row:u32,pub purpose_row:u32,
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
fn selected_status_property<let N:u32>(quads:[[crate::term::TermCell;4];N],count:u32,
    entry:crate::term::TermCell,predicate:crate::term::TermCell,row:u32) -> crate::term::TermCell {
    assert(row<count,"status property row");
    let selected=quads[row];
    assert(crate::term::same_term(selected[0],entry)
        &crate::term::same_term(selected[1],predicate)&(selected[3].kind()==0),"status property source");
    selected[2]
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
        // Row references are constrained to the signed entry and predicate. The
        // consumer retains complete presence and singleton-property checks.
        if e.present {
            list=selected_status_property(quads,c.count(),quads[i][2],crate::term::iri_status_list(),e.list_row);
            purpose=selected_status_property(quads,c.count(),quads[i][2],crate::term::iri_status_purpose(),e.purpose_row);
        }
        result[i]=BoundedStatusEvidence {depth:e.depth,present:e.present,snapshot:Snapshot {list,purpose,bit_len:e.bit_len,root:e.root},
            accepted:e.accepted,bit:e.bit,path:e.path,
            purpose:BoundedVec::from_parts_unchecked(e.purpose,e.purpose_len),
            index:BoundedVec::from_parts_unchecked(e.index,e.index_len),
            size:BoundedVec::from_parts_unchecked(e.size,e.size_len)};
    }
    result
}

/// Bind the public flags to verifier-selected circuit structure. Disabled values
/// retain their public slots and are constrained to their canonical zero encoding.
pub fn select_policy<let H:u32,let S:u32,let V:u32,let F:u32>(policy:PublicPolicy) -> PublicPolicy {
    assert((H<=1)&(S<=1)&(V<=1)&(F<=1),"envelope feature selection");
    assert(policy.holder_required==(H==1),"compiled holder policy");
    assert(policy.status_required==(S==1),"compiled status policy");
    assert(policy.validity_required==(V==1),"compiled validity policy");
    assert(policy.freshness_required==(F==1),"compiled freshness policy");
    let mut selected=policy;
    selected.holder_required=H==1;selected.status_required=S==1;
    selected.validity_required=V==1;selected.freshness_required=F==1;
    crate::envelope::check_policy::<4,4>(selected);
    selected
}

/// The holder relation is shared across credential slots. An omitted consumer
/// admits no signature evidence and performs no signature verification.
pub fn check_selected_holders<let H:u32,let C:u32>(digest:[u8;32],used:[bool;C],evidence:[HolderEvidence;C]) {
    assert(H<=1,"envelope feature selection");
    if H==1 {
        crate::envelope::check_holder_signatures(digest,used,evidence);
    } else {
        for i in 0..C {
            assert(!used[i]&!evidence[i].sig.present,"disabled holder evidence");
        }
    }
}

/// Each slot opens once. All policy decisions delegate to the envelope consumer.
pub fn open_slot_selected<let N:u32,let R:u32,let L:u32,let M:u32,let C:u32,
    let H:u32,let S:u32,let V:u32,let F:u32>(
    raw:[QuadOpening<R,L>;N],proof:BoundedCredentialProof<N>,
    count:u32,active:bool,quad_bound:u32,credential_index:u32,
    digest:[u8;32],mode:u8,revealed_message:Field,revealed_key:Field,
    policy:PublicPolicy,evidence:CredentialEvidence<N>,holders:[HolderEvidence;C],
) -> (OpenedCredential<N>,crate::term::TermCell,[bool;C]) {
    open_slot_inner::<N,R,L,M,C,H,S,V,F,1>(raw,proof,count,active,quad_bound,credential_index,digest,mode,revealed_message,revealed_key,policy,evidence,holders)
}

fn open_slot_inner<let N:u32,let R:u32,let L:u32,let M:u32,let C:u32,
    let H:u32,let S:u32,let V:u32,let F:u32,let SELECTED:u32>(
    raw:[QuadOpening<R,L>;N],proof:BoundedCredentialProof<N>,
    count:u32,active:bool,quad_bound:u32,credential_index:u32,
    digest:[u8;32],mode:u8,revealed_message:Field,revealed_key:Field,
    policy:PublicPolicy,evidence:CredentialEvidence<N>,holders:[HolderEvidence;C],
) -> (OpenedCredential<N>,crate::term::TermCell,[bool;C]) {
    let policy=if SELECTED==1 {select_policy::<H,S,V,F>(policy)} else {policy};
    assert(mode<=1,"suite signature mode");
    let mut view=crate::ops_core_suite::inactive_bounded(raw,proof,count,quad_bound,credential_index,active,
        revealed_message,revealed_key);
    let mut issuer=crate::term::padding();let mut used=[false;C];
    if active {
        let c=crate::ops_core_suite::authenticate_profile_bounded_mode::<N,R,L,M>(
            raw,proof,count,quad_bound,credential_index,mode,revealed_message,revealed_key);
        issuer=crate::envelope::check_issuer(c,policy.issuer_root,policy.issuer_depth,proof.pk,
            Membership{index:proof.issuer_index,siblings:proof.issuer_siblings});
        if (H==1)&crate::envelope::holder_applies(issuer,policy) {
            assert(evidence.holder_slot<C,"holder evidence slot");
            crate::envelope::check_holder_key(c,holders[evidence.holder_slot].pk);
            used[evidence.holder_slot]=true;
        }
        if (S==1)&policy.status_required {
            crate::envelope::check_status_bounded(c,policy.snapshot_root,policy.snapshot_depth,policy.allow_missing_status,
                status_evidence(c,evidence.status));
        }
        if (V==1)&policy.validity_required {
            crate::envelope::check_validity(c,policy.now,ValidityOpening{
                from:BoundedVec::from_parts_unchecked(evidence.validity.from,evidence.validity.from_len),
                until:BoundedVec::from_parts_unchecked(evidence.validity.until,evidence.validity.until_len)});
        }
        if F==1 {crate::envelope::check_freshness(c,policy.freshness_required,policy.earliest_created);}
        view=crate::ops_core_suite::from_authenticated_bounded::<N,R,L,0>(c,raw,credential_index,digest);
    }
    (view,issuer,used)
}


pub fn merge_coverage<let C:u32>(a:[bool;C],b:[bool;C]) -> [bool;C] {
    let mut used=[false;C];for i in 0..C {used[i]=a[i]|b[i];}used
}

// Dynamic fixture entry point; canonical generation uses open_slot_selected.
pub fn open_slot<let N:u32,let R:u32,let L:u32,let M:u32,let C:u32>(
    raw:[QuadOpening<R,L>;N],proof:BoundedCredentialProof<N>,
    count:u32,active:bool,quad_bound:u32,credential_index:u32,
    digest:[u8;32],mode:u8,revealed_message:Field,revealed_key:Field,
    policy:PublicPolicy,evidence:CredentialEvidence<N>,holders:[HolderEvidence;C],
) -> (OpenedCredential<N>,crate::term::TermCell,[bool;C]) {
    open_slot_inner::<N,R,L,M,C,1,1,1,1,0>(raw,proof,count,active,quad_bound,credential_index,digest,mode,revealed_message,revealed_key,policy,evidence,holders)
}

fn feature_test_policy() -> PublicPolicy {
    PublicPolicy {issuer_root:1,issuer_depth:0,holder_required:false,
        holder_issuers_present:false,holder_issuer_count:0,holder_issuers:[0;4],
        status_required:false,allow_missing_status:false,snapshot_root:0,snapshot_depth:0,
        validity_required:false,now:0,freshness_required:false,earliest_created:0}
}

#[test]
fn selected_policy_retains_zero_disabled_words() {
    let selected=select_policy::<0,0,0,0>(feature_test_policy());
    assert(!selected.holder_required&!selected.status_required&!selected.validity_required&!selected.freshness_required);
    assert(selected.now==0);
}
#[test(should_fail_with="compiled holder policy")]
fn selected_policy_cannot_drop_requested_holder() {
    let mut policy=feature_test_policy();policy.holder_required=true;
    let _selected=select_policy::<0,0,0,0>(policy);
}
#[test(should_fail_with="compiled status policy")]
fn selected_policy_cannot_drop_requested_status() {
    let mut policy=feature_test_policy();policy.status_required=true;policy.snapshot_root=1;
    let _selected=select_policy::<0,0,0,0>(policy);
}
#[test(should_fail_with="compiled validity policy")]
fn selected_policy_cannot_drop_requested_validity() {
    let mut policy=feature_test_policy();policy.validity_required=true;policy.now=1;
    let _selected=select_policy::<0,0,0,0>(policy);
}
#[test(should_fail_with="compiled freshness policy")]
fn selected_policy_cannot_drop_requested_freshness() {
    let mut policy=feature_test_policy();policy.freshness_required=true;policy.earliest_created=1;
    let _selected=select_policy::<0,0,0,0>(policy);
}
#[test(should_fail_with="compiled holder policy")]
fn selected_policy_cannot_enable_an_unrequested_check() {
    let _selected=select_policy::<1,0,0,0>(feature_test_policy());
}
#[test(should_fail_with="disabled status policy")]
fn selected_policy_cannot_hide_a_disabled_status_root() {
    let mut policy=feature_test_policy();policy.snapshot_root=1;
    let _selected=select_policy::<0,0,0,0>(policy);
}
#[test(should_fail_with="disabled validity policy")]
fn selected_policy_cannot_hide_a_disabled_validity_time() {
    let mut policy=feature_test_policy();policy.now=1;
    let _selected=select_policy::<0,0,0,0>(policy);
}
#[test(should_fail_with="disabled freshness policy")]
fn selected_policy_cannot_hide_a_disabled_freshness_time() {
    let mut policy=feature_test_policy();policy.earliest_created=1;
    let _selected=select_policy::<0,0,0,0>(policy);
}
#[test]
fn selected_policy_all_consumers_remain_required() {
    let mut policy=feature_test_policy();policy.holder_required=true;policy.status_required=true;
    policy.snapshot_root=1;policy.validity_required=true;policy.now=2;
    policy.freshness_required=true;policy.earliest_created=1;
    let selected=select_policy::<1,1,1,1>(policy);
    assert(selected.holder_required&selected.status_required&selected.validity_required&selected.freshness_required);
}
#[test(should_fail_with="disabled holder evidence")]
fn disabled_holder_consumer_refuses_signature_evidence() {
    let point=crate::envelope::Point{x:0,y:0};
    let evidence=HolderEvidence {pk:point,sig:crate::envelope::Signature{present:true,r:point,s:0,e:0,e_k:0}};
    check_selected_holders::<0,1>([0;32],[false],[evidence]);
}
#[test(should_fail_with="compiled status policy")]
fn selected_status_cannot_accept_a_false_public_flag() {
    let mut policy=feature_test_policy();policy.snapshot_root=1;
    let _selected=select_policy::<0,1,0,0>(policy);
}
#[test(should_fail)]
fn selected_holder_consumer_checks_a_required_signature() {
    let point=crate::envelope::Point{x:0,y:0};
    let evidence=HolderEvidence {pk:point,sig:crate::envelope::Signature{present:true,r:point,s:0,e:0,e_k:0}};
    check_selected_holders::<1,1>([0;32],[true],[evidence]);
}
#[test(should_fail_with="compiled validity policy")]
fn selected_validity_cannot_accept_a_false_public_flag() {
    let mut policy=feature_test_policy();policy.now=1;
    let _selected=select_policy::<0,0,1,0>(policy);
}
#[test(should_fail_with="compiled freshness policy")]
fn selected_freshness_cannot_accept_a_false_public_flag() {
    let mut policy=feature_test_policy();policy.earliest_created=1;
    let _selected=select_policy::<0,0,0,1>(policy);
}

fn selection_test_quads() -> [[crate::term::TermCell;4];4] {
    let graph=crate::term::open_term(0,BoundedVec::<u8,0>::new(),BoundedVec::<u8,0>::new(),BoundedVec::<u8,0>::new(),[0;31],0);
    let entry=crate::term::iri_subject();let predicate=crate::term::iri_status_list();
    let value=crate::term::iri_issuer();let other=crate::term::iri_status_purpose();
    [[entry,predicate,value,graph],[other,predicate,value,graph],
     [entry,other,value,graph],[entry,predicate,value,other]]
}
#[test]
fn status_property_selection_returns_the_authenticated_object() {
    let value=selected_status_property(selection_test_quads(),4,crate::term::iri_subject(),crate::term::iri_status_list(),0);
    assert(crate::term::same_term(value,crate::term::iri_issuer()));
}
#[test(should_fail_with="status property source")]
fn status_property_selection_refuses_another_subject_with_the_same_value() {
    let _value=selected_status_property(selection_test_quads(),4,crate::term::iri_subject(),crate::term::iri_status_list(),1);
}
#[test(should_fail_with="status property source")]
fn status_property_selection_refuses_another_predicate_with_the_same_value() {
    let _value=selected_status_property(selection_test_quads(),4,crate::term::iri_subject(),crate::term::iri_status_list(),2);
}
#[test(should_fail_with="status property source")]
fn status_property_selection_refuses_a_named_graph() {
    let _value=selected_status_property(selection_test_quads(),4,crate::term::iri_subject(),crate::term::iri_status_list(),3);
}
#[test(should_fail_with="status property row")]
fn status_property_selection_refuses_an_unsigned_row() {
    let _value=selected_status_property(selection_test_quads(),0,crate::term::iri_subject(),crate::term::iri_status_list(),0);
}
#[test(should_fail_with="envelope feature selection")]
fn selected_policy_refuses_an_unknown_feature_tag() {
    let _selected=select_policy::<2,0,0,0>(feature_test_policy());
}
#[test(should_fail_with="envelope feature selection")]
fn selected_holder_refuses_an_unknown_feature_tag() {
    let point=crate::envelope::Point{x:0,y:0};
    let evidence=HolderEvidence {pk:point,sig:crate::envelope::Signature{present:false,r:point,s:0,e:0,e_k:0}};
    check_selected_holders::<2,1>([0;32],[false],[evidence]);
}
