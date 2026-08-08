# Multi-stage build for the `obsidianlog` binary.
#
# Build context is the repository root:
#   docker build -f docker/Dockerfile -t obsidianlog .
#
# Built with the `sia` feature so the image *can* archive to Sia via a
# composed indexd (see docker-compose.yml) once you've completed indexd's own
# seed/config steps and ObsidianLog's app-key onboarding — both manual,
# one-time, and out of scope for this image. Without any of that, the image
# works standalone against the local backend (see the README's Docker
# Compose quickstart).

FROM rust:1-slim AS build
WORKDIR /src
COPY . .
RUN cargo build --release --bin obsidianlog --features sia \
    && strip target/release/obsidianlog

FROM debian:stable-slim AS runtime
# ca-certificates: TLS root certs for the Sia SDK's outbound HTTPS calls
# (rustls, never native-tls/OpenSSL — see ADR-0006). curl: HEALTHCHECK only.
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl \
    && rm -rf /var/lib/apt/lists/*

# All local state (config, keychain-fallback key file, local-backend archive
# data) lives under here, on the named volume the compose file mounts — so it
# survives container restarts instead of silently regenerating a fresh
# encryption key every time (the same failure mode ADR-0011 closed for the
# standalone ingest binary, just reachable via a different path: an ephemeral
# container filesystem).
ENV HOME=/var/lib/obsidianlog
WORKDIR /var/lib/obsidianlog

# Vector posts batches to the ingest endpoint on this port.
EXPOSE 7080
HEALTHCHECK --interval=10s --timeout=3s --start-period=5s \
    CMD curl -sf http://localhost:7080/health || exit 1

COPY --from=build /src/target/release/obsidianlog /usr/local/bin/obsidianlog
ENTRYPOINT ["obsidianlog"]
# Default to the ingest server; override the command for init/query/verify
# (e.g. `docker compose run --rm obsidianlog init --non-interactive`).
#
# --bind explicitly overrides config.toml's `serve.bind` (which `init`
# defaults to 127.0.0.1:7080 — correct for a native install, but that would
# bind to only the *container's* loopback, unreachable through the published
# port mapping).
CMD ["serve", "--config", "/etc/obsidianlog/config.toml", "--bind", "0.0.0.0:7080"]
