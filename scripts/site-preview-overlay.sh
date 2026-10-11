#!/usr/bin/env bash
# Overlays the hosted PR site previews into an assembled Pages tree at <out>/preview/pr-<N>/.
#
# A preview is the `site-preview-pr-<N>` artifact that .github/workflows/site-preview.yml
# builds (basePath /preview/pr-<N>). This script is the ONE place that decides which
# previews get published, so both deploy paths agree: pages.yml (every main deploy) and
# site-preview-deploy.yml (each preview build). It rebuilds <out>/preview/ from scratch,
# so a closed PR's preview disappears on the next deploy of either kind.
#
# Trust rule: only OPEN PRs whose head branch lives in this repository, and only artifacts
# uploaded by runs whose head repository is this repository. A fork PR's build runs its own
# code, so its artifact must never be served from the project's domain.
#
# Needs GH_TOKEN (actions: read, pull-requests: read) and GITHUB_REPOSITORY.
# Usage: site-preview-overlay.sh <out-dir>
set -euo pipefail

out="${1:?usage: site-preview-overlay.sh <out-dir>}"
repo="${GITHUB_REPOSITORY:?GITHUB_REPOSITORY must be set}"
# GitHub Pages caps a published site at 1 GB; stop adding previews once the tree passes
# this budget (newest PRs first) rather than failing the deploy.
budget_kb="${SITE_PREVIEW_BUDGET_KB:-800000}"

repo_id="$(gh api "repos/$repo" --jq .id)"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

rm -rf "$out/preview"
mkdir -p "$out/preview"

published=()
skipped=()
while read -r pr; do
  [ -n "$pr" ] || continue
  art="$(gh api "repos/$repo/actions/artifacts?name=site-preview-pr-$pr&per_page=50" --jq "
    [.artifacts[] | select(.expired | not) | select(.workflow_run.head_repository_id == $repo_id)]
    | sort_by(.created_at) | last | .id // empty")"
  [ -n "$art" ] || continue

  used_kb="$(du -sk "$out" | cut -f1)"
  if [ "$used_kb" -ge "$budget_kb" ]; then
    skipped+=("$pr")
    continue
  fi

  gh api "repos/$repo/actions/artifacts/$art/zip" > "$work/pr.zip"
  dest="$out/preview/pr-$pr"
  mkdir -p "$dest"
  unzip -q "$work/pr.zip" -d "$dest"
  rm -f "$work/pr.zip" "$dest/CNAME"
  published+=("$pr")
done < <(gh api --paginate "repos/$repo/pulls?state=open&per_page=100" \
           --jq ".[] | select(.head.repo.full_name == \"$repo\") | .number" | sort -rn)

echo "site previews published: ${published[*]:-none}"
if [ "${#skipped[@]}" -gt 0 ]; then
  echo "::warning::site previews skipped (Pages size budget reached): ${skipped[*]}"
fi
if [ -n "${GITHUB_STEP_SUMMARY:-}" ]; then
  echo "Site previews published: ${published[*]:-none}" >> "$GITHUB_STEP_SUMMARY"
fi
