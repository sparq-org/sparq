#!/usr/bin/env bash
# Reclaim disk on a GitHub-hosted ubuntu runner before the heavy
# build / nextest-archive / sharded-test steps (bead sq-c76x).
#
# WHY: the load-aware test shards were recurringly failing with
# `No space left on device` while writing the nextest archive (.tar.zst) or the
# bulk-shard `target/` extraction + per-test diagnostics. A stock ubuntu-latest
# runner ships only ~14 GB free on `/`, most of it consumed by pre-installed
# toolchains/SDKs this Rust workspace never uses (.NET, GHC/Haskell, the Android
# SDK, the bundled CodeQL bundle, and a large Docker image cache). Purging those
# reclaims ~20-30 GB, which is the PRIMARY fix for the disk-exhaustion shard
# failures — NOT a weakening of any test or gate.
#
# DESIGN NOTES:
#   • Pure shell + `sudo rm` / `docker image prune` / `apt-get clean` — NO
#     third-party action, so there is nothing new to SHA-pin or vet (keeps the
#     repo's SHA-pin / Scorecard posture intact). An equivalent maintained action
#     (jlumbroso/free-disk-space) exists; we deliberately keep it in-repo so the
#     reclaim is auditable and pinned-by-definition.
#   • Each removal is best-effort: a path that is absent on a given runner image
#     must NOT fail the step (the runner image contents drift). So we DO NOT
#     `set -e` on the removals — every `rm` is `|| true` and we never abort the
#     build just because a SDK we wanted gone was already gone.
#   • We print `df -h /` before and after so the reclaimed amount is visible in
#     the job log (and so a future regression in runner free-space is diagnosable
#     from the log alone).
#   • Safe to run locally / on a non-GitHub host: every target path is guarded by
#     existence and the whole thing is best-effort, so on a dev box where these
#     paths do not exist it simply reports "nothing to reclaim" and exits 0.
#     It NEVER touches the repo checkout or any tracked file.
set -uo pipefail

echo "::group::Free runner disk — before"
df -h / || true
echo "::endgroup::"

# Large pre-installed toolchains/SDKs this Rust workspace never builds against.
# Order: biggest reclaim first. `sudo` because they live under system prefixes on
# the hosted image; on a non-root dev box without these paths the rm is a no-op.
PURGE_PATHS=(
  /usr/share/dotnet           # .NET SDK + runtimes (~20+ GB on some images)
  /usr/local/lib/android      # Android SDK/NDK (~8+ GB)
  /opt/ghc                    # Haskell GHC
  /usr/local/.ghcup           # Haskell ghcup toolchains
  /opt/hostedtoolcache/CodeQL # bundled CodeQL pack (we run CodeQL in codeql.yml, not here)
  /usr/local/share/powershell # PowerShell
  /usr/share/swift            # Swift toolchain
  /usr/local/share/boost      # bundled Boost C++ headers/libs (folded from #462)
)

for p in "${PURGE_PATHS[@]}"; do
  if [ -e "$p" ]; then
    echo "removing $p"
    sudo rm -rf "$p" || true
  fi
done

# sq-6uc7: the hosted-toolcache root (env-named, NOT a fixed path) —
# folded from the closed #462. It holds preinstalled language runtimes
# (Node/Python/Go/…) keyed by version that this Rust workspace never uses; on a
# hosted image it is several GB. Handled SEPARATELY from PURGE_PATHS because:
#   • it is an ENV VAR that is unset on a non-GitHub host, and this script runs
#     `set -u` — so we expand it as `${AGENT_TOOLSDIRECTORY:-}` and skip when empty
#     rather than abort;
#   • every lane runs THIS reclaim BEFORE its `dtolnay/rust-toolchain` install, and
#     that install uses rustup under $HOME (~/.rustup, ~/.cargo) — NOT the
#     toolcache — so purging the toolcache here cannot remove a toolchain CI needs.
TOOLCACHE="${AGENT_TOOLSDIRECTORY:-}"
if [ -n "$TOOLCACHE" ] && [ -e "$TOOLCACHE" ]; then
  echo "removing $TOOLCACHE (hosted toolcache)"
  sudo rm -rf "$TOOLCACHE" || true
fi

# Reclaim the Docker image cache (the hosted image preloads several GB of images
# this lane does not use). Best-effort: `docker` may be absent in some contexts.
if command -v docker >/dev/null 2>&1; then
  echo "pruning docker images"
  sudo docker image prune -af || true
fi

# Drop the apt package cache.
if command -v apt-get >/dev/null 2>&1; then
  sudo apt-get clean || true
fi

echo "::group::Free runner disk — after"
df -h / || true
echo "::endgroup::"
