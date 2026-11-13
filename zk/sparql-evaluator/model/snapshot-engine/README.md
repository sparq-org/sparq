# V1 numeric snapshot adapter

<!-- internal-stub -->

Private implementation detail of the detached evaluator, not a published API.
The build copies the current engine sources into Cargo's output directory and
applies the reviewed V1 numeric consumer patch with exact context matching.
The main engine, parser, deterministic guest collections and regex configuration
remain current. Source files outside this directory are read only.
Only `regex`, `digest` and `deterministic-blank-nodes` are exposed as features,
matching the model's dependency; defaults are empty. Other engine feature
branches are recognised by the compiler and remain disabled in this adapter.

The patch restores the pre-#7317 arithmetic/comparison representation boundary.
Widening that boundary belongs to a future dialect/version bump. The model's
boundary and typed-cause tests exercise the resulting evaluator directly.

License: MIT.
