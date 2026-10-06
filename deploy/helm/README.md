# sparq Helm chart (`deploy/helm/sparq/`)

Helm chart for **sparq-server** (W3C SPARQL 1.1 Protocol HTTP server) and
**sparq-lws-core** (native Solid/LWS server, activates once sq-lmz40 ships).
Part of the cloud-deploy epic sq-3vjdr; see
`research/cloud-deploy-architecture.md` for the full design record.

## Secure-defaults posture (R9 — R1/R2/R4/R8)

> sparq-server is **open-by-default** at the image layer: it ships
> `SPARQ_ALLOW_REMOTE=1` and starts with NO auth unless `SPARQ_AUTH_TOKEN` is set.
> Anyone who can reach the published port can read AND write the dataset.
> **This chart enforces auth at the template layer** — do NOT remove the
> `auth.existingSecret` wiring.

The chart implements every applicable secure-default rule from the design record:

| Rule | What the chart does |
|------|---------------------|
| R1 — auth ON by default | `auth.existingSecret` is required for `server=sparq-server`; chart errors at render time if unset |
| R2 — no anonymous write | Token gates writes (set `auth.requireTokenForReads: true` to also gate reads); LWS is fail-closed by default |
| R3 — TLS at the edge | [GPT-5.6] The built-in ingress-nginx Ingress refuses to render without a TLS Secret and forces HTTP-to-HTTPS redirects |
| R4 — secrets in the store, never inlined | Token comes from an **existing** k8s Secret via `secretKeyRef`; no literal in `values.yaml` |
| R5 — least-priv identity | `ServiceAccount` created with `automountServiceAccountToken: false`; no cluster-admin |
| R6 — intended ingress only | [GPT-5.6] The Service is fixed to `ClusterIP`; public traffic can only use the optional TLS Ingress |
| R7 — per-server health path | `/health` (sparq-server) or `/livez`+`/readyz` (lws); parameterised in `_helpers.tpl`, never a shared constant |
| R8 — non-root + read-only rootfs | [GPT-5.6] Non-overridable workload settings use `runAsNonRoot: true`, `readOnlyRootFilesystem: true`, and `capabilities.drop: [ALL]`; `/data` and `/tmp` are separate writable volumes |

## Quick install (sparq-server)

```sh
# 1. Create the auth-token Secret (never commit this token to git)
kubectl create namespace sparq
kubectl create secret generic sparq-auth-token \
  --namespace sparq \
  --from-literal=SPARQ_AUTH_TOKEN=$(openssl rand -hex 32)

# 2. Install the chart
helm install sparq ./deploy/helm/sparq \
  --namespace sparq \
  --set auth.existingSecret=sparq-auth-token

# 3. Port-forward and query
kubectl port-forward -n sparq svc/sparq 3030:3030 # [GPT-5.6] release name is the Service name
curl -H "Authorization: Bearer <your-token>" \
     -H "Content-Type: application/sparql-query" \
     --data "SELECT * WHERE { ?s ?p ?o } LIMIT 10" \
     http://localhost:3030/sparql
```

## Quick install (lws — activates once sq-lmz40 ships)

```sh
# lws is fail-closed: anonymous mutation rejected, DPoP required, HTTPS-only WebIDs.
# An external OIDC IdP and a public base URL are required at boot.
helm install sparq-lws ./deploy/helm/sparq \
  --namespace sparq \
  --create-namespace \
  --set server=lws \
  --set lws.trustedIssuer=https://solidcommunity.net \
  --set lws.baseUrl=https://lws.example.com \
  --set ingress.enabled=true \
  --set ingress.host=lws.example.com \
  --set 'ingress.annotations.cert-manager\.io/cluster-issuer=letsencrypt-prod' \
  --set ingress.tls.enabled=true \
  --set ingress.tls.secretName=lws-tls
```

## Plain-manifest quickstart (no Helm)

See `deploy/helm/quickstart/sparq-server-quickstart.yaml` for a plain workload
manifest. [GPT-5.6] Create the namespace and `sparq-auth-token` Secret with the
commands in its header before applying it; the committed manifest contains no
Secret value or editable credential placeholder.

## Key values

| Value | Default | Description |
|-------|---------|-------------|
| `server` | `sparq-server` | `sparq-server` or `lws` |
| `image.repository` | selector-dependent | [GPT-5.6] Empty selects `ghcr.io/sparq-org/sparq-server` or `ghcr.io/sparq-org/sparq-lws-core`; set only for a fork/private registry |
| `image.tag` | `""` (chart appVersion) | Image tag |
| `replicaCount` | `1` | [GPT-5.6] Fixed for both servers; neither current writable data path is safe to replicate behind one Service |
| `auth.existingSecret` | `""` | **Required** for `server=sparq-server` — name of an existing k8s Secret |
| `auth.tokenKey` | `SPARQ_AUTH_TOKEN` | Key within the Secret |
| `auth.requireTokenForReads` | `false` | Also gate reads with the token |
| `service.type` | `ClusterIP` | [GPT-5.6] Fixed value; direct `LoadBalancer`/`NodePort` exposure is rejected |
| `service.port` | server port | Optional cluster-internal Service port override |
| `ingress.enabled` | `false` | Enable the ingress-nginx Ingress; host and TLS values then become required |
| `ingress.tls.enabled` | `false` | Must be `true` whenever the Ingress is enabled |
| `persistence.enabled` | `false` | PVC for `/data` (sparq-server dataset) |
| `lws.trustedIssuer` | `""` | **Required** for `server=lws` — OIDC IdP URL |
| `lws.baseUrl` | `""` | **Required** for `server=lws` — public https base URL |

Full value reference: `deploy/helm/sparq/values.yaml`.

## Replica safety

[GPT-5.6] The chart rejects `replicaCount` values other than one. `sparq-server`
has an independent writable in-memory store per pod. The planned canonical LWS
image does not compile its opt-in Redis replay backend, and the current LWS data
path has no shared durable blob store, so a Redis URL alone would not make
multiple replicas consistent.

## CI validation

`helm lint`, adversarial `helm template` cases, and strict `kubeconform` validation
are run in `.github/workflows/deploy-lint.yml` on every change to `deploy/helm/**`.
The schema checks the selector, HTTPS URLs, TLS settings, Secret refs,
Service exposure, and LWS replay-store requirement before Kubernetes sees a
manifest. The workflow is separate from the native Rust engine gate (`ci-fast`).

To run locally:

```sh
helm lint ./deploy/helm/sparq --set auth.existingSecret=my-secret
helm template sparq ./deploy/helm/sparq --set auth.existingSecret=my-secret
```
