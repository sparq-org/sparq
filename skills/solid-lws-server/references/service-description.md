# Storage service catalog

`describe::{ServiceCatalog, ServiceEntry, Capability}` is the shared boot-time
registration API. `service` and `capability` append entries, `extend` combines
catalogs, and `services()` / `capabilities()` inspect them. A service carries a
stable `fragment`, a `type_term`, an absolute `endpoint`, and a JSON object of
`extras`. A capability carries `type_term` and `extras`. Use absolute IRIs for
extension types and properties; the built-in LWS context terms are also accepted.
The builder is deployment metadata: register an entry when mounting its handler.

`app::build_router_with_catalog(state, catalog)` and
`lws::router_with_catalog(store, config, catalog)` accept additional registrations
and retain the built-in entries. Mount each additional endpoint on the returned
router. Existing `build_router` and `lws::router` use an empty additional catalog.
`LwsState::catalog` and `NotifyState::catalog` retain the assembled catalog.

```rust,ignore
use serde_json::Map;
use sparq_lws_core::describe::{ServiceCatalog, ServiceEntry};

let mut catalog = ServiceCatalog::default();
catalog.service(ServiceEntry {
    fragment: "custom".into(),
    type_term: "https://example.org/vocab#CustomService".into(),
    endpoint: "https://pod.example/custom".into(),
    extras: Map::new(),
});
// Pass this catalog to the selected router constructor, then mount /custom.
```

Solid's two representations are one RDF graph, serialized by the RDF writers.
The storage root is both `pim:Storage` and `lws:Storage`; `did:service` links
service nodes with `rdf:type` and `did:serviceEndpoint`. `lws:capability` links
capability nodes. The extension namespace is `https://jeswr.org/ns/solid-sd#`.
`describe::extension_context()` returns its inline mappings.

The deployed Solid entries are `lws:StorageRoot`, `lws:StorageDescription`,
`ssd:SolidNotificationService`, and (when mounted) `ssd:SparqlQueryService`.
The notification entry names the `notify:WebSocketChannel2023` channel type with
`ssd:channelType`. The original `notificationChannel` array, `channelType`, and
`subscriptionService` keys remain in compact JSON-LD, whose first context is the
standard notifications context followed by inline discovery mappings. The graph
includes `notify:subscription` from the storage root to the subscription service
and `notify:channelType` from that service to `notify:WebSocketChannel2023`.
The query entry carries the ACQ profile and is also linked directly with
`http://www.w3.org/ns/solid/sparql#endpoint`. Root GET/HEAD responses carry that
same link on allowed and denied reads. Discovery does not inspect resource ACLs.
Solid advertises `ssd:AccessControl` with `ssd:WAC`, and `ssd:PatchSupport` with
`ssd:format` nodes: `ssd:source "text/turtle"` and `ssd:patchType "text/n3"`,
`"application/sparql-update"`, derived from the handler's accepted patch formats.
The graph is available as `text/turtle` and `application/ld+json`; Solid mode does
not serve `application/lws+cid`.

LWS retains its root, authorization server, grants, requests, notifications, type
index and type search entries, and adds `lws:StorageDescription` pointing at the
storage URI. `conformsTo` names `https://www.w3.org/TR/lws10-core/`. Its first two
contexts stay CID v1 then LWS v1; the third is an inline extension mapping, with
no extension context URL to retrieve. `ssd:PatchSupport.mediaType` maps
`application/json`, `application/ld+json`, and `application/linkset+json` to `["application/json-patch+json"]`. This applies to data documents and
linksets; containers remain server-managed. The data-document PATCH handler also
accepts other media types with a `+json` structured syntax suffix, without listing
media-type patterns in the catalog.
`ssd:RecursiveDelete` reflects `DELETE` with `Depth: infinity`.
NotificationService's `subscriptionType` is currently `["WebhookSubscription"]`.
Neither mode advertises move, CRDT sync, versions, trash, or unmounted channels.
