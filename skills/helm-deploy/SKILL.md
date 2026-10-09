---
name: helm-deploy
description: Deploy sparq-server or configure the planned native sparq-lws-core deployment on Kubernetes with the in-repo Helm chart or plain quickstart manifest. Use when installing, configuring, exposing, upgrading, or validating deploy/helm/sparq, including existing-Secret auth, ingress-nginx TLS, selector-dependent images/ports/probes, persistent data, and single-replica safety constraints.
---

# Deploy sparq with Helm

> Verified against `deploy/helm/sparq` on 2026-07-15. The chart targets
> Kubernetes 1.20 or later and does not build images. The `sparq-server` GHCR image
> is published; the LWS selector becomes runnable only after sq-lmz40 publishes its image.

## Preserve the security boundary

Treat these as invariants:

- Keep `SPARQ_AUTH_TOKEN` in an existing Kubernetes `Secret`. Never pass a token
  through Helm values or commit a `Secret` manifest.
- Keep the built-in Service as `ClusterIP`. Use the optional ingress-nginx
  Ingress for public access; it requires a TLS Secret and forces HTTPS redirects.
- Keep the chart's non-root, read-only-root-filesystem, dropped-capability, and
  disabled-service-account-token settings.
- Use `/health` on port 3030 for `sparq-server`; use `/livez` and `/readyz` on
  port 3000 for `lws`.
- Run exactly one replica of either server. `sparq-server` has an
  unreplicated writable in-memory store. The planned canonical LWS image lacks
  its opt-in Redis replay backend, and Redis alone would not provide a shared blob store.

The `sparq-server` image is open by default at the image layer because it bakes
`SPARQ_ALLOW_REMOTE=1`. The chart refuses to render that server without an auth
Secret. Do not remove the token wiring. The token always gates writes; set
`auth.requireTokenForReads=true` to gate reads too.

## Install sparq-server privately

Create the Secret before installing the chart:

```sh
kubectl create namespace sparq
kubectl create secret generic sparq-auth-token \
  --namespace sparq \
  --from-literal=SPARQ_AUTH_TOKEN="$(openssl rand -hex 32)"

helm upgrade --install sparq ./deploy/helm/sparq \
  --namespace sparq \
  --set auth.existingSecret=sparq-auth-token
```

This creates only a `ClusterIP` Service. Test it over a local port-forward:

```sh
kubectl port-forward --namespace sparq svc/sparq 3030:3030
curl http://127.0.0.1:3030/health
```

## Expose sparq-server over HTTPS

Use the built-in Ingress only with ingress-nginx and a pre-provisioned or
cert-manager-managed TLS Secret:

```sh
helm upgrade --install sparq ./deploy/helm/sparq \
  --namespace sparq \
  --set auth.existingSecret=sparq-auth-token \
  --set auth.requireTokenForReads=true \
  --set ingress.enabled=true \
  --set ingress.host=sparq.example.com \
  --set-string 'ingress.annotations.cert-manager\.io/cluster-issuer=letsencrypt-prod' \
  --set ingress.tls.enabled=true \
  --set ingress.tls.secretName=sparq-tls
```

For a different Ingress or Gateway controller, leave `ingress.enabled=false`
and create a controller-specific HTTPS-only route to the `ClusterIP` Service.

## Install LWS

Use this path only after sq-lmz40 publishes `ghcr.io/sparq-org/sparq-lws-core`.
Supply HTTPS URLs for the public base URL and trusted OIDC issuer. Selecting
`server=lws` automatically selects `ghcr.io/sparq-org/sparq-lws-core`, port
3000, `SOLID_SERVER_BIND=0.0.0.0:3000`, `/livez`, and `/readyz`.

```sh
helm upgrade --install sparq-lws ./deploy/helm/sparq \
  --namespace sparq-lws \
  --create-namespace \
  --set server=lws \
  --set lws.trustedIssuer=https://idp.example.com \
  --set lws.baseUrl=https://pod.example.com \
  --set ingress.enabled=true \
  --set ingress.host=pod.example.com \
  --set ingress.tls.enabled=true \
  --set ingress.tls.secretName=lws-tls
```

Do not scale this release horizontally. A future image and chart revision must
provide both shared replay protection and a shared durable data/blob path before
raising `replicaCount`.

## Use persistence and private images

- Set `persistence.enabled=true` to create a PVC mounted at `/data`, or also set
  `persistence.existingClaim` to use a pre-provisioned PVC.
- Leave `image.repository` empty for the selector-dependent canonical image.
  Set `image.repository`, `image.tag`, and `imagePullSecrets` for a fork, pinned
  release, or private registry. The default `latest` tag uses pull policy
  `Always`; use `IfNotPresent` only with an immutable tag.
- Set `service.port` only to change the cluster-internal Service port. The
  container port and health contract remain selector-dependent.

## Use the plain manifest

Follow the header in
`deploy/helm/quickstart/sparq-server-quickstart.yaml`. Create the namespace and
`sparq-auth-token` Secret separately, then apply the file. It intentionally
contains no Ingress and no Secret object.

## Validate changes

```sh
helm lint ./deploy/helm/sparq --set auth.existingSecret=test-secret
helm template sparq ./deploy/helm/sparq \
  --set auth.existingSecret=test-secret > /tmp/sparq.yaml
kubeconform -strict -summary -exit-on-error /tmp/sparq.yaml
```

Read `deploy/helm/README.md` and `deploy/helm/sparq/values.yaml` for the complete
value reference. Unsafe combinations fail during `helm lint` or `helm template`
through `values.schema.json`.
