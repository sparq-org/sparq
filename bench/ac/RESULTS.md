# Access-controlled query benchmark — SF=1 provisional results

<!-- [SONNET-4.6] sq-i6du2.9 / #2834. -->

This is the provisional sparq evaluation from the shared work box on 2026-07-19.
The command was `bash bench/ac/run.sh --sf 1` with seed 42 and four live-reader
threads. It exited 0. The oracle driver completed before any result below was
recorded, and the live driver also completed its fail-closed authorization checks.
Canonical wall-clock measurements remain gated on the quiet EC2 runner (#1364).

Disk was checked before and after the run. The workspace had 86 GB available at
both checks. `/tmp` had 1.0 GB available before and 879 MB after; temporary metric
extraction used `/tmp`, while the harness used its git-ignored Cargo targets.

## Deterministic results

These counts are fixed by the SF=1, seed-42 generator output and are independent
of work-box load. "Resources" counts distinct subjects in the generated data
graph. Policy triples are the sum of the compiled policy records for that model.

| use case | resources | data triples | intents | decisions | queries |
|---|---:|---:|---:|---:|---:|
| personal | 43 | 125 | 37 | 525 | 4 |
| project management | 61 | 164 | 79 | 99 | 4 |
| financial | 10 | 28 | 14 | 12 | 3 |
| consortium | 36 | 66 | 42 | 117 | 4 |
| **total** | **150** | **383** | **172** | **753** | **15** |

### Oracle outcomes

| driver | passed lanes | failed lanes | skipped lanes | result |
|---|---:|---:|---:|---|
| standalone W1/W2-oracle/W3 | 21 | 0 | 29 | PASS |
| live W2/W4 | 16 | 0 | 8 | PASS |
| live anti-vacuity check | 1 | 0 | 0 | PASS |

Skipped rows are explicit unavailable workload/model combinations, not oracle
passes. They include absent W2/W3 fixtures, ODRL live evaluation requiring the
separate `odrl-bridge` feature, and the known consortium temporal-oracle
divergence.

> **Addendum (issue #4415).** The 8 skipped `live W2/W4` lanes above were the ODRL
> ones; `bench/ac/live` now enables `odrl-bridge` and runs them, so a re-run of this
> command records 24 passed / 0 skipped for that driver. The table is left as the
> record of the 2026-07-19 run it describes and was **not** re-measured. Every exercised lane had zero mismatches; the live lanes detected no
over-share. The emitted WAC under-share and ACP group-oracle notices are advisory
known divergences and did not weaken the fail-closed over-share check.

### Expressibility and policy expansion

Each cell is `native / expansion / approximation / unsupported` intent counts.

| use case | WAC | ACP | ODRL |
|---|---:|---:|---:|
| personal | 37 / 0 / 0 / 0 | 32 / 5 / 0 / 0 | 37 / 0 / 0 / 0 |
| project management | 77 / 0 / 0 / 2 | 63 / 16 / 0 / 0 | 79 / 0 / 0 / 0 |
| financial | 14 / 0 / 0 / 0 | 13 / 1 / 0 / 0 | 14 / 0 / 0 / 0 |
| consortium | 33 / 0 / 0 / 9 | 24 / 9 / 0 / 9 | 42 / 0 / 0 / 0 |
| **total** | **161 / 0 / 0 / 11** | **132 / 31 / 0 / 9** | **172 / 0 / 0 / 0** |

ACP group lowering produced a factor-4 expansion for all five personal, all 16
project-management, and all nine consortium expansion cells. The financial
expansion cell had factor 0 because the generated group had no resolved members.
The compiled graph sizes make the corresponding policy-size cost visible:

| use case | WAC policy triples | ACP policy triples | ODRL policy triples |
|---|---:|---:|---:|
| personal | 184 | 310 | 268 |
| project management | 316 | 609 | 522 |
| financial | 58 | 100 | 88 |
| consortium | 171 | 297 | 399 |

## Non-canonical work-box timing

Every number in this section is **non-canonical work-box indicative**. The box was
not certified quiet. Absolute values must not be used as a performance claim;
the ratios are preferred only as provisional within-run observations. The full
build-plus-run elapsed time was a non-canonical 59.940 seconds.

The table records the live driver's engine-internal microsecond totals. The ratio
is `(W4 time / W4 checks) / (W2 time / W2 checks)`, so it compares per-check cost
despite different lane sizes. All raw values and ratios are non-canonical.

| use case | model | W2 µs, non-canonical | W4 µs, non-canonical | W4/W2 per-check ratio, non-canonical |
|---|---|---:|---:|---:|
| personal | WAC | 2,070 | 966 | 0.681× |
| personal | ACP | 2,136 | 893 | 0.610× |
| project management | WAC | 515 | 941 | 0.580× |
| project management | ACP | 615 | 1,014 | 0.523× |
| financial | WAC | 103 | 407 | 0.988× |
| financial | ACP | 128 | 500 | 0.977× |
| consortium | WAC | 617 | 854 | 0.587× |
| consortium | ACP | 579 | 822 | 0.602× |

ODRL live timing was skipped because this run did not enable `odrl-bridge`; its
standalone by-construction oracle lanes did run and pass. Standalone oracle timing
was not summarized because its microsecond-scale samples include zero-valued
timer readings and are not useful work-box evidence.
