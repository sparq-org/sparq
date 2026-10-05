# shellcheck shell=bash
# [OPUS-5] sq-ffaa9 — opt-in durable S3 result egress for canonical sparq-bench EC2 gathers.
#
# 🤖 SPARQ agent. This file is a LIBRARY: source it, do not execute it.
#
# WHY. A canonical gather box is orphan-proof — it self-terminates — so every result must
# leave the box before it dies. Until now the only channel that survives termination was
# the serial console (`aws ec2 get-console-output`), and that channel is AMI-dependent: the
# 2026-07-10 x86_64 attempt on an AL2023/Nitro image (research/gap-vector-2026-07.md,
# "x86_64 EC2 attempt") got NO application output back through the API, so the DiskANN
# comparison that box ran is still recorded NOT-RUN. This library adds the second channel
# the issue asks for — an IAM instance profile attached at launch, `aws s3 cp` of each
# envelope from the box, and an `aws s3 sync` back down in the launcher.
#
# OPT-IN, NO-OP BY DEFAULT. With neither knob exported every function here is inert and
# returns success, so the console + SSH-pull behaviour of every launcher is unchanged.
# Egress needs BOTH knobs; a half-configured pair FAILS FAST in bench_egress_preflight
# rather than running a multi-hour gather whose results silently never upload — that
# silent-loss shape is the whole point of sq-ffaa9.
#
#   BENCH_IAM_PROFILE      instance-profile NAME attached at run-instances (launcher side)
#   BENCH_RESULTS_S3       destination root, s3://<bucket>[/<prefix>]     (launcher side)
#   BENCH_RESULTS_S3_URI   run-scoped destination the box writes under; the launcher
#                          derives it with bench_egress_run_uri and passes it in user-data
#   BENCH_EGRESS_REGION    optional --region for the s3 calls (defaults to AWS_REGION)
#
# The bucket / role / instance profile are a ONE-TIME MAINTAINER action — they need IAM +
# S3 permissions the bench role does not hold, which is exactly why sq-ffaa9 is labelled
# [MAINTAINER credential/access]. scripts/bench/bootstrap-bench-iam.sh creates them (run it
# with --dry-run first to read the exact calls) and prints the two exports above.
#
# Launcher side:                            Instance side (cloned repo, as root):
#   . scripts/bench/bench-result-egress.sh    . scripts/bench/bench-result-egress.sh
#   bench_egress_preflight || die ...         bench_egress_push "$envelope"   # one file
#   RUN_URI=$(bench_egress_run_uri "$KEY")    bench_egress_sweep "$OUT_DIR"   # new ones
#   aws ec2 run-instances ... $(bench_egress_launch_args)
#   bench_egress_pull "$RUN_URI" "$DIR"
#
# CALL IT AS THE WORK COMPLETES, NOT AT THE END. A gather box self-terminates and can die
# mid-run, so an upload deferred to an end-of-gather loop loses every already-completed
# envelope — the exact silent loss this channel exists to prevent. Each gather therefore
# uploads per unit of work (per suite / per cut / per scale) and calls the sweep once more
# at the end purely as the RETRY pass for uploads that failed earlier.
#
# Hermetically self-tested (PATH-shadowed `aws`, no network, no AWS account) by
# scripts/tests/test_bench_result_egress.sh; the mid-run-death BEHAVIOUR (a SIGKILLed
# gather has already uploaded its completed cut) by scripts/tests/test_beir_gather_sentinel.sh.

BENCH_IAM_PROFILE="${BENCH_IAM_PROFILE:-}"
# Trailing slashes are stripped at source time: an operator writing `s3://bucket/prefix/`
# is normal, but the validator below rejects a trailing slash (it would produce a `//`
# in the object key) and every consumer appends its own separator.
BENCH_RESULTS_S3="${BENCH_RESULTS_S3:-}"; BENCH_RESULTS_S3="${BENCH_RESULTS_S3%/}"
BENCH_RESULTS_S3_URI="${BENCH_RESULTS_S3_URI:-}"; BENCH_RESULTS_S3_URI="${BENCH_RESULTS_S3_URI%/}"
BENCH_EGRESS_REGION="${BENCH_EGRESS_REGION:-${AWS_REGION:-}}"
# Paths that uploaded SUCCESSFULLY in this process, so bench_egress_sweep can be called
# after every unit of work without re-uploading — and still retries a file whose upload
# failed. Process-local by design: it tracks this run's uploads, not the bucket's contents.
declare -A BENCH_EGRESS_PUSHED=()

