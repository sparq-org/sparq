// Research-grade, externally unaudited ABI fixture. Every credential witness
// is private. Configuration halves are verifier-agreed public inputs.
use core::suite_constants::{LEXICAL_SHORT};
use core::suite_entry::{TermOpening};
use compose::issuer::Point;
fn main(raw: [[TermOpening<LEXICAL_SHORT>; 4]; 1], salts: [Field; 1], salt: Field, count: u32, created_raw: TermOpening<LEXICAL_SHORT>, method_raw: TermOpening<LEXICAL_SHORT>,
    // Public issuer key identifies the verifier's accepted signing key.
    pk: pub Point,
    // Public configuration binds the verifier-agreed proof options.
    config: pub [u128; 2], r: Point, s: Field, e: Field, e_k: Field) {
    let _opened = core::suite_metadata::open_credential_hidden(raw, salts, salt, count, config, created_raw, method_raw, pk, 0, r, s, e, e_k);
}
