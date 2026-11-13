use envelope_core::envelope::{Credential, credential_dataset, TermCell};
use envelope_core::table::{assert_table, Table};
// Public: request digest only; the complete expected dataset is a private table.
fn main(request_digest: pub [u128; 2], c: Credential<4>, issuer: TermCell, expected: Table<9, 4>) {
    assert_table(
        credential_dataset::<4, 9>(c, issuer, request_digest),
        expected,
    );
}
