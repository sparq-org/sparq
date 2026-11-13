// Temporary lane-D contract. Lane A replaces this file with term.nr.
// Research grade; not externally audited. No credential encoding is defined here.
use crate::expr_core::Scalar;

pub struct TermCell {
    pub bound: bool,
    pub kind: u8,
    pub enc: Field,
    pub key_class: u8,
    pub key: Field,
    pub dt: Field,
    pub lang: Field,
}

// There is deliberately no implementation for a bare TermCell. The implementation
// supplied by the term module must open enc, validate the datatype/lexical form,
// recompute key_class/key, validate UTF-8 and padding, and derive ALL Scalar
// fields from that opening. IEEE bits come from the lexical opening, retaining
// signed zero even if a comparison key normalizes it. Blank
// node enc must include the credential scope. This trait is a compile-time seam,
// never a witness-supplied `key_checked` flag.
pub trait CheckedOpening<let L: u32> {
    fn open(self) -> Scalar<L>;
}
