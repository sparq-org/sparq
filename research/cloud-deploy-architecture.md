# Cloud-deploy architecture + secure-defaults design (sq-3vjdr)

> Design record for the maintainer-requested epic **sq-3vjdr** — "1-click / minimal-friction
> cloud deployment of the sparq servers to AWS, Azure, GCP + fast PaaS". `[OPUS-4.8]`
>
> **Status:** designed-only. NO templates are implemented here. This record makes the
> architecture + security decisions **once** so each child bead (sq-17rgw AWS, sq-zcou4
> Azure, sq-agolp GCP, sq-sos84 Terraform, sq-0d744 Helm, sq-dwame PaaS, sq-44ga1 site,
> and the shared-substrate work under sq-fqrv4 / sq-lmz40) becomes implementable against a
> single shared contract. `verify`/arm downstream is per-child.

## 0. Scope, premise-check, and what already exists

The epic asks for turnkey deploys of **two deployable servers**:

1. **`sparq-server`** — the W3C SPARQL 1.1 Protocol HTTP server (`crates/sparq-server`).
2. **The native Solid/LWS server** — `crates/sparq-lws-core`'s binary (`sparq-lws-core`,
   `src/main.rs`). This is the **native** deployable, distinct from the loopback
   `@sparq-org/solid-server` wasm/npm development host produced by the sq-6xasp epic (that npm
   package is a `npx`-spin-up dev convenience, NOT the cloud-deploy target — the container
   ships the native binary).

**Premise-check against origin/main (verified, not taken on faith):**

- ✅ **sparq-server image already ships.** `sq-fvzi6` is **CLOSED/merged**: the release
  pipeline (`.github/workflows/release.yml`, job `docker`) builds and pushes a multi-arch
  (`linux/amd64` + `linux/arm64`) OCI manifest to
  **`ghcr.io/${{ github.repository_owner }}/sparq-server`** on every `v*` tag, smoke-tested
  (`scripts/docker-smoke.sh`) and Trivy-scanned *before* push, with SBOM + SLSA provenance.
  The root `Dockerfile` is distroless-nonroot. **This image is the shared substrate — reuse
  it, do not rebuild it.**
- ⚠️ **GHCR owner is `sparq-org`, not `jeswr`.** The live remote is
  `github.com/sparq-org/sparq`, so `${{ github.repository_owner }}` resolves to
  **`sparq-org`** and the canonical ref is **`ghcr.io/sparq-org/sparq-server`**. Several
  in-tree comments (root `Dockerfile` header, `scripts/docker-smoke.sh`) still say
  `ghcr.io/sparq-org/sparq-server` from before the org move — templates MUST use the
  `sparq-org` ref (parameterised; see §1). Flagged as an open question for the maintainer.
- ⏳ **LWS image is NOT yet published.** `sq-lmz40` (child of `sq-fqrv4`) is OPEN and already
  carries a detailed, correct spec: new `crates/sparq-lws-core/Dockerfile`, port **3000**
  via `SOLID_SERVER_BIND=0.0.0.0:3000`, non-root, `/livez`+`/readyz`, anonymous mutation
  rejected, its own `.github/workflows/lws-container.yml`, multi-arch GHCR manifest with
  SBOM/provenance. This design **adopts that contract verbatim** as the LWS image contract
  in §1 — the per-provider templates depend on it. The "wasm build wrinkle that timed out"
  is a red herring for the *container*: the container ships the **native** `sparq-lws-core`
  binary (a normal `cargo build --release`), NOT a wasm build. The wasm timeout belongs to
  the `@sparq-org/solid-server` npm path (sq-6xasp), which is a separate PaaS-ish story (§3.6),
  not the container substrate.
- ✅ **No deploy assets exist yet** — no `deploy/`, no `*.tf`, no `Chart.yaml`, no `fly.toml`.
  This is greenfield template work. The site has a `/download` page (precedent for a
  sibling `/deploy` surface, sq-44ga1).

**Non-goal / honest scope:** this is deployment plumbing. It is **not** a managed service,
not autoscaling tuning, not a security *audit* of the servers themselves. The ZK/MPC estate
is untouched and unmentioned by these templates (no privacy-claims surface here).

## 1. Shared substrate — the two image contracts

Every template consumes **published container images**. Two contracts, verified from code.
Templates MUST treat the registry ref, tag, and org as **parameters** (default to the
values below) so a fork / private registry works without editing the template body.

