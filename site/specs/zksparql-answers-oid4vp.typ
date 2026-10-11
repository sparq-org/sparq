// Informative companion to zksparql-answers.typ: carrying the request and presentation over
// OpenID for Verifiable Presentations 1.0.

#import "_lib/spec.typ": spec-head, sotd, intro-section, references, dfn, note, cite

#set document(title: "Zero-Knowledge SPARQL Answers over OpenID for Verifiable Presentations")
#set text(size: 11pt)
#set par(justify: true)
#set heading(numbering: "1.")

#spec-head()

#intro-section("abstract", "Abstract")[
  How OpenID for Verifiable Presentations 1.0 can carry the query request and answer presentation of #emph[Zero-Knowledge SPARQL Answers: Request and Presentation Data Model] (https://sparq.jeswr.org/specs/zksparql-answers) without changing either, and the gaps that remain. This document is informative.
]

#sotd()

= Use with OpenID for Verifiable Presentations <sec-oid4vp>

This document is informative. It checks the data model against OpenID for Verifiable
Presentations 1.0 #cite("OID4VP") (OpenID4VP), the protocol most wallets and verifiers use to
request and return credentials. The conclusion is that OpenID4VP can carry a query request and
an answer presentation without changing either, by defining a new credential format.
@sec-oid4vp-gaps lists the gaps.

== Mapping

OpenID4VP lets a deployment define a new credential format identifier, with its own `meta`
parameters in a DCQL credential query and its own presentation encoding. The data model would
define one, here called `sparql_answer`:

#table(
  columns: 2,
  align: (left, left),
  table.header[OpenID4VP element][Carries],
  [Authorization request `dcql_query`], [One credential query with `"format": "sparql_answer"`,
    `"multiple": false` and `"require_cryptographic_holder_binding": false`. Its `meta` object
    holds the query request without `challenge` and `domain`, as the exact JSON-LD document.],
  [Authorization request `nonce`], [`challenge`.],
  [Authorization request `client_id`], [`domain`. Over the Digital Credentials API, the origin
    prefixed with `origin:`, as OpenID4VP requires.],
  [DCQL `trusted_authorities`], [Not needed for checking, because the request carries its
    trust requirements. A wallet MAY use it to choose credentials; a `TrustedList` or
    `FederationTrustAnchor` requirement corresponds to its `etsi_tl` or `openid_federation`
    type.],
  [`vp_formats_supported` metadata], [The proof method IRIs each side supports, under the
    `sparql_answer` key.],
  [Response `vp_token`], [`{ "<credential query id>": [ <answer presentation> ] }`, with the
    answer presentation as a JSON-LD object.],
  [Response mode], [Over HTTPS redirects, `direct_post` or `direct_post.jwt`; proofs can be
    large, so the fragment and query modes are unsuitable. Over the Digital Credentials API,
    `dc_api` or `dc_api.jwt`, as OpenID4VP requires there.],
)

Both sides rebuild the query request from `meta`, the nonce and the client identifier, and
compute its request digest. Every property is either carried exactly or set by a fixed rule,
and RDFC-1.0 is deterministic, so both reach the same digest.

A wallet that does not support `sparql_answer` finds no credential of that format and returns
an error (`access_denied` or `vp_formats_not_supported`). It cannot fall back to sending whole
credentials, because no other credential query was made. This matters because DCQL tells
implementations to ignore unknown properties: putting the query in an extra property of an
ordinary credential query would let an unaware wallet return the full credential.

== Gaps <sec-oid4vp-gaps>

+ #strong[A presentation from several credentials.] OpenID4VP defines a presentation as
  "derived from a Credential" and matches each one to a single credential query. An answer
  presentation is derived from a set of credentials. Carrying it as one `sparql_answer`
  presentation works, but stretches that definition. Wallet user interfaces that list "the
  credential being shared" would need to show the query and the answer instead.
+ #strong[Agreeing the input in advance.] An agreed input needs a step before the request in
  which the verifier accepts a dataset commitment. OpenID4VP has no such step. It could be a
  separate earlier exchange, but that is not specified anywhere.
+ #strong[Holder binding.] OpenID4VP requires cryptographic holder binding by default and binds
  replay protection to it. The data model has no holder binding, so the credential query sets
  `require_cryptographic_holder_binding` to false. Replay protection still holds because the
  proof binds the challenge and domain, but OpenID4VP then requires the request to carry
  `state` unless the Digital Credentials API is used.
+ #strong[Selective disclosure rules.] DCQL `claims` and `claim_sets` select claims to reveal.
  They have no meaning for an answer presentation and are omitted. The consent rules written
  for them do not cover a query, so wallets need their own way to show the user what the query
  reveals.
+ #strong[Registration.] `sparql_answer` is not a registered format identifier. Until it is,
  it works only between parties that agree on it, as OpenID4VP allows for deployment-defined
  formats.

None of these gaps needs a change to the query request or the answer presentation. They
concern how the carrying protocol presents and agrees inputs, not what the holder sends.

= References <sec-refs>

#references((
  ("OID4VP", [Terbu, O.; Lodderstedt, T.; Yasuda, K.; Fett, D.; Heenan, J. #emph[OpenID for
    Verifiable Presentations 1.0]. OpenID Foundation, Final Specification, 9 July 2025.
    https://openid.net/specs/openid-4-verifiable-presentations-1_0.html.]),
))
