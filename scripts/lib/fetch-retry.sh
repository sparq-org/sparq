# shellcheck shell=bash
# [OPUS-4.8] sq-nj0pd — shared network-retry helpers for the W3C / community
# conformance suite-fetch scripts. Source (do NOT execute) this file:
#
#   . "$(dirname "$0")/lib/fetch-retry.sh"
#
# Background: the conformance jobs (W3C SPARQL, inference, JSON-LD, ODRL) each
# `git clone`/`git fetch` a PINNED test suite at the START of the lane. A single
# transient reset on that clone — GitHub occasionally drops large transfers from
# CI runner IP ranges — exits the script non-zero under `set -euo pipefail`,
# which red-gates the lane in ~18s with no real conformance error. Because every
# one of these is a REQUIRED check feeding the ci-summary aggregator, one network
# blip stalls --auto merges on PRs that never touched the reasoner. sq-y5dz (#864)
# hardened the N3 clone + OWL curl inside fetch-inference-suites.sh but the
# DELEGATED first step (fetch-conformance.sh, the w3c/rdf-tests clone) plus the
# JSON-LD / ODRL fetchers were still bare git ops — this centralises the retry so
# every suite fetch is resilient and consistent.
#
# Pin discipline is UNCHANGED: every caller still verifies the cloned/fetched
# HEAD equals the pinned commit (or, for the OWL payload, its sha256) AFTER the
# retried transfer, so a retry can never silently substitute drifted test data.

# retry CMD [ARGS...]
#   Runs CMD until it succeeds or FETCH_RETRY_MAX (default 5) attempts elapse,
#   sleeping FETCH_RETRY_DELAY (default 5) seconds between tries. Returns the
#   command's last exit status on persistent failure. Safe to call under
#   `set -e` (the `until` loop consumes the non-zero status of each attempt).
retry() {
    local n=0
    local max="${FETCH_RETRY_MAX:-5}"
    local delay="${FETCH_RETRY_DELAY:-5}"
    local rc=0
    until "$@"; do
        rc=$?
        n=$((n + 1))
        if [ "$n" -ge "$max" ]; then
            echo "ERROR: '$*' failed after $max attempts (last exit $rc)." >&2
            return "$rc"
        fi
        echo "  attempt $n/$max failed (exit $rc); retrying in ${delay}s…" >&2
        sleep "$delay"
    done
}

# retry_git_clone_pinned URL DEST PIN
#   Resilient "clone (shallow) at, or re-pin to, PIN" idiom shared by the
#   git-based suite fetchers. If DEST already holds a checkout: no-op when it is
#   already at PIN, else retried `fetch` + detached checkout. Otherwise: retried
#   shallow clone, then a retried `fetch` + detached checkout when the default
#   branch HEAD is not PIN. Echos progress; relies on the caller having set
#   `set -euo pipefail`.
retry_git_clone_pinned() {
    local url="$1" dest="$2" pin="$3"
    if [ -d "$dest/.git" ]; then
        local have
        have="$(git -C "$dest" rev-parse HEAD)"
        if [ "$have" = "$pin" ]; then
            echo "$(basename "$dest") already at pinned commit $pin — nothing to do."
            return 0
        fi
        echo "$(basename "$dest") present at $have, re-pinning to $pin…"
        retry git -C "$dest" fetch --depth 1 origin "$pin"
        git -C "$dest" checkout --detach "$pin"
        return 0
    fi
    mkdir -p "$(dirname "$dest")"
    echo "Cloning $url (shallow) into $dest…"
    retry git clone --depth 1 "$url" "$dest"
    if [ "$(git -C "$dest" rev-parse HEAD)" != "$pin" ]; then
        retry git -C "$dest" fetch --depth 1 origin "$pin"
        git -C "$dest" checkout --detach "$pin"
    fi
}

# sha256_of PATH
#   The file's sha256 hex digest. Portable across the GNU (`sha256sum`) and
#   BSD/macOS (`shasum -a 256`) spellings.
sha256_of() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$1" | awk '{print $1}'
    else
        shasum -a 256 "$1" | awk '{print $1}'
    fi
}

# fetch_pinned_file URL DEST SHA256 [DESCRIPTION]
#   Content-addressed fetch of ONE pinned file.
#
#   The OFFLINE-ON-HIT property is the point (#4935): when DEST already holds the
#   pinned payload — sha256 == SHA256 — this returns 0 having made NO network
#   call at all. That is what lets a CI job restore DEST from an Actions cache and
#   run the whole lane without reaching the origin host, so third-party downtime
#   can no longer red a gating check. On a MISS it curls (retried) into DEST.tmp,
#   verifies the digest, and installs DEST only then — a mismatched or truncated
#   payload is never installed, so the pin can never silently drift.
#
#   Returns non-zero (never exits) on download failure or digest mismatch so the
#   caller can add its own source-specific diagnostics. `--retry-all-errors`
#   (curl >= 7.71) is required: plain `--retry` skips (56) Recv failure, the
#   dominant mid-transfer failure from CI runner IP ranges. Retry counts honour
#   FETCH_RETRY_MAX / FETCH_RETRY_DELAY, matching `retry` above.
fetch_pinned_file() {
    local url="$1" dest="$2" want="$3"
    local desc="${4:-$(basename "$dest")}"
    if [ -f "$dest" ] && [ "$(sha256_of "$dest")" = "$want" ]; then
        echo "$desc already present (sha256 ok) — nothing to do."
        return 0
    fi
    mkdir -p "$(dirname "$dest")"
    echo "Downloading $desc…"
    if ! curl -sSfL \
            --retry "${FETCH_RETRY_MAX:-5}" --retry-delay "${FETCH_RETRY_DELAY:-5}" \
            --retry-all-errors --retry-connrefused \
            --connect-timeout 30 --max-time 300 \
            -o "$dest.tmp" "$url"; then
        rm -f "$dest.tmp"
        echo "ERROR: could not download $desc after retries." >&2
        echo "  URL: $url" >&2
        return 1
    fi
    local have
    have="$(sha256_of "$dest.tmp")"
    if [ "$have" != "$want" ]; then
        rm -f "$dest.tmp"
        echo "ERROR: $desc checksum mismatch (got $have, want $want)." >&2
        return 1
    fi
    mv "$dest.tmp" "$dest"
    echo "$desc pinned (sha256 ok)."
}
