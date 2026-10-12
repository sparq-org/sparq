# sparq-lws-authn-openid

The OpenID Connect authentication suite (`lws10-authn-openid`) of the Linked Web Storage
authorization server: an ID Token from an OpenID Provider the agent's controlled identifier
document names, exchanged for an access token, with a DPoP proof when the token is bound to a
key (Solid-OIDC).

## 🚀 Quickstart

```bash
# On by default in sparq-lws-core (feature `authn-openid`)
cargo run -p sparq-lws-core
# Reserve half of the DPoP replay cache for these providers' identities
SOLID_SERVER_LWS_TRUSTED_OIDC_ISSUERS=https://idp.example/ cargo run -p sparq-lws-core
cargo test -p sparq-lws-authn-openid
```

## ✨ Features

- `OpenId::new` and `OpenId::from_env` (reading `SOLID_SERVER_LWS_TRUSTED_OIDC_ISSUERS`).
- Provider discovery and JWKS fetches through `sparq_lws_authn::fetch`, which refuses
  private and loopback addresses.
- `check_dpop` and `DpopReplay`: DPoP proof checks and a bounded replay cache shared fairly
  between providers and identities.

## 📚 Learn more

- `skills/solid-lws-server/SKILL.md`: what an ID Token must carry.
- [`sparq-lws-authn`](../sparq-lws-authn): the suite interface.

## License

MIT OR Apache-2.0.
