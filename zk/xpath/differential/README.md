<!-- [SONNET-4.6] sq-3x7dl.14.2: internal-stub README for a publish=false standalone harness. -->

# sparq-xpath-differential

PROOF **M1** (`sq-3x7dl.14.2`): differential harness for the `noir_XPath` circuit
primitives, using **sparq's own trusted Rust SPARQL/XSD scalar evaluator** as the oracle.

The `noir_XPath` source is **not in this repo** — it was externalized to the
[`sparq-org/noir_XPath`](https://github.com/sparq-org/noir_XPath) face repo
(`sq-5reoy` / #1599). This harness is the **oracle half**: it lives where the trusted Rust
evaluator lives, generates a Noir test file pinning that evaluator's answers, and
`scripts/run_differential_harness.sh` compiles that file against the pinned released face
repo.

## What it checks

For every sampled input, the Noir function output **must equal** the Rust XSD-evaluator
output — over a unicode-aware corpus that deliberately re-covers the edges the W3C qt3
corpus lacks:

| Primitive | Edge coverage |
| --- | --- |
| `fn:string-length` (STRLEN) | multibyte (2/3/4-byte), combining marks, astral, NUL-padded buffers |
| `fn:starts-with` / `ends-with` / `contains` | multibyte needles, empty needle, NUL-padded needle *and* haystack |
| `fn:substring` (SUBSTR) | `start < 1` window, zero length, length past the end, start past the end |
| `fn:substring` over multibyte | 2/3/4-byte codepoints, combining mark, astral, NUL-padded multibyte buffer — via the `sq-hjvte` codepoint→byte boundary conversion |
| `op:numeric-divide` | **non-exact** quotients (`1 div 3`, `100 div 7`) and sign combinations, plus the `idiv` de-aliasing control |
| mixed `xs:integer` ↔ `xs:double` compare | every integer **outside `[-128, 127]`**, including `2^53 + 1` |
| `xs:integer(xs:double)` | truncation toward zero, negatives, `2^53` |
| `fn:round` | ties toward `+∞`, negatives |
| `xs:dateTime` | **pre-1970** (negative epoch) component extraction + epoch-straddling ordering |

Those are exactly the cases the correctness beads `sq-3x7dl.4`/`.5`/`.6`/`.7` fixed, so the
harness doubles as their **regression oracle**.

## Two cross-checks, and what happens when they fire

No expected value is hand-written; each is read back from a real `BIND(<expr> AS ?out)`.
Each answer is then cross-checked, and a mismatch **aborts generation**:

- **IEEE** — every `xs:double` answer is recomputed with native Rust `f64` and compared
  bit-for-bit, so a lossy serialization can never pin a wrong bit pattern.
- **F&O** — every `fn:substring` answer is recomputed against an explicit XPath F&O 3.1
  §5.4.3 `fn:substring` window reference.

That F&O window reference is itself only as good as the person who wrote it, and it lives
in this repository next to the oracle it checks — two in-repo references agreeing is weaker
evidence than it looks. So a **third, out-of-repo** reference gates it in the harness's own
unit tests: **CPython string slicing**. Python's `str` is a sequence of CODEPOINTS, so
`s[start-1 : start+length-1]` (both ends floored at 0) is an independent statement of the
same window, and every substring case the generator emits — ASCII and multibyte — is
checked against it (`fo_substring_agrees_with_cpython_slicing`). A disagreement **fails**.
If `python3` is not installed the check reports loudly that it verified nothing and skips,
rather than passing quietly.

Where sparq's evaluator is itself wrong against the spec that `noir_XPath` implements, the
case is **not** dropped or downgraded: it stays a **live assertion**, but against the **F&O
spec value** rather than the oracle's, and is labelled **SPEC-REFERENCE** both at the row and
in the generated file's header. Read such a row as `noir_XPath == XPath F&O`, not as
`noir_XPath == sparq`. Keeping it live is the point: these are edges `noir_XPath` has already
*fixed*, so a regression on one must fail the run — a commented-out assertion cannot fail
and would verify nothing. Two divergences are recorded today — `SUBSTR` with `start < 1`
(the engine shifts the window instead of keeping it) and `ROUND` losing the sign of a
negative zero.

Three unit tests hold that arrangement in place: one asserts no assertion is ever emitted
commented out and that `substring("12345", 0, 3)` and `round_double(-0.5)` in particular
reach the circuit live; two assert each divergence still reproduces, so the special-casing
**expires** (goes red) the day the engine is fixed.

## Non-vacuity — proved per test function, not once per file

A green `nargo test` only means something if the generated file can actually go red, so the
driver script re-emits it with a deliberately wrong expected value and requires that run to
**fail**. It does that **once per generated `#[test]`**: `--list-fault-sites` names the test
functions, and each gets its own `--inject-fault-in NAME` variant and its own `nargo test`,
every one of which must fail.

The per-variant loop, not the fault count, is what carries the claim. A `nargo test` run
reports **one** exit status for the whole file, so one run over a file with every test
corrupted proves only that *some* test failed — nine sections that had drifted into something
incapable of failing (a corpus that emptied, an expected value derived from the very call
it asserts) would hide behind the one section that still works. A single-fault variant is
byte-identical to the oracle file that just ran green except for one value inside one test
function, so its failure is attributable to that function; running one per name discharges
the obligation for each. The cost is one `nargo` invocation per test function.

Generator unit tests keep the loop honest: every emitted test function must register a
fault site (generation panics otherwise); that site must be a **live** assertion, since a
fault hidden in a comment cannot fail `nargo test`; `--inject-fault-in` must change exactly
one assertion and only inside the named function; and an unknown name is a hard error, never
a silently uncorrupted file. The script itself refuses to run if the site list is empty or
does not cover every test function in the oracle file.

## TCB — stated honestly

This is **VERIFICATION, not proof**. Three things are trusted and unproven:

1. **The sparq Rust XSD evaluator.** It is the repo's reference semantics, *not* an audited
   or proven-correct implementation. Two live divergences from XPath F&O are already
   recorded above; there may be more that the corpus does not reach.
2. **The SAMPLE.** Coverage is hand-picked edge cases, not exhaustive. A wrong answer on an
   unsampled input is not caught. (Exhaustive coverage is milestone **M2**, and only for
   unary binary32 ops.)
3. **The Noir → ACIR → Barretenberg lowering.** `nargo test` exercises witness generation
   only. Nothing below ACIR is covered by any tier of this program.

**Known scope limit — `fn:substring` position units (`sq-hjvte`).** `noir_XPath`'s
`substring` indexes **byte** positions in the logical content (its own documented caveat)
while SPARQL `SUBSTR` / `fn:substring` index **codepoints** — and `string_length` counts
codepoints too since `sq-3x7dl.6`, so `substring(s, i, string_length(s))` is **wrong** for
multibyte `s`. The two units coincide on single-byte content and only there. Both regimes
are now sampled, in two separate generated test functions:

- `differential_oracle_substring` — **ASCII**, positions passed to the circuit
  **verbatim**. Only here does the primitive receive an un-normalized window, so this is
  what holds it to the F&O window arithmetic itself (the `start < 1` length-budget rule).
- `differential_oracle_substring_multibyte` — **multibyte**, with each **codepoint** window
  converted to the equivalent **byte** window by the generator before the call
  (`codepoint_window_to_byte_window`). The expected value is still the codepoint answer, so
  these rows hold the byte-positional primitive to `fn:substring` on content where the two
  units disagree. That catches a byte window landing mid-codepoint, a truncated
  continuation byte, and a logical-length (NUL) scan that misreads a `>= 0x80` byte.

The conversion is pinned to `fo_substring` by an exhaustive small-input sweep, and is
mutation-checked (corrupting it reds
`codepoint_window_to_byte_window_reproduces_the_fo_window`; corrupting the F&O reference
reds the CPython cross-check).

**What this does NOT claim.** The conversion runs on the **host**, which needs the string's
bytes. It does **not** make the in-circuit primitive codepoint-positional: for a **private
(witness)** string the conversion has to happen in-circuit, and that remains open as
`sq-hjvte` against the `sparq-org/noir_XPath` face repo. Until it lands, a caller composing
`substring` with `string_length` on a private multibyte string is still wrong. The sibling
position-unit question for `index_of` / `substring_before` / `substring_after` is
`sq-3x7dl.12` and is not sampled here at all.

**Known scope limit — scalars only.** Every corpus entry is a SCALAR (string, numeric,
`xs:dateTime`); the harness samples no sequence-valued or aggregate function, so it carries
no evidence either way about `fn:avg` / `fn:sum` / `fn:min` / `fn:max` on any input. The
one conformance question raised there has been **decided and recorded**, not implemented:
`fn:avg(())` returns the empty sequence per XPath F&O §15.4.4, while `noir_XPath` rejects
fail-closed — the decision is to KEEP the refusal (Noir has no empty-sequence inhabitant, and
refusing can only make a witness unsatisfiable whereas coercing to an in-band `0` would make
a false proposition witnessable). See the `fn:avg(())` bullet in
[`skills/zk-query-proofs/SKILL.md`](../../../skills/zk-query-proofs/SKILL.md) for the full
rationale, the reachability argument, and the condition under which it expires (`sq-jxh15`).

A green run makes **no soundness or privacy claim**. The ZK estate remains research-grade
and **NOT externally audited** (`sq-qhy4`).

## Running it

```sh
bash zk/xpath/scripts/run_differential_harness.sh                # cargo + nargo, full run
bash zk/xpath/scripts/run_differential_harness.sh --generate-only  # cargo only (drift guard)
bash zk/xpath/scripts/run_differential_harness.sh --update-committed  # refresh the golden
```

The committed golden is `zk/xpath/tests/differential_oracle/src/lib.nr`; CI diffs the
generator's output against it, so a corpus or oracle change lands as a reviewable diff.

Point the run at a different `noir_XPath` with `XPATH_GIT` / `XPATH_TAG` / `XPATH_DIRECTORY`,
or at a local checkout with `XPATH_PATH`. Both path forms name the library PACKAGE, not the
repo root: the face repo's root `Nargo.toml` is a `[workspace]`, so the git dep carries
`directory = "xpath"` (default) and `XPATH_PATH` wants `<checkout>/xpath`.

**License:** MIT
