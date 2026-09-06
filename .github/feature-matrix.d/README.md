<!-- [SONNET-4.6] sq-19f1i follow-up (issue #2384): the TWO-FILE scope contract for
     feature-matrix leg work. Written here, in the fragment directory itself, because
     this is the directory a bead spec names when it scopes a leg change. -->

# `.github/feature-matrix.d/` — opt-in feature-matrix leg fragments

One YAML fragment per crate. `scripts/assemble-feature-matrix.py` reads them all (in
sorted-filename order) and emits the `opt-in-features` strategy matrix consumed by
`.github/workflows/feature-matrix.yml`. Each leg's CI check-run name is
`opt-in <name>`, and the `ci-summary / gate` aggregator discovers those names as
REQUIRED checks — so the emitted name set is gate-critical.

## THE TWO-FILE SCOPE RULE

The rule keys off the emitted **leg-name set**, nothing else.

**Changing the leg-name set — adding, removing or renaming a leg — is a TWO-FILE
change.** A bead / issue / PR that does so must have BOTH of these in its permitted
file scope:

1. `.github/feature-matrix.d/<crate>.yml` — the leg itself; and
2. `scripts/tests/feature-matrix-legnames.golden.txt` — the gate-name golden.

Every **other** fragment edit leaves the name set untouched and stays a SINGLE-file
change — see *When the golden does NOT change* below. Do not scope the golden into a
task that cannot change a `name:`; that only adds contention on a file every leg PR
shares, for an update it must not make.

`scripts/tests/test_feature_matrix_assemble.py` compares the assembled leg-name set
against that golden **byte-for-byte**. A name-set change scoped to the fragment *alone*
forbids the golden update it simultaneously requires — a self-contradiction that leaves
the `assemble feature matrix` job red and, because that job produces the whole matrix,
makes **every** `opt-in *` required check go expected-but-unreported. Decomposing a
leg-name-set task without the golden in scope is the bug this file exists to prevent.

## Regenerating the golden

Regenerate it; never hand-edit it. The committed file is the exact stdout of:

```sh
python3 scripts/assemble-feature-matrix.py --names > scripts/tests/feature-matrix-legnames.golden.txt
```

That is pinned by `test_golden_is_exact_names_output`, so a hand-written line (wrong
sort position, a comment, a stray blank) fails the gate even when the *set* matches.

## When the golden does NOT change

Only a leg's `name:` reaches the golden. Editing a leg's `features:`, `test:`,
`tier:` or `tier-reason:` — or its comments — leaves the name set untouched, so those
are genuinely single-file changes. Changing `name:` is a rename: it removes one golden
line and adds another.

## Other rules for a leg

- Keep `name` free of the whole words "advisory" and "informational" — `ci-summary`
  excludes matching check-runs, so such a leg would silently stop gating.
- `name` must be unique across all fragments (duplicates collapse two gating checks
  into one); `features` must be a non-empty comma list.
- A crate's leg lives in that crate's fragment only, so concurrent PRs for DIFFERENT
  crates never conflict on the fragments. They can still conflict on the shared
  golden — resolve that by rerunning the command above, not by merging text by hand.
