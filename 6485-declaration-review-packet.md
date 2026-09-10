# PR 6486 V2 declaration supplement

Actual author: GPT-6 Astra xhigh. Source parent 73769114f081c8455aa21a393d0b992bd1283b67; metadata child 9ac4a59e96b1c69c8d617fbde9e98fe8c0436fed. Only the five-line declaration is added. Core source SHA256 e332be510f79a0374e3d930127229deeb77e3fa3523e3b9de9d2f5c4f4dfeade is unchanged. No source blockers were reported by the prior independent source review; this packet requests assessment of metadata and the reviewer precision correction.

## Delta

```diff
diff --git a/bench/feature-off-declarations/6486.json b/bench/feature-off-declarations/6486.json
new file mode 100644
index 000000000..34b9bbfe3
--- /dev/null
+++ b/bench/feature-off-declarations/6486.json
@@ -0,0 +1,5 @@
+{
+  "pr": 6486,
+  "date": "2026-09-10",
+  "reason": "[GPT-6 Astra] Intentional always-compiled core change: preserve the numeric precision memo when an update adds no stored dictionary terms. This is not a byte-neutrality assertion; the separate wasm size ratchet remains unchanged."
+}
```

## Validation and limits

{
  "head": "9ac4a59e96b1c69c8d617fbde9e98fe8c0436fed",
  "parent": "73769114f081c8455aa21a393d0b992bd1283b67",
  "actual_author": "GPT-6 Astra xhigh",
  "changed_paths": [
    "bench/feature-off-declarations/6486.json"
  ],
  "diff_stat": "bench/feature-off-declarations/6486.json | 5 +++++\n 1 file changed, 5 insertions(+)",
  "core_source_sha256": "e332be510f79a0374e3d930127229deeb77e3fa3523e3b9de9d2f5c4f4dfeade",
  "core_byte_identical_to_reviewed_parent": true,
  "clean": true,
  "declaration": {
    "pr": 6486,
    "date": "2026-09-10",
    "intent_only": true,
    "derived": false,
    "schema": "Documented {pr,date,reason}; checker detects new numeric filenames without parsing JSON fields."
  },
  "validation": [
    {
      "argv": [
        "python3",
        "-m",
        "json.tool",
        "bench/feature-off-declarations/6486.json"
      ],
      "exit": 0,
      "seconds": 0.24588900000000002,
      "log": "json-validation.log"
    },
    {
      "argv": [
        "python3",
        "scripts/check-vectorized-feature-off.py",
        "--self-test"
      ],
      "exit": 0,
      "seconds": 0.48563662499999993,
      "log": "self-test.log"
    },
    {
      "argv": [
        "git",
        "diff",
        "--check"
      ],
      "exit": 0,
      "seconds": 0.08985975000000002,
      "log": "diff-check.log"
    }
  ],
  "staged_diff_check": {
    "command": "git diff --cached --check",
    "exit": 0
  },
  "review_precision_correction": "has_high_precision_decimal checks self.numerics.lookup(id).is_some() before is_high_precision_decimal(&self.dict,id). The memo therefore depends on the coherent numeric cache and dictionary term pairing, not dictionary alone. Existing invariant comment is retained unchanged. Complete caller supplied; no optional wording change applied.",
  "limitations": [
    "No wasm build or byte comparison in this metadata task.",
    "No claim of byte neutrality, runtime speedup, size acceptance or final CI approval.",
    "Prior source behavioral tests were not repeated; production source is byte-identical."
  ],
  "pending_commands": 0,
  "remote_mutations": 0
}

## Actual self-test output

