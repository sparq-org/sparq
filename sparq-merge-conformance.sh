#!/usr/bin/env bash
# Merges sparq #7341 and #7348 (squash), each pinned to its reviewed head, after resolving
# the review threads that already have a reply (the bot findings, each answered on the PR).
# Then marks #7350 ready without merging it: its zk exact-evaluator lane only runs on a
# non-draft head and must pass before #7350 merges. Already-done steps are skipped.
set -euo pipefail
repo=sparq-org/sparq

resolve_answered_threads() {
  gh api graphql -F n="$1" -f query='query($n:Int!){repository(owner:"sparq-org",name:"sparq"){pullRequest(number:$n){reviewThreads(first:100){nodes{id isResolved comments{totalCount}}}}}}' \
    --jq '.data.repository.pullRequest.reviewThreads.nodes[] | select(.isResolved == false and .comments.totalCount > 1) | .id' |
  while read -r id; do
    gh api graphql -f id="$id" -f query='mutation($id:ID!){resolveReviewThread(input:{threadId:$id}){thread{isResolved}}}' >/dev/null
  done
}

merge() { # merge <pr> <sha> <title> <message>
  if [ "$(gh api "repos/$repo/pulls/$1" --jq .merged)" = true ]; then echo "#$1 already merged"; return; fi
  head="$(gh api "repos/$repo/pulls/$1" --jq .head.sha)"
  if [ "$head" != "$2" ]; then echo "#$1 head is $head, not the reviewed $2; stopping"; exit 1; fi
  resolve_answered_threads "$1"
  if [ "$(gh api "repos/$repo/pulls/$1" --jq .draft)" = true ]; then gh pr ready "$1" -R "$repo"; fi
  gh api -X PUT "repos/$repo/pulls/$1/merge" -f merge_method=squash -f sha="$2" \
    -f commit_title="$3 (#$1)" -f commit_message="$4" --jq '"#'"$1"' " + .message'
}

merge 7341 340632b3b884ca3b8e25a10d642ae5e0c951b2d2 "fix(jsonld): process HTML and remote documents" \
  "Processes HTML script elements and remote documents in sparq-jsonld, and reports Graph ingest separately from the API suites. Addresses #7326."
merge 7348 1a519205ac429dae5ea224f79c21d55faf077279 "feat(shacl): support SHACL 1.2 node-expression forms" \
  "Evaluates the SHACL 1.2 node-expression forms in both feature states, sharing one evaluator with SHACL-AF rules. Addresses #7326."
# #7350: ready only. Its merge comes in a later line, once zk-exact-evaluator is green.
resolve_answered_threads 7350
if [ "$(gh api "repos/$repo/pulls/7350" --jq .draft)" = true ]; then gh pr ready 7350 -R "$repo"; fi
echo "All done."
