<!-- [OPUS-4.8] sq-inzv: internal-stub README for a publish=false crate; full surface lives in skills/gpu-kernels/SKILL.md + research/gpu-verdict.md. -->
# sparq-gpu

Opt-in, **experimental** [wgpu](https://wgpu.rs) compute kernels for sparq's
hot-path primitive shapes — FILTER + count, hash-join probe, and GROUP BY
COUNT+SUM — over plain `ValueId`-shaped columns (`&[u32]` / `&[f64]`). It is a
roadmap-T24d **measurement prototype**, built to answer one question with numbers
instead of aspiration: where (if anywhere) does a GPU beat the CPU for sparq's hot
paths once the host→device transfer tax is charged honestly? The kernel API and the
exact-f64 bit-order trick live in [`skills/gpu-kernels/SKILL.md`](../../skills/gpu-kernels/SKILL.md);
the full measured tables, the verdict, and the re-open conditions live in
[`research/gpu-verdict.md`](../../research/gpu-verdict.md).

> **Internal crate — not published** to crates.io (`publish = false`).
> **Experimental and PARKED**: a measurement prototype, **not** a wired-in
> execution backend — nothing depends on it and `wgpu` never enters the wasm
> build. The measured ratios live in the verdict and on the benchmarks dashboard,
> never baked into docs. Re-open only per the conditions in the verdict.

How-to: [`skills/gpu-kernels/SKILL.md`](../../skills/gpu-kernels/SKILL.md).
Verdict: [`research/gpu-verdict.md`](../../research/gpu-verdict.md).
Threat model: **deferred** because nothing depends on this crate — [`research/gpu-threat-model-deferral.md`](../../research/gpu-threat-model-deferral.md) (sq-vrye) holds the exit trigger; `tests/deferral_premise.rs` fails when that premise breaks.
Contributing: [`AGENTS.md`](../../AGENTS.md).

## License

[MIT](../../LICENSE).
