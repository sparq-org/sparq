### scripts/feature_off_autodeclare.py:154-191

```python
def classify_paths(paths: list[str],
                   closure: list[str] | None = None) -> tuple[list[str], list[str], list[str]]:
    """Split changed paths into (blankable, manifest, in_closure_report).

    EVERY changed path that is not a manifest is BLANKABLE. The classification is
    deliberately NOT used to decide what the proof covers, because that is precisely how
    this tool produced a live false pass:

        sparq's root manifest carries `exclude = ["vendor/spargebra", ...]` together with
        `[patch.crates-io] spargebra = { path = "vendor/spargebra" }`, so spargebra IS
        compiled into the feature-OFF bundle while NOT being a workspace member. An earlier
        version scoped blanking to a closure derived from `cargo metadata --no-deps` —
        which returns workspace members only — so `vendor/spargebra/src/parser.rs`
        classified INERT, was never blanked, and `neutral == head` held BY CONSTRUCTION.
        Three genuinely non-benign changes there all came back `declared`.

    A skipped path cannot be proved harmless, so nothing is skipped. Blanking a file the
    build never reads is free — the bundle is unchanged either way — while blanking one it
    DOES read changes the bundle and the derivation refuses. Letting the compiler answer
    costs nothing and removes an entire class of classification bug.

    Manifests are handled separately (restored from the base tree) because blanking a line
    out of a `Cargo.toml` yields a manifest, not an absence.

    `closure` is reported as evidence only; it no longer gates anything.
    """
    blankable: list[str] = []
    manifest: list[str] = []
    in_closure: list[str] = []
    for p in paths:
        root_input = p in _ROOT_BUILD_INPUTS or p.startswith(".cargo/")
        if _is_manifest(p) or root_input:
            manifest.append(p)
        else:
            blankable.append(p)
        if closure is not None and (root_input or any(p.startswith(d) for d in closure)):
            in_closure.append(p)
    return blankable, manifest, in_closure
```

### scripts/feature_off_autodeclare.py:201-248

```python
def parse_unified_diff(diff_text: str) -> dict[str, dict[str, list[int]]]:
    """Map path -> {"added": [1-based new-file line numbers],
                    "removed": [1-based old-file line numbers]}.

    Only the line NUMBERS are needed: the content is read back from the real trees, so a
    truncated or context-less diff cannot smuggle content past the obligations.
    """
    out: dict[str, dict[str, list[int]]] = {}
    path: str | None = None
    old_path: str | None = None
    old_ln = new_ln = 0
    for line in diff_text.split("\n"):
        if line.startswith("diff --git "):
            path = old_path = None
            continue
        if line.startswith("--- "):
            src = line[4:].strip()
            old_path = None if src == "/dev/null" else (src[2:] if src.startswith("a/") else src)
            continue
        if line.startswith("+++ "):
            target = line[4:].strip()
            if target == "/dev/null":
                # File DELETED by this diff. Its removed lines must still be attributed, so
                # key them under the OLD path rather than dropping the hunk on the floor.
                path = old_path
            else:
                path = target[2:] if target.startswith("b/") else target
            if path is not None:
                out.setdefault(path, {"added": [], "removed": []})
            continue
        m = _HUNK.match(line)
        if m:
            old_ln = int(m.group(1))
            new_ln = int(m.group(3))
            continue
        if path is None:
            continue
        if line.startswith("+"):
            out[path]["added"].append(new_ln)
            new_ln += 1
        elif line.startswith("-"):
            out[path]["removed"].append(old_ln)
            old_ln += 1
        elif line.startswith(" ") or line == "":
            old_ln += 1
            new_ln += 1
        # "\\ No newline at end of file" and everything else: no line consumed.
    return out
```

### scripts/feature_off_autodeclare.py:251-262

```python
def blank_lines(text: str, line_numbers: list[int]) -> str:
    """Replace the given 1-based lines of `text` with EMPTY lines, preserving line count.

    Preserving the line count is the load-bearing property: it is what makes the neutral
    tree hold the base tree's content at the head tree's line POSITIONS, so a byte-identical
    build proves the blanked lines emitted nothing.
    """
    lines = text.split("\n")
    for n in line_numbers:
        if 1 <= n <= len(lines):
            lines[n - 1] = ""
    return "\n".join(lines)
```

