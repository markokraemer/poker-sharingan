# syntax=docker/dockerfile:1.7
# Poker Sharingan server images for Platinum. UNTESTED (never built).
#
# Build from the repo root (context = repo root):
#   docker build -f deploy/platinum/docker/Dockerfile --target arena -t ghcr.io/<org>/sharingan-arena:0.1.0 .
#   docker build -f deploy/platinum/docker/Dockerfile --target serve -t ghcr.io/<org>/sharingan-serve:0.1.0 .
#   docker build -f deploy/platinum/docker/Dockerfile --target core  -t ghcr.io/<org>/sharingan-core:0.1.0  .
#
# Targets
#   arena : sharingan-arena only (REST + WS, SQLite). No blueprint => no house:sharingan-1, no ladder-v1.
#   serve : sharingan-serve only (stateless /v1/act inference with turn/river search).
#   core  : both, in ONE VM, so they share a single page cache of the ~140 GB blueprint
#           (two Platinum sandboxes never share guest page cache). This is the production target.
#
# Platinum builds a template from a PINNED registry tag (no :latest), see apps/api/src/api/apps.ts isPinned().
# The image carries NO weights: the entrypoint pulls blueprint + buckets from object storage on first boot.

ARG RUST_IMAGE=rust:1-bookworm
ARG RUNTIME_IMAGE=debian:bookworm-slim

# ---------------------------------------------------------------- build
FROM ${RUST_IMAGE} AS build
WORKDIR /src
# Set e.g. RUSTFLAGS="-C target-cpu=x86-64-v3" if every Platinum host in the pool has AVX2 (EPYC does).
ARG RUSTFLAGS=""
ENV RUSTFLAGS=${RUSTFLAGS} CARGO_TERM_COLOR=never
COPY . .
# `serve` profile = release + panic=unwind (Cargo.toml [profile.serve]) so a panicking search is caught per request.
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target \
    cargo build --profile serve -p sharingan-arena -p sharingan-serve \
 && install -D -m0755 target/serve/sharingan-arena /out/sharingan-arena \
 && install -D -m0755 target/serve/sharingan-serve /out/sharingan-serve

# ---------------------------------------------------------------- runtime base
FROM ${RUNTIME_IMAGE} AS runtime-base
RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates curl awscli tini \
 && rm -rf /var/lib/apt/lists/*
COPY deploy/platinum/docker/entrypoint.sh /usr/local/bin/sharingan-entrypoint
RUN chmod +x /usr/local/bin/sharingan-entrypoint && mkdir -p /data /models
ENV ARENA_BIND=0.0.0.0:8080 \
    ARENA_DB=/data/arena.db \
    SHARINGAN_BIND=0.0.0.0:8081 \
    ASSETS_DIR=/models
ENTRYPOINT ["/usr/bin/tini", "--", "/usr/local/bin/sharingan-entrypoint"]

# ---------------------------------------------------------------- arena
FROM runtime-base AS arena
COPY --from=build /out/sharingan-arena /usr/local/bin/sharingan-arena
ENV ROLE=arena
EXPOSE 8080

# ---------------------------------------------------------------- serve
FROM runtime-base AS serve
COPY --from=build /out/sharingan-serve /usr/local/bin/sharingan-serve
ENV ROLE=serve
EXPOSE 8081

# ---------------------------------------------------------------- core (arena + serve, shared page cache)
FROM runtime-base AS core
COPY --from=build /out/sharingan-arena /usr/local/bin/sharingan-arena
COPY --from=build /out/sharingan-serve /usr/local/bin/sharingan-serve
ENV ROLE=core
EXPOSE 8080 8081
