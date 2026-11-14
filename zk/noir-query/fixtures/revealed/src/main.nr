// Research-grade, externally unaudited ABI fixture. Every credential witness
// is private. Configuration halves are verifier-agreed public inputs.
use core::suite_constants::{LEXICAL_SHORT};
use core::suite_entry::{TermOpening};
fn main(raw: [[TermOpening<LEXICAL_SHORT>; 4]; 1], salts: [Field; 1], salt: Field, count: u32, created_raw: TermOpening<LEXICAL_SHORT>, method_raw: TermOpening<LEXICAL_SHORT>,
    // Public configuration binds the verifier-agreed proof options.
    config: pub [u128; 2], pk: pub compose::issuer::Point) -> pub Field {
    // Sole public output: the salted issuer-signed message.
    let message = core::suite_metadata::open_credential_revealed(raw, salts, salt, count, config, created_raw, method_raw, pk, 0).message();
    println(message);
    message
}
