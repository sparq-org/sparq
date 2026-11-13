#!/usr/bin/env bash
# Run outside the restricted container. Requires Node >=24, npm, Rust, Python,
# curl, git, internet access for preparation, and a loopback TCP listener.
set -euo pipefail
vc_package_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
repo_dir=$(git -C "$vc_package_dir" rev-parse --show-toplevel)
vc_suite_root=${VC_SUITE_ROOT:-"${TMPDIR:-/tmp}/sparq-vc-suites"}
vc_report_dir=${VC_REPORT_DIR:-"${TMPDIR:-/tmp}/sparq-vc-reports"}
mkdir -p "$vc_suite_root" "$vc_report_dir"
for suite in vc-data-model-2.0-test-suite vc-di-eddsa-test-suite; do
  [[ ! -e "$vc_report_dir/$suite.json" ]] || {
    echo "Refusing to reuse $vc_report_dir/$suite.json; use a fresh VC_REPORT_DIR." >&2
    exit 1
  }
done
if curl --silent --max-time 1 --output /dev/null http://127.0.0.1:3000/did.json; then
  echo 'Port 3000 already serves HTTP; stop that service before this run.' >&2
  exit 1
fi
node -e 'if(Number(process.versions.node.split(".")[0]) < 24) throw new Error("Node >=24 required")'
# These are the exact source revisions inspected for the Rust fixture tests.
for entry in \
  vc-data-model-2.0-test-suite:62836458636bd1b8309214626ec8fc85de43f8bd \
  vc-di-eddsa-test-suite:1e86cc4ca16d77eb76cd15e3905a624bc16c0879 \
  vc-test-suite-implementations:e39022e808515d3095180032bb3fcf2e1bd754e2; do
  name=${entry%%:*}
  revision=${entry#*:}
  if [[ ! -d "$vc_suite_root/$name/.git" ]]; then
    git clone "https://github.com/w3c/$name.git" "$vc_suite_root/$name"
    git -C "$vc_suite_root/$name" checkout --detach "$revision"
  fi
  test "$(git -C "$vc_suite_root/$name" rev-parse HEAD)" = "$revision" || {
    echo "Expected $name at $revision; use a fresh VC_SUITE_ROOT or check out that revision." >&2
    exit 1
  }
done
python3 "$vc_package_dir/prepare-contexts.py"
export SPARQ_VC_CONTEXT_DIR="$vc_package_dir/contexts"
# Fresh test key, held only in the environment. Supply an existing test seed to
# repeat a run with the same public DID. No seed is printed or written to disk.
if [[ -z ${SPARQ_VC_TEST_SEED:-} ]]; then
  SPARQ_VC_TEST_SEED=$(python3 -c 'import secrets; print(secrets.token_hex(32))')
fi
export SPARQ_VC_TEST_SEED
export SPARQ_VC_BIND=127.0.0.1:3000
cargo build --manifest-path "$repo_dir/Cargo.toml" -p sparq-vc --features vc-api --bin sparq-vc-api
# cargo metadata respects CARGO_TARGET_DIR, including relative paths.
vc_target_dir=$(cargo metadata --manifest-path "$repo_dir/Cargo.toml" --format-version 1 --no-deps | python3 -c 'import json,sys; print(json.load(sys.stdin)["target_directory"])')
"$vc_target_dir/debug/sparq-vc-api" >"$vc_report_dir/service.log" 2>&1 &
vc_service_pid=$!
trap 'kill "$vc_service_pid" 2>/dev/null || true' EXIT
vc_ready=false
for attempt in {1..60}; do
  if ! kill -0 "$vc_service_pid" 2>/dev/null; then cat "$vc_report_dir/service.log"; exit 1; fi
  if curl --fail --silent http://127.0.0.1:3000/did.json >"$vc_report_dir/did.json"; then vc_ready=true; break; fi
  sleep 1
done
[[ $vc_ready = true ]] || { echo 'VC-API failed to become ready' >&2; exit 1; }
# The registry README's "Testing locally" format, with both requested tags.
# Presentation verification uses the same cryptosuite tags as credential verification.
python3 - "$vc_report_dir/did.json" "$vc_suite_root" <<'PY'
import json,pathlib,sys
issuer=json.loads(pathlib.Path(sys.argv[1]).read_text())['id']
base='http://127.0.0.1:3000'
config={'settings':{'enableInteropTests':True,'testAllImplementations':False},
 'implementations':[{'name':'sparq','implementation':'sparq-vc test endpoint',
 'issuers':[{'id':issuer,'endpoint':base+'/credentials/issue','tags':['vc2.0','eddsa-rdfc-2022']}],
 'vpVerifiers':[{'id':base+'/presentations/verify','endpoint':base+'/presentations/verify','tags':['vc2.0','eddsa-rdfc-2022']}],
 'verifiers':[{'id':base+'/credentials/verify','endpoint':base+'/credentials/verify','tags':['vc2.0','eddsa-rdfc-2022']}]}]}
for name in ['vc-data-model-2.0-test-suite','vc-di-eddsa-test-suite']:
 path=pathlib.Path(sys.argv[2])/name/'localConfig.cjs'
 if path.exists(): raise SystemExit(f'Refusing to overwrite {path}; use a fresh VC_SUITE_ROOT')
 path.write_text('module.exports = '+json.dumps(config,indent=2)+';\n')
PY
# Install outside the container. Pin the inspected registry checkout after npm
# install, and bypass npm test's pretest hook which otherwise fetches moving main.
(cd "$vc_suite_root/vc-test-suite-implementations" && npm install)
vc_failed=0
for suite in vc-data-model-2.0-test-suite vc-di-eddsa-test-suite; do
  (
    cd "$vc_suite_root/$suite"
    npm install || exit
    npm install --no-save "$vc_suite_root/vc-test-suite-implementations" || exit
    git rev-parse HEAD >"$vc_report_dir/$suite.revision"
    # JSON reporter is the unambiguous source of passing/failing/pending counts.
    # Direct mocha invocation avoids pretest and package-level reporter options.
    ./node_modules/.bin/mocha --no-config --no-package tests/ --reporter json \
      --reporter-options "output=$vc_report_dir/$suite.json" \
      --timeout 15000 --preserve-symlinks >"$vc_report_dir/$suite.log" 2>&1
  ) || vc_failed=1
done
python3 - "$vc_report_dir" <<'PY'
import json,pathlib,sys
root=pathlib.Path(sys.argv[1]); counts={}
for name in ['vc-data-model-2.0-test-suite','vc-di-eddsa-test-suite']:
 report=json.loads((root/(name+'.json')).read_text())
 stats=report['stats']
 counts[name]={k:stats[k] for k in ['tests','passes','failures','pending']}
 print(name, json.dumps(counts[name]))
(root/'counts.json').write_text(json.dumps(counts,indent=2)+'\n')
PY
exit "$vc_failed"