```text
[leg1] VIOLATION: sparq-engine node 'sparq-engine 0.1.0 (path+file:///repo/crates/sparq-engine)' has 'vectorized' in resolved features: ['vectorized']

[leg1] FAIL — 1 violation(s) found. The `vectorized` feature must NOT be in any resolved default feature set.
TRIPWIRE 1 (leg1): PASS — guard correctly detected 'vectorized' in synthetic fixture
[leg2] feature-OFF wasm DIFFERS from the base tree: base=10 head=11 bytes, size delta=+1 (+10.000%). Comparison is byte-for-byte, so same-length content changes are also detected.
[leg2] VIOLATION: the feature-OFF wasm bundle changed vs the base tree (size delta +1 bytes, +10.000%) but the change is NOT declared (no new bench/feature-off-declarations/<PR>.json file added, and the legacy scalar change_token is unchanged at 0).
  - If this is ACCIDENTAL vectorized / default-path code leaking into the feature-OFF build, REMOVE it or gate it behind #[cfg(feature = "vectorized")].
  - If this is an INTENTIONAL change to always-compiled engine/core code, DECLARE it (mechanism V2): add bench/feature-off-declarations/<PR-number>.json with {pr, date, reason}. If the SIZE moves > +/-2%, ALSO raise metrics.wasm_bundle_bytes.floor in bench/perf-baseline.json (the bench.yml ratchet).

[leg2] FAIL — undeclared feature-OFF bundle change. Declare it or remove it.
TRIPWIRE 2 (leg2): PASS — dynamic check correctly rejected an UNDECLARED feature-OFF byte change (bytes differ, change_token un-bumped)
[leg2] feature-OFF wasm DIFFERS from the base tree: base=10 head=11 bytes, size delta=+1 (+10.000%). Comparison is byte-for-byte, so same-length content changes are also detected.
[leg2] OK — change DECLARED (legacy scalar, transition window): bench/feature-off-declaration.json change_token 0 -> 1. The feature-OFF byte change is intentional. Its SIZE is governed by the wasm_bundle_bytes floor ratchet (+/-2% band) in bench.yml, unchanged by this gate. NOTE: the scalar mechanism is retired — new PRs should add a per-PR declaration file under bench/feature-off-declarations/ (mechanism V2) instead.
TRIPWIRE 3 (leg2): PASS — dynamic check accepted a DECLARED change (bytes differ, change_token 0 -> 1)
[leg2] feature-OFF wasm DIFFERS from the base tree: base=10 head=11 bytes, size delta=+1 (+10.000%). Comparison is byte-for-byte, so same-length content changes are also detected.
[leg2] OK — change DECLARED (mechanism V2): the head tree added bench/feature-off-declarations/['1720.json'] not present in the base tree. The feature-OFF byte change is intentional. Its SIZE is governed by the wasm_bundle_bytes floor ratchet (+/-2% band) in bench.yml, unchanged by this gate.
TRIPWIRE 4 (leg2 V2): PASS — dynamic check accepted a DECLARED change via a new per-PR file (bench/feature-off-declarations/1720.json added by head)
[leg2] feature-OFF wasm DIFFERS from the base tree: base=10 head=11 bytes, size delta=+1 (+10.000%). Comparison is byte-for-byte, so same-length content changes are also detected.
[leg2] VIOLATION: the feature-OFF wasm bundle changed vs the base tree (size delta +1 bytes, +10.000%) but the change is NOT declared (no new bench/feature-off-declarations/<PR>.json file added, and the legacy scalar change_token is unchanged at 0).
  - If this is ACCIDENTAL vectorized / default-path code leaking into the feature-OFF build, REMOVE it or gate it behind #[cfg(feature = "vectorized")].
  - If this is an INTENTIONAL change to always-compiled engine/core code, DECLARE it (mechanism V2): add bench/feature-off-declarations/<PR-number>.json with {pr, date, reason}. If the SIZE moves > +/-2%, ALSO raise metrics.wasm_bundle_bytes.floor in bench/perf-baseline.json (the bench.yml ratchet).

[leg2] FAIL — undeclared feature-OFF bundle change. Declare it or remove it.
TRIPWIRE 5 (leg2 V2): PASS — no new per-PR file (only README on both sides) correctly rejected as UNDECLARED

[self-test] OK — all tripwires fired correctly. The guards can fail (and the dynamic leg2 accepts a declared change).
```

## Complete precision-memo caller

The cold path consults numerics and term data; the invariant must preserve both. No dict-only correction is applied.

