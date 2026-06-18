# Lightbridge control plane (Rust/Axum). Build context = repo root (the Cargo workspace).
#   docker build -f services/control-plane/Dockerfile -t lightbridge-control-plane .
FROM rust:1-slim-bookworm AS builder
WORKDIR /build
# Workspace root manifests + the members needed to resolve and build the control plane.
COPY Cargo.toml Cargo.lock ./
COPY services services
COPY xtask xtask
RUN cargo build --release -p control-plane

FROM debian:bookworm-slim AS runtime
RUN apt-get update \
  && apt-get install -y --no-install-recommends ca-certificates \
  && rm -rf /var/lib/apt/lists/* \
  && useradd --uid 1000 --create-home --user-group app
COPY --from=builder /build/target/release/control-plane /usr/local/bin/control-plane
USER 1000
ENV BIND_ADDR=0.0.0.0:8080 \
    RUST_LOG=info
EXPOSE 8080
ENTRYPOINT ["/usr/local/bin/control-plane"]
