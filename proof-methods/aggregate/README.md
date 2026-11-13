# Verifiable joint aggregation

Research-grade, not externally audited. This detached, unpublished crate computes
an unsigned count and optional sum across an authenticated roster. Each input has
an Ed25519-signed Pedersen commitment and a Bulletproofs range proof. Recipients
verify releases, hidden-total threshold verdicts or minimum-group refusals.

It is separate from `sparq-mpc`, whose Shamir field, engine integration, native
simulated-party API and honest-majority assumptions differ. This protocol uses
Ristretto additive shares, public evidence and independent contributor steps.
The root workspace, root lockfile and engine wasm bundle are unchanged.

## Scope and trust

An input is intended to remain hidden from a coalition missing at least one of
the other parties, beyond what totals and the coalition's own inputs reveal.
After exchanging partials, contributors can reconstruct both totals, including
on refusal or a threshold verdict. A separate recipient receives only released
values, a comparison verdict or refusal. A recipient that is also a contributor
learns the totals as a contributor. Public rules, roster and proof metadata remain
visible. `computeOnly: true` slots prove zero count and sum and count toward the
contributor minimum; authenticate such roles before agreeing the rules.

Invalid protocol messages abort with a check and roster index; there is no
guaranteed output. A transport error's index is not transferable blame evidence.
Signed commitments fix contributed values for authorized later audit; they do
not establish truth or disjoint counts. The caller authenticates key bindings,
agrees one complete input set, uses fresh recipient-chosen random session IDs,
answers once per session, bounds incoming payloads and routes partials privately.
There is no network transport, reliable broadcast, key discovery or replay store.
Keep keys, local shares, audit openings and partials out of recipient messages
and logs. Inconsistent input sets or active attacks can cause aborts.

## Node and TypeScript

The vendorable package contains wasm, CommonJS glue, declarations and this guide;
it also carries the project licence and dependency licence notices.
installation needs neither Rust nor network: `npm install --offline ./package.tgz`.
Use Node 24. Exported wire types include `SessionParams`, `Contribution`, `Partial`,
`Outcome` (`Release | ThresholdVerdict | BelowMinimumGroup | TooFewContributors`)
and `VerifyResult`, plus `Session`, `PartyInput`, `Simulation` and `SignedOutcome`.
They describe parsed JSON; the step functions accept and return JSON strings.

```js
const a = require('sparq-verifiable-aggregate');
const { randomBytes } = require('node:crypto');
const call = (name, request) => JSON.parse(a[name](JSON.stringify(request)));
const keys = Array.from({ length: 3 }, () => JSON.parse(a.generateKeys()));
const params = {
  session_id: randomBytes(32).toString('base64url'), statistic: 'count-and-sum',
  min_group: 4, threshold: { statistic: 'sum', op: '>=', value: '20' },
  roster: keys.map((k, i) => ({ party_iri: `urn:party:${i}`,
    ed25519_public_key: k.ed25519_public_key,
    x25519_public_key: k.x25519_public_key, computeOnly: i === 2 })),
};
const partyInputs = keys.map((k, i) => ({ keys: k.secret,
  input: { count: i === 2 ? 0 : 2, sum: i === 2 ? '0' : '10' } }));
const simulation = JSON.parse(a.simulateSession(JSON.stringify(params),
  partyInputs.map((p) => JSON.stringify(p))));
const verified = call('verifyOutcome', { session: simulation.session,
  contributions: simulation.contributions, outcome: simulation.outcome });
```

`simulateSession(paramsJson, partyInputsJson[])` simulates all parties in one
process through the same message functions. It is not a deployment: it handles
all private keys and returns signed public contributions, private signed partials,
all encrypted partial messages and an outcome. Never send its transcript to the
recipient. Real parties use these steps, each holding only its own private state:

```js
const session = call('newSession', params);
const states = partyInputs.map((p, index) => call('contribute', { session, index, ...p }));
const contributions = states.map((s) => s.contribution);
const partials = keys.map((k, index) => call('receiveAndPartial', { session, index,
  keys: k.secret, contributions, local: states[index].local }));
const index = 2; // The requester can also be a contributor and finisher.
const outcome = call('finish', { session, index, keys: keys[index].secret,
  contributions, local: partials[index].local,
  messages: partials.flatMap((p) => p.messages.filter((m) => m.recipient === index)) });
call('verifyOutcome', { session, contributions, outcome });
```

With a roster below `min_contributors`, skip partial exchange and call `finish`
with `local: null, messages: []`. Every roster member must still attest its input;
missing, duplicate, unknown-key or invalid contributions yield `input-not-attested`.
Errors throw JSON text `{code,index,check}`. `generateKeys(seed?)` accepts an optional
base64url fixture seed; never use predictable seeds for production identities.
All protocol randomness comes from the OS, even with fixture keys.

## Rules, encoding and checks

Defaults: `min_contributors: 3`, `max_value_bits: 48`; `min_group: 0` disables the
group rule. Counts are unsigned 32-bit values; sums accept widths through 64.
The roster accepts up to 1024 distinct identities and keys. `statistic` is
`count-and-sum` or `count-only` (omit sums or use `sum: null` for count-only).
Sum inputs, released sums and threshold values are canonical unsigned decimal
strings; threshold values fit `u128`. Thresholds accept `count` or `sum`, and
`>=` or `<`; a sum comparison requires sum inputs. With a threshold, a successful
outcome releases only a verdict and proves the group minimum without opening CNT.
Below the group minimum it returns a count-hiding refusal instead. Count-only
messages omit sum fields. JSON bytes use unpadded base64url, points are compressed
Ristretto and scalars are canonical little endian.

G is the Ristretto basepoint; H hashes `sparq-verifiable-aggregate/v1/H` with SHA-512.
Complement range proofs enforce separate count/sum bounds and compute-only zeros.
Threshold proofs use hidden limbs to support aggregate integers beyond one proof
value's width. Rules, signatures, transcripts and AEAD bind the session and rules;
partials and outcomes also bind the ordered complete signed contribution set.
X25519 ephemeral keys, HKDF-SHA256 and ChaCha20-Poly1305 protect contributor messages.
The Rust API exports `Session::new`, `contribute`, `receive_and_partial`, `finish`,
`verify_outcome`, `simulate_session`, `generate_keys` and serde wire types.

## Build from source

Run `cargo test --manifest-path proof-methods/Cargo.toml -p sparq-verifiable-aggregate`.
Install the exact CLI version pinned in `Cargo.toml`, then run
`proof-methods/aggregate/build-node.sh /tmp/aggregate-node` (destination must be new).
Run `AGGREGATE_PACKAGE=/tmp/aggregate-node node --test proof-methods/aggregate/tests/node-smoke.cjs`.
The optional `wasm` feature enables JSON bindings and JavaScript entropy.
`AGGREGATE_EVIDENCE` optionally selects a local evidence file outside the repository.