### 1.1 `sparq-server` image contract (shipping today — `sq-fvzi6`)

| Field | Value (verified) | Source |
|---|---|---|
| Image | `ghcr.io/sparq-org/sparq-server` | `release.yml` docker job |
| Tags | `{version}`, `{major}.{minor}`, `latest` | `docker/metadata-action` config |
| Platforms | `linux/amd64`, `linux/arm64` | multi-arch manifest (`sq-fvzi6`) |
| Container port | **3030** | `EXPOSE 3030`, `ENTRYPOINT --addr 0.0.0.0:3030` |
| Query endpoint | `GET/POST /sparql` | `http.rs` route table |
| **Health** | `GET /health` → body **`ok`** (200), **ungated** | `http.rs:3236` |
| Runs as | non-root (`distroless/cc-debian12:nonroot`) | `Dockerfile` runtime stage |
| Baked env | `SPARQ_ALLOW_REMOTE=1` (permits the `0.0.0.0` bind so it boots) | `Dockerfile` |
| Auth env | `SPARQ_AUTH_TOKEN` (Bearer for writes), `SPARQ_AUTH_TOKEN_READ=1` (also gate reads) | `ServerConfig::from_env`, `http.rs:1025` |
| Other env | `SPARQ_CORS_ALLOW_ORIGIN`, `SPARQ_MAX_BODY_BYTES`, `SPARQ_MAX_CONCURRENT`, `SPARQ_QUERY_TIMEOUT`, `SPARQ_SERVICE_ALLOW`, `SPARQ_ACCESS_AUDIT`, `SPARQ_HEALTH_PROBE_ADDR` | `from_env` |
| Dataset | positional CMD args (`--format turtle /data/x.ttl`); `/data` is a `VOLUME` | `Dockerfile`, `main.rs` |
| Self-probe | `sparq-server --health-probe` (exit 0 healthy) — for `HEALTHCHECK` in shell-less runtimes | `health_probe.rs`, default `127.0.0.1:3030` |

> **⚠️ The load-bearing security fact.** The image is **open by default**: it bakes
> `SPARQ_ALLOW_REMOTE=1` and ships **no auth** unless `SPARQ_AUTH_TOKEN` is set. Anyone who
> can reach the published port can **read AND write** the dataset. The binary logs a loud
> no-auth warning at boot. **This is why "auth ON by default" must be enforced at the
> template layer, not the image layer** (see §2). Templates never expose this image without
> a token wired in.

### 1.2 `sparq-lws-core` image contract (to be built — `sq-lmz40`, adopted here)

The LWS server is **fail-closed by design** (opposite posture to sparq-server): anonymous
mutation is rejected, HTTPS-only IdP/WebID (loopback refused), DPoP required, strict
bidirectional WebID↔issuer check — all **default-on** (`main.rs` config block, verified).

| Field | Value (verified / from sq-lmz40 spec) | Source |
|---|---|---|
| Image | `ghcr.io/sparq-org/sparq-lws-core` (build in `sq-lmz40`) | sq-lmz40 spec |
| Container port | **3000** via `SOLID_SERVER_BIND=0.0.0.0:3000` | `main.rs` ENV_BIND; sq-lmz40 |
| **Health** | `GET /livez` (process up) + `GET /readyz` (ready), **ungated** | `app.rs:56/58`, mounted outside the auth gate |
| Runs as | non-root | sq-lmz40 spec |
| Required prod env | `SOLID_SERVER_BASE_URL` (public https URL), `SOLID_SERVER_TRUSTED_ISSUER` (OIDC IdP), `SOLID_SERVER_AUDIENCE` (defaults to base URL) | `main.rs` config block |
| TLS env | `SOLID_SERVER_TLS_CERT` + `SOLID_SERVER_TLS_KEY` (PEM paths; both-set ⇒ TLS, else plain TCP) | `tls.rs`, `main.rs` |
| Secure defaults | anonymous mutation rejected; HTTPS-only WebIDs; DPoP required; `SOLID_SERVER_BIDIRECTIONAL=strict` | `main.rs` (all default-on) |
| Dev-only escape hatches (NEVER in a prod template) | `SOLID_SERVER_ALLOW_LOOPBACK`, `SOLID_SERVER_SEED_CONFORMANCE`, `SOLID_SERVER_SEED_BENCH`, `SOLID_SERVER_ALLOW_SEED_NONMEMORY` | `main.rs` (documented dev/IT-only) |
| Scale-out seam | `SOLID_SERVER_REPLAY_REDIS_URL` (+ `redis-replay` feature) for a shared DPoP-jti replay store across replicas | `main.rs` |

