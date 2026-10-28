#!/bin/bash
# Entry point for the arena / serve / core images. UNTESTED.
#
# Env contract
#   ROLE                     arena | serve | core
#   ASSETS_S3_URI            s3://bucket/prefix holding sharingan-1.plcp and buckets-200/  (needed for blueprint roles)
#   AWS_ACCESS_KEY_ID / AWS_SECRET_ACCESS_KEY / AWS_DEFAULT_REGION / AWS_ENDPOINT_URL   (read-only creds for that prefix)
#   BLUEPRINT_FILE           file name under $ASSETS_DIR           (default sharingan-1.plcp)
#   USE_BLUEPRINT            1 (default when ASSETS_S3_URI set) | 0 to run arena with house bots only
#   ARENA_ADMIN_TOKEN, SHARINGAN_API_KEYS             required secrets (inject via Platinum env / secrets, never bake)
# Everything in arena/server/README.md and sharingan-1/serve/README.md also applies.
set -eu

ROLE="${ROLE:-arena}"
ASSETS_DIR="${ASSETS_DIR:-/models}"
BLUEPRINT_FILE="${BLUEPRINT_FILE:-sharingan-1.plcp}"
USE_BLUEPRINT="${USE_BLUEPRINT:-}"
[ -z "$USE_BLUEPRINT" ] && { [ -n "${ASSETS_S3_URI:-}" ] && USE_BLUEPRINT=1 || USE_BLUEPRINT=0; }

log() { echo "{\"entrypoint\":\"$1\"}" >&2; }

fetch_assets() {
  [ "$USE_BLUEPRINT" = 1 ] || return 0
  mkdir -p "$ASSETS_DIR"
  aws configure set default.s3.max_concurrent_requests 64
  aws configure set default.s3.multipart_chunksize 256MB
  if [ ! -f "$ASSETS_DIR/.complete" ]; then
    log "fetching assets from $ASSETS_S3_URI (first boot, ~140 GB)"
    aws s3 sync "$ASSETS_S3_URI/buckets-200" "$ASSETS_DIR/buckets-200" --only-show-errors
    aws s3 cp   "$ASSETS_S3_URI/$BLUEPRINT_FILE" "$ASSETS_DIR/$BLUEPRINT_FILE.part" --only-show-errors
    mv "$ASSETS_DIR/$BLUEPRINT_FILE.part" "$ASSETS_DIR/$BLUEPRINT_FILE"
    touch "$ASSETS_DIR/.complete"
  fi
  # Warm the page cache (cheap if already resident). The 140 GB file must fit in guest RAM or lookups page-fault from disk.
  ( cat "$ASSETS_DIR/$BLUEPRINT_FILE" > /dev/null 2>&1 & ) || true
}

run_arena() {
  if [ "$USE_BLUEPRINT" = 1 ]; then
    export ARENA_BLUEPRINT="$ASSETS_DIR/$BLUEPRINT_FILE"
    export ARENA_BUCKETS="tables:$ASSETS_DIR/buckets-200"
    export ARENA_SIZING="${ARENA_SIZING:-medium}"
  fi
  exec sharingan-arena
}

run_serve() {
  export SHARINGAN_BLUEPRINT="$ASSETS_DIR/$BLUEPRINT_FILE"
  export SHARINGAN_BUCKETS="$ASSETS_DIR/buckets-200"
  export SHARINGAN_SIZING="${SHARINGAN_SIZING:-medium}"
  exec sharingan-serve
}

case "$ROLE" in
  arena) fetch_assets; run_arena ;;
  serve) USE_BLUEPRINT=1; fetch_assets; run_serve ;;
  core)
    fetch_assets
    ( run_arena ) & A=$!
    ( run_serve ) & S=$!
    trap 'kill $A $S 2>/dev/null' TERM INT
    # If either dies, take the VM's service down so Platinum/our health check notices.
    wait -n || true
    kill $A $S 2>/dev/null || true
    exit 1
    ;;
  *) echo "unknown ROLE=$ROLE" >&2; exit 2 ;;
esac
