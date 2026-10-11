# sparq-lws-authn

The interface between the Linked Web Storage authorization server in `sparq-lws-core` and its
authentication suites (LWS 1.0 core section 5.2.3), and what the suites share. A suite in its
own package depends on this crate only, never on the server.

## 🚀 Quickstart

```rust,ignore
use sparq_lws_authn::{Exchange, Suite, Verified};

struct MySuite;

#[async_trait::async_trait]
impl Suite for MySuite {
    fn name(&self) -> &'static str { "my-suite" }
    fn token_types(&self) -> &[&'static str] { &["urn:example:my-token"] }
    fn identifier_types(&self) -> &[&'static str] { &["https"] }
    async fn verify(&self, ex: &Exchange<'_>) -> Result<Verified, String> {
        todo!("check ex.subject_token")
    }
}
// In the server: cfg.register_suite(Box::new(MySuite))?;
```

```bash
cargo test -p sparq-lws-authn
```

## ✨ Features

- `Suite`: the subject token types a suite takes and how it checks one, giving the agent and
  client (`Verified`) the server issues an access token for.
- `Suites`: the suites a server offers. No two take the same token type.
- `jose`: ES256 keys and compact JWS, JWK thumbprints, base64url.
- `parse_datetime`, `SKEW_SECS`, `is_uri`: subject token times and IRIs.

## 📚 Learn more

- `skills/solid-lws-server/SKILL.md`: configuring the suites a server offers.
- [`sparq-lws-authn-saml`](../sparq-lws-authn-saml): a suite built on this crate.

## License

MIT OR Apache-2.0.
