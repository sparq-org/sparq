// Research-grade, externally unaudited ABI fixture. Every credential witness
// is private. Configuration halves are verifier-agreed public inputs.
use core::suite_entry::{TermOpening, open_quads};
use compose::issuer::Point;
fn main(raw: [[TermOpening<32>; 4]; 1], salts: [Field; 1], salt: Field, count: u32,
    // Public issuer key identifies the verifier's accepted signing key.
    pk: pub Point,
    // Public configuration binds the verifier-agreed proof options.
    config: pub [u128; 2], r: Point, s: Field, e: Field, e_k: Field) {
    core::suite::open_credential_hidden(open_quads(raw, 0), salts, salt, count, config, pk, r, s, e, e_k);
}
