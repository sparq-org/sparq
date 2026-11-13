// Research-grade, externally unaudited ABI fixture. Every credential witness
// is private. Configuration halves are verifier-agreed public inputs.
use core::suite_entry::{TermOpening, open_quads};
fn main(raw: [[TermOpening<32>; 4]; 1], salts: [Field; 1], salt: Field, count: u32,
    // Public configuration binds the verifier-agreed proof options.
    config: pub [u128; 2]) -> pub Field {
    // Sole public output: the salted issuer-signed message.
    let message = core::suite::open_credential_revealed(open_quads(raw, 0), salts, salt, count, config);
    println(message);
    message
}
