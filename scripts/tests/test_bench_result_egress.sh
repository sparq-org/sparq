#!/usr/bin/env bash
# [OPUS-5] sq-ffaa9 — hermetic self-test for the durable S3 result-egress channel
# (scripts/bench/bench-result-egress.sh + scripts/bench/bootstrap-bench-iam.sh).
#
# 🤖 SPARQ agent. NO AWS ACCOUNT, NO NETWORK, NO /root: `aws` is PATH-shadowed by a stub
# that records its argv into a scratch file and can be told to fail, so every assertion is
# about what the library WOULD invoke and about the failure modes that lose results. Each
# case runs under `env -i` so a knob can never leak from one assertion into the next, and
# candidate URIs travel through the ENVIRONMENT rather than through `eval` (one of the
# hostile fixtures is a `;rm -rf /` injection — a test harness that eval'd it would run it).
#
# What it pins (each was a real way to silently lose a canonical gather):
#   1. Unconfigured => byte-identical to today: no --iam-instance-profile flag, no aws call.
#   2. Destination WITHOUT a profile => preflight FAILS. The box would have no credentials,
#      every upload would 403, and the box self-terminates — the sq-ffaa9 loss shape.
#   3. Both configured => the launch flag and the run-scoped URI are exactly right.
#   4. Malformed / hostile URIs, run ids and profile names are rejected before any argv.
#   5. push/pull invoke the expected `aws s3 cp|sync` argv, and a FAILING aws still returns
#      0 (a lost upload must never kill a multi-hour gather; the console is the backstop).
#   6. The bootstrap policy is least-privilege (write-only, prefix-scoped, no wildcards)
#      and --print-policy / --dry-run make no AWS call at all.
#   7. All three canonical launchers + their instance-side gathers stay WIRED to the
#      library (unwired wiring is indistinguishable from the bug this closes).
#
# Run: bash scripts/tests/test_bench_result_egress.sh   (exit 0 = all pass)
set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
LIB="$ROOT/scripts/bench/bench-result-egress.sh"
BOOTSTRAP="$ROOT/scripts/bench/bootstrap-bench-iam.sh"
[ -f "$LIB" ] || { echo "missing $LIB" >&2; exit 1; }
[ -f "$BOOTSTRAP" ] || { echo "missing $BOOTSTRAP" >&2; exit 1; }

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT
pass=0; fail=0
ok()  { pass=$((pass+1)); printf '  ok   %s\n' "$1"; }
bad() { fail=$((fail+1)); printf '  FAIL %s\n' "$1"; }
check() { if [ "$2" = "$3" ]; then ok "$1"; else bad "$1 (want '$3', got '$2')"; fi; }

# ---- the aws stub --------------------------------------------------------------------
STUB_DIR="$WORK/bin"; mkdir -p "$STUB_DIR"
cat > "$STUB_DIR/aws" <<'STUB'
#!/usr/bin/env bash
printf '%s\n' "$*" >> "${AWS_STUB_LOG:?}"
# `iam get-instance-profile --query …Roles[0].RoleName --output text` answers with the
# attached role name, or the literal "None" when the profile holds no role yet — that is
# how the real CLI renders a null JMESPath result, and the bootstrap branches on it.
case "$*" in
  *get-instance-profile*) printf '%s\n' "${AWS_STUB_ATTACHED_ROLE:-None}"; exit 0 ;;
esac
exit "${AWS_STUB_RC:-0}"
STUB
chmod +x "$STUB_DIR/aws"
STUB_PATH="$STUB_DIR:$PATH"
AWS_STUB_LOG="$WORK/aws.log"; : > "$AWS_STUB_LOG"

# lib_run [K=V ...] -- <snippet>
# Sources the library in a pristine environment and runs the snippet. K=V pairs are the
# ONLY channel for fixture values, so hostile fixtures never reach the harness's own shell.
lib_run() {
  local envs=()
  while [ "${1:-}" != "--" ]; do envs+=("$1"); shift; done
  shift
  env -i "PATH=$STUB_PATH" "HOME=$WORK" "AWS_STUB_LOG=$AWS_STUB_LOG" \
      "AWS_STUB_RC=${AWS_STUB_RC:-0}" "SPARQ_EGRESS_LIB=$LIB" "${envs[@]}" \
      bash -c '. "$SPARQ_EGRESS_LIB"; '"$1"
}

