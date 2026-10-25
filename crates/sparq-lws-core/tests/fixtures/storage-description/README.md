# Storage description fixtures

The notification and Activity Streams contexts are vendored from CommunitySolidServer's
`templates/contexts/notification.jsonld` and `activitystreams.jsonld` (MIT).
The legacy notification document is the pre-change description at the fixed test origin.

`cid-context.jsonld` and `lws-context.jsonld` are local discovery-term projections,
not complete normative context documents. The pinned LWS draft's context is unfinished;
the shared context registry does not carry CID or LWS. These mappings follow ADR-008
and the pinned LWS Discovery data model. Tests substitute these files explicitly and
use a parser with no remote loader. The test key is the public P-256 generator and
private scalar one, exclusively for deterministic fixtures.

The upstream JLWS shape uses its own namespace and strict ODRL profile. Tests run
its constraints after explicitly mapping those identifiers to the server's pinned
W3C LWS namespace and AccessProfile. Production context and access-profile versions
are unchanged. The source revision appears in the shape's header.
