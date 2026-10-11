# sparq-lws-authn

The interface between the Linked Web Storage authorization server in `sparq-lws-core` and its
authentication suites (LWS 1.0 core section 5.2.3), and what the suites share.

- `Suite`: the subject token types a suite takes and how it checks one, giving the agent and
  client (`Verified`) the server issues an access token for.
- `Suites`: the suites a server offers. No two take the same token type.
- `jose`: ES256 keys and compact JWS, JWK thumbprints, base64url.
- `parse_datetime`, `SKEW_SECS`, `is_uri`: subject token times and IRIs.

A suite in its own package depends on this crate only, never on the server. See
`skills/solid-lws-server/SKILL.md`.