echo "== 1. unconfigured: inert, no aws call, no launch flag =="
: > "$AWS_STUB_LOG"
echo '{"suite":"x"}' > "$WORK/materialize-lubm1.json"
OUT=$(lib_run -- 'bench_egress_preflight && echo "preflight=0"; printf "[%s]" "$(bench_egress_launch_args)"; bench_egress_push "$HOME/materialize-lubm1.json" && echo " push=0"' 2>/dev/null)
case "$OUT" in *"preflight=0"*) ok "preflight passes when unconfigured" ;; *) bad "preflight when unconfigured: $OUT" ;; esac
case "$OUT" in *"[]"*) ok "no --iam-instance-profile flag when unconfigured" ;; *) bad "launch_args leaked a flag: $OUT" ;; esac
case "$OUT" in *"push=0"*) ok "push is a successful no-op when unconfigured" ;; *) bad "push when unconfigured: $OUT" ;; esac
check "no aws invocation when unconfigured" "$(wc -l < "$AWS_STUB_LOG" | tr -d ' ')" "0"

echo "== 2. destination WITHOUT a profile => preflight FAILS (the sq-ffaa9 loss shape) =="
if lib_run "BENCH_RESULTS_S3=s3://bkt/gathers" -- 'bench_egress_preflight' >/dev/null 2>&1; then
  bad "preflight accepted a destination with no instance profile"
else
  ok "preflight rejects a destination with no instance profile"
fi
ERRTXT=$(lib_run "BENCH_RESULTS_S3=s3://bkt/gathers" -- 'bench_egress_preflight' 2>&1 >/dev/null)
case "$ERRTXT" in *"no credentials"*) ok "the rejection names the missing credentials" ;;
  *) bad "rejection message unhelpful: $ERRTXT" ;; esac

echo "== 3. fully configured: launch flag + run-scoped URI =="
OUT=$(lib_run "BENCH_IAM_PROFILE=sparq-bench-results" "BENCH_RESULTS_S3=s3://sparq-bench-results-000000000000/gathers/" -- \
  'bench_egress_preflight && bench_egress_launch_args && bench_egress_run_uri sparq-bench-canonical-mat-42' 2>/dev/null)
check "launch flag" "$(printf '%s' "$OUT" | sed -n 1p)" "--iam-instance-profile Name=sparq-bench-results"
check "run-scoped URI (single slash, run id appended)" "$(printf '%s' "$OUT" | sed -n 2p)" \
  "s3://sparq-bench-results-000000000000/gathers/sparq-bench-canonical-mat-42"

echo "== 4. malformed / hostile destinations, run ids and profile names are rejected =="
while IFS= read -r badu; do
  [ -n "$badu" ] || continue
  if lib_run "BENCH_IAM_PROFILE=p" "BENCH_RESULTS_S3=$badu" -- 'bench_egress_preflight' >/dev/null 2>&1; then
    bad "preflight accepted malformed destination '$badu'"
  else
    ok "rejected malformed destination '$badu'"
  fi
done <<'FIXTURES'
http://bkt/gathers
s3://
s3:///gathers
s3://BKT-UPPER/gathers
s3://bkt/../etc/passwd
s3://bkt/gathers;rm -rf /
s3://bkt/gathers /elsewhere
s3://bkt/$(id)
FIXTURES
if lib_run "BENCH_IAM_PROFILE=p" "BENCH_RESULTS_S3=s3://bkt/gathers" -- 'bench_egress_run_uri "../evil"' >/dev/null 2>&1; then
  bad "run_uri accepted a path-climbing run id"
else
  ok "run_uri rejects a path-climbing run id"
fi
if lib_run "BENCH_IAM_PROFILE=bad profile name" "BENCH_RESULTS_S3=s3://bkt/gathers" -- 'bench_egress_preflight' >/dev/null 2>&1; then
  bad "preflight accepted an invalid instance-profile name"
else
  ok "preflight rejects an invalid instance-profile name"
fi

echo "== 5. push / pull argv, and a failing aws never kills the gather =="
: > "$AWS_STUB_LOG"
lib_run "BENCH_RESULTS_S3_URI=s3://bkt/gathers/run7" "BENCH_EGRESS_REGION=eu-west-2" -- \
  'bench_egress_push "$HOME/materialize-lubm1.json"' >/dev/null 2>&1
