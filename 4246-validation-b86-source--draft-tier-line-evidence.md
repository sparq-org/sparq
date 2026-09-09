### .github/workflows/ci-select.yml:74-183

```
74: 
75: permissions:
76:   contents: read
77: 
78: jobs:
79:   select:
80:     # [FABLE-5] Draft-tier CI (docs/branch-protection.md §Draft-tier CI): the job
81:     # name APPENDS the ", draft-tier" marker when the calling run was triggered by
82:     # a DRAFT pull_request (a reusable workflow sees the caller's event payload;
83:     # every non-PR event evaluates the marker empty). The tier thus travels in the
84:     # check-run NAME — the same name-as-contract mechanism as the advisory rule —
85:     # and scripts/ci_summary_gate.py partitions draft-assembled from full-
86:     # assembled selection runs on a head SHA with it: a FULL-tier gate refuses to
87:     # conclude success while any draft-tier-marked select INSTANCE lacks its own
88:     # later full-tier successor (per-instance because ci.yml, bench.yml,
89:     # feature-matrix.yml and fuzz.yml all expose this IDENTICAL check-run name —
90:     # one workflow's full-tier select must never release the hold for the
91:     # others), so a draft-tier leg set can never admit a non-draft PR to the
92:     # merge queue. The stable phrase "change-based test selection" (the SELECT_RE
93:     # detection contract) is a prefix of ALL THREE spellings, and none contains the
94:     # words "advisory"/"informational".
95:     #
96:     # [OPUS-5] #3781 — THE ", no-leg" MARKER TAKES PRECEDENCE OVER ", draft-tier".
97:     # A `labeled`/`unlabeled` pull_request event whose label is NOT one of
98:     # ci-full / bench-full / fuzz-full is a GUARDED NO-OP for every caller: the
99:     # #2546 label-trigger guard `if:`s off every root job of ci.yml, bench.yml,
100:     # feature-matrix.yml and fuzz.yml, so the run assembles ZERO legs and THIS
101:     # unconditional pre-job is its only non-skipped job (measured on sparq #3472:
102:     # 8 label-flip runs at 07:28:57, each with exactly one non-skipped job — this
103:     # select). Marking such a run ", draft-tier" (which is what happened, because
104:     # the review pipeline re-drafts the PR in the same breath as the label flip)
105:     # manufactured a hold NOTHING could ever discharge: a full-tier successor is
106:     # only produced by a NON-draft payload, so the gate burned all 155 polls and
107:     # fail-closed-refused a leg set with zero failing legs. The tier of a run that
108:     # assembled no legs is not "draft" — it is NOT EVIDENCE AT ALL, and the gate
109:     # ignores such a run entirely (scripts/ci_summary_gate.py NO_LEG_MARKER /
110:     # no_leg_run_ids), which leaves the PREVIOUS real run authoritative instead of
111:     # letting a vacuous all-skipped run supersede it. CONSERVATIVE BY DESIGN: on a
112:     # ci-full/bench-full/fuzz-full flip at least one caller DOES do real work, so
113:     # no marker is claimed and the pre-#3781 behaviour stands unchanged (a
114:     # bench-full flip therefore still lets ci.yml's no-op run supersede — today's
115:     # behaviour, not a regression; the review pipeline never flips those labels).
116:     name: select (change-based test selection${{ (github.event_name == 'pull_request' && contains(fromJSON('["labeled","unlabeled"]'), github.event.action) && !contains(fromJSON('["ci-full","bench-full","fuzz-full"]'), github.event.label.name)) && ', no-leg' || (github.event.pull_request.draft == true && ', draft-tier' || '') }})
117:     runs-on: ubuntu-latest
118:     permissions:
119:       contents: read
120:     outputs:
121:       mode: ${{ steps.sel.outputs.mode }}
122:       affected: ${{ steps.sel.outputs.affected }}
123:       filterset: ${{ steps.sel.outputs.filterset }}
124:     steps:
125:       # Full history so the merge-base for the three-dot diff (base...head) is
126:       # always resolvable; an unfetchable base fail-closes to mode=full anyway.
127:       - uses: actions/checkout@9c091bb21b7c1c1d1991bb908d89e4e9dddfe3e0 # v7.0.0
128:         with:
129:           persist-credentials: false
130:           fetch-depth: 0
131:       - name: Compute the affected crate closure (scripts/ci_select.py)
132:         id: sel
133:         env:
134:           # cargo metadata resolves the registry index; same transient-network
135:           # hardening as every other lane (ci.yml header note).
136:           CARGO_NET_RETRY: "10"
137:           EVENT: ${{ github.event_name }}
138:           # pull_request: PR base/head. merge_group: the queue entry's target-tip
139:           # base_sha/head_sha (the diff is then the union of queued content — sound
140:           # per design §3.1/P8). Exactly one pair is non-empty per event.
141:           BASE: ${{ github.event.pull_request.base.sha || github.event.merge_group.base_sha }}
142:           HEAD: ${{ github.event.pull_request.head.sha || github.event.merge_group.head_sha }}
143:           # [FABLE-5] sq-fmx4u.5 ENFORCEMENT FLIP: enforce is the DEFAULT now. The
144:           # selector's mode=selected skips are HONORED unless CI_SELECT_MODE is set
145:           # to the literal "shadow" (the report-only rollback escape hatch below).
146:           CI_SELECT_MODE: ${{ vars.CI_SELECT_MODE }}
147:           # [FABLE-5] sq-fmx4u.5 ci-full label override (design §6.2): applying the
148:           # `ci-full` label to a PR forces the FULL matrix (mode=full, nothing
149:           # skipped); removing it re-selects. The calling workflows react to
150:           # labeled/unlabeled PR events so a toggle re-runs this pre-job. On non-PR
151:           # events there is no pull_request payload, so this evaluates false and the
152:           # override is inert (non-PR events already resolve to full by construction).
153:           CI_FULL_LABEL: ${{ contains(github.event.pull_request.labels.*.name, 'ci-full') }}
154:           # [FABLE-5] Draft-tier CI: surfaced in the step summary so a human reading
155:           # the run sees which tier this selection was assembled for (the machine
156:           # contract is the job NAME marker above).
157:           IS_DRAFT_PR: ${{ github.event.pull_request.draft == true }}
158:           # [OPUS-5] #3781: the same no-leg condition as the job name above, surfaced
159:           # for the human reader. A no-leg run's step summary must NOT claim "Tier:
160:           # draft" — it assembled nothing, and saying "draft" is exactly the confusion
161:           # that cost a full drain pass to unpick.
162:           IS_NO_LEG_RUN: ${{ github.event_name == 'pull_request' && contains(fromJSON('["labeled","unlabeled"]'), github.event.action) && !contains(fromJSON('["ci-full","bench-full","fuzz-full"]'), github.event.label.name) }}
163:         run: |
164:           set -euo pipefail
165:           if [ "${IS_NO_LEG_RUN:-}" = "true" ]; then
166:             echo "**Tier: no-leg** — this run was started by a \`${EVENT}\` label flip that the label-trigger guard turns into a NO-OP: every leg is skipped and this selection pre-job is the run's only real job. The ci-summary gate therefore IGNORES this whole run and keeps the previous real run of this workflow authoritative (#3781)." >> "$GITHUB_STEP_SUMMARY"
167:           elif [ "${IS_DRAFT_PR:-}" = "true" ]; then
168:             echo "**Tier: draft** — draft-tier CI (reduced sibling set; the full matrix re-runs at ready_for_review and its fresh gate supersedes this head's draft-tier results)." >> "$GITHUB_STEP_SUMMARY"
169:           fi
170:           args=(--event "$EVENT")
171:           case "$EVENT" in
172:             pull_request|merge_group) args+=(--base "$BASE" --head "$HEAD") ;;
173:           esac
174:           if [ "$CI_FULL_LABEL" = "true" ]; then
175:             # ci-full label: force the full matrix regardless of mode (design §6.2).
176:             args+=(--full)
177:           elif [ "${CI_SELECT_MODE:-}" = "shadow" ]; then
178:             # Report-only rollback escape hatch (design §6.4): set the repo variable
179:             # CI_SELECT_MODE=shadow to compute + report the would-skip set while every
180:             # job still runs. Any other value (incl. unset / "enforce") => enforce.
181:             args+=(--shadow)
182:           fi
183:           python3 scripts/ci_select.py "${args[@]}"
```

