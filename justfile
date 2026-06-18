# Lightbridge Code Intelligence — task runner.
# `just` is the single human-facing entrypoint; heavier Rust automation lives in `cargo xtask`.
# Quality recipes are meant to be run locally BEFORE pushing (shift-left).

set shell := ["zsh", "-cu"]

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
dev-backend:
    cargo run -p control-plane

# Run the web app only.
dev-web:
    pnpm --filter @lightbridge/web dev

# --- Quality (shift-left: run before pushing) ---

# Format everything (Biome for JS/TS, rustfmt for Rust).
fmt:
    pnpm format
    cargo fmt

# Lint + format-check everything.
lint:
    pnpm lint
    cargo clippy --all-targets -- -D warnings

# Run all tests (JS via Turborepo, Rust via cargo-nextest).
test:
    pnpm test
    cargo nextest run

# The full local CI gate (delegates the Rust side to cargo xtask).
ci:
    pnpm lint
    pnpm build
    cargo xtask ci

# --- Local infra (docker compose: Postgres+pgvector, Neo4j) ---

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
