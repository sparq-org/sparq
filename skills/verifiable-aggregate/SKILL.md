---
name: verifiable-aggregate
description: Run the detached research-grade signed-commitment joint count, sum and hidden-total comparison protocol from Rust or Node WebAssembly. Use for verifiable aggregate sessions, not the engine's Shamir MPC integration.
license: MIT
---

# Verifiable joint count and sum

Use `proof-methods/aggregate` (`sparq-verifiable-aggregate`, unpublished) from the
**detached** `proof-methods/Cargo.toml` workspace. It is research-grade and not
externally audited. Read its [scope and protocol](../../proof-methods/aggregate/README.md)
before integrating it; contributors can reconstruct totals even on refusal.

Authenticate the ordered roster and agree the rules before contributing. Generate
a fresh random recipient session ID. Freeze a single complete signed contribution
set, answer once per session and route partials only to contributors. The API does
not provide transport, reliable broadcast, identity discovery or replay storage.

Rust exports `Session::new`, `contribute`, `receive_and_partial`, `finish`,
`verify_outcome`, `simulate_session`, `generate_keys` and the serde wire types. Borrow session and
secret keys at each step. Keep `ContributionState.local`,
`ContributionState.audit_opening`, `PartialDispatch.local` and keys private. Send each `PartialDispatch.messages` entry only to its
`recipient`; each finisher receives its own local partial and one message from
every peer. Public verification needs only the session, signed contributions and
signed outcome. Retain the original `audit_opening` only for an authorized later
opening of that party's input commitment. A finisher signature identifies the
signer; openings or refusal proofs establish the result independently.

Build the optional Node package with `proof-methods/aggregate/build-node.sh <new-dir>`
after installing the exact CLI version pinned in the crate manifest. It refuses
existing output. With the resulting package:

```js
const aggregate = require('/tmp/aggregate-node');
const call = (name, request) => JSON.parse(aggregate[name](JSON.stringify(request)));
const keys = JSON.parse(aggregate.generateKeys());
```

All five step functions take one JSON string and return one JSON string:

| Function | Request | Result |
|---|---|---|
| `newSession` | parameters | session including `parameters`, `rule_digest` |
| `contribute` | `{session,index,keys,input}` | `{contribution,local,audit_opening}` |
| `receiveAndPartial` | `{session,index,keys,contributions,local}` | `{messages,local}` |
| `finish` | `{session,index,keys,contributions,local,messages}` | signed outcome |
| `verifyOutcome` | `{session,contributions,outcome}` | `{verified:true}` |

`keys` is `generateKeys()`'s `secret` object, not its whole result. An optional
base64url seed to `generateKeys` derives reproducible **fixture keys only**;
production identities need unpredictable private keys. Protocol randomness is
always drawn from the OS, including with seeded fixture keys.

Parameters are `{session_id,statistic,min_group,min_contributors?,max_value_bits?,threshold?,roster}`.
`session_id` is 32 random bytes as unpadded base64url; `statistic` is
`count-and-sum` or `count-only`. Defaults are `min_contributors:3` and
`max_value_bits:48`; `min_group:0` disables the group threshold. A roster entry is
`{party_iri,ed25519_public_key,x25519_public_key,computeOnly?}`, with base64url public keys.
A `computeOnly: true` entry must supply zero count and zero sum (or no sum for
count-only); its proof enforces this role. Such entries count toward the contributor
minimum. The requester may use a roster identity and participate in both rounds;
it then learns totals as a contributor.
Each party uses its zero-based roster index. `input` is `{count,sum}`, with an
unsigned count and unsigned canonical decimal **string** sum; count-only uses
`sum:null`. Points, proof bytes, ciphertexts and scalars also use base64url.

For a roster below `min_contributors`, skip partial exchange and call `finish`
with `local:null,messages:[]` after collecting every signed contribution. Missing
or invalid hospital input is an error, not a successful threshold refusal.
Errors throw JSON text `{code,index,check}`; treat `input-not-attested` as a failed
session. `below-minimum-group` exposes a proof with no count or sum opening;
`release` exposes the count and optional sum plus blindings. Only contributor
processes receive partial plaintexts; recipient processes receive public
contributions and outcomes.

The [offline smoke test](../../proof-methods/aggregate/tests/node-smoke.cjs) is a
complete routing example and drives every public step through the generated
package. It uses Node 24 and needs no network.

Optional `threshold` is `{statistic:"count"|"sum",op:">="|"<",value}` with `value`
a canonical unsigned decimal string fitting `u128`. Sum comparisons require
`count-and-sum`. On meeting the group minimum, the finisher returns
`threshold-verdict` with `{verdict,limb_commitments,range_proof}` and no aggregate
opening. The proof establishes the comparison (including a false verdict) and the
group minimum. Below the minimum the normal count-hiding refusal takes precedence.
Contributors can still reconstruct both totals; separate recipients see only
the verdict or refusal.

`simulateSession(paramsJson, partyInputsJson[])` accepts a parameter JSON string
and an array of individual `{keys,input}` JSON strings in roster order. It uses
the real step functions and returns `{session,contributions,partials,outcome}`.
The partials include signed plaintexts and all encrypted peer messages. This is
a simulation of all parties in one process, not a deployment; it handles every
private key and its transcript must never be sent to a separate recipient.
Keep the per-step functions as the independent-party API.

The built package exports TypeScript wire types `SessionParams`, `Contribution`,
`Partial`, `Outcome` (`Release | ThresholdVerdict | BelowMinimumGroup |
TooFewContributors`) and `VerifyResult`, plus the request-related types in
`wire.d.ts`. All functions still use JSON strings. Pack the generated directory
with `npm pack`; install the tarball with `npm install --offline ./package.tgz`.
The package contains wasm, JavaScript glue, declarations, a licence and a standalone
README with exact calls, the project licence and dependency licence notices.
It has no runtime package dependencies or install hooks.