> **Note the health-path asymmetry.** sparq-server = `/health`; LWS = `/livez` + `/readyz`.
> Templates MUST make the health path a **per-server parameter**, not a constant. This is
> the single most common cross-provider footgun; §2 states it as a rule.

### 1.3 What the LWS image needs (the "wrinkle")

The only real wrinkle for the LWS *container* is that its **prod posture requires an
external OIDC issuer** (`SOLID_SERVER_TRUSTED_ISSUER`) and a **public base URL** to be
known at boot — a Solid RS cannot self-issue. So an LWS template cannot be truly
zero-input the way sparq-server can: the minimum viable LWS deploy still needs the operator
to name a trusted IdP. Templates surface this as a required parameter with a documented
default IdP guidance note (e.g. a public Solid OIDC provider), and the `/readyz` gate is
the seam that keeps a mis-configured instance from serving traffic behind a load balancer.
The `redis-replay` scale-out is the only path to >1 replica with correct replay protection
(single-instance in-memory replay set otherwise) — Helm/ECS multi-replica values MUST wire
Redis or pin `replicas: 1`.

## 2. Secure defaults — copy-into-each-template rules (the hard part)

These are **normative**. Each child template MUST satisfy every applicable rule; the
site/docs child (sq-44ga1) documents them; the CI-smoke child asserts the testable ones.
Phrased as concrete, copy-pasteable rules so no per-provider judgment call is needed.

**R1 — Auth ON by default (no open sparq-server).** A template MUST NOT expose the
sparq-server image to public ingress without an auth token wired in. Options, in preference
order: **(a)** template generates a random token at deploy time into the cloud's secret
store and injects it as `SPARQ_AUTH_TOKEN`; **(b)** template makes the token a **required**
input parameter with no default. A template MUST NOT default sparq-server to the image's
open posture on a public endpoint. (LWS is already fail-closed; no action needed there
beyond R2.)

**R2 — No anonymous write, ever, on a public endpoint.** sparq-server: set
`SPARQ_AUTH_TOKEN` (writes gated) at minimum. LWS: rely on its default fail-closed
anonymous-mutation rejection AND never set a dev seed / `ALLOW_LOOPBACK` escape hatch in a
prod template. The CI smoke MUST prove an **anonymous mutation is rejected** (LWS) / an
**unauthenticated write is 401/403** (sparq-server) — this is the answer-safety invariant
the acceptance tests pin.

