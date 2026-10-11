# sparq-lws-authn-saml

The SAML 2.0 authentication suite (`lws10-authn-saml`) of the Linked Web Storage authorization
server: a signed `saml:Assertion`, base64url-encoded, from an identity provider the server
trusts, exchanged for an access token.

`Saml::new` takes the trusted identity providers (entity id to PEM certificate, PEM public key
or public JWK); `Saml::from_env` reads them from the JSON file `SOLID_SERVER_LWS_SAML_IDPS_FILE`
names. In `sparq-lws-core`, the `authn-saml` feature builds it in and offers it when that file
is set. See `skills/solid-lws-server/SKILL.md` for what an assertion must carry.
