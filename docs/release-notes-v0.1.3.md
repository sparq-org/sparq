# sparq v0.1.3 — complete-release recovery candidate

[GPT-6] Prepared 2026-09-21. This candidate includes the Bash 3.2 GUI staging fix
from PR #6544 after the [incomplete v0.1.2 attempt](release-notes-v0.1.2.md).
It is not yet a published release. Every new artifact must be built from the same
new immutable `v0.1.3` tag and pass the unchanged source, completeness, alias,
checksum and provenance controls. The old v0.1.2 tag and containers remain intact.

The native workspace, Python distribution, all three public npm packages and the
desktop installer metadata target 0.1.3. External dependency versions are unchanged
from v0.1.2; the private LWS crate retains its independent Cargo version. See the
[changelog](../CHANGELOG.md#013---2026-09-21) for changes and the
[release runbook](release.md) for publication order and verification.

The dated [preflight record](release-preflight-v0.1.3.json) records version
availability, not reservation or publication authorization. At preparation time,
the normal cadence guard refused until 2026-09-21 at 22:03:11 UTC. The maintainer
subsequently authorized the fixed v0.1.3 recovery exception in the runbook (§8d):
it requires the exact immutable predecessor/tag inventory and definitive absence of
every publishable crate on crates.io. Fresh authoritative reads must pass; v0.1.4
and other versions retain the full 24-hour interval. Keep the version PR in draft
pending operator review and the hosted checks.

After the corresponding publications and attestations have been verified:

```sh
cargo install sparq-cli --version 0.1.3
npm install @sparq-org/sparq@0.1.3
npm install @sparq-org/solid-server@0.1.3
npm install @sparq-org/eyereasoner-compat@0.1.3
python -m pip install sparq-rdf==0.1.3
docker pull ghcr.io/sparq-org/sparq-server:0.1.3
```

APIs remain experimental and unstable. Desktop installers remain unsigned; the Solid
npm host is for local development, not production deployment. Crates.io builds use
upstream `spargebra` rather than the repository's vendored parser fixes. ZK/MPC
capabilities remain research-grade, with external cryptographer sign-off pending.
New attestations do not retroactively cover the old npm 0.1.1 or container bytes.