### scripts/ci_select.py:67-145

```
67: # Each entry is (pattern, reason). A pattern ending in "/" is a directory prefix
68: # (matches the dir itself or anything beneath it); otherwise it is an EXACT
69: # repo-relative path. Root "Cargo.toml" therefore matches ONLY the workspace
70: # root manifest — a crate's "crates/x/Cargo.toml" is attributed to crate x by
71: # ownership (and a version bump there rewrites Cargo.lock => full anyway, §3.3).
72: _FULL_TRIGGERS: list[tuple[str, str]] = [
73:     ("Cargo.lock", "Cargo.lock changed (resolved versions feed every build)"),
74:     ("Cargo.toml", "root Cargo.toml changed (workspace members / deps / lints / profiles / [patch])"),
75:     ("rust-toolchain", "toolchain pin changed (compiler version affects all codegen)"),
76:     ("rust-toolchain.toml", "toolchain pin changed (compiler version affects all codegen)"),
77:     (".cargo/", "cargo config changed (global rustflags / registries / build config)"),
78:     (".github/", "CI definition changed (including the selection wiring itself)"),
79:     ("scripts/", "shared CI/gate/coverage script changed (executed by CI)"),
80:     ("deny.toml", "cargo-deny config changed (redefines a supply-chain green)"),
81:     ("supply-chain/", "cargo-vet / supply-chain config changed"),
82:     ("ci/path-ownership.toml", "selection policy (ownership map) changed"),
83: ]
84: 
85: 
86: # --- ORCHESTRATION-ONLY carve-out (change-class layer; sq path-aware CI) ------
87: # [OPUS-4.8] The broad `.github/` and `scripts/` full-run triggers above are
88: # CORRECT-BY-DEFAULT but OVER-BROAD: a PR that changes ONLY orchestration/workflow
89: # tooling (PR/issue/bead automation, routing, the merge-queue batchers, agent
90: # config) forces the FULL Rust matrix even though NOTHING it touches is read by any
91: # Rust build/test/clippy/coverage/bench/fuzz/CodeQL job. The maintainer's ask:
92: # stop running the engine CI on those PRs (they dominate the drain).
93: #
94: # SOUNDNESS (§2 — a skip must be a PROOF of non-interference, not a guess): this is
95: # a small, EXPLICIT, AUDITED ALLOWLIST of paths PROVEN inert for the Rust matrix,
96: # consulted BEFORE `_trigger_match` so it is the ONLY thing that can rescue a
97: # `.github/`/`scripts/` path from the full-run trigger. It is a WHITELIST, never a
98: # denylist: a `.github/`/`scripts/` path NOT matched here still hits the trigger and
99: # forces full (absence of proof ⇒ run). A matched path is treated exactly like an
100: # ownership-map `safe = true` verdict — it contributes NO crate, so if EVERY changed
101: # path is orchestration-safe the selection is mode=selected with an empty affected
102: # closure and every Rust lane (incl. the bench/fuzz/wasm SEED lanes) skips.
103: #
104: # THE INERTNESS OBLIGATION (enforced by scripts/tests/test_ci_select.py
105: # `OrchestrationSafeInertnessTests`): every entry here must be a path that NO
106: # `.github/workflows/*.yml` step feeding a Rust build/test/gate ever reads, and that
107: # no crate `include_str!`/`include_bytes!`/`../`-escapes to. The test greps the
108: # Rust-CI workflows for a reference to each allowlisted script/dir and FAILS if one
109: # is referenced (i.e. is actually Rust-CI-affecting) — so an entry can never silently
110: # become unsound when a script is later wired into a gate. A pattern ending in "/" is
111: # a directory prefix; otherwise an exact repo-relative path.
112: #
113: # NOT here (deliberately — these ARE read by the Rust matrix, so they must keep
114: # triggering full): every Rust-CI script (ci_select.py, ci_summary_gate.py,
115: # coverage*.py/sh, perf-gate.py, assemble-feature-matrix.py, feature-matrix-tiers.py,
116: # check-*.py gates, fetch-*.sh conformance fetchers, ci-bench.sh, ci-free-disk.sh,
117: # unsafe/mutants gates, sbom/vex tooling, docker-smoke.sh, wasm-deps-guard.sh, the
118: # fv/formal lane scripts, and scripts/tests/* that gate the engine); the Rust-CI
119: # workflow files themselves (ci.yml, feature-matrix.yml, codeql.yml, supply-chain.yml,
120: # bench.yml, fuzz.yml, miri.yml, asan.yml, kani.yml, metamorph.yml,
121: # vectorized-feature-off.yml, ci-select.yml, ci-summary.yml, conformance/coverage
122: # lanes); `.github/feature-matrix.d/**`; `.github/codeql/**`; `.github/actions/**`.
123: _ORCHESTRATION_SAFE: list[str] = [
124:     # Orchestration configuration + agent harness (never compiled/tested by cargo).
125:     "orchestration/",       # routing.toml + orchestration policy (the #3416 class)
126:     ".claude/",             # agent definitions / skills / workflows / settings
127:     ".beads/",              # the bead task DB (never read by any build/test)
128:     # Orchestration-only workflow files (PR/issue/bead/merge automation — none run
129:     # cargo build/test/clippy/coverage/bench/fuzz/CodeQL).
130:     ".github/workflows/triage-issue.yml",
131:     ".github/workflows/retriage.yml",
132:     ".github/workflows/pr-backlog.yml",
133:     ".github/workflows/pr-title.yml",
134:     ".github/workflows/batch-merge.yml",
135:     ".github/workflows/bead-autoclose.yml",
136:     ".github/workflows/promote-on-approval.yml",
137:     ".github/workflows/differential-update.yml",
138:     ".github/workflows/kb-dump.yml",
139:     ".github/workflows/pkg-ingest.yml",
140:     # NOTE deliberately NOT here: selection-alarm.yml / formal-alarm.yml — they are
141:     # monitors for the Rust/formal lanes (borderline), so they keep triggering full
142:     # (fail-closed; the value of skipping them is negligible and the audit is cleaner).
143:     # Orchestration-only scripts (PR/issue/bead/routing/dispatch automation). Each is
144:     # pinned inert by the OrchestrationSafeInertnessTests grep.
145:     "scripts/triage.py",
```

### scripts/ci_select.py:685-720

```
685: 
686:     changed_crates: set[str] = set()
687:     file_owners: list[tuple[str, str]] = []
688: 
689:     for path in changed_paths:
690:         path = path.strip()
691:         if not path:
692:             continue
693:         # [OPUS-4.8] ORCHESTRATION-ONLY carve-out (BEFORE the trigger check — the sole
694:         # rescue from a .github/scripts full-run trigger, and only for a PROVEN-inert
695:         # path). Treated exactly like a SAFE-listed path: contributes no crate, so a
696:         # pure-orchestration diff selects an empty closure and every Rust lane skips.
697:         if _orchestration_safe_match(path):
698:             file_owners.append((path, "ORCH-SAFE"))
699:             continue
700:         # [OPUS-5] sq-g25hr: the DEPLOY-manifest carve-out, same position and same
701:         # rationale as the orchestration one above — it is the only rescue for the
702:         # two `.github/workflows/deploy-*.yml` files from the `.github/` full-run
703:         # trigger, and it contributes no crate, so a deploy-only diff selects an
704:         # empty closure. `deploy/**` alone would also be rescued by the
705:         # `safe = true` map entry below (like `.beads/**`, which is on both), but
706:         # listing it here keeps the selection and the CLASS on one path list.
707:         if _deploy_only_match(path):
708:             file_owners.append((path, "DEPLOY-SAFE"))
709:             continue
710:         trig = _trigger_match(path)
711:         if trig is not None:
712:             file_owners.append((path, "FULL-TRIGGER"))
713:             return full(trig)
714: 
715:         # [FABLE-5] sq-m4bxc: additional-readers (monotone union). Extra reader
716:         # crates declared for this path are added REGARDLESS of ownership — the
717:         # union can only ENLARGE the affected set (design §4.2, fail-safe §2). It
718:         # never rescues an unowned/unmapped path (that still forces full below),
719:         # so the result stays a strict superset of the no-readers selection.
720:         extra = additional_readers(path, map_entries)
```

### .github/workflows/ci.yml:34-48

```
34:   # group (below), so a flip can never concurrency-cancel an in-flight run on the
35:   # same head SHA — those same-SHA cancellations were the actual gate poison.
36:   # [FABLE-5] Draft-tier CI: + ready_for_review, so the un-draft moment re-runs the
37:   # whole matrix at FULL tier on the same head SHA (coverage + heavy shards run
38:   # again) and the fresh check-runs supersede the draft-tier ones — see
39:   # docs/branch-protection.md §Draft-tier CI. The label-trigger guard above and the
40:   # draft-tier guards below are independent and AND-compose on each affected job:
41:   # a non-`ci-full` label flip stays a no-op regardless of draft state, and a draft
42:   # head stays tier-reduced regardless of label events.
43:   pull_request:
44:     types: [opened, synchronize, reopened, labeled, unlabeled, ready_for_review]
45:   # [OPUS-4.8] Merge queue: re-run the identical PR check-set on the queue's
46:   # merge_group ref so the required ci-summary gate finds the same siblings it finds
47:   # on a PR. The per-commit `coverage` job (if: event_name != 'schedule') runs here;
48:   # the nightly tier (if: event_name == 'schedule' || 'workflow_dispatch') stays off.
```

### .github/workflows/ci.yml:210-217

```
210:     # matrix on an unchanged head SHA — skipping this root job skips every
211:     # needs:-dependent below it. All other events pass the first two disjuncts.
212:     if: >-
213:       github.event_name != 'pull_request' ||
214:       !contains(fromJSON('["labeled","unlabeled"]'), github.event.action) ||
215:       github.event.label.name == 'ci-full'
216:     runs-on: ubuntu-latest
217:     permissions:
```

### .github/workflows/ci.yml:772-824

```
772:           # below). `matrix.filter` is non-empty ONLY on those two heavy shards, so this
773:           # env is the shard-kind discriminator the run script reads (it is empty on the
774:           # bulk shards). Passed via env — no inline `${{ }}` in the shell body.
775:           SHARD_FILTER: ${{ matrix.filter }}
776:           # [FABLE-5] Draft-tier CI: "true" only on a DRAFT pull_request head — the
777:           # heavy recall shards are ALSO demoted there (see the draft-tier guard
778:           # below); every other event/state evaluates "false" and changes nothing.
779:           IS_DRAFT_PR: ${{ github.event_name == 'pull_request' && github.event.pull_request.draft == true }}
780:         run: |
781:           set -euo pipefail
782:           # [OPUS-4.8] sq-6vshe.6 (research/ci-structural-speedup.md §7, round 2):
783:           # DEMOTE the two heavy sparq-vectors recall shards (heavy-diskann/heavy-hnsw)
784:           # off the merge_group ref. WHY it is SAFE (per-leg cross-PR-interaction
785:           # analysis): each heavy shard runs exactly ONE deterministic, self-contained,
786:           # single-crate accuracy gate ({diskann,hnsw}_recall_at_10_vs_brute_force_on_50k
787:           # in crates/sparq-vectors/tests/, fixed seeds, `approx-ann` feature). Its
788:           # outcome is a pure function of sparq-vectors' ANN impl + its pinned
789:           # instant-distance dep — NOTHING else in the workspace. A squash-merge group
790:           # re-running it on the combined tree reproduces the identical result each PR
791:           # already gated, so it carries no cross-PR-interaction signal (unlike the bulk
792:           # workspace-unification build+test, which stays). FULL FORM is UNCHANGED: the
793:           # heavy shards still run at PR level AND on push-to-main (both force
794:           # rust_changed=true and neither hits this guard), so the recall floors gate
795:           # every PR + the canonical main build exactly as before. A post-merge
796:           # regression is caught by the push-to-main heavy run (this same job on the
797:           # `push` event) + the dedicated `heavy-recall-demoted-filer` job below, which
798:           # auto-files a P1 bead + GitHub issue if a heavy shard fails on main. This ONLY
799:           # affects the merge_group ref, and reports `success` (exit 0) so the ci-summary gate is
800:           # satisfied (the poller discovers whatever check-runs exist; an absent-by-design
801:           # sub-result never hangs it). Mirrors the selection skip idiom directly below.
802:           if [ "${{ github.event_name }}" = "merge_group" ] && [ -n "$SHARD_FILTER" ]; then
803:             echo "heavy shard '${{ matrix.name }}' DEMOTED off merge_group (sq-6vshe.6):"
804:             echo "  deterministic single-crate recall gate — no cross-PR interaction;"
805:             echo "  full form runs at PR level + push-to-main. Reporting success."
806:             exit 0
807:           fi
808:           # [FABLE-5] Draft-tier CI: ALSO demote the heavy recall shards on DRAFT
809:           # pull_request heads (docs/branch-protection.md §Draft-tier CI). Safe for
810:           # the same reason as the merge_group demotion above — deterministic
811:           # single-crate gates — plus the draft-tier invariant: un-drafting fires
812:           # ready_for_review, which re-runs this job at FULL tier on the same head
813:           # SHA, and a draft-tier gate result can never admit a PR to the merge
814:           # queue (scripts/ci_summary_gate.py refuses it). Bulk shards are NEVER
815:           # demoted (SHARD_FILTER is empty there); non-draft PRs and push-to-main
816:           # keep the full form.
817:           if [ "$IS_DRAFT_PR" = "true" ] && [ -n "$SHARD_FILTER" ]; then
818:             echo "heavy shard '${{ matrix.name }}' DEMOTED on this DRAFT head (draft-tier CI):"
819:             echo "  the full form re-runs at ready_for_review + push-to-main; a draft-tier"
820:             echo "  gate result can never admit a PR to the merge queue. Reporting success."
821:             exit 0
822:           fi
823:           # [FABLE-5] sq-fmx4u.3 (design §5.2): single-crate shard skip guard.
824:           # FAIL-CLOSED: only the exact mode 'selected' AND a non-empty SHARD_CRATE
```

### .github/workflows/ci.yml:3238-3250

```
3238:     # satisfied, so the ci-summary gate still sees ONE terminal coverage check on the
3239:     # merge_group ref — never an expected-but-missing one.
3240:     needs: [changes, coverage-floors, select]
3241:     if: >-
3242:       github.event_name != 'schedule' &&
3243:       github.event_name != 'merge_group' &&
3244:       (github.event_name != 'pull_request' || github.event.pull_request.draft != true) &&
3245:       needs.changes.outputs.rust_changed == 'true' &&
3246:       (needs.select.outputs.mode != 'selected' || needs.select.outputs.affected != '[]')
3247:     runs-on: ubuntu-latest
3248:     # [SONNET-4.6] sq-6vshe.11: RE-DELIVER the workflow-level lld link flag to this job.
3249:     # cargo-llvm-cov sets RUSTFLAGS/CARGO_ENCODED_RUSTFLAGS to inject -C instrument-coverage,
3250:     # and cargo's rustflags sources are first-match-wins, so the workflow-level
```

### .github/workflows/ci.yml:3492-3504

```
3492:     # needs `coverage-engine-run.result == 'success'`, so it skips with these — and the
3493:     # `coverage` aggregator counts both `skipped` results as satisfied.
3494:     needs: [changes, coverage-floors, select]
3495:     if: >-
3496:       github.event_name != 'schedule' &&
3497:       github.event_name != 'merge_group' &&
3498:       (github.event_name != 'pull_request' || github.event.pull_request.draft != true) &&
3499:       needs.changes.outputs.rust_changed == 'true' &&
3500:       (needs.select.outputs.mode != 'selected' || needs.select.outputs.affected != '[]')
3501:     runs-on: ubuntu-latest
3502:     # [SONNET-4.6] sq-6vshe.11: re-deliver the lld link flag (see coverage-measure above and
3503:     # the env: block at the top of this file). EXTRA constraint here: the cross-runner
3504:     # .profraw merge requires the run partitions and coverage-engine-merge to compile with
```

### .github/workflows/ci.yml:3723-3735

```
3723:     # measurements. A non-`ci-full` label flip stays a no-op regardless of draft
3724:     # state; a draft head stays coverage-skipped regardless of label events.
3725:     if: >-
3726:       always() && github.event_name != 'schedule' &&
3727:       (github.event_name != 'pull_request' ||
3728:        !contains(fromJSON('["labeled","unlabeled"]'), github.event.action) ||
3729:        github.event.label.name == 'ci-full') &&
3730:       (github.event_name != 'pull_request' || github.event.pull_request.draft != true)
3731:     runs-on: ubuntu-latest
3732:     steps:
3733:       - name: Aggregate coverage floor + shard results
3734:         env:
3735:           EVENT: ${{ github.event_name }}
```

### .github/workflows/bench.yml:50-69

```
50:   # the `ci-full` / `bench-full` toggles start real work off a label event. The
51:   # review pipeline's review:*/status:* flips otherwise re-queued this workflow on an
52:   # unchanged head SHA on every flip. The guard is AND-composed into the
53:   # `dashboard-labels` and `bench` job `if:`s (the `select` pre-job must stay
54:   # unconditional — the ci-summary gate REDs on any non-success selection
55:   # check-run), and label events get a per-run concurrency group below.
56:   # [FABLE-5] Draft-tier CI: + ready_for_review, so the un-draft moment runs the
57:   # deterministic perf ratchet the draft head skipped (the `bench` job is skipped
58:   # entirely on draft PR heads — see its `if:` guard + docs/branch-protection.md
59:   # §Draft-tier CI). This is independent of the label guard above and AND-composes
60:   # with it on the `bench` job.
61:   pull_request:
62:     types: [opened, synchronize, reopened, labeled, unlabeled, ready_for_review]
63:   # [FABLE-5] (sq-6vshe.6, MAINTAINER-DIRECTED) merge_group REMOVED. The deterministic
64:   # byte-count ratchet is a pure function of the code: it already ran + hard-gated on the
65:   # PR head (--deterministic-only above), it re-runs on push-to-main below, and the
66:   # merged-tree wasm-feature-OFF invariant is INDEPENDENTLY guarded on merge_group by
67:   # vectorized-feature-off.yml's `artifact-exact-equality` leg (a dynamic same-run
68:   # byte-identity build of the merged tree). So re-running this job on the ephemeral
69:   # merge_group ref added NO protection while occupying a runner slot + adding the full
```

### .github/workflows/bench.yml:181-206

```
181:     # `github.event_name != 'pull_request'` => the selection disjunction decides as
182:     # before). This also removes the per-PR benchmark comparison comment
183:     # (comment-always) + alert comment on draft heads — both are steps of THIS job.
184:     # The ready_for_review full-tier run executes the deterministic ratchet before
185:     # the PR can reach the merge queue; docs/branch-protection.md §Draft-tier CI.
186:     needs: select
187:     # [FABLE-5] label-trigger guard (full rationale: ci.yml's pull_request note): the
188:     # first AND-term skips this job on any labeled/unlabeled event whose label is not
189:     # ci-full/bench-full — the review pipeline's label flips must not re-queue the
190:     # bench suite on an unchanged head SHA (with cancel-in-progress:false a flip can
191:     # still CANCEL a PENDING real run via the one-pending-slot-per-group rule).
192:     # `bench-full` passes because its whole point is re-running this workflow with
193:     # the well-known suites enabled (the toolchain-install step reads the label).
194:     # `select` stays unguarded (required green by the gate). The second AND-term is
195:     # the unchanged fail-closed selection guard.
196:     if: >-
197:       (github.event_name != 'pull_request' ||
198:        !contains(fromJSON('["labeled","unlabeled"]'), github.event.action) ||
199:        contains(fromJSON('["ci-full","bench-full"]'), github.event.label.name)) &&
200:       (github.event_name != 'pull_request' || github.event.pull_request.draft != true) &&
201:       (needs.select.outputs.mode != 'selected' ||
202:        contains(needs.select.outputs.affected, '"sparq-engine"') ||
203:        contains(needs.select.outputs.affected, '"sparq-cli"') ||
204:        contains(needs.select.outputs.affected, '"sparq-bench"') ||
205:        contains(needs.select.outputs.affected, '"sparq-wasm"'))
206:     runs-on: ubuntu-latest
```

### .github/workflows/ci-summary.yml:201-210

```
201: on:
202:   # [FABLE-5] Draft-tier CI: the default types PLUS ready_for_review — without it
203:   # the un-draft moment would run NOTHING and the head would keep its draft-tier
204:   # gate result (the exact stale-green the integrity invariant forbids).
205:   pull_request:
206:     types: [opened, synchronize, reopened, ready_for_review]
207:   # [OPUS-4.8] Merge queue: GitHub runs the required check(s) on a temporary
208:   # `merge_group` ref before merging. ci-summary is the SINGLE required check, so it
209:   # MUST run here to produce the `ci-summary / gate` check on that ref. Without this
210:   # the queue would hang forever waiting for the check.
```

### .github/workflows/ci-summary.yml:255-272

```
255: jobs:
256:   gate:
257:     # [FABLE-5] Draft-tier CI: the check-run name is TIERED (the same marker
258:     # expression ci-select.yml uses). A draft pull_request payload renders
259:     # `gate, draft-tier`; every other event/state (non-draft PR, merge_group,
260:     # push) renders EXACTLY `gate` — the branch-protection ruleset's required
261:     # context, which a draft-tier run therefore can never satisfy (the
262:     # structural half of the integrity invariant above). Pinned by
263:     # scripts/tests/test_ci_select_wiring.py::TestRequiredCheckAnchor.
264:     name: gate${{ github.event.pull_request.draft == true && ', draft-tier' || '' }}
265:     runs-on: ubuntu-latest
266:     # > the loop's ABSOLUTE budget: ~37 min base (unchanged from the pre-sq-90cv4
267:     # cap) + up to ~30 min of saturation extension + API overhead. The loop caps
268:     # itself under this wall-clock so it emits a clear timeout error first. In the
269:     # normal (unsaturated) case the gate converges exactly as fast as before —
270:     # the extension only ever activates at base-budget exhaustion under a
271:     # saturated/progressing queue.
272:     timeout-minutes: 80
```
