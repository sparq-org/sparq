// [GPT-6 Astra] Compile this actual std-only module directly with rustc --test.
#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Barrier};

    fn allocation_after_denial() -> (bool, (u64, u64, u64)) {
        let layout = Layout::from_size_align(128, 8).unwrap();
        let active = ACTIVE.load(Relaxed);
        // Panic payloads have already been dropped. Measure only this known request,
        // not the invalid window's totals, which can include panic/thread machinery.
        let before = (ALLOCS.load(Relaxed), REALLOCS.load(Relaxed), BYTES.load(Relaxed));
        // SAFETY: The nonzero layout is valid, and a successful System allocation is
        // released once with that same layout. No allocated byte is dereferenced.
        unsafe {
            let ptr = ALLOCATOR.alloc(layout);
            if ptr.is_null() {
                std::alloc::handle_alloc_error(layout);
            }
            ALLOCATOR.dealloc(ptr, layout);
        }
        (
            active,
            (
                ALLOCS.load(Relaxed) - before.0,
                REALLOCS.load(Relaxed) - before.1,
                BYTES.load(Relaxed) - before.2,
            ),
        )
    }

    #[test]
    fn window_ownership_and_calibration() {
        // One serial test owns the process-global allocator and panic hook. Hook
        // changes/output/assertions stay outside clean calibration windows.
        calibrate();
        calibrate();
        let old_hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(|_| {}));

        let baseline = begin();
        let nested_denied = std::panic::catch_unwind(begin).is_err();
        let nested_probe = allocation_after_denial();
        let _ = end(baseline);

        // Prepare the caller before the window; admit its attempt only after the
        // owner's begin returns. No timing threshold or probabilistic stress loop.
        let barrier = Arc::new(Barrier::new(2));
        let worker_barrier = Arc::clone(&barrier);
        let worker = std::thread::spawn(move || {
            worker_barrier.wait();
            worker_barrier.wait();
            std::panic::catch_unwind(begin).is_err()
        });
        barrier.wait();
        let baseline = begin();
        barrier.wait();
        let competing_denied = worker.join().unwrap();
        let competing_probe = allocation_after_denial();
        let _ = end(baseline);
        drop(barrier);

        std::panic::set_hook(old_hook);
        calibrate();
        calibrate();
        println!("calibration_windows=4 nested={:?} competing={:?}",
            (nested_denied, nested_probe), (competing_denied, competing_probe));
        assert_eq!((nested_denied, nested_probe), (true, (true, (1, 0, 128))));
        assert_eq!((competing_denied, competing_probe), (true, (true, (1, 0, 128))));
    }
}
