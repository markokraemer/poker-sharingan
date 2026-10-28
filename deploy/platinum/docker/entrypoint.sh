#!/bin/bash
# Entry point for the arena / serve / core images.
#
# Env contract
#   ROLE                     arena | serve | core
#   ASSETS_DIR               where the blueprint lives (default /models): BLUEPRINT_FILE + buckets-200/
#   BLUEPRINT_FILE           file name under $ASSETS_DIR           (default sharingan-1.plcp)
#   BUCKETS_DIR              abstraction tables directory           (default $ASSETS_DIR/buckets-200; "coarse" for toy blueprints)
#   SIZING                   bet-size menu the blueprint was trained with (default medium); ARENA_SIZING / SHARINGAN_SIZING win
#   USE_BLUEPRINT            1 | 0. Default: 1 when $ASSETS_DIR/$BLUEPRINT_FILE exists or ASSETS_S3_URI is set, else 0
#                            (arena then runs with house bots only: no house:sharingan-1, ladder-v1 and sharingan-1 return 400)
#   ASSETS_S3_URI            s3://bucket/prefix holding the blueprint file and buckets-200/. Only used when the blueprint file is
#                            not already present (a mounted/baked blueprint never touches S3)
#   AWS_ACCESS_KEY_ID / AWS_SECRET_ACCESS_KEY / AWS_DEFAULT_REGION / AWS_ENDPOINT_URL   (read-only creds for that prefix)
#   ARENA_ADMIN_TOKEN, SHARINGAN_API_KEYS             secrets (inject via Platinum env / secrets, never bake)
# Everything in arena/server/README.md and sharingan-1/serve/README.md also applies.
set -eu

ROLE="${ROLE:-arena}"
ASSETS_DIR="${ASSETS_DIR:-/models}"
BLUEPRINT_FILE="${BLUEPRINT_FILE:-sharingan-1.plcp}"
BUCKETS_DIR="${BUCKETS_DIR:-$ASSETS_DIR/buckets-200}"
SIZING="${SIZING:-medium}"
USE_BLUEPRINT="${USE_BLUEPRINT:-}"
if [ -z "$USE_BLUEPRINT" ]; then
  if [ -f "$ASSETS_DIR/$BLUEPRINT_FILE" ] || [ -n "${ASSETS_S3_URI:-}" ]; then USE_BLUEPRINT=1; else USE_BLUEPRINT=0; fi
fi
[ "$ROLE" = serve ] && USE_BLUEPRINT=1

log() { echo "{\"entrypoint\":\"$1\"}" >&2; }

fetch_assets() {
  [ "$USE_BLUEPRINT" = 1 ] || return 0
  if [ -f "$ASSETS_DIR/$BLUEPRINT_FILE" ] && { [ -f "$ASSETS_DIR/.complete" ] || [ -z "${ASSETS_S3_URI:-}" ]; }; then
    log "blueprint present at $ASSETS_DIR/$BLUEPRINT_FILE; no fetch"
  else
    [ -n "${ASSETS_S3_URI:-}" ] || { log "no blueprint at $ASSETS_DIR/$BLUEPRINT_FILE and ASSETS_S3_URI is unset"; exit 3; }
    mkdir -p "$ASSETS_DIR"
    local ep=""
    [ -n "${AWS_ENDPOINT_URL:-}" ] && ep="--endpoint-url $AWS_ENDPOINT_URL"   # awscli 2.9 (Debian) ignores the env var
    aws configure set default.s3.max_concurrent_requests 64
    aws configure set default.s3.multipart_chunksize 256MB
    log "fetching assets from $ASSETS_S3_URI (first boot, ~140 GB)"
    # shellcheck disable=SC2086
    aws $ep s3 sync "$ASSETS_S3_URI/buckets-200" "$ASSETS_DIR/buckets-200" --only-show-errors
    # shellcheck disable=SC2086
    aws $ep s3 cp   "$ASSETS_S3_URI/$BLUEPRINT_FILE" "$ASSETS_DIR/$BLUEPRINT_FILE.part" --only-show-errors
    mv "$ASSETS_DIR/$BLUEPRINT_FILE.part" "$ASSETS_DIR/$BLUEPRINT_FILE"
    touch "$ASSETS_DIR/.complete"
  fi
  # Warm the page cache (cheap if already resident). The 140 GB file must fit in guest RAM or lookups page-fault from disk.
  # Skipped for small files? No: cat of a 250 MB file is instant. Set WARM=0 to disable.
  if [ "${WARM:-1}" = 1 ]; then ( cat "$ASSETS_DIR/$BLUEPRINT_FILE" > /dev/null 2>&1 & ) || true; fi
}

# "coarse" is a built-in bucketer name; anything else is a directory of tables.
arena_buckets() { if [ "$BUCKETS_DIR" = coarse ]; then echo coarse; else echo "tables:$BUCKETS_DIR"; fi; }

run_arena() {
  if [ "$USE_BLUEPRINT" = 1 ]; then
    export ARENA_BLUEPRINT="$ASSETS_DIR/$BLUEPRINT_FILE"
    export ARENA_BUCKETS="${ARENA_BUCKETS:-$(arena_buckets)}"
    export ARENA_SIZING="${ARENA_SIZING:-$SIZING}"
  fi
  exec sharingan-arena
}

run_serve() {
  export SHARINGAN_BLUEPRINT="$ASSETS_DIR/$BLUEPRINT_FILE"
  export SHARINGAN_BUCKETS="${SHARINGAN_BUCKETS:-$BUCKETS_DIR}"
  export SHARINGAN_SIZING="${SHARINGAN_SIZING:-$SIZING}"
  exec sharingan-serve
}

case "$ROLE" in
  arena) fetch_assets; run_arena ;;
  serve) fetch_assets; run_serve ;;
  core)
    USE_BLUEPRINT=1
    fetch_assets
    ( run_arena ) & A=$!
    ( run_serve ) & S=$!
    TERMED=0
    trap 'TERMED=1; kill $A $S 2>/dev/null' TERM INT
    # If either dies, take the VM's service down so Platinum/our health check notices.
    # `wait -n` returns as soon as one child exits; a TERM/INT trap interrupts it too.
    wait -n || true
    kill $A $S 2>/dev/null || true
    wait 2>/dev/null || true         # let the arena drain (ARENA_SHUTDOWN_GRACE_MS)
    [ "$TERMED" = 1 ] && exit 0      # asked to stop: clean exit
    exit 1                           # one half died on its own: fail the unit
    ;;
  *) echo "unknown ROLE=$ROLE" >&2; exit 2 ;;
esac
