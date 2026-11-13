# solid-answer-dist

Offline Node 24 packages built for the Solid App Suite answer. They are research-grade and not externally audited.
Each file name carries the sparq commit it was built from. Check every download against SHA256SUMS.

| File | Source | Status |
|---|---|---|
| sparq-verifiable-aggregate-c832012a.tgz | proof-methods/aggregate from main at c832012a (PR #7400, merged) | current |
| sparq-verifiable-aggregate-61989f5.tgz | proof-methods/aggregate (PR #7389, merged) | superseded |
| sparq-zk-result-271502ae.tgz | js/zk-result from main at 271502ae (PRs #7396, #7397, #7399 merged) | current |
| sparq-zk-result-7a56f943.tgz | js/zk-result from main at 7a56f943 (PR #7392, merged) | superseded |
| sparq-zk-result-fb601bc16e1e4a92c3693558ca9367fa9405449f.tgz | js/zk-result, pre-merge build | superseded |

sparq-zk-result-271502ae.tgz adds issuerKeygen() from OS entropy, hidden xsd:date and unsigned integer comparisons,
testIssue (test-only, salt reproducible from a seed), the verify report and the public-input layout in its README.
The issuer-scoped GRAPH form (single issuer per proof) arrives in a later build.

sparq-verifiable-aggregate-c832012a.tgz: only parties that commit inputs count toward `min_contributors` (compute-only roles never count);
a missing or malformed signature field reports the attestation error; adds an ESM entry (`import` and `require` both work).
It was built from the PR head 7ce9843, whose package sources, Cargo.lock and LICENSE are identical to main at c832012a.

Install: `npm install --offline ./<file>.tgz`
