use std::cell::Cell;

thread_local! {
    static NOW: Cell<Option<i64>> = const { Cell::new(None) };
}

/// [`epoch_secs`] calls that found NO active instant and fell back to a fresh
/// clock sample (the pre-sq-98w7z.1 per-call behaviour). Under a correctly
/// scoped execution — including its rayon worker items — this never fires; the
/// parallel NOW-constancy test asserts a zero delta, which is what makes it a
/// probe for any expression-evaluation path missing the `worker_install`.
#[cfg(test)]
pub(crate) static UNSCOPED_FALLBACKS: std::sync::atomic::AtomicU64 =
    std::sync::atomic::AtomicU64::new(0);

/// Guard from [`scope`]: clears the slot on drop iff THIS guard installed it.
pub(crate) struct Guard(bool);
impl Drop for Guard {
    fn drop(&mut self) {
        if self.0 {
            NOW.with(|c| c.set(None));
        }
    }
}

/// Pins this execution's instant if none is active (outermost-wins).
pub(crate) fn scope() -> Guard {
    NOW.with(|c| {
        if c.get().is_some() {
            Guard(false)
        } else {
            c.set(Some(sample()));
            Guard(true)
        }
    })
}

/// The active execution's instant (unix seconds). An un-scoped call — no
/// execution in progress on this thread — falls back to a fresh sample.
pub(crate) fn epoch_secs() -> i64 {
    NOW.with(Cell::get).unwrap_or_else(|| {
        #[cfg(test)]
        UNSCOPED_FALLBACKS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        sample()
    })
}

fn sample() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

/// Snapshot for the rayon-parallel branches (`Copy`, free).
#[cfg_attr(not(feature = "parallel"), allow(dead_code))]
pub(crate) fn snapshot() -> Option<i64> {
    NOW.with(Cell::get)
}

/// Scoped re-install of a snapshot inside a rayon worker item; restores the
/// PREVIOUS value on drop (rayon runs some items on the installing thread).
pub(crate) struct WorkerGuard(Option<Option<i64>>);
impl Drop for WorkerGuard {
    fn drop(&mut self) {
        if let Some(prev) = self.0.take() {
            NOW.with(|c| c.set(prev));
        }
    }
}

#[cfg_attr(not(feature = "parallel"), allow(dead_code))]
pub(crate) fn worker_install(snap: Option<i64>) -> WorkerGuard {
    match snap {
        None => WorkerGuard(None),
        Some(s) => WorkerGuard(Some(NOW.with(|c| c.replace(Some(s))))),
    }
}