check "push argv" "$(cat "$AWS_STUB_LOG")" \
  "s3 cp --only-show-errors --region eu-west-2 $WORK/materialize-lubm1.json s3://bkt/gathers/run7/materialize-lubm1.json"
: > "$AWS_STUB_LOG"
if AWS_STUB_RC=1 lib_run "BENCH_RESULTS_S3_URI=s3://bkt/gathers/run7" -- \
     'bench_egress_push "$HOME/materialize-lubm1.json"' >/dev/null 2>&1; then
  ok "push returns 0 even when aws s3 cp fails (console stays the backstop)"
else
  bad "a failing upload propagated a non-zero exit into the gather"
fi
: > "$AWS_STUB_LOG"
lib_run -- 'bench_egress_pull s3://bkt/gathers/run7 "$HOME/pulled"' >/dev/null 2>&1
check "pull argv" "$(cat "$AWS_STUB_LOG")" "s3 sync s3://bkt/gathers/run7 $WORK/pulled"
if [ -d "$WORK/pulled" ]; then ok "pull created the destination directory"; else bad "pull did not create the destination directory"; fi
: > "$AWS_STUB_LOG"
lib_run -- 'bench_egress_push "$HOME/materialize-lubm1.json"' >/dev/null 2>&1
check "push with no destination makes no aws call" "$(wc -l < "$AWS_STUB_LOG" | tr -d ' ')" "0"

echo "== 5b. bench_egress_sweep: incremental, idempotent, retries a FAILED upload =="
# The gathers call sweep after every unit of work AND once at the end. That is only
# affordable if a re-sweep is free — so an envelope that already uploaded is skipped,
# while one whose upload FAILED is tried again (the end-of-gather call is the retry pass).
SWEEP="$WORK/sweep"; mkdir -p "$SWEEP"
echo '{"a":1}' > "$SWEEP/one.json"
printf 'not an envelope\n' > "$SWEEP/notes.txt"
: > "$AWS_STUB_LOG"
lib_run "BENCH_RESULTS_S3_URI=s3://bkt/gathers/run7" "SWEEP=$SWEEP" -- \
  'bench_egress_sweep "$SWEEP"; echo "{\"b\":2}" > "$SWEEP/two.json"; bench_egress_sweep "$SWEEP"; bench_egress_sweep "$SWEEP"' \
  >/dev/null 2>&1
check "three sweeps upload each of the two envelopes exactly once" "$(grep -c 's3 cp' "$AWS_STUB_LOG")" "2"
check "sweep ignores non-.json files" "$(grep -c 'notes.txt' "$AWS_STUB_LOG")" "0"
: > "$AWS_STUB_LOG"
AWS_STUB_RC=1 lib_run "BENCH_RESULTS_S3_URI=s3://bkt/gathers/run7" "SWEEP=$SWEEP" -- \
  'bench_egress_sweep "$SWEEP"; bench_egress_sweep "$SWEEP"' >/dev/null 2>&1
check "a FAILED upload is retried by the next sweep (2 files x 2 sweeps)" "$(grep -c 's3 cp' "$AWS_STUB_LOG")" "4"
: > "$AWS_STUB_LOG"
if lib_run "SWEEP=$SWEEP" -- 'bench_egress_sweep "$SWEEP"' >/dev/null 2>&1; then
  ok "sweep is a successful no-op when unconfigured"
else
  bad "sweep returned non-zero when unconfigured"
fi
check "unconfigured sweep makes no aws call" "$(wc -l < "$AWS_STUB_LOG" | tr -d ' ')" "0"
: > "$AWS_STUB_LOG"
lib_run "BENCH_RESULTS_S3_URI=s3://bkt/gathers/run7" -- 'bench_egress_sweep "$HOME/no-such-dir"' >/dev/null 2>&1
check "sweep of a missing directory makes no aws call" "$(wc -l < "$AWS_STUB_LOG" | tr -d ' ')" "0"

echo "== 6. bootstrap policy is least-privilege; --print-policy/--dry-run make no AWS call =="
: > "$AWS_STUB_LOG"
POLICY=$(PATH="$STUB_PATH" AWS_STUB_LOG="$AWS_STUB_LOG" bash "$BOOTSTRAP" --print-policy \
  --bucket sparq-bench-results-000000000000 --prefix gathers 2>/dev/null)
