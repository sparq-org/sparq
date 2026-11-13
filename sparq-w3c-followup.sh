#!/usr/bin/env bash
# sparq W3C follow-up. Uses your gh login and your forks. Nothing is force-pushed, and every
# step is skipped if it was already done, so it is safe to run again.
#   1. Merges sparq #7335 and #7346 (squash, pinned to their reviewed heads).
#   2. Opens one small CI fix PR on each of w3c/rdf-tests, w3c/json-ld-api and w3c/rdf-canon.
#      Their report-generation workflows pin an exact Ruby patch release that the new
#      ubuntu-26.04 runners no longer provide, so the check fails on every PR, including ours.
#   3. Opens two report PRs on w3c/rdf-tests: RDF/XML 1.2, and the RDF 1.1 suites.
#      (#416 is already approved, so the RDF/XML 1.2 report goes in its own PR instead.)
set -euo pipefail
me="$(gh api user --jq .login)"
work="$(mktemp -d)"
src=https://raw.githubusercontent.com/sparq-org/sparq/wip/earl-reports-20261112/earl
footer=$'\n\n🤖 Generated with [Claude Code](https://claude.com/claude-code)'

# --- 1. sparq merges ------------------------------------------------------------------------
merge() { # merge <pr> <sha> <title>
  if [ "$(gh api "repos/sparq-org/sparq/pulls/$1" --jq .merged)" = true ]; then
    echo "sparq #$1 already merged"; return; fi
  gh api -X PUT "repos/sparq-org/sparq/pulls/$1/merge" -f merge_method=squash -f sha="$2" \
    -f commit_title="$3 (#$1)" -f commit_message="$3" --jq .message
}
merge 7335 36a911999fca88c15ab9e2e86e91e3aa674c1cab "test(geo): ratchet the external GeoSPARQL compliance benchmark"
merge 7346 aa9767cff461125f8e0b6ba5ed1e9bd8df8e64c3 "fix(vc): make the Vercel adapter build on Vercel"

# --- helpers --------------------------------------------------------------------------------
# start <repo> <branch>: returns 1 (skip) if the PR already exists, else clones upstream and branches.
start() {
  if [ -n "$(gh pr list --repo "w3c/$1" --head "$2" --author "$me" --state all --json number --jq '.[].number')" ]; then
    echo "w3c/$1: PR from $2 already exists, skipping"; return 1; fi
  gh repo fork "w3c/$1" --clone=false >/dev/null 2>&1 || true
  git clone -q --depth 1 "https://github.com/w3c/$1.git" "$work/$1-$2"
  cd "$work/$1-$2"
  git checkout -q -b "$2"
}
# finish <repo> <branch> <title> <body>: commits, pushes to your fork, opens the PR.
finish() {
  git add -A && git commit -q -m "$3"
  git push -q "https://github.com/$me/$1.git" "$2"
  gh pr create --repo "w3c/$1" --base main --head "$me:$2" --title "$3" --body "$4$footer"
  cd "$work"
}

# --- 2. CI fixes ----------------------------------------------------------------------------
if start rdf-tests ci-ruby-3.4; then
  sed -i 's/ruby-version: 3\.4\.3$/ruby-version: '"'3.4'"'/' .github/workflows/generate.yml
  sed -i 's/^ruby "3\.4\.3"$/ruby "~> 3.4.3"/' Gemfile
  finish rdf-tests ci-ruby-3.4 "CI: use the latest Ruby 3.4 patch release" \
"The report generation workflow pins Ruby 3.4.3. Since \`ubuntu-latest\` moved to ubuntu-26.04, ruby/setup-ruby only provides the latest patch release of each series there, so the workflow fails at setup with \"Unavailable version 3.4.3 for ruby on ubuntu-26.04\". That fails the check on every open PR (for example #415 and #416), and it also stops the gh-pages regeneration on pushes to main.

