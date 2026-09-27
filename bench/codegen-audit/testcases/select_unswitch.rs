//! Minimized testcase for the `DataChunk::select_decoded` codegen question
//! (sq-98w7z.5 / #3079): a loop-INVARIANT 5-way enum match inside a scan loop.
//!
//! `select_a` is the exact shape of `chunk.rs::select_decoded` (match inside the
//! loop). `select_b` is the manually unswitched shape (match outside, five copies
//! of the loop). `count_a` drops the conditional push to isolate whether the
//! compare itself vectorizes once the push is gone.
//!
//! Compile: rustc -Copt-level=3 -Ccodegen-units=1 --emit asm --crate-type lib
#![crate_type = "lib"]

#[derive(Clone, Copy)]
pub enum Cmp {
    Gt(f64),
    Ge(f64),
    Lt(f64),
    Le(f64),
    Eq(f64),
}

impl Cmp {
    #[inline]
    pub fn test(self, x: f64) -> bool {
        match self {
            Cmp::Gt(t) => x > t,
            Cmp::Ge(t) => x >= t,
            Cmp::Lt(t) => x < t,
            Cmp::Le(t) => x <= t,
            Cmp::Eq(t) => x == t,
        }
    }
}

/// Shape A — verbatim `select_decoded`: invariant match dispatched per iteration?
#[no_mangle]
pub fn select_a(decoded: &[f64], cmp: Cmp) -> Vec<usize> {
    let mut sel = Vec::with_capacity(decoded.len());
    for (r, &x) in decoded.iter().enumerate() {
        if cmp.test(x) {
            sel.push(r);
        }
    }
    sel
}

/// Shape B — manual unswitch: five specialized scan loops.
#[no_mangle]
pub fn select_b(decoded: &[f64], cmp: Cmp) -> Vec<usize> {
    let mut sel = Vec::with_capacity(decoded.len());
    macro_rules! scan {
        ($op:tt, $t:expr) => {
            for (r, &x) in decoded.iter().enumerate() {
                if x $op $t {
                    sel.push(r);
                }
            }
        };
    }
    match cmp {
        Cmp::Gt(t) => scan!(>, t),
        Cmp::Ge(t) => scan!(>=, t),
        Cmp::Lt(t) => scan!(<, t),
        Cmp::Le(t) => scan!(<=, t),
        Cmp::Eq(t) => scan!(==, t),
    }
    sel
}

/// Control — no conditional push: does the bare compare reduce vectorize?
#[no_mangle]
pub fn count_a(decoded: &[f64], cmp: Cmp) -> usize {
    decoded.iter().filter(|&&x| cmp.test(x)).count()
}
