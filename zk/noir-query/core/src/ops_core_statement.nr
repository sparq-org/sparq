//! Public statement binding and the explicit supplied-cell dataset stand-in.

use crate::table::{TermCell, Table};
use compose::hashes::h2;

global STATEMENT_DOMAIN: Field = 0x4e5153544d5431;
global CREDENTIAL_DOMAIN: Field = 0x4e5143454c4c31;
global DATASET_DOMAIN: [u8; 16] = [78,81,47,83,85,80,80,76,73,69,68,47,86,49,0,0];

fn absorb_cell(mut state: Field, cell: TermCell) -> Field {
    for word in [cell.bound as Field, cell.kind as Field, cell.enc, cell.key_class as Field,
        cell.key, cell.dt, cell.lang, cell.scope as Field] { state = h2(state, word); }
    state
}

/// STAND-IN: suite integration replaces the private credential message derivation.
/// The presentation salt and all credential cells remain private.
pub fn check_dataset<let C: u32, let N: u32>(
    credentials: [[[TermCell; 4]; N]; C], counts: [u32; C], credential_count: u32,
    presentation_salt: [u8; 32], input_commitment: [u8; 32], signature_mode: u8,
) {
    assert(signature_mode == 0, "supplied-cell signature mode");
    let mut bytes: [u8; 48 + C * 32] = [0; 48 + C * 32];
    for i in 0..16 { bytes[i] = DATASET_DOMAIN[i]; }
    for i in 0..32 { bytes[16 + i] = presentation_salt[i]; }
    for c in 0..C {
        let mut message = h2(CREDENTIAL_DOMAIN, counts[c] as Field);
        for q in 0..N { if q < counts[c] {
            for p in 0..4 { message = absorb_cell(message, credentials[c][q][p]); }
        } }
        let encoded: [u8; 32] = message.to_be_bytes();
        for i in 0..32 { bytes[48 + c * 32 + i] = if c < credential_count { encoded[i] } else { 0 }; }
    }
    let digest = hash_sha256::sha256_var(bytes, 48 + credential_count * 32);
    assert(digest == input_commitment, "dataset commitment");
}

fn prefix<let P: u32, let I: u32>(
    request_digest: [u8; 32], input_commitment: [u8; 32], signature_mode: u8,
    bounds: [u32; 3], parameters: [TermCell; P], indices: [u32; I],
) -> Field {
    let mut state = STATEMENT_DOMAIN;
    for byte in request_digest { state = h2(state, byte as Field); }
    for byte in input_commitment { state = h2(state, byte as Field); }
    state = h2(state, signature_mode as Field);
    for bound in bounds { state = h2(state, bound as Field); }
    for parameter in parameters { state = absorb_cell(state, parameter); }
    for index in indices { state = h2(state, index as Field); }
    state
}

pub fn bind_select<let P: u32, let I: u32, let R: u32, let V: u32>(
    request_digest: [u8; 32], input_commitment: [u8; 32], signature_mode: u8,
    bounds: [u32; 3], parameters: [TermCell; P], indices: [u32; I],
    result: Table<R, V>, statement_binding: Field,
) {
    let mut state = prefix(request_digest, input_commitment, signature_mode, bounds, parameters, indices);
    for row in result.rows { for cell in row { state = absorb_cell(state, cell); } }
    state = h2(state, result.len as Field);
    assert(state == statement_binding, "statement binding");
}

pub fn bind_ask<let P: u32, let I: u32>(
    request_digest: [u8; 32], input_commitment: [u8; 32], signature_mode: u8,
    bounds: [u32; 3], parameters: [TermCell; P], indices: [u32; I],
    result: bool, statement_binding: Field,
) {
    let state = h2(prefix(request_digest, input_commitment, signature_mode, bounds, parameters, indices), result as Field);
    assert(state == statement_binding, "statement binding");
}

global SUITE_DATASET_DOMAIN: [u8; 16] = [78,81,47,83,85,73,84,69,47,86,49,0,0,0,0,0];

pub fn check_suite_dataset<let C: u32>(messages: [Field; C], count: u32, salt: [u8; 32], commitment: [u8; 32]) {
    assert(count <= C, "credential count");
    let mut bytes: [u8; 48 + C * 32] = [0; 48 + C * 32];
    for i in 0..16 { bytes[i] = SUITE_DATASET_DOMAIN[i]; }
    for i in 0..32 { bytes[16 + i] = salt[i]; }
    for c in 0..C {
        assert((c < count) | (messages[c] == 0), "message padding");
        let encoded: [u8; 32] = messages[c].to_be_bytes();
        for i in 0..32 { bytes[48 + c * 32 + i] = encoded[i]; }
    }
    assert(hash_sha256::sha256_var(bytes, 48 + count * 32) == commitment, "suite dataset commitment");
}