if python3 - "$POLICY" <<'PY'
import json, sys
d = json.loads(sys.argv[1])
perm = d["permissions"]["Statement"]
assert len(perm) == 1, perm
acts = perm[0]["Action"]
assert set(acts) <= {"s3:PutObject", "s3:AbortMultipartUpload"}, acts
assert "s3:GetObject" not in acts and "s3:DeleteObject" not in acts and "*" not in acts, acts
res = perm[0]["Resource"]
assert res == "arn:aws:s3:::sparq-bench-results-000000000000/gathers/*", res
assert d["trust"]["Statement"][0]["Principal"]["Service"] == "ec2.amazonaws.com"
assert d["trust"]["Statement"][0]["Action"] == "sts:AssumeRole"
PY
then ok "policy documents are least-privilege (write-only, prefix-scoped)"
else bad "policy documents are NOT least-privilege"
fi
check "bootstrap --print-policy makes no aws call" "$(wc -l < "$AWS_STUB_LOG" | tr -d ' ')" "0"
: > "$AWS_STUB_LOG"
DRY=$(PATH="$STUB_PATH" AWS_STUB_LOG="$AWS_STUB_LOG" bash "$BOOTSTRAP" --dry-run \
  --bucket sparq-bench-results-000000000000 2>&1)
check "bootstrap --dry-run makes no aws call" "$(wc -l < "$AWS_STUB_LOG" | tr -d ' ')" "0"
case "$DRY" in *"iam create-instance-profile"*) ok "dry-run prints the instance-profile creation" ;;
  *) bad "dry-run never mentions the instance profile" ;; esac
case "$DRY" in *"add-role-to-instance-profile"*) ok "dry-run attaches the role to the profile" ;;
  *) bad "dry-run never attaches the role" ;; esac

# The DOCUMENTED invocation is a bare `--dry-run`, with no --bucket. That is the branch that
# used to reach `aws sts get-caller-identity` to derive the default bucket name, so a preview
# spent credentials before printing anything. `env -i` keeps BENCH_RESULTS_BUCKET from leaking
# in and re-hiding the branch.
: > "$AWS_STUB_LOG"
DRY0=$(env -i "PATH=$STUB_PATH" "HOME=$WORK" "AWS_STUB_LOG=$AWS_STUB_LOG" \
  bash "$BOOTSTRAP" --dry-run 2>&1)
check "bootstrap --dry-run WITHOUT --bucket makes no aws call" "$(wc -l < "$AWS_STUB_LOG" | tr -d ' ')" "0"
case "$DRY0" in *"sts get-caller-identity"*) ok "dry-run prints the account lookup a real run would make" ;;
  *) bad "dry-run hides the account lookup entirely" ;; esac
case "$DRY0" in *"s3api create-bucket"*) ok "dry-run without --bucket still previews the whole plan" ;;
  *) bad "dry-run without --bucket stopped short of the plan" ;; esac

echo "== 6b. re-running the bootstrap is a no-op; a foreign role in the profile fails loudly =="
# An instance profile holds at most ONE role and AWS answers a re-attach with a quota error,
# not an AlreadyExists — so idempotency here has to come from asking before attaching.
: > "$AWS_STUB_LOG"
env -i "PATH=$STUB_PATH" "HOME=$WORK" "AWS_STUB_LOG=$AWS_STUB_LOG" "AWS_STUB_ATTACHED_ROLE=None" \
  bash "$BOOTSTRAP" --bucket sparq-bench-results-000000000000 >/dev/null 2>&1
if grep -q 'add-role-to-instance-profile' "$AWS_STUB_LOG"; then
  ok "first run (empty profile) attaches the role"
else
  bad "first run never attached the role to the instance profile"
fi

: > "$AWS_STUB_LOG"
if env -i "PATH=$STUB_PATH" "HOME=$WORK" "AWS_STUB_LOG=$AWS_STUB_LOG" \
     "AWS_STUB_ATTACHED_ROLE=sparq-bench-results-writer" \
     bash "$BOOTSTRAP" --bucket sparq-bench-results-000000000000 >/dev/null 2>&1; then
  ok "re-run succeeds when the SAME role is already attached"
else
  bad "re-run failed even though the same role was already attached"