### scripts/feature_off_autodeclare.py:285-324

```python
def cargo_wasm_builder(tree: str) -> bytes | None:
    """Build the feature-OFF sparq-wasm bundle in `tree`; return its bytes, or None on failure.

    Mirrors the leg-2 build exactly: default features only (no `--features vectorized`),
    the `release-wasm` profile, the wasm32 target.
    """
    proc = subprocess.run(
        ["cargo", "build", "--profile", "release-wasm", "-p", "sparq-wasm",
         "--target", "wasm32-unknown-unknown",
         "--target-dir", os.path.join(tree, "target")],
        cwd=tree, capture_output=True,
        env={**os.environ, "CARGO_TARGET_DIR": os.path.join(tree, "target")},
    )
    if proc.returncode != 0:
        sys.stderr.write(proc.stderr.decode("utf-8", "replace")[-4000:])
        return None
    # PER-TREE target directory, deliberately. Pointing several materialised trees at ONE
    # shared CARGO_TARGET_DIR to keep the extra builds warm was TRIED and REVERTED: it
    # produced a demonstrably WRONG verdict. Run against #4350 — a PR whose merge-base diff
    # is a 299-line rewrite of crates/sparq-engine/src/exec.rs — the shared-directory run
    # reported the base and head bundles BYTE-IDENTICAL ("no-drift"), i.e. a stale artefact
    # was read back as a fresh build. `git archive` stamps each exported tree's files with
    # its COMMIT time, and cargo's freshness check is mtime-based, so an out-of-order
    # timestamp can leave a previous tree's `sparq_wasm.wasm` in place.
    #
    # A false "identical" is the worst failure this tool has: it would auto-declare a real
    # code change as line-position churn. Cold builds are the price of the proof meaning
    # anything, so `--target-dir` stays inside the tree being built and the environment
    # cannot override it.
    env_override = os.environ.get("CARGO_TARGET_DIR")
    if env_override:
        sys.stderr.write(
            f"[autodeclare] ignoring CARGO_TARGET_DIR={env_override!r}: each tree must build "
            "into its own target directory (a shared one has been observed returning a stale "
            "bundle, which reads as a false 'identical').\n")
    out = os.path.join(tree, "target", "wasm32-unknown-unknown", "release-wasm", "sparq_wasm.wasm")
    if not os.path.exists(out):
        return None
    with open(out, "rb") as fh:
        return fh.read()
```

### scripts/feature_off_autodeclare.py:352-544

