<!-- [GPT-5.6] sq-44ga1 — orchestration contract for deployment-template validation. -->

# Deployment CI orchestration

This document maps every deployment family to the strongest useful check that can run without
standing cloud credentials. It is the control plane for `deploy/**`; provider templates remain
owned by their provider directories.

Deployment validation is deliberately separate from the native Rust workspace gate. Dedicated
deploy workflows use path filters plus `workflow_dispatch`, and their advisory jobs are not required
checks (the only required check is `ci-fast`). A cloud CLI outage or unavailable provider account therefore cannot turn the
engine workspace red.

## Common smoke contract

Every live smoke that can boot a server must exercise the same minimum sequence:

1. Start the exact image or rendered template under test.
2. Poll the server-specific health endpoint until it is ready:
   `GET /health` for `sparq-server`; `GET /livez` and `GET /readyz` for Solid/LWS.
3. Send one representative request. For `sparq-server`, issue an authenticated SPARQL `ASK`.
   For Solid/LWS, a fully authenticated request needs an external OIDC issuer, so the
   credential-free smoke may use a read plus the fail-closed mutation check instead.
4. Prove the public-write invariant: an unauthenticated SPARQL Update returns 401/403, or an
   anonymous Solid mutation returns 401/403.
5. Stop the workload and print logs on failure.

The smoke token must be generated for the job and passed through the runtime secret mechanism; it
must not be committed to a manifest or printed. Health endpoints remain ungated. A smoke that boots
the bare open-by-default `sparq-server` image proves packaging only; it does not prove the
template-layer authentication contract.

## Orchestration matrix

| Family | Credential-free validation | Live boot and one request | Workflow / entry point |
|---|---|---|---|
| AWS CloudFormation | `cfn-lint` both YAML templates. `aws cloudformation validate-template` is a manual account-backed check because the API call requires IAM credentials. | Provider deployment only; after launch run the common smoke against the ALB HTTPS hostname. | `deploy/aws/README.md`; provider CI remains separate from the Rust gate. |
| Azure Bicep/ARM | Build and lint both Bicep sources, then compare/review the compiled ARM artifacts. | Provider deployment only; after launch run the common smoke against Container Apps HTTPS ingress. | `deploy/azure/README.md`; provider CI remains separate from the Rust gate. |
| GCP Cloud Run | Validate both service YAML documents and assert the Secret Manager reference, dedicated service account, per-server probes, and single-instance annotations. | Provider deployment only; after `gcloud run services replace`, run the common smoke against the `run.app` HTTPS URL. | `deploy/gcp/README.md`; provider CI remains separate from the Rust gate. |
| Terraform | `terraform init -backend=false`, `terraform validate`, and recursive `terraform fmt -check` for the root and all three provider modules; scan for literal credentials and verify sensitive inputs. | A plan/apply requires provider credentials. Run the common smoke after an account-backed apply. | `.github/workflows/deploy-terraform-lint.yml` (`push`/PR path filters + manual dispatch). |
| Helm / Kubernetes | `helm lint`, render SPARQL and Solid/LWS variants, assert fail-closed values, run strict Kubernetes schema validation, and scan for literal credentials. | Where a runner can load the images, install into `kind`, wait on `/health` or `/readyz`, port-forward, then run the common smoke. | `.github/workflows/deploy-lint.yml` (`push`/PR path filters + manual dispatch). |
| Fly.io | Parse both `fly.toml` files and assert HTTPS forcing, per-server probes, required bind/auth posture, and one-Machine configuration. | Provider credentials are required. Run the common smoke after the secret-first CLI flow. | `deploy/paas/README.md`; provider CI remains separate from the Rust gate. |
| Render | Parse both Blueprint YAML files and assert secret inputs/generation, per-server health paths, and one-instance configuration. | Provider credentials are required. Run the common smoke after Blueprint deployment. | `deploy/paas/README.md`; provider CI remains separate from the Rust gate. |
| Railway | Parse both `railway.toml` files and assert per-server health paths, ports, restart policy, and one-replica configuration. | Provider credentials are required. Run the common smoke after variables are installed and the service deploys. | `deploy/paas/README.md`; provider CI remains separate from the Rust gate. |
| Shared `sparq-server` image | Build the release Dockerfile, run it, poll `/health`, and issue a SPARQL `ASK`. This is a packaging smoke of the bare image, not the token-gated template posture. | Runs locally in Docker on relevant Rust or Docker changes. | `.github/workflows/ci.yml` → `scripts/docker-smoke.sh`. |
| Shared Solid/LWS image | Build the release Dockerfile, assert non-root execution, poll `/livez` and `/readyz`, and prove anonymous mutation is rejected. | Runs on release tags before the image is published. | `.github/workflows/lws-container.yml` → `crates/sparq-lws-core/tests/container-smoke.sh`. |

## Provider workflow checklist

A provider-specific deploy job is complete only when all applicable items below are explicit:

- It is in a dedicated deploy workflow, with `workflow_dispatch` and a narrow `deploy/**` path
  filter, and is not a required check.
- Tool versions are pinned; downloaded tools are checksum-verified where the repository installs
  binaries directly.
- Both `sparq-server` and Solid/LWS variants are parsed, rendered, or synthesized.
- Static assertions cover token/secret references, HTTPS ingress, least-privilege identity,
  server-specific health paths, and the current single-instance constraint.
- The secret-hygiene scan rejects committed credential values without rejecting secret-reference
  names.
- No credential-free job claims to have deployed a cloud resource. Account-backed validation is
  documented as manual and should clean up resources in the same run.
- A feasible local Docker or `kind` smoke follows the common contract above and emits logs when it
  fails.

The page at `/deploy` links to these provider-owned assets and repeats the essential posture:
`sparq-server` is open by default at the image layer, while the templates gate it with a token.
Removing that wiring changes the security posture and is not a supported production default.