This asks setup-ruby for \`'3.4'\` (currently 3.4.12) and relaxes the Gemfile to \`~> 3.4.3\` so Bundler accepts the newer patch. Nothing else changes.

Separately, the \"Add PR to project\" job fails with \"Bad credentials\". That one needs the \`ADD_TO_PROJECT_PAT\` secret refreshed, so it isn't something a PR can fix."
fi

if start json-ld-api ci-ruby-3.2; then
  sed -i 's/ruby-version: 3\.2\.2$/ruby-version: '"'3.2'"'/' .github/workflows/generate.yml
  finish json-ld-api ci-ruby-3.2 "CI: use the latest Ruby 3.2 patch release" \
"The report and manifest generation workflow pins Ruby 3.2.2. Since \`ubuntu-latest\` moved to ubuntu-26.04, ruby/setup-ruby only provides the latest patch release of each series there, so the workflow fails at setup with \"Unavailable version 3.2.2 for ruby on ubuntu-26.04\" on every PR that touches \`reports/\` or \`tests/\` (for example #742).

This asks setup-ruby for \`'3.2'\` (currently 3.2.11). The Gemfile has no Ruby pin, so nothing else changes."
fi

if start rdf-canon ci-ruby-3.4; then
  sed -i 's/ruby-version: 3\.4\.1$/ruby-version: '"'3.4'"'/' .github/workflows/generate.yml
  sed -i 's/^      - uses: stefanzweifel\/git-auto-commit-action@v4$/      - uses: stefanzweifel\/git-auto-commit-action@v4\n        if: github.event_name == '"'push'"' || github.event.pull_request.head.repo.full_name == github.repository/' .github/workflows/generate.yml
  sed -i 's/^ruby "3\.4\.1"$/ruby "~> 3.4.1"/' Gemfile
  finish rdf-canon ci-ruby-3.4 "CI: use the latest Ruby 3.4 patch release" \
"The report generation workflow pins Ruby 3.4.1. Since \`ubuntu-latest\` moved to ubuntu-26.04, ruby/setup-ruby only provides the latest patch release of each series there, so the workflow fails at setup with \"Unavailable version 3.4.1 for ruby on ubuntu-26.04\" on every PR that touches \`reports/\` or \`tests/\` (for example #234, #235 and #236).

This asks setup-ruby for \`'3.4'\` (currently 3.4.12) and relaxes the Gemfile to \`~> 3.4.1\` so Bundler accepts the newer patch.

It also skips the auto-commit step on PRs from forks, the same guard json-ld-api's copy of this workflow already has. A fork PR's token can't push back to the fork's branch, so without the guard the job would still fail at that step once the Ruby setup works. Pushes to main and same-repository PRs behave as before."
fi

# --- 3. rdf-tests report PRs ----------------------------------------------------------------
if start rdf-tests sparq-rdfxml12-earl; then
  curl -fsSL "$src/sparq-rdf12-rdf-xml-earl.ttl" -o rdf/rdf12/rdf-xml/reports/sparq-earl.ttl
  git add rdf/rdf12/rdf-xml/reports/sparq-earl.ttl
  finish rdf-tests sparq-rdfxml12-earl "Add sparq RDF/XML 1.2 implementation report" \
"This adds an EARL report for [sparq](https://github.com/sparq-org/sparq) against the RDF 1.2 RDF/XML test suite. It complements #416 (Turtle, TriG, N-Triples and N-Quads).

- 29 of 31 pass. Two evaluation cases fail and are reported as failed. RDF/XML parsing is an opt-in sparq feature (\`rdfxml\`), which was enabled for this run.
- Built from sparq commit \`77072da807ad658244c8702d24ec774fae3ae35b\` and run against this repository at commit \`7b5afb91857a9c282c03d2e45d3f76485797c901\`. Generated by \`crates/sparq-conformance/earl-report.sh\`, which records each manifest entry's outcome from sparq's own suite runners. Withdrawn or rejected upstream entries are omitted; unsupported entries are reported as failed."
fi

if start rdf-tests sparq-rdf11-earl; then
  for s in turtle trig n-triples n-quads xml mt; do
    curl -fsSL "$src/sparq-rdf11-rdf-$s-earl.ttl" -o "rdf/rdf11/rdf-$s/reports/sparq-earl.ttl"
    git add "rdf/rdf11/rdf-$s/reports/sparq-earl.ttl"
  done
  finish rdf-tests sparq-rdf11-earl "Add sparq RDF 1.1 implementation reports" \
"This adds EARL reports for [sparq](https://github.com/sparq-org/sparq) against the RDF 1.1 test suites.

- Turtle: 313 of 313 pass.
- TriG: 357 of 357 pass.
- N-Triples: 70 of 70 pass.
- N-Quads: 87 of 87 pass.
- RDF/XML: 164 of 166 pass. Two evaluation cases fail and are reported as failed. RDF/XML parsing is an opt-in sparq feature (\`rdfxml\`), which was enabled for this run.
- RDF 1.1 Semantics: 48 of 48 pass.
- Built from sparq commit \`77072da807ad658244c8702d24ec774fae3ae35b\` and run against this repository at commit \`7b5afb91857a9c282c03d2e45d3f76485797c901\`. Generated by \`crates/sparq-conformance/earl-report.sh\`. Withdrawn or rejected upstream entries are omitted; unsupported entries are reported as failed."
fi
echo "All done."
