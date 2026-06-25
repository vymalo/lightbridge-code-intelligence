# Kubernetes and Deployment

## Namespace model

Recommended namespaces:
- `lightbridge-system`
- `lightbridge-indexing`
- `lightbridge-agents`
- `lightbridge-data` if self-hosting data plane components in-cluster

Apply Pod Security Admission labels per namespace. Default recommendation:
- `enforce=restricted` for agent and control-plane namespaces
- exceptions only where explicitly needed

## Labels

Use the standard `app.kubernetes.io/*` labels consistently.

Example:
- `app.kubernetes.io/name=lightbridge`
- `app.kubernetes.io/component=control-plane`
- `app.kubernetes.io/part-of=lightbridge-platform`
- `app.kubernetes.io/managed-by=Helm`

## Indexer Job template

```yaml
apiVersion: batch/v1
kind: Job
metadata:
  name: lightbridge-indexer-{{ .TaskID }}
  namespace: lightbridge-indexing
spec:
  backoffLimit: 2
  activeDeadlineSeconds: 3600
  ttlSecondsAfterFinished: 1800
  template:
    metadata:
      labels:
        app.kubernetes.io/name: lightbridge
        app.kubernetes.io/component: indexer
    spec:
      serviceAccountName: lightbridge-indexer
      restartPolicy: Never
      containers:
      - name: main
        image: ghcr.io/vymalo/lightbridge-indexer:latest
        envFrom:
        - secretRef:
            name: lightbridge-indexer-secrets
        - configMapRef:
            name: lightbridge-config
        resources:
          requests:
            cpu: "1"
            memory: "2Gi"
          limits:
            cpu: "4"
            memory: "8Gi"
```

## Agent Job template

```yaml
apiVersion: batch/v1
kind: Job
metadata:
  name: lightbridge-agent-{{ .TaskID }}
  namespace: lightbridge-agents
spec:
  backoffLimit: 1
  activeDeadlineSeconds: 900
  ttlSecondsAfterFinished: 900
  template:
    metadata:
      labels:
        app.kubernetes.io/name: lightbridge
        app.kubernetes.io/component: agent
    spec:
      serviceAccountName: lightbridge-agent
      restartPolicy: Never
      containers:
      - name: runner
        image: ghcr.io/vymalo/lightbridge-agent-runner:latest
        env:
        - name: TASK_ID
          value: "{{ .TaskID }}"
        - name: GITHUB_INSTALLATION_TOKEN
          valueFrom:
            secretKeyRef:
              name: lightbridge-task-credentials
              key: github_token
        resources:
          requests:
            cpu: "250m"
            memory: "512Mi"
          limits:
            cpu: "2"
            memory: "2Gi"
      - name: opencode
        image: ghcr.io/vymalo/opencode:latest
        resources:
          requests:
            cpu: "250m"
            memory: "512Mi"
          limits:
            cpu: "1"
            memory: "1Gi"
```

> Resource sizes are illustrative defaults. The design specifies no fixed resource ceiling, so
> treat them as recommended starting points rather than constraints.
>
> The runner Job's resources can be set **per task kind** via the agent config: `indexer_resources`
> for index Jobs (the heavy path — tree-sitter parse + embeddings + Graphify) and `review_resources`
> for review Jobs (read-mostly: they reuse the latest indexed snapshot, [ADR-0050](adr/0050-retrieval-pins-to-latest-indexed-snapshot.md),
> so they run leaner). Each falls back to the shared `resources` when unset, so a single `resources`
> block keeps uniform sizing.

One Kubernetes Job per task is a deliberate isolation decision (TTL cleanup, per-task creds). See
[ADR-0004](adr/0004-one-k8s-job-per-task.md).

## RBAC example

```yaml
apiVersion: v1
kind: ServiceAccount
metadata:
  name: lightbridge-agent
  namespace: lightbridge-agents
---
apiVersion: rbac.authorization.k8s.io/v1
kind: Role
metadata:
  name: lightbridge-agent-read-own-config
  namespace: lightbridge-agents
rules:
- apiGroups: [""]
  resources: ["configmaps", "secrets"]
  verbs: ["get"]
  resourceNames: ["lightbridge-config", "lightbridge-task-credentials"]
---
apiVersion: rbac.authorization.k8s.io/v1
kind: RoleBinding
metadata:
  name: lightbridge-agent-read-own-config
  namespace: lightbridge-agents
subjects:
- kind: ServiceAccount
  name: lightbridge-agent
roleRef:
  apiGroup: rbac.authorization.k8s.io
  kind: Role
  name: lightbridge-agent-read-own-config
```

## Network policy example

```yaml
apiVersion: networking.k8s.io/v1
kind: NetworkPolicy
metadata:
  name: lightbridge-agent-egress
  namespace: lightbridge-agents
spec:
  podSelector:
    matchLabels:
      app.kubernetes.io/component: agent
  policyTypes: ["Egress"]
  egress:
  - to:
    - namespaceSelector:
        matchLabels:
          kubernetes.io/metadata.name: lightbridge-data
  - to:
    - ipBlock:
        cidr: 0.0.0.0/0
    ports:
    - protocol: TCP
      port: 443
```

## Helm vs Kustomize

| Tool | Best use | Recommendation |
|---|---|---|
| Helm | Packaging, reusable chart values, release workflow | Use for app distribution |
| Kustomize | Environment overlays and patching | Use for repo-local overlays |

Deployment manifests live under deploy/. Replica counts are set in the external
`ADORSYS-GIS/ai-helm` GitOps chart; image tags live in the `adorsys-gis/ai-helm-values` repo
(`environments/prod/values/lightbridge-code-intelligence.yaml`), promoted by argocd-image-updater.

## Scaling and replica model

The web app runs multiple replicas; the control plane currently runs one. That is a correctness
constraint, not a capacity choice — the webhook handler holds delivery dedup in process memory, so a
second replica would process duplicate deliveries.
[RFC-0001](rfc/0001-horizontally-scalable-control-plane.md) proposes removing that state (durable
dedup in Postgres), splitting the binary into stateless roles (`serve` / `dispatcher` /
`scheduler`), and distributing work through a Postgres-backed queue, so the control plane can scale
horizontally like the web app. The `dispatcher` still creates one Kubernetes Job per task per
[ADR-0004](adr/0004-one-k8s-job-per-task.md); it does not run task work itself.

## Local cluster (TENTATIVE: multipass + k3s)

For testing closer to production than docker compose, a **tentative** local-cluster option is
multipass + k3s. The `justfile` exposes `just k3s-up` / `just k3s-down`, which launch a
`lightbridge-k3s` multipass VM and install k3s into it. This path is **tentative** — it is not
required for everyday development (use `just up` / docker compose for the data plane) and may
change. See [ADR-0013](adr/0013-local-dev-and-build-tooling.md).

## CI/CD outline

1. build images
2. run tests
3. scan images
4. publish versioned images
5. render Helm or Kustomize manifests
6. deploy to dev
7. smoke tests
8. promote to staging and prod

## Probe guidance

Use:
- startup probe for the control plane if migrations or caches delay readiness
- readiness probe for API availability
- liveness probe only where restart semantics are genuinely safe