```rust
    /// [OPUS-4.8] (sq-lr2ii) `true` if the graph holds any `xsd:decimal` literal with MORE
    /// than 15 significant digits — a value the f64 `numerics` cache CANNOT represent exactly,
    /// so the engine's f64-based sargable FILTER fast path could decide `=`/`<`/`>`/`<=`/`>=`
    /// WRONGLY for it (e.g. `"1.000000000000000001"^^xsd:decimal` shares the f64 `1.0` with the
    /// constant `1`, yet is not equal to it). The engine consults this at the sargable-decision
    /// point to DECLINE that fast path and fall back to the exact general evaluator; a graph
    /// without any such decimal keeps the fast path (a decimal of `<= 15` significant digits
    /// round-trips through f64 unambiguously, and a large-integer collision needs a `> 15`-digit
    /// CONSTANT, which the engine already declines). Integers/float/double are exempt: an
    /// integer's exactness is the constant-side guard's job and a float/double's value IS its f64.
    ///
    /// Memoised (see `high_precision_decimal`): the first call scans the graph's numeric terms
    /// once (short-circuiting on the first offender); later calls are an atomic load. A delta
    /// that appends terms resets the memo so it is recomputed over the grown dictionary. This is
    /// a CONSERVATIVE, graph-wide gate — one offending decimal declines the numeric fast path for
    /// every comparison on the graph — chosen for safety (correctness is never at risk; only the
    /// pushdown optimisation is skipped for affected graphs).
    pub fn has_high_precision_decimal(&self) -> bool {
        use std::sync::atomic::Ordering::Relaxed;
        match self.high_precision_decimal.load(Relaxed) {
            2 => true,
            1 => false,
            _ => {
                let found = (1..=self.dict.len() as Id)
                    .any(|id| self.numerics.lookup(id).is_some() && is_high_precision_decimal(&self.dict, id));
                self.high_precision_decimal.store(if found { 2 } else { 1 }, Relaxed);
                found
            }
        }
    }

```

## Full V2 acceptance helpers

