# Version history and trash

The native server can retain data resource states and move deleted data resources to a trash container. Both services are off by default. They require the memory or embedded index backend; an HTTP index with either service enabled is refused at startup.

| Environment variable | Default | Effect |
|---|---|---|
| `SOLID_SERVER_VERSIONS` | unset | `1` retains superseded data resource states and mounts version routes. |
| `SOLID_SERVER_TRASH` | unset | `1` moves deleted data resources to the trash container. |
| `SOLID_SERVER_VERSIONS_MAX` | `20` | Positive maximum retained states per resource, excluding its live state. |
| `SOLID_SERVER_VERSIONS_RETENTION` | `P30D` | Positive day/time XSD duration for retained versions. |
| `SOLID_SERVER_TRASH_RETENTION` | `P30D` | Positive day/time XSD duration for trash entries. |

Retention accepts day, hour, minute and second components, for example `P30D` or `P1DT12H`. Calendar year/month components are not accepted. Each background pass selects and removes a bounded set of expired records, oldest first, and reclaims bytes after removing their index references.

The container image accepts these settings through its environment. Solid Suite deployments enable both services explicitly:

```sh
SOLID_SERVER_VERSIONS=1
SOLID_SERVER_TRASH=1
```

## Versions

Solid serves `GET /.versions/?resource=<absolute-resource-IRI>` as Turtle or expanded JSON-LD. LWS serves `GET /.lws/versions/?resource=<absolute-resource-IRI>` as `application/lws+json`. Percent-encode the query parameter. Each listing includes the retained states and the live resource as the final entry, with its ETag and modification time. Thus an initial state followed by two overwrites lists three states. LWS resources are initially created by POST, as in the existing protocol.

A retained state is read-only at `/.versions/<sha>/<id>` or `/.lws/versions/<sha>/<id>`. Its response includes its retained `ETag`, `Memento-Datetime`, and a `Link` with `rel="original"`. Reads of original resources advertise a `rel="version-history"` link. The catalog advertises `ssd:VersioningService` with an endpoint template containing `{resource}`. Versions are authorized against the original resource's current read permissions.

To restore a version, GET it and PUT its bytes to the live resource with `If-Match` for the current live ETag. This creates another history entry. Containers, ACL documents, LWS metadata and service records are not versioned. At the index record ceiling an overwrite exchanges its oldest retained state for the newly retained state; when no retained slot exists, only the live state is kept. Blob storage admission still applies to newly written bytes.

## Trash

The Solid trash container is `/.trash/`; the LWS trash container is `/.lws/trash/`. GET lists visible entries. GET of a member returns its original bytes and `Link: <original-resource-IRI>; rel="original"`. Solid listings carry `dct:modified` equal to the trash time. LWS members expose it in their linkset's `self.modified` attribute, through `GET <member>.meta`.

Restore a trash member with:

```http
POST /.trash/
Content-Type: application/json

{"restore":"https://pod.example/.trash/<id>"}
```

The response is `201 Created` with `Location` pointing to the original resource in Solid mode, or a fresh identifier under the original parent in LWS mode. The target must be absent and its parent must already exist; otherwise restore returns `409 Conflict`. Restore also requires create permission on the parent. Parents are not created during restore.

DELETE of a trash member purges it. Purge and permanent resource deletion also remove that resource's retained history. Moving a resource into trash keeps its history, so a Solid restore reconnects it. A move emits a live Delete notification; restore emits Create; purge emits no live notification.

Trash entries are visible to a Solid caller with Control on the storage root, or the LWS owner. A deleting agent may also access its own entry when it could read the resource at deletion time. Listings apply the same permissions as member reads, restore and purge.

Empty-container deletion remains permanent. Recursive LWS deletion with `Depth: infinity` also remains permanent and bypasses trash; the service description states that recursive deletion does not use trash. Subtree trash requires a separate atomic storage operation.

## Storage API

`HistoryConfig` configures `CompositeStore::with_history` before router assembly. The builder returns a result and refuses enabled history over an index that is not exclusively held, including the HTTP backend. The index trait exposes `replace_meta_retaining`, `move_to_trash` and `restore_from_trash`; their defaults return `SparqError::Unsupported`, with no sequence of ordinary writes substituted. Memory performs each operation under its index lock; embedded storage commits every effect through one journal transaction. `VersionEntry` and `TrashEntry` carry typed side fields without changing `ResourceMeta`. Private index IRIs are not HTTP resources or containment members.

Permanent deletion also returns its retired history keys from the same index transaction through `delete_resource_retiring`, so `CompositeStore` can reclaim them without a separate history lookup, including when the services are disabled after an earlier run. Backends without history support retain their existing delete path while both services are off.
