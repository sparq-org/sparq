Confirmed: begin clears ACTIVE before rejection and leaves initialization unreserved; end releases ACTIVE before readout. Use a separate ownership AtomicBool: acquire by CAS before reset; keep existing ACTIVE boundaries; snapshot results before releasing ownership. Preserve the single-coordinator/quiescent-worker contract.

All current paths are single coordinator. concurrent-cold spawns and joins two readers inside one measure call; neither calls begin/end. Frozen data remain exact-old-head diagnostic evidence; no trigger for this ownership bug was found. Global allocator noise and prior limits remain.

A std-only actual-module rustc test is feasible. Keep clean calibrations separate from caught-panic/competing-begin state assertions, since panic/harness activity can allocate. The identical test against old source must compile and fail ACTIVE preservation. A serial active-contention test does not exercise the precise reset/readout race; review that ownership span explicitly. No existing detached-bench CI execution is claimed.

No edits, tests, builds, remeasurement or remote actions performed.
