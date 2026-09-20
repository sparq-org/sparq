// [SONNET-4.6] sq-zcou4 — Azure Container Apps: sparq-server
// [GPT-5.6] Security/correctness review: Key Vault-backed auth, valid ACA schema, genuine ARM output.
//
// SECURE-DEFAULTS NOTICE (R1/R9):
// The sparq-server image is open-by-default at the image layer (bakes SPARQ_ALLOW_REMOTE=1,
// no auth). Anyone reaching the published port can read AND write the dataset.
// This template enforces auth ON at the template layer:
//   - SPARQ_AUTH_TOKEN is stored in Key Vault and exposed through a Container Apps secretRef.
//   - Do NOT remove the secretRef wiring. Do NOT add a default token value here.
//
// DECISION LOG (sq-zcou4):
//   - Azure Container Apps rather than Container Instances or managed Kubernetes: serverless,
//     no cert/IP management needed — aligns with R3 requirement for managed TLS.
//   - A dedicated Key Vault stores the token; Container Apps resolves a versionless Key Vault
//     reference through its user-assigned managed identity (R4/R5).
//   - The vault contains only this app's token and its access policy grants the identity only
//     secrets/get. No subscription/resource-group wildcard role is created (R5).
//   - External HTTPS-only ingress on targetPort 3030 only; no management ports public (R6).
//   - Container Apps auto-provisions a managed TLS cert on the *.azurecontainerapps.io FQDN
//     (R3). Custom domain TLS follows the same managed path.
//   - Both replica bounds are pinned to 1: every replica owns an independent in-memory dataset,
//     so horizontal scaling requires a shared persistent backing store this template does not provide.
//   - sparq-server runs non-root in the distroless image; Container Apps' 2024-03-01 API has no
//     container securityContext/readOnlyRootFilesystem property, so the template does not emit an
//     invalid no-op security block (R8 applies where the platform supports it).

metadata gpt56Review = '[GPT-5.6] Corrected by the stronger-tier IaC security review.'
metadata securityNotice = 'sparq-server is open-by-default at the image layer; this template gates it with a Key Vault token. Do not remove the token wiring.'

@description('Container Apps Environment name — created by this template.')
param environmentName string = 'sparq-server-env'

@description('Container App name.')
param containerAppName string = 'sparq-server'

@description('Dedicated Key Vault name. The default is stable and globally unique for this deployment.')
@minLength(3)
@maxLength(24)
param keyVaultName string = 'sparq-${uniqueString(subscription().id, resourceGroup().id, containerAppName)}'

@description('Azure region to deploy into.')
param location string = resourceGroup().location

@description('''
  Full image reference (registry/image:tag).
  Defaults to the canonical GHCR image. Pin a specific version in production
  (e.g. ghcr.io/sparq-org/sparq-server:0.1.2).
''')
param imageRef string = 'ghcr.io/sparq-org/sparq-server:latest'

@description('HTTP path used by startup, liveness, and readiness probes (R7).')
@minLength(1)
param healthPath string = '/health'

@description('''
  SPARQ_AUTH_TOKEN bearer value.
  REQUIRED — a strong random secret (e.g. openssl rand -hex 32).
  Stored in the dedicated Key Vault; never logged or exposed in template outputs. (R1/R4)
''')
@secure()
@minLength(32)
param authToken string

@description('Minimum replica count. Pinned to 1 for the single-writer in-memory server; raise only after wiring a shared persistent backing store.')
@minValue(1)
@maxValue(1)
param minReplicas int = 1

@description('Maximum replica count. Pinned to 1 for the single-writer in-memory server; raise only after wiring a shared persistent backing store.')
@minValue(1)
@maxValue(1)
param maxReplicas int = 1

@description('''
  Set to "1" to also require the auth token for read (SELECT/CONSTRUCT) queries.
  Leave empty to allow unauthenticated reads while gating writes.
''')
@allowed(['', '1'])
param authTokenRead string = ''