```python
# scripts/check-vectorized-feature-off.py:158
def _read_change_token(decl_path: str) -> int:
    """Read the integer 'change_token' from a feature-off declaration JSON file (LEGACY).

    Retained for the TRANSITION WINDOW (sq-v3nel-v2): the scalar mechanism is retired but
    in-flight branches that already bumped the token must still be accepted. The V2
    per-PR-file mechanism (_new_declaration_files below) is the going-forward path.

    Fail-SAFE default: a missing file, missing field, or non-integer value reads as 0.
    Because an ABSENT declaration reads the same as an UN-bumped one, an undeclared byte
    change still fails the gate (the check compares base vs head token for INEQUALITY).
    """
    try:
        with open(decl_path) as fh:
            data = json.load(fh)
    except (OSError, ValueError):
        return 0
    tok = data.get("change_token", 0) if isinstance(data, dict) else 0
    try:
        return int(tok)
    except (TypeError, ValueError):
        return 0

_DECL_FILE_RE = re.compile(r'^\d+\.(?:json|md)$')

# scripts/check-vectorized-feature-off.py:187
def _list_declaration_files(dir_path: str | None) -> set[str]:
    """Return the set of per-PR declaration file names in a declarations directory.

    Fail-SAFE: a None / missing / non-directory path reads as the EMPTY set, so an absent
    directory can never be mistaken for a declaration (keeps the gate fail-closed).
    Only names matching <digits>.json|md are counted (README.md/.gitkeep are ignored).
    """
    if not dir_path or not os.path.isdir(dir_path):
        return set()
    return {name for name in os.listdir(dir_path) if _DECL_FILE_RE.match(name)}

# scripts/check-vectorized-feature-off.py:199
def _new_declaration_files(base_dir: str | None, head_dir: str | None) -> set[str]:
    """Return declaration files the HEAD tree has that the BASE tree does NOT (set diff).

    A non-empty result means the PR ADDED at least one bench/feature-off-declarations/
    <PR-number>.json — i.e. it DECLARED an intentional feature-OFF byte change under
    mechanism V2. Because each PR adds a distinctly-named file, this never collides with
    another declaring PR (unlike the old shared scalar token line).
    """
    return _list_declaration_files(head_dir) - _list_declaration_files(base_dir)

# scripts/check-vectorized-feature-off.py:210
def check_leg2_dynamic(base_wasm: str, head_wasm: str,
                       base_decl: str, head_decl: str,
                       base_decls_dir: str | None = None,
                       head_decls_dir: str | None = None) -> int:
    """DYNAMIC byte-identity gate for the feature-OFF wasm bundle (sq-v3nel / sq-v3nel-v2).

    Compares the feature-OFF wasm built from the BASE (target-branch / merge-base) tree
    against the one built from the HEAD (PR / merged) tree IN THE SAME CI RUN. Same
    toolchain + same runner in one run => a deterministic comparison with NO static byte
    pin, so NO merge-order dependence.

    Policy:
      * bytes byte-for-byte IDENTICAL  -> PASS. The PR touched no always-compiled code
        that reaches the feature-OFF bundle. (This is the common case for non-engine PRs.)
      * bytes DIFFER + change DECLARED -> PASS. 'declared' is satisfied by EITHER mechanism
        (transition window, sq-v3nel-v2):
          V2 (going-forward): the head-tree bench/feature-off-declarations/ directory
            contains a <PR-number>.json/.md file the base tree does NOT (set difference).
            Conflict-free: each PR adds its own file.
          LEGACY (retired, still accepted): change_token differs between the base-tree and
            head-tree bench/feature-off-declaration.json (a pre-V2 branch bumped it).
        The SIZE of the change is governed SEPARATELY, unchanged, by the wasm_bundle_bytes
        floor ratchet (+/-2% band) in bench.yml — this gate governs INTENT, not magnitude.
      * bytes DIFFER + NOT declared    -> FAIL. Either accidental default-path/vectorized
        code leaked into the feature-OFF build (remove / cfg-gate it) or an intentional
        always-compiled change was not declared (add a per-PR declaration file).

    Returns 0 on pass, 1 on fail/error.
    """
    try:
        with open(base_wasm, "rb") as fh:
            base_bytes = fh.read()
    except OSError as exc:
        print(f"[leg2] ERROR: cannot read base wasm {base_wasm!r}: {exc}", file=sys.stderr)
        return 1
    try:
        with open(head_wasm, "rb") as fh:
            head_bytes = fh.read()
    except OSError as exc:
        print(f"[leg2] ERROR: cannot read head wasm {head_wasm!r}: {exc}", file=sys.stderr)
        return 1

    base_len, head_len = len(base_bytes), len(head_bytes)

    if base_bytes == head_bytes:
        print(
            f"[leg2] OK — feature-OFF wasm is byte-for-byte IDENTICAL to the base tree "
            f"({head_len} bytes). No always-compiled code changed the default build; "
            "deterministic same-run comparison, no static pin, no merge-order dependence."
        )
        return 0

    # Bytes differ (size and/or content). Require an audit-visible declaration.
    delta = head_len - base_len
    pct = (delta / base_len * 100) if base_len else float("inf")

    print(
        f"[leg2] feature-OFF wasm DIFFERS from the base tree: "
        f"base={base_len} head={head_len} bytes, size delta={delta:+d} ({pct:+.3f}%). "
        "Comparison is byte-for-byte, so same-length content changes are also detected."
    )

    # V2 (going-forward): a per-PR declaration FILE was added under
    # bench/feature-off-declarations/ (set difference on the directory listing). Preferred
    # because each PR adds a distinct file, so declaring PRs never textually conflict.
    new_files = _new_declaration_files(base_decls_dir, head_decls_dir)
    if new_files:
        print(
            "[leg2] OK — change DECLARED (mechanism V2): the head tree added "
            f"bench/feature-off-declarations/{sorted(new_files)} not present in the base "
            "tree. The feature-OFF byte change is intentional. Its SIZE is governed by the "
            "wasm_bundle_bytes floor ratchet (+/-2% band) in bench.yml, unchanged by this gate."
        )
        return 0

    # LEGACY (retired, accepted during the transition window): the scalar change_token in
    # bench/feature-off-declaration.json differs between base and head tree. Kept live so
    # pre-V2 branches that already bumped the token do not break.
    base_tok = _read_change_token(base_decl)
    head_tok = _read_change_token(head_decl)
    if head_tok != base_tok:
        print(
            f"[leg2] OK — change DECLARED (legacy scalar, transition window): "
            f"bench/feature-off-declaration.json change_token {base_tok} -> {head_tok}. "
            "The feature-OFF byte change is intentional. Its SIZE is governed by the "
            "wasm_bundle_bytes floor ratchet (+/-2% band) in bench.yml, unchanged by this gate. "
            "NOTE: the scalar mechanism is retired — new PRs should add a per-PR declaration "
            "file under bench/feature-off-declarations/ (mechanism V2) instead."
        )
        return 0

    print(
        f"[leg2] VIOLATION: the feature-OFF wasm bundle changed vs the base tree "
        f"(size delta {delta:+d} bytes, {pct:+.3f}%) but the change is NOT declared "
        f"(no new bench/feature-off-declarations/<PR>.json file added, and the legacy scalar "
        f"change_token is unchanged at {base_tok}).\n"
        "  - If this is ACCIDENTAL vectorized / default-path code leaking into the "
        "feature-OFF build, REMOVE it or gate it behind #[cfg(feature = \"vectorized\")].\n"
        "  - If this is an INTENTIONAL change to always-compiled engine/core code, DECLARE "
        "it (mechanism V2): add bench/feature-off-declarations/<PR-number>.json with "
        "{pr, date, reason}. If the SIZE moves > +/-2%, ALSO raise "
        "metrics.wasm_bundle_bytes.floor in bench/perf-baseline.json (the bench.yml ratchet)."
    )
    print("\n[leg2] FAIL — undeclared feature-OFF bundle change. Declare it or remove it.")
    return 1
```