```python
def decide(base_tree: str, head_tree: str, diff_text: str, builder,
           base_bytes: bytes | None = None, head_bytes: bytes | None = None) -> Verdict:
    """Derive whether the base->head feature-OFF drift is line-position churn ONLY.

    `builder(tree_dir) -> bytes | None` compiles a materialised tree. Injected so the
    decision logic is testable without cargo; production passes `cargo_wasm_builder`.

    `base_bytes` / `head_bytes` let a caller hand in bundles the leg-2 job already built,
    so the derivation costs one extra build (two when the diff deletes code) rather than
    four. They are used verbatim — the obligations below still rebuild the NEUTRAL trees
    with the same builder, so a handed-in bundle cannot short-circuit any proof.
    """
    changes = parse_unified_diff(diff_text)
    if not changes:
        return Verdict(REFUSE_NO_DIFF, "the base..head diff is empty — nothing to attribute")

    closure = closure_dirs(head_tree)
    rust_paths, manifest_paths, closure_paths = classify_paths(sorted(changes), closure)

    # The diff and the trees must actually describe the same thing. A changed path that
    # exists in NEITHER tree means they disagree — a mis-parsed diff, a bad path prefix, a
    # stale export. Every such path would be silently skipped by the blanking loop, so the
    # proof would quietly cover less than it claims. Caught for real: an ad-hoc harness
    # rewrote `git diff --no-index` prefixes in the wrong order, yielding paths like
    # `bvendor/spargebra/src/parser.rs`; nothing matched, nothing was blanked, and only the
    # non-vacuity guard stood between that and a false declaration. Name it instead.
    missing = [p for p in rust_paths
               if not os.path.exists(os.path.join(head_tree, p))
               and not os.path.exists(os.path.join(base_tree, p))]
    if missing:
        return Verdict(REFUSE_DIFF_TREE_MISMATCH,
                       "the diff names paths that exist in neither the base nor the head "
                       "tree, so the diff and the trees disagree and the proof would cover "
                       "less than it claims: " + ", ".join(sorted(missing)[:10]))

    # Blanking rewrites files in place, which is meaningless for a symlink or a submodule
    # gitlink — the proof would silently cover nothing. Refuse rather than pretend.
    nonregular = [p for p in rust_paths
                  if os.path.islink(os.path.join(head_tree, p))
                  or os.path.islink(os.path.join(base_tree, p))]
    if nonregular:
        return Verdict(REFUSE_UNSUPPORTED_FILE_CHANGE,
                       "changed inside the wasm build closure but not a regular file, so "
                       "blanking cannot speak for it: " + ", ".join(sorted(nonregular)))

    if base_bytes is None:
        base_bytes = builder(base_tree)
    if base_bytes is None:
        return Verdict(REFUSE_BASE_BUILD_FAILED, "the BASE tree did not build")
    if head_bytes is None:
        head_bytes = builder(head_tree)
    if head_bytes is None:
        return Verdict(REFUSE_HEAD_BUILD_FAILED, "the HEAD tree did not build")

    if base_bytes == head_bytes:
        return Verdict(OUTCOME_NO_DRIFT,
                       "base and head bundles are byte-identical; leg 2 already passes",
                       {"bundle_bytes": len(head_bytes)})

    evidence = {
        "base_bundle_bytes": len(base_bytes),
        "head_bundle_bytes": len(head_bytes),
        "size_delta_bytes": len(head_bytes) - len(base_bytes),
        "differing_bytes": differing_bytes(base_bytes, head_bytes),
        "closure_files_changed": rust_paths,
        "manifest_files_changed": manifest_paths,
        "files_in_compiled_closure": closure_paths,
        "closure_dirs": closure,
    }

    # ---- Obligation 1: additions contribute no compiled code ----------------
    # Neutral tree = HEAD with every added .rs line blanked and every changed manifest
    # restored from BASE. It therefore holds BASE's compiled content at HEAD's line
    # positions. If its bundle equals HEAD's, the additions emitted nothing.
    removed_nonblank = 0
    to_blank: dict[str, list[int]] = {}
    for path in rust_paths:
        removed = changes[path]["removed"]
        if not removed:
            continue
        src = os.path.join(base_tree, path)
        if not os.path.exists(src):
            continue
        with open(src, encoding="utf-8", errors="surrogateescape") as fh:
            text = fh.read()
        n = nonblank_count(text, removed)
        if n:
            removed_nonblank += n
            to_blank[path] = removed
    evidence["deleted_nonblank_lines"] = removed_nonblank

    neutral = os.path.join(tempfile.mkdtemp(prefix="featoff-neutral-"), "tree")
    shutil.copytree(head_tree, neutral, symlinks=True)
    added_nonblank = 0
    mutated: list[str] = []
    for path in rust_paths:
        added = changes[path]["added"]
        if not added:
            continue
        target = os.path.join(neutral, path)
        if not os.path.exists(target):
            continue
        with open(target, encoding="utf-8", errors="surrogateescape") as fh:
            text = fh.read()
        added_nonblank += nonblank_count(text, added)
        blanked = blank_lines(text, added)
        if blanked != text:
            mutated.append(path)
        with open(target, "w", encoding="utf-8", errors="surrogateescape") as fh:
            fh.write(blanked)
    for path in manifest_paths:
        src = os.path.join(base_tree, path)
        dst = os.path.join(neutral, path)
        head_bytes_of = open(dst, "rb").read() if os.path.exists(dst) else None
        if os.path.exists(src):
            shutil.copyfile(src, dst)
            if open(dst, "rb").read() != head_bytes_of:
                mutated.append(path)
        elif os.path.exists(dst):
            os.remove(dst)
            mutated.append(path)
    evidence["added_nonblank_lines_blanked"] = added_nonblank
    evidence["neutral_tree_files_mutated"] = len(mutated)

    # NON-VACUITY. Obligation 1 concludes "the additions emitted nothing" from
    # `build(neutral) == build(head)`. If the neutral tree is IDENTICAL to the head tree,
    # that comparison is `head == head` — it holds by construction and proves nothing, so a
    # declaration derived from it would be exactly the rubber stamp this leg exists to
    # prevent. This is the general form of the vendored-crate false pass: whenever the diff
    # is real but nothing got blanked, the drift came from somewhere the proof does not
    # reach. Refuse, and say where the diff actually was.
    if not mutated and not to_blank:
        return Verdict(
            REFUSE_VACUOUS_PROOF,
            "neither obligation has anything to prove: the neutral tree is identical to the "
            "head tree (nothing was blanked) and the diff deletes no non-blank line, so "
            "comparing the bundles would be `head == head` and would hold by construction. "
            "The bundle moved anyway, so the drift originates outside what the proof "
            "reaches — do not declare it. Changed paths: "
            + ", ".join(sorted(changes)[:20]),
            evidence)

    neutral_bytes = builder(neutral)
    if neutral_bytes is None:
        # Blanking the added lines broke the build => at least one of them was load-bearing.
        return Verdict(REFUSE_NEUTRAL_BUILD_FAILED,
                       "blanking the added lines broke the build, so they are compiled code, "
                       "not comments or compiled-out cfg-gated tokens",
                       evidence)
    if neutral_bytes != head_bytes:
        evidence["neutral_vs_head_size_delta"] = len(head_bytes) - len(neutral_bytes)
        evidence["neutral_vs_head_differing_bytes"] = differing_bytes(neutral_bytes, head_bytes)
        return Verdict(REFUSE_ADDED_LINES_ARE_SEMANTIC,
                       "blanking the added lines CHANGED the compiled bundle, so the diff adds "
                       "code to the default build; an intentional always-compiled change must "
                       "be declared by its author, not derived",
                       evidence)

    # ---- Obligation 2: deletions removed no compiled code -------------------
    # Deleted lines are absent from BOTH head and neutral, so obligation 1 cannot speak for
    # them. Run the same proof on the base side: blank exactly those lines in BASE and
    # require the bundle to be unchanged.
    if to_blank:
        del_neutral = os.path.join(tempfile.mkdtemp(prefix="featoff-delneutral-"), "tree")
        shutil.copytree(base_tree, del_neutral, symlinks=True)
        for path, removed in to_blank.items():
            target = os.path.join(del_neutral, path)
            with open(target, encoding="utf-8", errors="surrogateescape") as fh:
                text = fh.read()
            with open(target, "w", encoding="utf-8", errors="surrogateescape") as fh:
                fh.write(blank_lines(text, removed))
        del_bytes = builder(del_neutral)
        if del_bytes is None:
            return Verdict(REFUSE_DELETION_BUILD_FAILED,
                           "blanking the deleted lines in the BASE tree broke the build, so the "
                           "diff removes compiled code",
                           evidence)
        if del_bytes != base_bytes:
            evidence["delneutral_vs_base_size_delta"] = len(base_bytes) - len(del_bytes)
            evidence["delneutral_vs_base_differing_bytes"] = differing_bytes(del_bytes, base_bytes)
            return Verdict(REFUSE_DELETED_LINES_ARE_SEMANTIC,
                           "blanking the deleted lines in the BASE tree CHANGED the compiled "
                           "bundle, so the diff removes code from the default build; that is an "
                           "always-compiled change its author must declare",
                           evidence)

    return Verdict(
        OUTCOME_DECLARED,
        "every added line was proved to emit nothing (neutral bundle == head bundle) and "
        "every deleted line was proved to have emitted nothing (base-neutral bundle == base "
        "bundle); the drift is line-position metadata only",
        evidence,
    )
```
