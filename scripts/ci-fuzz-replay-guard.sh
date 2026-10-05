#!/usr/bin/env bash
# [OPUS-5] Fail loud on a VACUOUS libFuzzer corpus-replay (bead sq-c9q4r).
#
# WHY: .github/workflows/fuzz.yml runs the per-PR fuzz leg in REPLAY mode — libFuzzer
# `-runs=0` executes every input in `fuzz/corpus/<target>` (the restored cache) and
# `fuzz/seeds/<target>` (the committed minimal seeds) EXACTLY ONCE and generates none.
# The workflow `mkdir -p`s both dirs first so a brand-new target does not error on a
# missing dir — but that forgiveness is also the hole: a target with NO committed seeds
# and NO cached corpus replays ZERO inputs, libFuzzer exits 0, and a BLOCKING per-PR
# gate reports green having executed nothing. Every target in the tree today ships
# seeds, so this guard fires only for a FUTURE target added without them — which is
# exactly the case that would otherwise ship an always-green leg nobody notices.
#
# Randomized mode is deliberately EXEMPT: `-max_total_time` generates its own inputs,
# so starting from an empty corpus is normal there, not vacuous.
#
# USAGE
#   scripts/ci-fuzz-replay-guard.sh <mode> <target> <dir> [dir ...]
#     mode   : the workflow's run mode; only `replay` is checked, anything else is a
#              no-op success (so the caller can pass "$MODE" unconditionally).
#     target : target name, for the error message only.
#     dir... : the input directories to count regular files across.
#   exit 0 = the replay has at least one input (or mode is not replay)
#   exit 1 = replay mode with zero inputs — the caller must fail the gate
#
#   scripts/ci-fuzz-replay-guard.sh --self-test   # hermetic; scratch dirs only
set -uo pipefail

# Count regular files across the given dirs. A dir that does not exist contributes 0
# rather than erroring: "missing" and "empty" are the same vacuous-replay condition.
count_inputs() {
  local n=0 d
  for d in "$@"; do
    [ -d "$d" ] || continue
    n=$((n + $(find "$d" -type f | wc -l)))
  done
  printf '%s' "$n"
}

guard() {
  local mode="$1" target="$2"
  shift 2
  if [ "$mode" != "replay" ]; then
    return 0
  fi
  local inputs
  inputs="$(count_inputs "$@")"
  echo "replay inputs for '${target}': ${inputs} ($*)"
  if [ "$inputs" -eq 0 ]; then
    echo "::error::fuzz target '${target}' has NO replay inputs — $* are all empty or absent, so -runs=0 would execute zero inputs and this gate would pass VACUOUSLY. Commit at least one minimal seed under fuzz/seeds/${target}."
    return 1
  fi
  return 0
}

# ── hermetic self-test (scratch dirs only; never touches the repo) ────────────────
self_test() {
  local td rc fails=0
  td="$(mktemp -d)"
  # shellcheck disable=SC2064  # expand $td now, at trap-set time
  trap "rm -rf '$td'" EXIT

  check() {  # check <expected-rc> <description> <args...>
    local want="$1" desc="$2"
    shift 2
    if guard "$@" >/dev/null 2>&1; then rc=0; else rc=$?; fi
    if [ "$rc" -ne "$want" ]; then
      echo "FAIL: ${desc} (want exit ${want}, got ${rc})" >&2
      fails=$((fails + 1))
    fi
  }

  mkdir -p "$td/empty_corpus" "$td/empty_seeds" "$td/seeded"
  : > "$td/seeded/seed-0"

  # THE DEFECT: replay over two empty dirs must be RED, not a silent green.
  check 1 "replay + both dirs empty is fatal" replay t "$td/empty_corpus" "$td/empty_seeds"
  # ...and a target whose dirs were never created at all (no mkdir, no cache).
  check 1 "replay + absent dirs is fatal" replay t "$td/nope_a" "$td/nope_b"
  # A single committed seed in EITHER dir is enough — the replay is non-vacuous.
  check 0 "replay + a seed in the second dir passes" replay t "$td/empty_corpus" "$td/seeded"
  check 0 "replay + a seed in the first dir passes" replay t "$td/seeded" "$td/empty_seeds"
  # Nested layout (libFuzzer never nests, but the count must not miss one if it did).
  mkdir -p "$td/nested/sub"
  : > "$td/nested/sub/in-0"
  check 0 "replay counts files in subdirectories" replay t "$td/nested"
  # Randomized mode generates its own inputs — an empty start is NOT a finding.
  check 0 "fuzz mode + empty dirs is exempt" fuzz t "$td/empty_corpus" "$td/empty_seeds"

  if [ "$fails" -ne 0 ]; then
    echo "[ci-fuzz-replay-guard] self-test FAILED (${fails} failure(s))" >&2
    return 1
  fi
  echo "[ci-fuzz-replay-guard] self-test OK"
  return 0
}

if [ "${1:-}" = "--self-test" ]; then
  self_test
  exit
fi

if [ "$#" -lt 3 ]; then
  echo "usage: $0 <mode> <target> <dir> [dir ...]   (or --self-test)" >&2
  exit 2
fi

guard "$@"