## Documented declaration procedure

<!-- [OPUS-4.8] sq-v3nel-v2 (2026-07-07): per-PR feature-OFF change declarations. -->

# feature-OFF wasm-bundle change declarations (mechanism V2)

The `vectorized-feature-off` gate (leg 2,
`.github/workflows/vectorized-feature-off.yml`) builds the feature-OFF `sparq-wasm`
bundle from the **base** tree and the **head** tree in the same CI run and compares
them **byte-for-byte**. Identical bytes pass automatically. If the bytes differ, the
change must be **declared** — otherwise the gate fails (the whole point: an accidental
`vectorized`/default-path leak into the feature-OFF build is caught).

## How to declare

If your PR *intentionally* changes always-compiled engine/core code in a way that moves
the feature-OFF bundle bytes, add **one file** named after your PR number:

```
bench/feature-off-declarations/<PR-number>.json
```

with the shape:

```json
{
  "pr": 1234,
  "date": "2026-07-07",
  "reason": "one line on what always-compiled change moves the feature-OFF bytes"
}
```

The gate is satisfied when the head tree's declarations directory contains at least one
`<digits>.json` (or `.md`) file the base tree's does **not** — a set difference on the
directory listing. Only names matching `<digits>.json|md` count as declarations, so this
`README.md` (and any `.gitkeep`) is ignored.

## Why per-PR files (V2)

