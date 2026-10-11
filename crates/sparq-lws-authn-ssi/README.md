# sparq-lws-authn-ssi

The self-issued authentication suites of the Linked Web Storage authorization server
(`lws10-authn-ssi-did-key` and `lws10-authn-ssi-cid`): a JWT the agent signs itself, exchanged for an access token. Its subject is
a did:key (the key comes from the identifier) or an HTTPS controlled identifier (the key comes
from the identifier's document).

## 🚀 Quickstart

```bash
# On by default in sparq-lws-core (feature `authn-ssi`)
cargo run -p sparq-lws-core
# Offer only the self-issued suites
SOLID_SERVER_LWS_AUTHN=ssi cargo run -p sparq-lws-core
cargo test -p sparq-lws-authn-ssi
```

## ✨ Features

- `Ssi`: the suite, for a server's `sparq_lws_authn::Suites`.
- did:key subjects with P-256 (ES256) or Ed25519 (EdDSA) keys; `kid` must name the DID
  document's one verification method.
- Controlled identifier subjects: the document is fetched under the server's fetch policy, and
  `kid` must name an authentication method the subject controls that is neither revoked nor
  expired.
- `self_issued`: `sub = iss = client_id`, an `aud` naming the authorization server, and valid
  times.

## 📚 Learn more

- `skills/solid-lws-server/SKILL.md`: configuring the suites a server offers.
- [`sparq-lws-authn`](../sparq-lws-authn): the suite interface.

## License

MIT OR Apache-2.0.