@description('Value for SPARQ_CORS_ALLOW_ORIGIN (e.g. https://example.com). Leave empty to disable.')
param corsAllowOrigin string = ''

@description('Maximum concurrent SPARQL query workers (SPARQ_MAX_CONCURRENT).')
@minValue(1)
@maxValue(256)
param maxConcurrent int = 16

@description('Per-query timeout in seconds (SPARQ_QUERY_TIMEOUT).')
@minValue(5)
@maxValue(300)
param queryTimeoutSeconds int = 30

@description('CPU allocation per replica. Memory is derived to keep a valid Container Apps consumption profile.')
@allowed([
  '0.25'
  '0.5'
  '0.75'
  '1.0'
  '1.25'
  '1.5'
  '1.75'
  '2.0'
])
param cpuCores string = '0.5'

var memoryByCpu = {
  '0.25': '0.5Gi'
  '0.5': '1Gi'
  '0.75': '1.5Gi'
  '1.0': '2Gi'
  '1.25': '2.5Gi'
  '1.5': '3Gi'
  '1.75': '3.5Gi'
  '2.0': '4Gi'
}

// ---------------------------------------------------------------------------
// User-assigned managed identity (R5 — least privilege)
// ---------------------------------------------------------------------------
resource managedIdentity 'Microsoft.ManagedIdentity/userAssignedIdentities@2023-01-31' = {
  name: '${containerAppName}-identity'
  location: location
}

// ---------------------------------------------------------------------------
// Dedicated Key Vault + token (R1/R4/R5)
// [GPT-5.6] The app identity can only get secrets from this one-secret, app-specific vault.
// ---------------------------------------------------------------------------
resource keyVault 'Microsoft.KeyVault/vaults@2023-07-01' = {
  name: keyVaultName
  location: location
  properties: {
    tenantId: tenant().tenantId
    sku: {
      family: 'A'
      name: 'standard'
    }
    accessPolicies: [
      {
        tenantId: tenant().tenantId
        objectId: managedIdentity.properties.principalId
        permissions: {
          certificates: []
          keys: []
          secrets: [
            'get'
          ]
          storage: []
        }
      }
    ]
    enableRbacAuthorization: false
    enablePurgeProtection: false
    enableSoftDelete: true
    enabledForDeployment: false
    enabledForDiskEncryption: false
    enabledForTemplateDeployment: false
    publicNetworkAccess: 'Enabled'
    softDeleteRetentionInDays: 7
    networkAcls: {
      bypass: 'None'
      defaultAction: 'Allow'
      ipRules: []
      virtualNetworkRules: []
    }
  }
}

resource authSecret 'Microsoft.KeyVault/vaults/secrets@2023-07-01' = {
  parent: keyVault
  name: 'sparq-auth-token'
  properties: {
    attributes: {
      enabled: true
    }
    contentType: 'SPARQ bearer token'
    value: authToken
  }
}

// ---------------------------------------------------------------------------
// Log Analytics workspace (telemetry for Container Apps environment)
// ---------------------------------------------------------------------------
resource logAnalytics 'Microsoft.OperationalInsights/workspaces@2022-10-01' = {
  name: take('${environmentName}-logs', 63)
  location: location
  properties: {
    sku: {
      name: 'PerGB2018'
    }
    retentionInDays: 30
  }
}

// ---------------------------------------------------------------------------
// Container Apps Environment
// ---------------------------------------------------------------------------
resource containerAppsEnv 'Microsoft.App/managedEnvironments@2024-03-01' = {
  name: environmentName
  location: location
  properties: {
    appLogsConfiguration: {
      destination: 'log-analytics'
      logAnalyticsConfiguration: {
        customerId: logAnalytics.properties.customerId
        sharedKey: logAnalytics.listKeys().primarySharedKey
      }
    }
  }
}

