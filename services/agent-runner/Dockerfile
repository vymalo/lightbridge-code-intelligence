# Lightbridge agent runner (Rust). The per-task Job image the dispatcher launches.
# Build context = repo root (the Cargo workspace).
#   docker build -f services/agent-runner/Dockerfile -t lightbridge-agent-runner .
FROM rust:1-slim-bookworm AS builder
WORKDIR /build
# Workspace root manifests + every member, so the workspace resolves and builds.
COPY Cargo.toml Cargo.lock ./
COPY services services
COPY xtask xtask
# --locked: build only from the reviewed Cargo.lock (fail on drift). Cache mounts persist the
# cargo registry + target across builds; copy the binary out before the cached target dir vanishes.
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/local/cargo/git \
    --mount=type=cache,target=/build/target \
    cargo build --release --locked -p agent-runner \
    && cp target/release/agent-runner /usr/local/bin/agent-runner

FROM debian:bookworm-slim AS runtime
# git: the runner shells out to it to clone the target repo (see src/clone.rs).
# ca-certificates: TLS to the control plane + GitHub.
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