/// Canonical policy words. Policy enforcement is supplied by the envelope consumer.
pub fn envelope_words(
    issuer_root: Field, issuer_depth: u32, holder_required: bool, holder_issuers_present: bool,
    holder_issuer_count: u32, holder_issuers: [Field;4], status_required: bool,
    allow_missing_status: bool, snapshot_root: Field, snapshot_depth: u32,
    validity_required: bool, now: u64, freshness_required: bool, earliest_created: u64,
) -> [Field;17] {
    assert(holder_issuer_count <= 4, "holder issuer capacity");
    assert(holder_required | !holder_issuers_present, "inactive holder policy");
    assert(holder_issuers_present | (holder_issuer_count == 0), "holder issuer presence");
    assert(!holder_required | !holder_issuers_present | (holder_issuer_count > 0), "empty holder issuer list");
    for i in 0..4 {
        if i < holder_issuer_count {
            assert(holder_issuers[i] != 0, "zero holder issuer");
            if i > 0 { assert(crate::term::field_lt(holder_issuers[i-1], holder_issuers[i]), "holder issuer order"); }
        } else { assert(holder_issuers[i] == 0, "holder issuer padding"); }
    }
    assert(status_required | (!allow_missing_status & (snapshot_root == 0) & (snapshot_depth == 0)), "inactive status policy");
    assert(validity_required | (now == 0), "inactive validity policy");
    assert(freshness_required | (earliest_created == 0), "inactive freshness policy");
    [issuer_root, issuer_depth as Field, holder_required as Field, holder_issuers_present as Field,
        holder_issuer_count as Field, holder_issuers[0], holder_issuers[1], holder_issuers[2], holder_issuers[3],
        status_required as Field, allow_missing_status as Field, snapshot_root, snapshot_depth as Field,
        validity_required as Field, now as Field, freshness_required as Field, earliest_created as Field]
}
#[test]
fn envelope_word_order_and_wide_timestamps() {
    assert(envelope_words(9, 4, true, true, 2, [3,5,0,0], true, true, 7, 3, true,
        0x100000000, true, 0x100000001) == [9,4,1,1,2,3,5,0,0,1,1,7,3,1,0x100000000,1,0x100000001]);
}
#[test]
fn envelope_inactive_values_are_zero() {
    assert(envelope_words(9,4,false,false,0,[0;4],false,false,0,0,false,0,false,0)
        == [9,4,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0]);
}
#[test(should_fail_with = "inactive holder policy")]
fn envelope_inactive_holder() { let _ = envelope_words(9,4,false,true,1,[3,0,0,0],false,false,0,0,false,0,false,0); }
#[test(should_fail_with = "holder issuer presence")]
fn envelope_issuer_presence() { let _ = envelope_words(9,4,true,false,1,[3,0,0,0],false,false,0,0,false,0,false,0); }
#[test(should_fail_with = "holder issuer capacity")]
fn envelope_issuer_capacity() { let _ = envelope_words(9,4,true,true,5,[3,5,7,9],false,false,0,0,false,0,false,0); }
#[test(should_fail_with = "zero holder issuer")]
fn envelope_zero_issuer() { let _ = envelope_words(9,4,true,true,1,[0;4],false,false,0,0,false,0,false,0); }
#[test(should_fail_with = "holder issuer padding")]
fn envelope_issuer_padding() { let _ = envelope_words(9,4,true,true,1,[3,5,0,0],false,false,0,0,false,0,false,0); }
#[test(should_fail_with = "holder issuer order")]
fn envelope_issuer_order() { let _ = envelope_words(9,4,true,true,2,[5,3,0,0],false,false,0,0,false,0,false,0); }
#[test(should_fail_with = "holder issuer order")]
fn envelope_issuer_unique() { let _ = envelope_words(9,4,true,true,2,[3,3,0,0],false,false,0,0,false,0,false,0); }
#[test(should_fail_with = "inactive status policy")]
fn envelope_inactive_status() { let _ = envelope_words(9,4,false,false,0,[0;4],false,true,0,0,false,0,false,0); }
#[test(should_fail_with = "inactive status policy")]
fn envelope_inactive_snapshot_root() { let _ = envelope_words(9,4,false,false,0,[0;4],false,false,7,0,false,0,false,0); }
#[test(should_fail_with = "inactive status policy")]
fn envelope_inactive_snapshot_depth() { let _ = envelope_words(9,4,false,false,0,[0;4],false,false,0,3,false,0,false,0); }
#[test(should_fail_with = "inactive validity policy")]
fn envelope_inactive_time() { let _ = envelope_words(9,4,false,false,0,[0;4],false,false,0,0,false,1,false,0); }
#[test(should_fail_with = "inactive freshness policy")]
fn envelope_inactive_freshness() { let _ = envelope_words(9,4,false,false,0,[0;4],false,false,0,0,false,0,false,1); }
fn issuer_only_envelope(root: Field) -> [Field;17] {
    envelope_words(root,4,false,false,0,[0;4],false,false,0,0,false,0,false,0)
}