// ---------------------------------------------------------------------------
// Container App: sparq-server
// ---------------------------------------------------------------------------
resource containerApp 'Microsoft.App/containerApps@2024-03-01' = {
  name: containerAppName
  location: location
  identity: {
    type: 'UserAssigned'
    userAssignedIdentities: {
      '${managedIdentity.id}': {}
    }
  }
  properties: {
    environmentId: containerAppsEnv.id
    configuration: {
      activeRevisionsMode: 'Single'
      // -----------------------------------------------------------------------
      // Secrets (R4/R5): versionless Key Vault reference enables automatic rotation.
      // The token is never copied into a plaintext environment-variable value field.
      // -----------------------------------------------------------------------
      secrets: [
        {
          name: 'sparq-auth-token'
          identity: managedIdentity.id
          keyVaultUrl: '${keyVault.properties.vaultUri}secrets/${authSecret.name}'
        }
      ]
      // -----------------------------------------------------------------------
      // Ingress: HTTPS-only external on targetPort 3030 only (R3/R6)
      // Container Apps auto-TLS on the *.azurecontainerapps.io FQDN.
      // -----------------------------------------------------------------------
      ingress: {
        external: true
        targetPort: 3030
        transport: 'http'
        allowInsecure: false // R3: redirect plain HTTP to HTTPS
      }
    }
    template: {
      containers: [
        {
          name: 'sparq-server'
          image: imageRef
          // [GPT-5.6] The distroless image supplies non-root; ACA has no securityContext here.
          resources: {
            cpu: json(cpuCores)
            memory: memoryByCpu[cpuCores]
          }
          // -------------------------------------------------------------------
          // Environment — secrets injected via secretRef (R1/R4)
          // -------------------------------------------------------------------
          env: concat(
            [
              {
                name: 'SPARQ_AUTH_TOKEN'
                secretRef: 'sparq-auth-token' // R1: token from secret, never literal
              }
              {
                name: 'SPARQ_MAX_CONCURRENT'
                value: string(maxConcurrent)
              }
              {
                name: 'SPARQ_QUERY_TIMEOUT'
                value: string(queryTimeoutSeconds)
              }
            ],
            authTokenRead == '1' ? [{ name: 'SPARQ_AUTH_TOKEN_READ', value: '1' }] : [],
            !empty(corsAllowOrigin) ? [{ name: 'SPARQ_CORS_ALLOW_ORIGIN', value: corsAllowOrigin }] : []
          )
          // -------------------------------------------------------------------
          // Health probes (R7): Container Apps HTTP probes on /health
          // sparq-server /health → 200 body "ok", ungated, no token needed.
          // -------------------------------------------------------------------
          probes: [
            {
              type: 'Liveness'
              httpGet: {
                path: healthPath
                port: 3030
              }
              initialDelaySeconds: 10
              periodSeconds: 30
              failureThreshold: 3
            }
            {
              type: 'Readiness'
              httpGet: {
                path: healthPath
                port: 3030
              }
              initialDelaySeconds: 5
              periodSeconds: 10
              failureThreshold: 3
            }
            {
              type: 'Startup'
              httpGet: {
                path: healthPath
                port: 3030
              }
              initialDelaySeconds: 5
              periodSeconds: 6
              failureThreshold: 10 // [GPT-5.6] API maximum; preserves a 60s startup window
            }
          ]
        }
      ]
      scale: {
        minReplicas: minReplicas
        // [GPT-5.6] Single-writer safety boundary: replicas do not share the in-memory dataset.
        maxReplicas: maxReplicas
      }
    }
  }
}

// ---------------------------------------------------------------------------
// Outputs
// ---------------------------------------------------------------------------
output fqdn string = containerApp.properties.configuration.ingress.fqdn
output sparqlEndpoint string = 'https://${containerApp.properties.configuration.ingress.fqdn}/sparql'
output keyVaultName string = keyVault.name
output managedIdentityClientId string = managedIdentity.properties.clientId
output managedIdentityPrincipalId string = managedIdentity.properties.principalId
