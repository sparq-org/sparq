use std::cell::Cell;

thread_local! {
    static CURRENT: Cell<Option<u64>> = const { Cell::new(None) };
}

/// Sets the current group member's multiplicity for the duration of the
/// returned guard, restoring the previous value on drop (also on unwind).
pub(crate) struct Guard(Option<u64>);
impl Drop for Guard {
    fn drop(&mut self) {
        CURRENT.with(|c| c.set(self.0.take()));
    }
}

#[inline]
pub(crate) fn set(card: u64) -> Guard {
    let prev = CURRENT.with(|c| c.replace(Some(card)));
    Guard(prev)
}

/// The current member multiplicity, or `None` outside an aggregate argument.
#[inline]
pub(crate) fn current() -> Option<u64> {
    CURRENT.with(Cell::get)
}
