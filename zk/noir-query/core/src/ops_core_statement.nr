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
    let digest = sha256::sha256_var(bytes, 48 + credential_count * 32);
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
