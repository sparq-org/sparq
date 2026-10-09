use std::cell::RefCell;
use std::rc::Rc;

/// Hard cap on memo entries; on overflow the memo is CLEARED, so a pathological
/// dynamic-pattern workload degrades to the old compile-per-row behaviour,
/// never to unbounded memory. Lookup is a linear scan — with ≤ CAP entries and
/// the typical one-or-two distinct patterns per query, that is an
/// allocation-free string compare, cheaper than hashing an owned key.
const CAP: usize = 64;

thread_local! {
    #[allow(clippy::type_complexity)]
    static MEMO: RefCell<Vec<((String, String), Option<Rc<regex::Regex>>)>> =
        const { RefCell::new(Vec::new()) };
}

// Compiles actually performed on this thread (memo misses) — the compile-once
// counter tests assert on deltas of this.
#[cfg(test)]
thread_local! {
    static COMPILES: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
pub(super) fn compile_count() -> u64 {
    COMPILES.with(std::cell::Cell::get)
}

/// The compiled regex for `(pattern, flags)`, or `None` for an invalid
/// pattern/flag — both memoised.
pub(super) fn get(pattern: &str, flags: &str) -> Option<Rc<regex::Regex>> {
    MEMO.with(|m| {
        let mut m = m.borrow_mut();
        if let Some((_, re)) = m.iter().find(|((p, f), _)| p == pattern && f == flags) {
            return re.clone();
        }
        #[cfg(test)]
        COMPILES.with(|c| c.set(c.get() + 1));
        let re = super::build_regex(pattern, flags).map(Rc::new);
        if m.len() >= CAP {
            m.clear();
        }
        m.push(((pattern.to_string(), flags.to_string()), re.clone()));
        re
    })
}