pub fn bind_select_authenticated<let C: u32, let P: u32, let I: u32, let R: u32, let V: u32>(
    envelope: [Field;17], credential_quad_bounds: [u32; C], request_digest: [u8; 32], input_commitment: [u8; 32], signature_mode: u8,
    bounds: [u32; 3], parameters: [TermCell; P], indices: [u32; I],
    result: Table<R, V>, statement_binding: Field,
) {
    let mut state = prefix(request_digest, input_commitment, signature_mode, bounds, parameters, indices);
    for row in result.rows { for cell in row { state = absorb_cell(state, cell); } }
    state = h2(state, result.len as Field);
    let mut state = state;
    for word in envelope { state = h2(state, word); }
    for bound in credential_quad_bounds { state = h2(state, bound as Field); }
    assert(state == statement_binding, "authenticated statement binding");
}

pub fn bind_ask_authenticated<let C: u32, let P: u32, let I: u32>(
    envelope: [Field;17], credential_quad_bounds: [u32; C], request_digest: [u8; 32], input_commitment: [u8; 32], signature_mode: u8,
    bounds: [u32; 3], parameters: [TermCell; P], indices: [u32; I],
    result: bool, statement_binding: Field,
) {
    let state = h2(prefix(request_digest, input_commitment, signature_mode, bounds, parameters, indices), result as Field);
    let mut state = state;
    for word in envelope { state = h2(state, word); }
    for bound in credential_quad_bounds { state = h2(state, bound as Field); }
    assert(state == statement_binding, "authenticated statement binding");
}

#[test]
fn suite_dataset_empty_vector_matches_the_host() {
    check_suite_dataset([0],0,[0;32],[221,90,92,201,14,84,121,10,133,160,143,37,218,1,88,150,180,168,181,120,184,67,40,113,173,128,217,66,12,56,36,177]);
}
#[test(should_fail_with = "message padding")]
fn suite_dataset_message_padding_is_canonical() {
    check_suite_dataset([1],0,[0;32],[221,90,92,201,14,84,121,10,133,160,143,37,218,1,88,150,180,168,181,120,184,67,40,113,173,128,217,66,12,56,36,177]);
}
#[test(should_fail_with = "suite dataset commitment")]
fn suite_dataset_commitment_is_bound() { check_suite_dataset([0],0,[0;32],[0;32]); }
#[test]
fn authenticated_ask_vector_matches_the_host() {
    bind_ask_authenticated(issuer_only_envelope(9),[4],[0;32],[0;32],1,[1,4,2],[crate::table::unbound()],[0],true,
        0x24f30c25df1c0fc5b5f48ffa5334cf17f017543a3de15e9efd687707e13ec18d);
}
#[test(should_fail_with = "authenticated statement binding")]
fn authenticated_ask_binds_the_issuer_root() {
    bind_ask_authenticated(issuer_only_envelope(10),[4],[0;32],[0;32],1,[1,4,2],[crate::table::unbound()],[0],true,
        0x24f30c25df1c0fc5b5f48ffa5334cf17f017543a3de15e9efd687707e13ec18d);
}
#[test(should_fail_with = "authenticated statement binding")]
fn authenticated_ask_binds_each_quad_limit() {
    bind_ask_authenticated(issuer_only_envelope(9),[3],[0;32],[0;32],1,[1,4,2],[crate::table::unbound()],[0],true,
        0x24f30c25df1c0fc5b5f48ffa5334cf17f017543a3de15e9efd687707e13ec18d);
}

#[test]
fn authenticated_select_vector_matches_the_host() {
    bind_select_authenticated(issuer_only_envelope(9),[4],[0;32],[0;32],1,[1,4,2],[crate::table::unbound()],[0],crate::table::empty::<2,1>(),
        0x2838f9a4ff1acbd2604d8b50f2ddd4e8073784c67a8c56d8a7368d13a0239b5d);
}
#[test(should_fail_with = "authenticated statement binding")]
fn authenticated_select_binds_the_issuer_root() {
    bind_select_authenticated(issuer_only_envelope(10),[4],[0;32],[0;32],1,[1,4,2],[crate::table::unbound()],[0],crate::table::empty::<2,1>(),
        0x2838f9a4ff1acbd2604d8b50f2ddd4e8073784c67a8c56d8a7368d13a0239b5d);
}
#[test(should_fail_with = "authenticated statement binding")]
fn authenticated_select_binds_each_quad_limit() {
    bind_select_authenticated(issuer_only_envelope(9),[3],[0;32],[0;32],1,[1,4,2],[crate::table::unbound()],[0],crate::table::empty::<2,1>(),
        0x2838f9a4ff1acbd2604d8b50f2ddd4e8073784c67a8c56d8a7368d13a0239b5d);
}

#[test]
fn absent_holder_scope_selects_all_trusted_issuers() {
    let all=envelope_words(9,4,true,false,0,[0;4],false,false,0,0,false,0,false,0);
    assert(all[2] == 1);assert(all[3] == 0);
}
#[test(should_fail_with="empty holder issuer list")]
fn explicit_empty_holder_scope_is_refused() {
    let _none=envelope_words(9,4,true,true,0,[0;4],false,false,0,0,false,0,false,0);
}
