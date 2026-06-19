# Lightbridge agent runner. The per-task Job image the dispatcher launches.
#
# It bundles three things (ADR-0019):
#   1. the Rust runner binary (orchestration: clone, embed→pgvector, report)
#   2. Python + Graphify (`graphifyy`) — the 36-language structural-graph extractor it spawns
#   3. git + ca-certificates — clone the target repo, TLS to the control plane
#
# Build context = repo root (the Cargo workspace).
#   docker build -f services/agent-runner/Dockerfile -t lightbridge-agent-runner .

# ── Stage 1: build the Rust runner ──────────────────────────────────────────────────────────────
FROM rust:1-slim-bookworm AS builder
WORKDIR /build
COPY Cargo.toml Cargo.lock ./
COPY services services
COPY xtask xtask
# Builds all three bins: the runner + the two stdio MCP servers (slice 4b). Copy them out before
# the cached target dir vanishes.
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/local/cargo/git \
    --mount=type=cache,target=/build/target \
    cargo build --release --locked -p agent-runner \
    && cp target/release/agent-runner \
          target/release/lightbridge-vector-mcp \
          target/release/lightbridge-graph-mcp \
          /usr/local/bin/

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

# ── Stage 3: runtime ────────────────────────────────────────────────────────────────────────────
FROM python:3.12-slim-bookworm AS runtime
# git: clone the target repo (src/clone.rs). ca-certificates: TLS to control plane + GitHub.
RUN apt-get update \
  && apt-get install -y --no-install-recommends ca-certificates git \
  && rm -rf /var/lib/apt/lists/* \
  && useradd --uid 1000 --create-home --user-group app \
  && mkdir -p /workspace && chown 1000:1000 /workspace
# Graphify (venv) + the Rust runner + the stdio MCP servers (OpenCode spawns the latter, slice 4b).
COPY --from=pybuild /opt/venv /opt/venv
COPY --from=builder /usr/local/bin/agent-runner /usr/local/bin/agent-runner
COPY --from=builder /usr/local/bin/lightbridge-vector-mcp /usr/local/bin/lightbridge-vector-mcp
COPY --from=builder /usr/local/bin/lightbridge-graph-mcp /usr/local/bin/lightbridge-graph-mcp
USER 1000
ENV PATH="/opt/venv/bin:$PATH" \
    WORKDIR=/workspace \
    RUST_LOG=info
ENTRYPOINT ["/usr/local/bin/agent-runner"]
