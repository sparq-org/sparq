// Test helper: the term encoding of a literal hash.
use sparq_zk_compose_core::hashes::{h2, TYPE_CODE_LITERAL};

fn main(hs: Field) -> pub Field {
    h2(TYPE_CODE_LITERAL, hs)
}