The previous mechanism stored one scalar `change_token` in
`bench/feature-off-declaration.json`. Every declaring PR edited the **same line**, so the
first declared PR to merge made every other declared PR textually **CONFLICTING** in git
(`#1720` and `#1718` both went `DIRTY` after `#1726`'s declaration merged). The gate check
was order-independent but the file was not. Per-PR files remove the shared line: different
PRs add different files, so git never conflicts regardless of merge order.

The legacy scalar file `bench/feature-off-declaration.json` is **retired** (frozen, kept
parseable). During the transition window the gate still accepts a scalar-token inequality
so in-flight pre-V2 branches keep working; new PRs must use a per-PR file here instead.

## Scope

This declaration governs **intent** (did you mean to change the feature-OFF bytes at all?).
The **size** of an accepted change is governed separately, unchanged, by the
`metrics.wasm_bundle_bytes` floor ratchet (±2% band) in `bench/perf-baseline.json` /
`bench.yml`. A change moving the bundle > ±2% must also raise that floor.

## Why the gate fires on comment-only edits (measured)

<!-- [OPUS-5] sq-v3nel-v3 (2026-07-28): the dominant benign class, and the tool that derives it. -->

Leg 2 compares the bundles **byte-for-byte**, and a bundle byte can move without a single
instruction changing. The workspace `release` profile sets `panic = "abort"` but not
`panic_immediate_abort`, so every panicking call site still carries a `core::panic::Location`
record — and that record embeds the call site's **line number**. Insert a comment block, or an
off-by-default `#[cfg(feature = ...)]` item, anywhere **above** always-compiled code in the same
file, and every line below it moves; the line numbers baked into those records move with it.

This was measured directly on this repo (`cargo build --profile release-wasm -p sparq-wasm
--target wasm32-unknown-unknown`, the leg's own build), each row a single change against the
same tree:

| change                                                              | bundle verdict | size delta | differing bytes |
| ------------------------------------------------------------------- | -------------- | ---------- | --------------- |
| rebuild, no source change                                            | identical      | 0          | 0               |
| 34 pure comment lines inserted mid-file (`sparq-core/src/compress.rs`) | **differs**    | **0**      | **3**           |
| off-by-default `#[cfg]` module added *below* all compiled code       | identical      | 0          | 0               |
| comment block + off-by-default `#[cfg]` statement inserted mid-file  | **differs**    | **0**      | **2**           |
| four **ungated** code lines added to `Graph::build`                  | **differs**    | **+228**   | **373027**      |

The separation is not subtle: a change that adds no compiled code moves single-digit bytes and
leaves the size **exactly** unchanged, while real code entering the default build moves
hundreds of thousands of bytes and shifts the size.

**This is not a reason to loosen the gate.** The last row is precisely what the leg exists to
catch, and byte-for-byte is what catches it. It *is* a reason not to make a human hand-write a
prose declaration for the rows above it.

## Deriving a declaration instead of asserting one

`scripts/feature_off_autodeclare.py` decides that case from the compiler:

```
python3 scripts/feature_off_autodeclare.py --repo . \
    --base-sha <pr base sha> --head-sha <pr head sha> --pr <number> --write
```

It rebuilds the head tree with every **added** line replaced by an **empty** line — line count
preserved, so every line position is unchanged — and requires the result to be **byte-identical**
to the head bundle. If blanking the additions changes nothing the compiler emits, the additions
emitted nothing. When the diff also removes lines it runs the same proof on the base side. Only
then is a declaration written, carrying the measured byte counts.

Everything else **refuses**, with a named reason, and the leg stays red:

| refusal                          | meaning                                                       |
| -------------------------------- | ------------------------------------------------------------- |
| `added-lines-are-semantic`       | blanking the additions changed the bundle — real code was added |
| `deleted-lines-are-semantic`     | blanking the deletions in the base changed it — real code was removed |
| `neutral-build-failed`           | the additions were load-bearing (the tree stopped compiling)   |
| `proof-would-be-vacuous`         | nothing was blanked and nothing was deleted, so the comparison would be `head == head` |
| `unsupported-file-change`        | a symlink or gitlink, which blanking cannot speak for          |

### Why every changed path is blanked, not just the ones in the build closure

An earlier version scoped blanking to a closure derived from `cargo tree` ∩ `cargo metadata
--no-deps`, and that shipped a **live false pass**. `--no-deps` returns **workspace members
only**, while sparq's root manifest carries `exclude = ["vendor/spargebra", …]` *together
with* `[patch.crates-io] spargebra = { path = "vendor/spargebra" }` — so spargebra is
compiled into the feature-OFF bundle while not being a member. A change under `vendor/`
classified inert, was never blanked, and `neutral == head` therefore held **by
construction**: three genuinely non-benign changes there all came back `declared`.

The emitted declaration even confessed it — *"rebuilding the head tree with all **0** added
non-blank line(s) blanked produced a BYTE-IDENTICAL bundle"* — recording that nothing had
been proved, and declaring anyway.

Two fixes, and the second matters more than the first:

1. **Nothing is skipped.** Every changed non-manifest path is blanked, wherever it lives.
   Blanking a file the build never reads is free; blanking one it *does* read moves the
   bundle and the derivation refuses. The closure is now **reporting only** — no soundness
   claim rests on it — and its query no longer uses `--no-deps`.
2. **A proof that proves nothing must refuse.** If the neutral tree comes out identical to
   the head tree *and* the diff deletes no non-blank line, the comparison is `head == head`
   and holds regardless. That guard catches the whole class without anyone needing to know
   which path type was missed.

An **intentional always-compiled change** — a hot-path optimisation, a refactor of code the
default build ships — is refused on purpose. Its author is asserting *intent*, and intent cannot
be derived; write the declaration by hand, as `3679.json` and `3755.json` do.

Three deliberate limits:

* The tool **does not make the leg pass**. It writes a file that still has to be committed, so
  the escape hatch stays inside the reviewed diff. An auto-passing gate whose derivation had a
  hole would be exactly the rubber stamp this leg exists to prevent.
* Every obligation builds into its **own** target directory. Sharing one warm directory
  across the materialised trees was tried for speed and reverted: on #4350 it reported a
  299-line `crates/sparq-engine/src/exec.rs` rewrite as *byte-identical* to its base, because
  `git archive` stamps each tree's files with its commit time and cargo's freshness check is
  mtime-based. A false "identical" is the one outcome that would auto-declare a real code
  change, so cold builds are the price.
* It attributes against the **merge base**, not `pull_request.base.sha`. Those differ whenever a
  branch is behind its base, and leg 2 itself compares `base.sha` — so on such a PR the leg's
  reported difference also carries, in reverse, base-branch commits the PR does not have. The
  tool says so in its output rather than attributing them to the PR.

A census of the live class (how many open PRs are red on this leg, how many already carry a
declaration) is available without running any build:

```
python3 scripts/feature_off_autodeclare.py --census sparq-org/sparq
```
