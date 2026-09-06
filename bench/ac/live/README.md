<!-- internal-stub -->
# ac-bench-live

> `publish = false` — dev-only live driver for the AC-query benchmark (bead sq-kvvcl).

This is the **live** counterpart to `bench/ac/` (the oracle-only harness, bead sq-i6du2.7).
It links `sparq-solid` (with `odrl-bridge` ON) and runs the real WAC/ACP/ODRL engine.

## What it does

- **W2 live**: builds a `PodStore` from the generator's intent table, materializes the auth
  view (WAC/ACP statically, ODRL through `materialize_odrl_policy` on a FRESH store per
  request), and runs `query_as` for each expected decision. Hard fail-closed gates: any
  over-share (oracle=Deny but engine returns rows) exits non-zero immediately, and an ODRL
  lane that materializes zero allow-rows FAILS as vacuous.
- **W4 concurrent**: wraps the `PodStore` in `Arc` and spawns N threads that concurrently
  call `decide_batch` + `query_as` via the `&self` read-side, asserting consistency.
- **Anti-vacuity**: a `PodStore` with no policy must deny all queries.

## Honest scope boundary

Under-share (oracle=Allow, engine=Deny) is logged as advisory, not a failure. For WAC the
`sparq-acbench` oracle evaluates all matching intents while the engine uses "nearest ACL
document wins" (bead sq-kvvcl.2). For ODRL the driver deliberately supplies no world-state
evidence for group / subtree / temporal / purpose / count intents, because deriving it
would test the oracle against itself — see the translation contract at the top of
`src/main.rs` (issue #4415) for why each rewrite is faithful or narrowing. The
security-critical check (over-share) is fully fail-closed for all three models.

## Running

```
cd bench/ac && bash run.sh --smoke
```

Or invoke directly: `cargo run -- --smoke` (from `bench/ac/live/`).

## License

MIT — see the workspace root `LICENSE` file.
