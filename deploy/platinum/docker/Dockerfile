# syntax=docker/dockerfile:1.7
# Poker Sharingan server images (arena / serve / core). Built and run with rootless podman; see deploy/local/e2e.sh.
#
# Build from the repo root (context = repo root):
#   podman build -f deploy/platinum/docker/Dockerfile --target arena -t ghcr.io/<org>/sharingan-arena:0.1.0 .
#   podman build -f deploy/platinum/docker/Dockerfile --target serve -t ghcr.io/<org>/sharingan-serve:0.1.0 .
#   podman build -f deploy/platinum/docker/Dockerfile --target core  -t ghcr.io/<org>/sharingan-core:0.1.0  .
#   (add --build-arg CARGO_BUILD_JOBS=16 to cap the compiler; default 16)
#
# Targets
#   arena : sharingan-arena only (REST + WS; SQLite, or Postgres with ARENA_DATABASE_URL). Without a blueprint there is no
#           house:sharingan-1, so ladder-v1 / sharingan-1 return 400.
#   serve : sharingan-serve only (stateless /v1/act inference with turn/river search). Needs a blueprint.
#   core  : both, in ONE VM, so they share a single page cache of the ~140 GB blueprint
#           (two Platinum sandboxes never share guest page cache). This is the production target.
#
# Platinum builds a template from a PINNED registry tag (no :latest), see apps/api/src/api/apps.ts isPinned().
# The image carries NO weights: either mount them at $ASSETS_DIR (blueprint file + buckets-200/), or let the entrypoint pull
# them from object storage on first boot (ASSETS_S3_URI). Runs as uid 10001 (sharingan).

ARG RUST_IMAGE=docker.io/library/rust:1-bookworm
ARG RUNTIME_IMAGE=docker.io/library/debian:bookworm-slim

# ---------------------------------------------------------------- build
FROM ${RUST_IMAGE} AS build
WORKDIR /src
# Set e.g. RUSTFLAGS="-C target-cpu=x86-64-v3" if every Platinum host in the pool has AVX2 (EPYC does).
ARG RUSTFLAGS=""
ARG CARGO_BUILD_JOBS=16
ENV RUSTFLAGS=${RUSTFLAGS} CARGO_BUILD_JOBS=${CARGO_BUILD_JOBS} CARGO_TERM_COLOR=never
# Only the Rust workspace goes in (keeps the compile layer cached when the runner / SDK / docs change).
COPY Cargo.toml Cargo.lock ./
COPY engine engine
COPY sharingan-1 sharingan-1
COPY arena arena
COPY research research
# `serve` profile = release + panic=unwind (Cargo.toml [profile.serve]) so a panicking search is caught per request.
RUN --mount=type=cache,target=/usr/local/cargo/registry,sharing=locked \
    --mount=type=cache,target=/src/target,sharing=locked \
    cargo build --locked --profile serve -p sharingan-arena -p sharingan-serve \
 && install -D -m0755 target/serve/sharingan-arena /out/sharingan-arena \
 && install -D -m0755 target/serve/sharingan-serve /out/sharingan-serve

# ---------------------------------------------------------------- runtime base
FROM ${RUNTIME_IMAGE} AS runtime-base
RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates curl awscli tini \
 && rm -rf /var/lib/apt/lists/* \
 && useradd --uid 10001 --create-home --shell /usr/sbin/nologin sharingan \
 && mkdir -p /data /models \
 && chown sharingan:sharingan /data /models
COPY deploy/platinum/docker/entrypoint.sh /usr/local/bin/sharingan-entrypoint
RUN chmod 0755 /usr/local/bin/sharingan-entrypoint
ENV ARENA_BIND=0.0.0.0:8080 \
    ARENA_DB=/data/arena.db \
    SHARINGAN_BIND=0.0.0.0:8081 \
    ASSETS_DIR=/models
USER sharingan
WORKDIR /data
ENTRYPOINT ["/usr/bin/tini", "--", "/usr/local/bin/sharingan-entrypoint"]

# ---------------------------------------------------------------- arena
FROM runtime-base AS arena
COPY --from=build /out/sharingan-arena /usr/local/bin/sharingan-arena
ENV ROLE=arena
EXPOSE 8080
VOLUME ["/data"]

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
VOLUME ["/data"]
