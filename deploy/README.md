# Deployment

Lightbridge Code Intelligence runs on the **home-remote** Kubernetes cluster, deployed by
ArgoCD. The Helm chart and the ArgoCD `Application` live in the
[`ADORSYS-GIS/ai-helm`](https://github.com/ADORSYS-GIS/ai-helm) GitOps repo
(`charts/lightbridge-code-intelligence`, Application `aii-lightbridge-code-intelligence`).
This repo only owns the **production image tags**.

## Continuous deployment (image-updater + git write-back)

```
push to main ──▶ build-images.yml ──▶ ghcr.io/vymalo/lightbridge-{control-plane,web}:sha-<gitsha>
                                       (+ keyless cosign signature)
                                                │
                          argocd-image-updater  │ picks newest *signed* sha-<gitsha>
                          (home-os ImageUpdater  ▼ verifies cosign identity, then
                           CR selects the app)  git-commits the tag back here ──▶
                                       deploy/envs/production/values.yaml  (main)
                                                │
                                ArgoCD $values  ▼ auto-syncs the new tag
                                       lightbridge-ci-{control-plane,web} pods roll
```

- **`deploy/envs/production/values.yaml`** — the only deploy knob in this repo. The
  `lci.controllers.{control-plane,web}.containers.main.image.tag` fields are **owned by
  argocd-image-updater** — do not hand-edit them. ai-helm's Application mounts this file as
  its ArgoCD `$values` source (`targetRevision: main`).
- **Image signing** — `.github/workflows/build-images.yml` cosign-signs every pushed digest
  (keyless, GitHub OIDC). image-updater verifies the signature traces back to this workflow
  before promoting an image.

The chart structure itself stays pinned to an immutable `ai-helm` release tag; only the image
tags flow continuously from this repo's `main`. This mirrors the `vymalo-shop` / `opfs-webauthn`
deployments in the same cluster.
