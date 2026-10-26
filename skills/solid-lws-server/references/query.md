# Query readable resources

The default `sparql-endpoint` feature mounts `/sparql` in Solid mode and
`/.lws/sparql` in LWS mode. Disabling it removes the route and its catalog entry.
In LWS, `/sparql` remains available as an ordinary client-created resource.

```sh
curl -G https://lws.example/.lws/sparql \
  -H 'Authorization: Bearer ACCESS_TOKEN' \
  -H 'Accept: application/sparql-results+json' \
  --data-urlencode 'query=SELECT ?g WHERE { GRAPH ?g {} }'
```

In Solid mode use `/sparql` and the ordinary Solid-OIDC DPoP headers:
`Authorization: DPoP ACCESS_TOKEN` and `DPoP: FRESH_REQUEST_BOUND_PROOF`.
For `POST`, send a direct `application/sparql-query` body or an
`application/x-www-form-urlencoded` body with a `query` field. Both modes support
`SELECT`, `ASK`, and `CONSTRUCT`; LWS also supports `DESCRIBE`. Result types are
`application/sparql-results+json` for SELECT/ASK and `application/n-triples` for
CONSTRUCT/DESCRIBE. Unsupported Accept types return `406`. LWS refuses Update
parameters, valid Update text and `application/sparql-update` with `405` and
`Allow: GET, POST`; other unsupported body types return `415`.

Each admitted resource has a named graph identified by its resource URI. The
default graph is empty. Query `FROM` clauses and protocol `default-graph-uri` /
`named-graph-uri` parameters select only from this authorized dataset. Either
protocol parameter replaces the query's complete dataset selection.
`FROM <http://www.w3.org/ns/solid/sparql#union-default-graph>` (or that IRI as
`default-graph-uri`) expands to the union of the admitted named graphs.

Solid uses the same per-resource WAC Read decision as LDP GET, with Control
required for ACL resources. Failed enumeration, authorization, byte reads,
media classification or parsing exclude that resource. A reclaimed version is
re-planned and re-authorized; an exhausted race returns `503` and `Retry-After`.
Its snapshot read guard spans enumeration through evaluation. Existing Solid
responses, including its rejection of DESCRIBE and Update, are retained.

LWS evaluates `access::allowed(Read, ...)` live per resource, using the token's
subject and client id, the current dateTime and the resource's format and types.
The owner and creator retain their usual access. Purpose constraints retain the
existing access layer's behavior: there is no request purpose carrier, so they
are unsatisfied. This endpoint defines no new purpose header or token claim.
Revoking a grant changes the next query's view.

LWS traverses containment, omitting `/.lws/`, `/.well-known/` and `.meta` records.
A readable container or non-RDF resource has an empty named graph. A readable
RDF document that cannot be parsed also has an empty named graph. Other failed
reads or authorization checks omit the resource; a failed container enumeration
omits that branch. No placeholder triples are added. Blank node identifiers
are scoped separately for each document, including nested triple terms.

LWS holds the optimistic read side of a storage-wide generation guard through
enumeration, authorization and evaluation. Every tracked store mutation and
logical content/metadata change marks activity and advances the generation;
grant publication and settlement do the same. Queries start only between
logical changes and validate the generation before returning. An overlapping
change discards the whole attempt, including any error, and the query rebuilds
after activity settles. Independent mutations keep their existing concurrency;
queries acquire no per-resource locks.

With `sparql-endpoint`, `LwsConfig::sparql_max_resources` defaults to 10000 and
`LwsConfig::sparql_max_bytes` to 67108864 bytes. `LwsConfig::from_env` reads
`SOLID_SERVER_LWS_SPARQL_MAX_RESOURCES` and `SOLID_SERVER_LWS_SPARQL_MAX_BYTES`,
respectively; both require a positive integer. Only admitted resources and
admitted RDF bytes count, including malformed RDF bytes. Crossing the resource
cap returns `507`; crossing the byte cap returns `413`. Both are
`application/problem+json` responses and do not return a partial query result.

`LwsConfig::sparql_timeout` defaults to the server request timeout. `from_env`
uses `SOLID_SERVER_REQUEST_TIMEOUT_SECS` (default 30 seconds; `0` disables it).
Embedders may set an optional duration directly. The deadline covers snapshot
waiting and assembly and is also passed to the engine's cooperative query
budget. Expiry returns `504` problem details.
