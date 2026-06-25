# Lightbridge agent runner. The per-task Job image the dispatcher launches.
#
# This Dockerfile has TWO final targets (runner-differentiation, RFC-0001) that share one Rust
# `builder` stage so the runner binary is compiled once:
#
#   • `indexer` (DEFAULT) — the full image. Bundles (ADR-0019):
#       1. the Rust runner binary (orchestration: clone, embed→pgvector, report)
#       2. Python + Graphify (`graphifyy`) — the 36-language structural-graph extractor it spawns
#       3. git + ca-certificates — clone the target repo, TLS to the control plane
#   • `review` — a leaner image WITHOUT the Python/Graphify venv (debian-slim + git + ca-certs +
#       the runner binary). The review path never invokes Graphify, so it needs no Python runtime.
#
# Build context = repo root (the Cargo workspace). Pick a target with `--target`:
#   docker build -f services/agent-runner/Dockerfile --target indexer -t lightbridge-agent-runner .
#   docker build -f services/agent-runner/Dockerfile --target review  -t lightbridge-agent-review .

# ── Stage 1: build the Rust runner ──────────────────────────────────────────────────────────────
FROM rust:1-slim-bookworm AS builder
WORKDIR /build
COPY Cargo.toml Cargo.lock ./
COPY services services
COPY xtask xtask
# Copy the runner out before the cached target dir vanishes.
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/local/cargo/git \
    --mount=type=cache,target=/build/target \
    cargo build --release --locked -p agent-runner \
    && cp target/release/agent-runner /usr/local/bin/

# ── Stage 2: install Graphify into a self-contained venv ────────────────────────────────────────
# Built with compilers present (some tree-sitter grammars may ship sdists) but the toolchain is
# left behind — only the venv is copied into the runtime image.
FROM python:3.12-slim-bookworm AS pybuild
RUN apt-get update \
  && apt-get install -y --no-install-recommends build-essential \
  && rm -rf /var/lib/apt/lists/*
RUN python -m venv /opt/venv
ENV PATH="/opt/venv/bin:$PATH"
# Pin the version the runner's graph.json parser was validated against.
RUN pip install --no-cache-dir "graphifyy==0.8.44" \
  && graphify --help >/dev/null

# ── Stage 3a: indexer runtime (DEFAULT) ──────────────────────────────────────────────────────────
# The full image: python-slim + Graphify venv + the runner. Used for `index` Jobs, which run the
# heavy structural-graph extraction. This is the default target so a bare `docker build` (and any
# tooling that doesn't pass `--target`) keeps producing today's image.
FROM python:3.12-slim-bookworm AS indexer
# git: clone the target repo (src/clone.rs). ca-certificates: TLS to control plane + GitHub.
# The native review agent (ADR-0026/0037) runs in-process in the Rust runner — no subprocess to install.
RUN apt-get update \
  && apt-get install -y --no-install-recommends ca-certificates git \
  && rm -rf /var/lib/apt/lists/* \
  && useradd --uid 1000 --create-home --user-group app \
  && mkdir -p /workspace && chown 1000:1000 /workspace
# Graphify (venv) + the Rust runner.
COPY --from=pybuild /opt/venv /opt/venv
COPY --from=builder /usr/local/bin/agent-runner /usr/local/bin/agent-runner
USER 1000
ENV PATH="/opt/venv/bin:$PATH" \
    WORKDIR=/workspace \
    RUST_LOG=info
ENTRYPOINT ["/usr/local/bin/agent-runner"]

# ── Stage 3b: review runtime (leaner — no Python/Graphify) ────────────────────────────────────────
# The review path is LLM/network-bound and reuses the latest indexed snapshot (ADR-0050); it never
# spawns Graphify, so this image drops the entire Python venv (~hundreds of MB) and ships only the
# runner binary on a plain debian-slim base with git + ca-certificates.
#
# Cold-repo caveat: a *cold-repo* review still self-indexes today (ADR-0050 only makes *warm* reviews
# reuse the snapshot). On this image the structural-graph step is simply skipped — `index_graph`
# (services/agent-runner/src/indexer/graph.rs) is best-effort/non-fatal: it logs and continues if
# `graphify` is absent. So the first cold review still builds the semantic (pgvector) index and just
# defers the graph until the next `index` task runs on the indexer image. (Proper fix — review never
# self-indexes — is a separate runner-differentiation slice.)
FROM debian:bookworm-slim AS review
RUN apt-get update \
  && apt-get install -y --no-install-recommends ca-certificates git \
  && rm -rf /var/lib/apt/lists/* \
  && useradd --uid 1000 --create-home --user-group app \
  && mkdir -p /workspace && chown 1000:1000 /workspace
COPY --from=builder /usr/local/bin/agent-runner /usr/local/bin/agent-runner
USER 1000
ENV WORKDIR=/workspace \
    RUST_LOG=info
ENTRYPOINT ["/usr/local/bin/agent-runner"]