bench_egress_log() { printf '[bench-egress %s] %s\n' "$(date -u +%H:%M:%S)" "$*" >&2; }

# s3://<bucket>[/<key>] validator. The value reaches an `aws` argv AND the user-data
# heredoc, so the allowlist is deliberately narrow: no whitespace, no shell metacharacters,
# no `..` segment that could climb out of the run-scoped prefix.
bench_egress_valid_uri() {
  local uri="${1:-}" rest bucket key=""
  [[ "$uri" == s3://* ]] || return 1
  rest="${uri#s3://}"
  bucket="${rest%%/*}"
  [[ "$rest" == */* ]] && key="${rest#*/}"
  [[ "$bucket" =~ ^[a-z0-9][a-z0-9.-]{1,61}[a-z0-9]$ ]] || return 1
  [ -n "$key" ] || return 0
  [[ "$key" =~ ^[A-Za-z0-9][A-Za-z0-9._/-]*$ ]] || return 1
  case "$key" in
    *..*) return 1 ;;
    */) return 1 ;;
  esac
  return 0
}

# True only when the full egress channel is configured (destination AND credentials).
bench_egress_enabled() { [ -n "$BENCH_RESULTS_S3" ] && [ -n "$BENCH_IAM_PROFILE" ]; }

# Launcher-side gate. Returns non-zero ONLY for a configuration that would lose results.
bench_egress_preflight() {
  if [ -z "$BENCH_RESULTS_S3" ] && [ -z "$BENCH_IAM_PROFILE" ]; then
    return 0   # disabled: serial console + SSH pull only, exactly as before
  fi
  if [ -n "$BENCH_IAM_PROFILE" ] && \
     [[ ! "$BENCH_IAM_PROFILE" =~ ^[A-Za-z0-9._+=,@-]{1,128}$ ]]; then
    bench_egress_log "ERROR: BENCH_IAM_PROFILE='$BENCH_IAM_PROFILE' is not a valid instance-profile name"
    return 1
  fi
  if [ -z "$BENCH_RESULTS_S3" ]; then
    bench_egress_log "WARN: BENCH_IAM_PROFILE set with no BENCH_RESULTS_S3 — the profile is attached but nothing is uploaded; results still depend on the console channel"
    return 0
  fi
  if [ -z "$BENCH_IAM_PROFILE" ]; then
    bench_egress_log "ERROR: BENCH_RESULTS_S3='$BENCH_RESULTS_S3' set with no BENCH_IAM_PROFILE — the box would have no credentials, every upload would fail, and a self-terminating gather loses its results silently. Run scripts/bench/bootstrap-bench-iam.sh and export both."
    return 1
  fi
  if ! bench_egress_valid_uri "$BENCH_RESULTS_S3"; then
    bench_egress_log "ERROR: BENCH_RESULTS_S3='$BENCH_RESULTS_S3' is not a well-formed s3://bucket[/prefix] URI"
    return 1
  fi
  bench_egress_log "durable S3 egress ENABLED: profile=$BENCH_IAM_PROFILE dest=$BENCH_RESULTS_S3"
  return 0
}

# run-instances flag fragment; EMPTY (and still exit 0) when no profile is configured, so
# the default launch command is byte-identical to today's.
bench_egress_launch_args() {
  [ -n "$BENCH_IAM_PROFILE" ] || return 0
  printf '%s\n' "--iam-instance-profile Name=$BENCH_IAM_PROFILE"
}

# Run-scoped destination, e.g. s3://bucket/prefix/<run-id>. Empty when egress is off.
bench_egress_run_uri() {
  local run_id="${1:-}"
  bench_egress_enabled || return 0
  if [[ ! "$run_id" =~ ^[A-Za-z0-9][A-Za-z0-9._-]*$ ]]; then
    bench_egress_log "ERROR: run id '$run_id' is not a safe single S3 key segment"
    return 1
  fi
  printf '%s\n' "${BENCH_RESULTS_S3%/}/$run_id"
}

# The Ubuntu server AMIs the canonical launchers pin do NOT ship the aws CLI, so the box
# installs it on first push. Best-effort; returns non-zero if `aws` is still unavailable.
bench_egress_ensure_cli() {
  command -v aws >/dev/null 2>&1 && return 0
  bench_egress_log "aws CLI absent — installing (the Ubuntu server AMIs do not ship it)"
  if command -v apt-get >/dev/null 2>&1; then
    DEBIAN_FRONTEND=noninteractive apt-get install -y -qq awscli >/dev/null 2>&1 || true
  fi
  command -v aws >/dev/null 2>&1 && return 0
  local zip="/tmp/awscliv2.zip"
  if command -v curl >/dev/null 2>&1 && command -v unzip >/dev/null 2>&1 \
     && curl -fsSL "https://awscli.amazonaws.com/awscli-exe-linux-$(uname -m).zip" -o "$zip" 2>/dev/null \
     && unzip -q -o "$zip" -d /tmp/awscliv2 2>/dev/null; then
    /tmp/awscliv2/aws/install --update >/dev/null 2>&1 || true
    hash -r 2>/dev/null || true
  fi
  command -v aws >/dev/null 2>&1
}

# Upload ONE result file. BEST-EFFORT BY CONTRACT: never returns non-zero and never aborts
# a gather — the console dump stays the backstop, so a failed upload degrades to a loud
# WARN rather than killing a multi-hour run.
bench_egress_push() {
  local file="${1:-}" dest="${2:-$BENCH_RESULTS_S3_URI}" rc=0
  [ -n "$dest" ] || return 0
  if [ ! -f "$file" ]; then
    bench_egress_log "WARN: push skipped, no such file: $file"; return 0
  fi
  if ! bench_egress_valid_uri "$dest"; then
    bench_egress_log "WARN: push skipped, malformed destination: $dest"; return 0
  fi
  if ! bench_egress_ensure_cli; then
    bench_egress_log "WARN: push skipped, aws CLI unavailable — the console dump is the only channel"
    return 0
  fi
  local args=(s3 cp --only-show-errors)
  [ -n "$BENCH_EGRESS_REGION" ] && args+=(--region "$BENCH_EGRESS_REGION")
  args+=("$file" "$dest/$(basename "$file")")
  aws "${args[@]}" || rc=$?
  if [ "$rc" -eq 0 ]; then
    BENCH_EGRESS_PUSHED["$file"]=1
    bench_egress_log "uploaded $(basename "$file") -> $dest/"
  else
    bench_egress_log "WARN: aws s3 cp exited $rc for $file -> $dest/ (console dump remains the backstop)"
  fi
  return 0
}

# Upload every *.json in a directory that has not ALREADY uploaded successfully in this
# process. This is what a gather calls at every point a unit of work completes (a cut, a
# scale, a suite) so a finished envelope is durable BEFORE the next multi-hour stage
# starts — a self-terminating box that dies later has already put it in S3. Repeat calls
# are cheap and idempotent: a file recorded in BENCH_EGRESS_PUSHED is skipped, a file
# whose upload FAILED is retried, so the end-of-gather call doubles as the retry sweep.
#
# Everything present is uploaded, valid JSON or not: a truncated envelope is still the
# partial evidence a re-run decision needs, and dropping it here would lose exactly what
# this channel exists to preserve. Validation stays where it already is — the gather's own
# outcome check, which decides canonical/partial/failed.
#
# Best-effort like bench_egress_push: always returns 0, never aborts a gather.
bench_egress_sweep() {
  local dir="${1:-}" f
  [ -n "$BENCH_RESULTS_S3_URI" ] || return 0
  [ -d "$dir" ] || return 0
  for f in "$dir"/*.json; do
    [ -f "$f" ] || continue
    [ -n "${BENCH_EGRESS_PUSHED[$f]+x}" ] && continue
    bench_egress_push "$f"
  done
  return 0
}

# Launcher-side retrieval. Also best-effort: the launcher's own role may be able to write
# via the instance profile but not read the bucket itself, in which case the results are
# still durable in S3 and the maintainer retrieves them (README: "reader access").
bench_egress_pull() {
  local uri="${1:-}" dir="${2:-}" rc=0
  [ -n "$uri" ] || return 0
  if [ -z "$dir" ]; then
    bench_egress_log "WARN: pull skipped, no destination directory"; return 0
  fi
  if ! bench_egress_valid_uri "$uri"; then
    bench_egress_log "WARN: pull skipped, malformed source: $uri"; return 0
  fi
  mkdir -p "$dir"
  local args=(s3 sync)
  [ -n "$BENCH_EGRESS_REGION" ] && args+=(--region "$BENCH_EGRESS_REGION")
  args+=("$uri" "$dir")
  aws "${args[@]}" || rc=$?
  if [ "$rc" -eq 0 ]; then
    bench_egress_log "synced $uri -> $dir"
  else
    bench_egress_log "WARN: aws s3 sync exited $rc ($uri -> $dir) — results may still be in the bucket; see scripts/bench/README.md"
  fi
  return 0
}