fi
if grep -q 'add-role-to-instance-profile' "$AWS_STUB_LOG"; then
  bad "re-run re-issued add-role-to-instance-profile (AWS answers that with a quota error)"
else
  ok "re-run skips add-role-to-instance-profile (genuinely idempotent)"
fi

: > "$AWS_STUB_LOG"
CONFLICT_RC=0
CONFLICT=$(env -i "PATH=$STUB_PATH" "HOME=$WORK" "AWS_STUB_LOG=$AWS_STUB_LOG" \
  "AWS_STUB_ATTACHED_ROLE=someone-elses-role" \
  bash "$BOOTSTRAP" --bucket sparq-bench-results-000000000000 2>&1) || CONFLICT_RC=$?
check "a DIFFERENT role occupying the profile is a hard failure" "$CONFLICT_RC" "1"
case "$CONFLICT" in *someone-elses-role*) ok "the conflict error names the occupying role" ;;
  *) bad "the conflict error does not name the occupying role" ;; esac
if grep -q 'add-role-to-instance-profile' "$AWS_STUB_LOG"; then
  bad "attached over a foreign role instead of failing"
else
  ok "no attach attempted while a foreign role occupies the profile"
fi

echo "== 7. the canonical launchers + gathers are wired to the library =="
for l in canonical-materialize-bench.sh canonical-competitor-bench.sh canonical-beir-bench.sh; do
  f="$ROOT/scripts/bench/$l"
  if grep -q 'bench-result-egress.sh' "$f"; then ok "$l sources the egress library"; else bad "$l does not source the egress library"; fi
  if grep -q 'bench_egress_launch_args' "$f"; then ok "$l passes the instance profile to run-instances"; else bad "$l never attaches an instance profile"; fi
  if grep -q 'bench_egress_preflight' "$f"; then ok "$l preflights the egress configuration"; else bad "$l skips the egress preflight"; fi
  if grep -q 'bench_egress_pull' "$f"; then ok "$l syncs results back from S3"; else bad "$l never pulls results from S3"; fi
done
for g in canonical-materialize-gather-instance.sh canonical-http-gather-instance.sh canonical-beir-gather-instance.sh; do
  f="$ROOT/scripts/bench/$g"
  if grep -q 'bench_egress_push\|bench_egress_sweep' "$f"; then ok "$g uploads its envelopes"; else bad "$g never uploads its envelopes"; fi
done

echo "== 7b. the gathers upload DURING the run, not only in the end-of-gather dump loop =="
# An upload that runs only after every stage is worthless on a box that self-terminates
# or dies mid-run: the already-completed envelopes never leave. So the FIRST egress call
# must sit strictly before the final ===ENVELOPE-BEGIN console dump. This is a structural
# check; the resulting BEHAVIOUR is pinned by test_beir_gather_sentinel.sh case 10, which
# SIGKILLs the gather after one cut and asserts the aws stub already recorded that cut's
# upload — a bare grep for the function name cannot tell the two apart.
for g in canonical-materialize-gather-instance.sh canonical-beir-gather-instance.sh; do
  f="$ROOT/scripts/bench/$g"
  first="$(grep -n '^[^#]*bench_egress_\(push\|sweep\) ' "$f" | head -1 | cut -d: -f1)"
  dump="$(grep -n 'ENVELOPE-BEGIN \$' "$f" | head -1 | cut -d: -f1)"
  if [ -n "$first" ] && [ -n "$dump" ] && [ "$first" -lt "$dump" ]; then
    ok "$g uploads before its end-of-gather console dump (line $first < $dump)"
  else
    bad "$g uploads only in its final dump loop (first=$first dump=$dump) — a box dying mid-run loses every completed envelope"
  fi
done
HTTPG="$ROOT/scripts/bench/canonical-http-gather-instance.sh"
if awk '/^run_suite_http\(\)/,/^}$/' "$HTTPG" | grep -q 'bench_egress_push'; then
  ok "canonical-http-gather-instance.sh uploads each suite envelope as it is written"
else
  bad "canonical-http-gather-instance.sh does not upload from inside run_suite_http"
fi

echo "test_bench_result_egress: ${pass} passed, ${fail} failed."
[ "$fail" -eq 0 ] || exit 1
echo "test_bench_result_egress: OK — egress is inert unless configured, half-configured fails fast, uploads are best-effort, and the launchers stay wired."
