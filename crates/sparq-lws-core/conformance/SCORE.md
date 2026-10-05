<!-- [SONNET-4.6] sq-gg0qq.7: ported from jeswr/solid-server-rs@1e555b10 `conformance/SCORE.md`
     MINUS the committed score history — see "Why there are no scores in this file" below. -->

# Conformance score — how it is produced

**There are no scores in this file.** The Solid-CTH score for `sparq-lws-core` is *generated* by
`./run.sh` on every run and ratcheted against a machine-readable floor. Committed prose is not the
source of truth for it.

## Where the numbers live

| Artifact | Role | Committed? |
|---|---|---|
| `reports/report.ttl` | EARL report — the **authoritative** per-case verdict | no (gitignored) |
| `reports/report.html` | Human-readable harness report | no (gitignored) |
| `reports/server.log` | The booted server's log for the run | no (gitignored) |
| `run.sh` `CONFORMANCE RESULT:` line | The headline, re-derived from the EARL report | n/a (stdout) |
| `baseline.json` | The **pinned floor** the run is ratcheted against | yes |

`run.sh` counts `earl:outcome earl:passed|failed|untested|inapplicable` directly out of
`report.ttl`, so the headline can never drift from the report that produced it.

## Why there are no scores in this file

The upstream `SCORE.md` this was ported from carried a per-suite table, a per-test-case verdict list,
and a narrative history of which branch fixed what. All of it was **prose asserting a number**, which
is exactly the shape that goes stale silently: the repo house rule (`AGENTS.md`, `CLAUDE.md`) is that
generated figures do not get baked into markdown. The durable, non-numeric knowledge from that
document — what the suites cover, why each skip tag is set, why CORS is hand-rolled — was kept and
moved into [`README.md`](./README.md) and `config/test-subjects.ttl`, next to the thing it describes.

What replaced the table is stronger than the table was: a floor that **fails the run**.

## The ratchet

After a valid run, `run.sh` compares the generated score to `baseline.json`:

- `min_passed` — a **floor**. Passing fewer cases fails the run.
- `max_failed` — a **ceiling**. More failures fail the run.
- `expected_total` — the suite **size** the TestSubject claims after its skip tags. A mismatch is not
  a regression, it is a signal that the manifests or the skip tags moved; `run.sh` says so distinctly
  and asks for re-triage rather than a floor edit.

This is what gives the standing "keep the CTH green through any later change" rule teeth: a change
that silently drops conformance fails the harness instead of quietly rewriting a markdown table.

**Raise `min_passed` when a run beats it** — `run.sh` prints a reminder when the score exceeds the
floor. **Never lower it to go green.** A genuine, understood scope reduction (a newly-justified skip
tag, say) is a deliberate edit to `baseline.json` *and* `config/test-subjects.ttl` in the same change,
with the reason in the commit message.

## Reproducing a run

```sh
cargo build --release -p sparq-lws-core
./crates/sparq-lws-core/conformance/run.sh
```

Prerequisites (Keycloak realm, `ath`-patched CTH image, Docker) are in
[`README.md`](./README.md#prerequisites). Set `CTH_ENFORCE_BASELINE=0` for an exploratory run that
reports the score without failing on a regression — never in CI.
