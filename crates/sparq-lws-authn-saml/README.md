# sparq-lws-authn-saml

The SAML 2.0 authentication suite (`lws10-authn-saml`) of the Linked Web Storage authorization
server: a signed `saml:Assertion`, base64url-encoded, from an identity provider the server
trusts, exchanged for an access token.

## 🚀 Quickstart

```bash
# Trusted identity providers: entity id -> PEM certificate, PEM public key or public JWK
echo '{"https://idp.example/": "-----BEGIN CERTIFICATE-----..."}' > idps.json
SOLID_SERVER_LWS_SAML_IDPS_FILE=idps.json cargo run -p sparq-lws-core --features authn-saml
cargo test -p sparq-lws-authn-saml
```

## ✨ Features

- `Saml::new` takes the trusted identity providers and checks every key at start-up.
- `Saml::from_env` reads them from the JSON file `SOLID_SERVER_LWS_SAML_IDPS_FILE` names.
- In `sparq-lws-core`, the `authn-saml` feature builds the suite in and offers it when that
  file is set.

## 📚 Learn more

- `skills/solid-lws-server/SKILL.md`: what an assertion must carry.
- [`sparq-lws-authn`](../sparq-lws-authn): the suite interface.

## License

MIT OR Apache-2.0.