**R3 — TLS/HTTPS at the edge.** Terminate TLS at the platform's managed layer wherever it
exists (ALB/ACM on AWS, Container Apps ingress on Azure, Cloud Run's built-in HTTPS, a
`kubernetes.io/tls` Ingress + cert-manager on Helm, the PaaS's automatic certs). Only pass
`SOLID_SERVER_TLS_CERT`/`_KEY` into the LWS container for platforms with **no** managed TLS
(the EC2 fallback, bare Helm without an ingress controller). A bare Bearer token on
plaintext HTTP is sniffable — templates MUST NOT emit a public plaintext sparq-server.
Document HTTPS as mandatory for any credentialed surface.

**R4 — Secrets in the cloud secret store, never in the template.** Tokens, TLS keys, and
Redis URLs go to **AWS Secrets Manager / SSM**, **Azure Key Vault (or Container Apps
secrets)**, **GCP Secret Manager**, **k8s `Secret` (referenced, not inlined; values from
`--set-file`/`--set` or an external secrets operator)**, or the PaaS secret store. No
plaintext secret literal may appear in any committed `.yaml`/`.tf`/`.json`/`.bicep`. The
CI-lint step greps templates for obvious secret literals (see §4).

**R5 — Least-privilege identity.** Each template creates a **dedicated** task/service
identity scoped to only what it needs (pull the image, read *its* secrets, write *its*
logs). AWS: a task role + a separate execution role, no `*` resource on secrets beyond the
deploy's own ARNs. GCP: a dedicated runtime service account, not the default compute SA.
Azure: a user-assigned managed identity with Key Vault `get` on only its secrets. k8s: a
minimal `ServiceAccount`, no cluster-admin. No template grants a wildcard admin role.

**R6 — Ingress only where intended.** Public ingress is opt-in and defaults to the intended
surface only: the app port. Management/metrics/debug surfaces are NOT publicly exposed.
Health endpoints (`/health`, `/livez`, `/readyz`) are reachable by the platform's probe but
need not be public. Default security groups / ingress rules allow inbound only on the app
port from the load balancer, not `0.0.0.0/0` on arbitrary ports.

**R7 — Health-check contract (per-server, parameterised).** Every template wires a
platform health/readiness probe:
- sparq-server → HTTP `GET /health` expect 200 body `ok`.
- LWS → HTTP `GET /readyz` for readiness (503 = not ready → deregister), `GET /livez` for
  liveness.
The health path is a **template parameter defaulted per server**, never a hard-coded
constant shared across both. For shell-less `docker run`/compose/Swarm the sparq-server
image already self-declares a `HEALTHCHECK` via `--health-probe`; orchestrators (k8s,
Container Apps, Cloud Run, ECS) use their own HTTP probe and ignore the image one.

**R8 — Non-root + read-only rootfs where the platform allows.** Both images already run
non-root; templates MUST NOT override to root. Prefer a read-only root filesystem with a
writable `/data` (sparq-server) / store volume mount; never grant added Linux capabilities.

**R9 — Honest posture in the rendered docs.** The `/deploy` surface and every template's
header comment MUST state plainly: "sparq-server is open-by-default at the image layer; this
template gates it with a token — do not remove the token wiring." No template may imply a
security guarantee the servers do not provide. (No ZK/MPC claims arise here.)

## 3. Per-provider approach (one section each; all reference §1 + §2)

Each section lists **resources**, **auth/secrets/TLS wiring (R1–R5)**, **health check
(R7)**, and **user-set parameters**. All consume the published images from §1; none rebuilds
a server. Each supports **both** servers (a `server` = `sparq-server | lws` selector), so a
user deploys one or the other with the same template family.

### 3.1 AWS — `sq-17rgw` (`deploy/aws/`)
- **Resources:** an **ECS Fargate** service (primary) — cluster, task definition (image
  from §1), service, an Application Load Balancer + target group + listener (HTTPS via ACM),
  a security group (R6), CloudWatch log group. Plus an **EC2 fallback** template (a single
  `t3`/`t4g` instance user-data `docker run`s the image, behind an ALB or an Elastic IP).
  Packaged as **CloudFormation** with a **"Launch Stack"** URL (and/or a thin CDK app that
  synthesises the same).
- **Auth/secrets/TLS:** token generated (or required) into **AWS Secrets Manager**; injected
  via the task definition `secrets:` block as `SPARQ_AUTH_TOKEN` / LWS `SOLID_SERVER_*`
  (R1/R4). TLS terminated at the ALB with an **ACM** cert (R3). A dedicated **task role** +
  **execution role** (R5). SG allows inbound 443 from the internet, app port only from the
  ALB SG (R6).
- **Health:** ALB target-group health check → `/health` (sparq-server) or `/readyz` (LWS),
  matcher 200 (R7).
- **User sets:** `server` selector, `image` ref + tag (default §1), an ACM cert ARN (or a
  "no-TLS dev" toggle for the EC2 path), instance/task size, `SOLID_SERVER_TRUSTED_ISSUER`
  + `SOLID_SERVER_BASE_URL` when `server=lws`.

### 3.2 Azure — `sq-zcou4` (`deploy/azure/`)
- **Resources:** an **Azure Container Apps** environment + a Container App (image from §1),
  its managed ingress. Authored in **Bicep** (compiled to ARM) with a **"Deploy to Azure"**
  button (the ARM-template deep link).
- **Auth/secrets/TLS:** token → **Container Apps secrets** (backed by / or a **Key Vault**
  reference with a user-assigned **managed identity**, R4/R5); injected as env
  `secretRef`. TLS/HTTPS is automatic on the Container Apps ingress FQDN (R3). External
  ingress enabled only on the app `targetPort` (R6).
- **Health:** Container Apps liveness + readiness probes → `/health` / `/livez`+`/readyz`
  (R7).
- **User sets:** `server` selector, image ref/tag, min/max replicas, `SPARQ_AUTH_TOKEN`
  (or generated), LWS `SOLID_SERVER_TRUSTED_ISSUER` + `SOLID_SERVER_BASE_URL`.

### 3.3 GCP — `sq-agolp` (`deploy/gcp/`)
- **Resources:** a single **Cloud Run** service (fully managed) from the §1 image. A
  one-click **"Run on Google Cloud"** button (Cloud Run Button / a `gcloud run deploy`
  one-liner in docs).
- **Auth/secrets/TLS:** token → **GCP Secret Manager**, mounted as an env var secret ref
  (R4); a dedicated **runtime service account** with only `secretmanager.secretAccessor` on
  its own secret (R5). Cloud Run provides **HTTPS automatically** on the `run.app` URL (R3).
  App-level auth is the token (R1); optionally note Cloud Run's own IAM `--no-allow-
  unauthenticated` as a stronger front door for internal use.
- **Health:** Cloud Run startup + liveness HTTP probes → `/health` / `/readyz`; set
  `min-instances` ≥ 1 to avoid cold-start on the health path (R7). LWS multi-instance needs
  Redis replay (§1.3) or `max-instances: 1`.
- **User sets:** `server` selector, image ref/tag, region, min/max instances, token,
  LWS issuer/base-URL.

### 3.4 Multi-cloud Terraform module — `sq-sos84` (`deploy/terraform/`)
- **Resources:** a root module with a `provider`/`target` variable selecting an
  **`aws`/`azure`/`gcp`** submodule, each provisioning the same logical shape as §3.1–§3.3
  (Fargate service / Container App / Cloud Run) from the §1 image. Submodules reuse the
  provider's native resources; **no new server build**.
- **Auth/secrets/TLS:** secrets created in the target cloud's secret store via that
  provider's resource (`aws_secretsmanager_secret`, `azurerm_key_vault_secret`,
  `google_secret_manager_secret`), never in `.tfvars` committed to git; token marked
  `sensitive = true` (R4). TLS + identity per the target submodule (R3/R5).
- **Health:** per-target probe wired as in §3.1–§3.3 (R7).
- **User sets:** `target` (aws|azure|gcp), `server`, `image`, region/location, token (or
  `generate_token = true`), LWS issuer/base-URL. `terraform plan` is the CI dry-run (§4).

### 3.5 Helm chart — `sq-0d744` (`deploy/helm/sparq/`)
- **Resources:** a chart deploying a `Deployment` + `Service` + optional `Ingress`
  (+ TLS via cert-manager annotations) + `ServiceAccount` + `Secret` reference, templated
  over `values.yaml` (image, tag, `server` selector, replicas, resources, storage, ingress
  host, TLS, auth). A **plain-manifest quickstart** (`kubectl apply -f`) is generated
  alongside for the no-Helm path.
- **Auth/secrets/TLS:** token referenced from a k8s `Secret` (values via `--set` /
  `--set-file` / external-secrets, never inlined in `values.yaml`, R4); a minimal
  `ServiceAccount` (R5); TLS at the Ingress via cert-manager (R3). LWS multi-replica ⇒
  `values.redis.url` wired to a shared replay store or `replicas: 1` enforced (§1.3).
- **Health:** container `livenessProbe`/`readinessProbe` → `/health` (sparq-server) or
  `/livez`+`/readyz` (LWS), defaulted by the `server` selector (R7).
- **User sets:** `server`, image/tag, `replicaCount`, `ingress.host`, `ingress.tls`,
  `auth.token` (secret ref), LWS `solid.trustedIssuer`/`solid.baseUrl`, `redis.url`.
- **CI:** `helm lint` + `helm template` render + `kubeconform`/`kubeval` schema validation
  (§4); optionally a `kind` cluster boot→`/readyz` smoke.

### 3.6 Fast PaaS — `sq-dwame` (`deploy/paas/`)
- **Resources:** **Fly.io** (`fly.toml` per server, internal port 3030/3000, an HTTP health
  check, an auto-generated app name), **Render** (`render.yaml` Blueprint, a Web Service
  from the §1 image, a health-check path), **Railway** (a template / `railway.json`
  referencing the image). One-click **deploy buttons** per PaaS.
- **Auth/secrets/TLS:** token set as a PaaS **secret/env** (Fly `fly secrets set`, Render
  `sync: false` env, Railway variables — R4); each PaaS provides **automatic HTTPS** on its
  managed domain (R3). R1 enforced by the config declaring the token env as required.
- **Health:** Fly `[[http_service.checks]]` / Render `healthCheckPath` / Railway healthcheck
  → `/health` or `/readyz` (R7).
- **User sets:** the token, `server` choice (two configs or one parameterised), LWS
  issuer/base-URL. **Note:** the `@sparq-org/solid-server` **npm** path (sq-6xasp) is a separate,
  even-lower-friction dev spin-up (`npx`) and is **out of scope for this container-based PaaS
  child** — link it from the site (§3.7) but do not conflate the wasm dev host with the
  native PaaS container.

### 3.7 Site `/deploy` surface — `sq-44ga1` (`site/src/app/deploy/`)
- A statically-exported `/deploy` page (sibling of `/download`) wiring **every** provider
  button + copy-paste one-liners (`gcloud run deploy …`, `fly launch`, `helm install …`,
  `terraform apply`), each annotated with the §2 secure-defaults guidance (R1/R2/R3/R9 in
  plain language) and the honest open-by-default caveat for sparq-server. It **references**
  the button assets the other children produce; it does not author templates.
- Owns the **CI-smoke orchestration doc** for §4 (the checklist of which template is
  validated how). Gates on the site's green static export + lint + typecheck.

## 4. CI-smoke-test contract (non-gating to the native workspace)

All deploy CI lives in a **dedicated workflow** (or a set), **`workflow_dispatch` +
`paths: deploy/**`**, and is **NON-gating** to the native Rust workspace gate (`ci-summary`)
— a cloud-CLI outage or a provider-credential gap must never redden the engine's merge gate.
Per template family, the strongest feasible check that needs **no cloud credentials**:

| Template | Static validation (no creds) | Live boot smoke (where feasible) |
|---|---|---|
| CloudFormation (AWS) | `cfn-lint` + `aws cloudformation validate-template` (offline lint) | — (needs an AWS account; documented manual) |
| CDK (AWS) | `cdk synth` (renders CFN, no deploy) | — |
| Bicep/ARM (Azure) | `bicep build` + `az bicep lint` / ARM-TTK | — |
| Cloud Run (GCP) | `gcloud … --dry-run` / YAML schema lint | — |
| Terraform | `terraform init -backend=false` + `terraform validate` + `terraform fmt -check` + `terraform plan` against a fake/`null` target where possible | — |
| Helm | `helm lint` + `helm template` + `kubeconform`/`kubeval` | **`kind` cluster: `helm install` → poll `/readyz`/`/health` → one authenticated request; assert an anonymous write is rejected (R2)** |
| PaaS (fly/render/railway) | schema-lint the `fly.toml`/`render.yaml`/`railway.json` | — (provider deploy needs creds) |
| **Shared substrate (both images)** | — | **`docker run` the image locally → health → one request → assert unauthenticated write 401/403 (sparq-server) / anonymous mutation rejected (LWS)** — this is the R1/R2/R7 proof and the highest-value smoke. Extends `scripts/docker-smoke.sh` for sparq-server; `sq-lmz40`'s `container-smoke.sh` for LWS. |
| Secret hygiene (R4) | a lint step greps `deploy/**` for plaintext secret literals / high-entropy strings; fails on a match | (all templates) |

**Rule:** the `kind`/`docker run` smokes are the only ones that actually *boot* a server, so
they carry the R1/R2/R7 acceptance proofs. Cloud-provider deploys are validated by
lint/synth/plan only (no standing cloud spend, per the EC2/cost discipline in AGENTS.md).

## 5. Child bead → section map (each child now unambiguously implementable)

Each child owns a **disjoint file-area** (a `deploy/<provider>/` subtree or a distinct
crate/site path) — no two children touch the same file, so the fleet parallelises with zero
merge conflict. `model_tier` = cheapest sound tier.

| Bead | Owns (file-area) | Implements | Acceptance criterion | Tier |
|---|---|---|---|---|
| **sq-lmz40** (under sq-fqrv4) | `crates/sparq-lws-core/Dockerfile`, `.../tests/container-smoke.sh`, `.../README.md`, `.github/workflows/lws-container.yml` | §1.2 LWS image contract | native image reaches `/livez`+`/readyz`, anonymous mutation rejected, non-root, multi-arch GHCR manifest w/ SBOM/provenance | sonnet |
| **sq-17rgw** | `deploy/aws/**` | §3.1 + §2 rules | `cfn-lint`/`validate-template` (and `cdk synth`) pass; token wired from Secrets Manager; ALB health check on `/health`\|`/readyz`; no plaintext secret | sonnet |
| **sq-zcou4** | `deploy/azure/**` | §3.2 + §2 | `bicep build`/lint pass; Container Apps secret + managed identity; auto-HTTPS ingress; probe wired; no plaintext secret | sonnet |
| **sq-agolp** | `deploy/gcp/**` | §3.3 + §2 | Cloud Run YAML/`--dry-run` valid; Secret Manager + dedicated SA; startup/liveness probe; min-instances set | haiku |
| **sq-sos84** | `deploy/terraform/**` | §3.4 + §2 | `terraform validate`+`fmt -check`+`plan` green for each target submodule; secrets `sensitive`, in cloud secret store | sonnet |
| **sq-0d744** | `deploy/helm/**` (+ plain-manifest quickstart) | §3.5 + §2 | `helm lint`+`template`+`kubeconform` pass; **`kind` boot→`/readyz`→authed request→anonymous-write-rejected** smoke; secret referenced not inlined | sonnet |
| **sq-dwame** | `deploy/paas/**` (`fly.toml`, `render.yaml`, `railway.json`) | §3.6 + §2 | each config schema-lints; token declared as required secret; health check path set; auto-HTTPS documented | haiku |
| **sq-44ga1** | `site/src/app/deploy/**` (+ the deploy-CI orchestration doc) | §3.7 + §4 | green static export + lint + typecheck; page renders every button + the §2 secure-defaults guidance incl. the open-by-default caveat | haiku |

Dependency edges (real ordering only; everything else parallelises):

- `sq-lmz40 → {sq-17rgw, sq-zcou4, sq-agolp, sq-sos84, sq-0d744, sq-dwame}` — every
  provider template that deploys **the LWS server** references the LWS image, which
  `sq-lmz40` must publish first. (The sparq-server side is already unblocked by the merged
  `sq-fvzi6` image.) A provider child MAY land its sparq-server path first and add the LWS
  path once `sq-lmz40` is in; but to keep beads atomic, sequence `sq-lmz40` ahead.
- `{all provider children} → sq-44ga1` — the site wires the buttons the provider children
  produce, so it lands last (or renders "coming soon" for any not-yet-merged button).

No cross-child file collisions: `deploy/aws`, `deploy/azure`, `deploy/gcp`,
`deploy/terraform`, `deploy/helm`, `deploy/paas`, `site/src/app/deploy`, and the
`crates/sparq-lws-core/**` image files are mutually disjoint. Disjointness holds.

## 6. Open questions for the maintainer (steer post-hoc; not blocking)

1. **GHCR org ref.** Confirm the canonical published ref is `ghcr.io/sparq-org/sparq-server`
   (the live remote resolves `${{ github.repository_owner }}` to `sparq-org`), and update the
   stale `ghcr.io/jeswr/...` comments in the root `Dockerfile` + `scripts/docker-smoke.sh`.
   Templates default to the `sparq-org` ref, parameterised.
2. **sparq-server open-by-default vs. R1.** This design enforces "auth ON" at the template
   layer (the image stays open-by-default because bare `docker run` needs to boot). If the
   maintainer prefers the *image* to fail-closed on a public bind, that's a separate
   sparq-server change (out of this epic's scope) — flag if desired.
3. **LWS requires an external OIDC IdP.** An LWS deploy cannot be truly zero-input (it needs
   `SOLID_SERVER_TRUSTED_ISSUER`). Confirm the recommended default IdP guidance to put on the
   `/deploy` page, or whether to ship a dev-only "loopback IdP" note (kept out of prod
   templates per R2).
4. **Standing cloud spend.** The CI contract (§4) deliberately does **no** standing
   cloud-provider deploy (lint/synth/plan only) to avoid cost/credential coupling. Confirm
   that's the desired posture vs. an occasional credentialed end-to-end deploy smoke.

---

*Authored by a SPARQ agent (Fable tier, running on Opus 4.8). Grounded against origin/main
`crates/sparq-server`, `crates/sparq-lws-core`, `Dockerfile`, `release.yml`,
`scripts/docker-smoke.sh` — verified, not assumed. Designed-only: no template is implemented
here. Per-child `verify`/arm is downstream.*
