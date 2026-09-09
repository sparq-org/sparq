The test is an intentional **physical-footprint regression check**, and the metadata-only control proves it detects a cost that heap/row checks miss: **440 versus 248 bytes**, after all earlier behavioral checks pass. No current-target runtime defect is demonstrated.

Copilot is correct about the language limit: `repr(Rust)` does not promise fixed padding or alignment. The comment currently overstates that guarantee. Recommend only a scope/comment clarification (and descriptive failure messages if authorized), preserving the footprint check as an empirical budget across the supported pinned configurations. The expected size is computed from current field types, not hard-coded 248, and no field order/offset is tested.

A portable exhaustive-field/type check on the **actual** `Overlay` can reject extra metadata fields without a layout assumption, but cannot detect layout/attribute-only physical growth. It is a different contract; no alternative was implemented or tested. A mirrored nominal struct is not a portable equivalence proof.

Stable unit tests use pinned 1.97.1; Miri explicitly uses pinned nightly-2026-06-11 and can execute this test. One native control does not prove all-target portability, and no nightly/target matrix was run. Source and declaration remain frozen.

Primary references: [Rust representation](https://doc.rust-lang.org/reference/type-layout.html#the-rust-representation), [size_of](https://doc.rust-lang.org/std/mem/fn.size_of.html).
