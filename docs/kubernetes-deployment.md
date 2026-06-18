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

Deployment manifests live under `deploy/` (planned).

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
