# Lightbridge Code Intelligence — task runner.
# `just` is the single human-facing entrypoint; heavier Rust automation lives in `cargo xtask`.
# Quality recipes are meant to be run locally BEFORE pushing (shift-left).

# Recipes use just's default `sh` shell so the entrypoint works on minimal environments
# (no zsh required). Keep recipe bodies POSIX-compatible.

# `chronic` (moreutils) runs a command and prints its output only if it fails — keeps a green gate
# quiet. Resolves to the empty string when not installed, so commands just run normally. The Rust
# side delegates to `cargo xtask`, which applies the same chronic wrapping internally.
chronic := `command -v chronic >/dev/null 2>&1 && printf chronic || true`

# List available recipes.
default:
    @just --list

# --- Setup ---

# Install JS deps and pre-fetch the Rust toolchain crates.
setup:
    pnpm install
    cargo fetch

# --- Dev ---

# Run the whole stack in dev (Next.js + control plane) via Turborepo.
dev:
    pnpm dev

# Run the control plane (standalone Rust backend) only.
# ALLOW_NO_DB=1 lets dev run degraded without a database (in-memory dedup, single-replica). In
# production DATABASE_URL is set instead; a pod missing it fails readiness on purpose, rather than
# silently dedup'ing per-replica in memory (RFC-0001 Phase 0). Export DATABASE_URL to use Postgres.
# NEO4J_URI points at the compose Neo4j so structural-graph ingestion works locally (ADR-0019);
# unset, the /internal/tasks/{id}/graph route fails closed (503).
dev-backend:
    ALLOW_NO_DB=1 \
    NEO4J_URI=${NEO4J_URI:-bolt://localhost:7687} \
    NEO4J_USER=${NEO4J_USER:-neo4j} \
    NEO4J_PASSWORD=${NEO4J_PASSWORD:-lightbridge} \
    cargo run -p control-plane

# Run the web app only.
dev-web:
    pnpm --filter @lightbridge/web dev

# --- Quality (shift-left: run before pushing) ---

# Format everything (Biome for JS/TS; rustfmt via xtask).
fmt:
    {{chronic}} pnpm format
    cargo xtask fmt

# Lint + format-check everything (Biome for JS/TS; clippy via xtask).
lint:
    {{chronic}} pnpm lint
    cargo xtask lint

# Run all tests (JS via Turborepo; Rust via xtask — nextest, falling back to cargo test).
test:
    {{chronic}} pnpm test
    cargo xtask test

# Validate the cratestack schema against the documented 0.4.x grammar (codegen deferred, ADR-0005).
# Best-effort: skips with a hint when cratestack-cli is absent. See xtask for the logic.
validate-schema:
    cargo xtask validate-schema

# The full local CI gate. The Rust side (schema check, fmt-check, clippy, tests) lives in xtask.
ci:
    {{chronic}} pnpm lint
    {{chronic}} pnpm build
    cargo xtask ci

# --- Local infra (docker compose: Postgres+pgvector, Neo4j, Keycloak) ---

# Keycloak comes up on http://localhost:8081 (admin/admin) with the `lightbridge` realm imported
# (client `lightbridge-web`, dev user `dev` / `password`). The web app and control plane read the
# OIDC issuer from env — copy apps/web/.env.example to apps/web/.env.local, and for the backend set
# OIDC_ISSUER=http://localhost:8081/realms/lightbridge and OIDC_AUDIENCE=lightbridge-api.
up:
    docker compose up -d

down:
    docker compose down

logs:
    docker compose logs -f

# --- Local cluster (TENTATIVE: multipass + k3s) ---
# A lighter-than-prod cluster for closer-to-prod local testing. See docs/kubernetes-deployment.md.

k3s-up:
    @echo "TENTATIVE path — see docs/kubernetes-deployment.md. Requires multipass."
    multipass launch --name lightbridge-k3s --cpus 2 --memory 4G --disk 20G
    multipass exec lightbridge-k3s -- bash -c "curl -sfL https://get.k3s.io | sh -"

k3s-down:
    multipass delete lightbridge-k3s
    multipass purge
