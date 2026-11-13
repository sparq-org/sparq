# solid-answer-dist

Offline Node 24 packages built for the Solid App Suite answer. They are research-grade and not externally audited.
Each file name carries the sparq commit it was built from. Check every download against SHA256SUMS.

| File | Source | Status |
|---|---|---|
| sparq-verifiable-aggregate-61989f5.tgz | proof-methods/aggregate (PR #7389, merged) | current |
| sparq-zk-result-7a56f943.tgz | js/zk-result from main at 7a56f943 (PR #7392, merged) | current |
| sparq-zk-result-fb601bc16e1e4a92c3693558ca9367fa9405449f.tgz | js/zk-result, pre-merge build | superseded |

sparq-zk-result-7a56f943.tgz bundles the two public-predicate members and test-only issuer keys.
Issuer key generation and the hidden date and integer filter members arrive in a later build.

Install: `npm install --offline ./<file>.tgz`
