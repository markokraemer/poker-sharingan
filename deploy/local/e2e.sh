#!/usr/bin/env bash
# End to end, locally, with no Platinum and no Stripe account:
#
#   build images -> compose up (postgres + arena + runner) -> top up credits with a SIMULATED, SIGNED Stripe webhook ->
#   submit the SDK's example bot for a 2,000-hand ladder-v1 run -> wait -> assert:
#     * the run completed and the result is verified:true
#     * credits were debited by the hands played (and nothing is left reserved)
#     * the sandbox container was torn down
#
# Usage:   deploy/local/e2e.sh
# Env:     E2E_MODE=worker|connect   worker = match worker in the sandbox, signed result POSTed to /v1/results (default)
#                                    connect = `sharingan connect` over WebSocket + hosted:true run
#          E2E_HANDS=2000            hands for the run
#          E2E_BUILD=1|0             build the images first (default 1)
#          E2E_KEEP=1                leave the stack running afterwards
#          BLUEPRINT_SRC / BUCKETS_SRC / SIZING   the blueprint the arena and the sandboxes load
#                                    (default: the tiny test blueprint on the dev box, sizing tiny)
#          E2E_NICE="nice -n 19"     prefix for the image builds;   E2E_CPUSET=0-15  taskset the builds;   CARGO_BUILD_JOBS=16
# Needs: podman (rootless is fine) or docker, a compose (docker-compose v2 / docker compose / podman-compose), curl, openssl, python3.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
HERE="$ROOT/deploy/local"
STATE="${E2E_DIR:-$ROOT/data/local-e2e}"
PROJECT="${E2E_PROJECT:-sharingan-e2e}"
MODE="${E2E_MODE:-worker}"
HANDS="${E2E_HANDS:-2000}"
TRACK=ladder-v1
BLUEPRINT_SRC="${BLUEPRINT_SRC:-/home/ubuntu/dev-serve/tinybp/tiny.plcp}"
BUCKETS_SRC="${BUCKETS_SRC:-/home/ubuntu/pluribus/data/buckets-200}"
SIZING="${SIZING:-tiny}"
ARENA_PORT="${ARENA_PORT:-18080}"
RUNNER_PORT="${RUNNER_PORT:-18090}"
NICE="${E2E_NICE-nice -n 19}"
CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-16}"
mkdir -p "$STATE"

say()  { printf '\n\033[1m== %s\033[0m\n' "$*"; }
note() { printf '   %s\n' "$*"; }
PASS=0; FAIL=0
check() { # check "description" "actual" "expected"
  if [ "$2" = "$3" ]; then PASS=$((PASS+1)); printf '   PASS  %-62s %s\n' "$1" "$2"
  else FAIL=$((FAIL+1)); printf '   FAIL  %-62s got %s, want %s\n' "$1" "$2" "$3"; fi
}
jget() { python3 -c 'import json,sys
d=json.load(sys.stdin)
for k in sys.argv[1].split("."):
    d=d[int(k)] if isinstance(d,list) else d.get(k) if isinstance(d,dict) else None
print("null" if d is None else ("true" if d is True else "false" if d is False else d))' "$1"; }

# ---------------------------------------------------------------- container engine + compose
if command -v podman >/dev/null 2>&1; then ENGINE=podman; else ENGINE=docker; fi
if [ "$ENGINE" = podman ]; then
  export XDG_RUNTIME_DIR="${XDG_RUNTIME_DIR:-/run/user/$(id -u)}"
  SOCK="${CONTAINER_SOCKET:-$XDG_RUNTIME_DIR/podman/podman.sock}"
else
  SOCK="${CONTAINER_SOCKET:-/var/run/docker.sock}"
fi
SVC_PID=""
if [ "$ENGINE" = podman ] && [ ! -S "$SOCK" ]; then
  systemctl --user start podman.socket 2>/dev/null || true
  if [ ! -S "$SOCK" ]; then
    mkdir -p "$(dirname "$SOCK")"
    podman system service --time=0 "unix://$SOCK" >"$STATE/podman-service.log" 2>&1 &
    SVC_PID=$!
    for _ in $(seq 1 30); do [ -S "$SOCK" ] && break; sleep 0.2; done
  fi
fi
[ -S "$SOCK" ] || { echo "no container engine socket at $SOCK"; exit 2; }
export DOCKER_HOST="unix://$SOCK"

if command -v docker-compose >/dev/null 2>&1; then COMPOSE=(docker-compose)
elif [ -x /usr/libexec/docker/cli-plugins/docker-compose ]; then COMPOSE=(/usr/libexec/docker/cli-plugins/docker-compose)
elif docker compose version >/dev/null 2>&1; then COMPOSE=(docker compose)
elif command -v podman-compose >/dev/null 2>&1; then COMPOSE=(podman-compose)
else echo "no compose found (docker-compose v2, docker compose or podman-compose)"; exit 2; fi
echo "engine: $ENGINE ($(${ENGINE} --version))   compose: ${COMPOSE[*]}   socket: $SOCK   mode: $MODE"

# ---------------------------------------------------------------- blueprint (staged copy; read-only mount)
MODELS="$STATE/models"
BP_FILE="$(basename "$BLUEPRINT_SRC")"
if [ -f "$BLUEPRINT_SRC" ] && [ -d "$BUCKETS_SRC" ]; then
  mkdir -p "$MODELS"
  [ -f "$MODELS/$BP_FILE" ] || $NICE cp "$BLUEPRINT_SRC" "$MODELS/$BP_FILE"
  [ -d "$MODELS/buckets-200" ] || $NICE cp -r "$BUCKETS_SRC" "$MODELS/buckets-200"
else
  echo "blueprint not found ($BLUEPRINT_SRC / $BUCKETS_SRC): ladder-v1 cannot run"; exit 2
fi

# ---------------------------------------------------------------- images
ARENA_IMAGE=localhost/sharingan-arena:local; RUNNER_IMAGE=localhost/sharingan-runner:local; BOT_IMAGE=localhost/sharingan-bot:local
if [ "${E2E_BUILD:-1}" = 1 ]; then
  say "building images (nice, ${E2E_CPUSET:+taskset $E2E_CPUSET, }CARGO_BUILD_JOBS=$CARGO_BUILD_JOBS)"
  PRE="$NICE ${E2E_CPUSET:+taskset -c $E2E_CPUSET}"
  # shellcheck disable=SC2086
  $PRE $ENGINE build -q -f "$ROOT/deploy/platinum/docker/Dockerfile" --target arena --build-arg "CARGO_BUILD_JOBS=$CARGO_BUILD_JOBS" -t "$ARENA_IMAGE" "$ROOT" >/dev/null
  # shellcheck disable=SC2086
  $PRE $ENGINE build -q -f "$ROOT/deploy/platinum/docker/Dockerfile.bot" --build-arg "ARENA_IMAGE=$ARENA_IMAGE" -t "$BOT_IMAGE" "$ROOT" >/dev/null
  # shellcheck disable=SC2086
  $PRE $ENGINE build -q -f "$ROOT/deploy/platinum/docker/Dockerfile.runner" -t "$RUNNER_IMAGE" "$ROOT" >/dev/null
fi
$ENGINE image exists "$ARENA_IMAGE" 2>/dev/null || $ENGINE image inspect "$ARENA_IMAGE" >/dev/null

# ---------------------------------------------------------------- stack
rnd() { openssl rand -hex 16; }
RUNNER_AUTH_TOKEN="$(rnd)"; WEBHOOK_SECRET="whsec_e2e_$(rnd)"
cat >"$STATE/e2e.env" <<EOF
COMPOSE_PROJECT_NAME=$PROJECT
POSTGRES_PASSWORD=$(rnd)
ARENA_ADMIN_TOKEN=$(rnd)
ARENA_RUNNER_TOKEN=$(rnd)
RUNNER_AUTH_TOKEN=$RUNNER_AUTH_TOKEN
STRIPE_WEBHOOK_SECRET=$WEBHOOK_SECRET
WORKER_KEY_ID=runner1
WORKER_SIGN_KEY=$(rnd)
BLUEPRINT_DIR=$MODELS
BLUEPRINT_FILE=$BP_FILE
SIZING=$SIZING
SANDBOX_BINDS=$MODELS:/models:ro
WORKER_ENV_JSON='{"ARENA_BLUEPRINT":"/models/$BP_FILE","ARENA_BUCKETS":"tables:/models/buckets-200","ARENA_SIZING":"$SIZING"}'
CONTAINER_SOCKET=$SOCK
ARENA_PORT=$ARENA_PORT
RUNNER_PORT=$RUNNER_PORT
BOTNET_NAME=$PROJECT-botnet
EGRESS_NAME=$PROJECT-egress
DEFAULT_JOB_MODE=$MODE
PRICE_USD_PER_1K_HANDS=0.05
EOF
dc() { "${COMPOSE[@]}" -f "$HERE/compose.yml" --env-file "$STATE/e2e.env" -p "$PROJECT" "$@"; }
JOB_IDS=""
cleanup() {
  rc=$?
  if [ "${E2E_KEEP:-0}" != 1 ]; then
    say "tearing down"
    for j in $JOB_IDS; do $ENGINE rm -f "run-$j" >/dev/null 2>&1 || true; done
    dc down -v --remove-orphans >/dev/null 2>&1 || true
  else
    note "E2E_KEEP=1: stack left running (project $PROJECT, runner :$RUNNER_PORT, arena :$ARENA_PORT)"
  fi
  [ -n "$SVC_PID" ] && kill "$SVC_PID" 2>/dev/null || true
  exit $rc
}
trap cleanup EXIT

say "compose up"
dc down -v --remove-orphans >/dev/null 2>&1 || true
dc up -d --no-build 2>&1 | tail -n 12
ARENA="http://127.0.0.1:$ARENA_PORT"; RUNNER="http://127.0.0.1:$RUNNER_PORT"
for i in $(seq 1 90); do
  if curl -fsS -m 2 "$ARENA/readyz" >/dev/null 2>&1 && curl -fsS -m 2 "$RUNNER/readyz" >/dev/null 2>&1; then break; fi
  [ "$i" = 90 ] && { dc logs --tail 40; echo "stack did not become ready"; exit 1; }
  sleep 1
done
note "arena $ARENA   runner $RUNNER"
api() { curl -sS -m 60 -H "authorization: Bearer $RUNNER_AUTH_TOKEN" -H 'content-type: application/json' "$@"; }
check "ladder-v1 is available on the arena (blueprint loaded)" "$(curl -sS "$ARENA/v1/benchmarks" | python3 -c 'import json,sys;print([t["available"] for t in json.load(sys.stdin) if t["track"]=="ladder-v1"][0])')" "True"

# ---------------------------------------------------------------- credits
USER_ID="user_e2e_$(openssl rand -hex 3)"
say "credits for $USER_ID"
check "balance starts at 0" "$(api "$RUNNER/v1/credits/$USER_ID" | jget balance_micro)" "0"

mkdir -p "$STATE"
tar czf "$STATE/bot.tgz" -C "$HERE/bots" tag_bot.py
python3 - "$STATE/bot.tgz" "$STATE/submit.json" "$USER_ID" "$HANDS" "$MODE" <<'PY'
import base64, json, sys
tgz, out, user, hands, mode = sys.argv[1:6]
json.dump({"userId": user, "botName": "tag-bot", "language": "python", "entry": "tag_bot.py", "track": "ladder-v1",
           "hands": int(hands), "lanes": 4, "mode": mode, "idempotencyKey": f"e2e-{user}",
           "artifactTgzBase64": base64.b64encode(open(tgz, "rb").read()).decode()}, open(out, "w"))
PY
CODE=$(api -o "$STATE/resp.json" -w '%{http_code}' -X POST --data-binary @"$STATE/submit.json" "$RUNNER/v1/jobs")
check "submit with no credits is refused (HTTP 402 insufficient_credits)" "$CODE $(jget error.code < "$STATE/resp.json")" "402 insufficient_credits"

stripe_event() { # signed checkout.session.completed for $USER_ID: stripe_event <event id> <session id> <cents> [secret]
  local body ts sig
  body=$(printf '{"id":"%s","type":"checkout.session.completed","data":{"object":{"id":"%s","object":"checkout.session","payment_status":"paid","currency":"usd","amount_total":%s,"client_reference_id":"%s","metadata":{"kind":"sharingan_credits","user_id":"%s"}}}}' "$1" "$2" "$3" "$USER_ID" "$USER_ID")
  ts=$(date +%s)
  sig=$(printf '%s.%s' "$ts" "$body" | openssl dgst -sha256 -hmac "${4:-$WEBHOOK_SECRET}" -hex | awk '{print $NF}')
  curl -sS -m 20 -o "$STATE/resp.json" -w '%{http_code}' -X POST -H "stripe-signature: t=$ts,v1=$sig" -H 'content-type: application/json' --data-binary "$body" "$RUNNER/v1/webhooks/stripe"
}
SESSION=$(api -X POST -d "{\"userId\":\"$USER_ID\",\"amountUsd\":10}" "$RUNNER/v1/credits/checkout")
SESSION_ID=$(printf '%s' "$SESSION" | jget id)
check "checkout session is simulated (no Stripe call)" "$(printf '%s' "$SESSION" | jget simulated)" "true"
check "webhook with a WRONG secret is rejected (400)" "$(stripe_event evt_bad "$SESSION_ID" 1000 whsec_wrong)" "400"
check "balance still 0 after the rejected webhook" "$(api "$RUNNER/v1/credits/$USER_ID" | jget balance_micro)" "0"
check "signed webhook credits \$10 (HTTP 200)" "$(stripe_event evt_e2e_1 "$SESSION_ID" 1000) $(jget outcome < "$STATE/resp.json")" "200 credited"
check "replayed webhook is a no-op (duplicate)" "$(stripe_event evt_e2e_1 "$SESSION_ID" 1000) $(jget outcome < "$STATE/resp.json")" "200 duplicate"
check "balance is \$10.00 = 10000000 micro-USD" "$(api "$RUNNER/v1/credits/$USER_ID" | jget balance_micro)" "10000000"

# ---------------------------------------------------------------- the job
say "submit: $HANDS hands of $TRACK, mode=$MODE, bot=deploy/local/bots/tag_bot.py"
CODE=$(api -o "$STATE/resp.json" -w '%{http_code}' -X POST --data-binary @"$STATE/submit.json" "$RUNNER/v1/jobs")
JOB=$(jget id < "$STATE/resp.json"); JOB_IDS="$JOB"
check "submit accepted (HTTP 202)" "$CODE" "202"
note "job $JOB   reserved_micro=$(jget reserved_micro < "$STATE/resp.json")"
T0=$(date +%s); LAST=""
while :; do
  api "$RUNNER/v1/jobs/$JOB" >"$STATE/job.json"
  ST=$(jget status < "$STATE/job.json"); SETTLED=$(jget settled < "$STATE/job.json")
  LINE="$ST  $(jget hands_done < "$STATE/job.json")/$(jget hands_total < "$STATE/job.json") hands"
  [ "$LINE" != "$LAST" ] && note "[$(( $(date +%s) - T0 ))s] $LINE" && LAST="$LINE"
  case "$ST" in finished|failed|aborted) [ "$SETTLED" = true ] && break ;; esac
  [ $(( $(date +%s) - T0 )) -gt "${E2E_TIMEOUT_S:-600}" ] && { echo "timed out"; cat "$STATE/job.json"; exit 1; }
  sleep 1
done

say "assertions"
J="$STATE/job.json"
if [ "$(jget status < "$J")" != finished ]; then echo "job did not finish: $(jget error < "$J") [$(jget error_class < "$J")]"; jget bot_log_tail < "$J" | tail -n 15; fi
HANDS_DONE=$(jget hands_done < "$J"); CHARGE=$(jget charge_micro < "$J"); RUN_ID=$(jget run_id < "$J")
EXPECT_HANDS=$(( (HANDS + 5) / 6 * 6 ))
check "run completed: job status" "$(jget status < "$J")" "finished"
check "result verified:true (job)" "$(jget verified < "$J")" "true"
check "hands played = $HANDS rounded to whole deals" "$HANDS_DONE" "$EXPECT_HANDS"
check "AIVAT score present (mbb_per_game is a number)" "$(jget mbb_per_game < "$J" | python3 -c 'import sys;v=sys.stdin.read().strip();print("number" if v not in ("null","") and float(v)==float(v) else v)')" "number"
RUN="$(curl -sS "$ARENA/v1/benchmarks/$TRACK/runs/$RUN_ID")"
check "arena agrees: run status / verified / hosted" "$(printf '%s' "$RUN" | jget status) $(printf '%s' "$RUN" | jget verified) $(printf '%s' "$RUN" | jget hosted)" "finished true true"
check "arena agrees: hands" "$(printf '%s' "$RUN" | jget hands_done)" "$EXPECT_HANDS"
check "credits debited by hands played (50 micro-USD/hand at \$0.05/1k)" "$CHARGE" "$(( HANDS_DONE * 50 ))"
C="$(api "$RUNNER/v1/credits/$USER_ID")"
check "balance = 10000000 - charge, nothing left reserved" "$(printf '%s' "$C" | jget balance_micro) $(printf '%s' "$C" | jget reserved_micro)" "$(( 10000000 - CHARGE )) 0"
check "ledger kinds: topup, reserve, settle" "$(api "$RUNNER/v1/credits/$USER_ID/entries" | python3 -c 'import json,sys;print(",".join(e["kind"] for e in reversed(json.load(sys.stdin)["entries"]) if e["kind"]!="release"))')" "topup,reserve,settle"
check "job reports the sandbox deleted" "$(jget sandbox_deleted < "$J")" "true"
LEFT="$($ENGINE ps -a --filter label=sharingan.sandbox=1 --filter "name=run-$JOB" --format '{{.Names}}' | tr -d '\n')"
check "no sandbox container of this job is left" "${LEFT:-none}" "none"
CODE=$(api -o "$STATE/resp.json" -w '%{http_code}' -X POST --data-binary @"$STATE/submit.json" "$RUNNER/v1/jobs")
check "resubmitting the same idempotency key is deduped (200, same job)" "$CODE $(jget id < "$STATE/resp.json") $(jget deduped < "$STATE/resp.json")" "200 $JOB true"
check "... and does not charge again" "$(api "$RUNNER/v1/credits/$USER_ID" | jget balance_micro)" "$(( 10000000 - CHARGE ))"
note "score: $(jget mbb_per_game < "$J") +- $(jget ci95_mbb < "$J") mbb/game, sandbox $(jget sandbox_seconds < "$J")s, our cost $(jget cogs_micro < "$J") micro-USD vs charge $CHARGE"

# ---------------------------------------------------------------- a broken bot: free, not on the leaderboard, still torn down
if [ "$MODE" = worker ]; then
  say "a bot that cannot even import is not charged and not recorded"
  printf 'def act(:\n' >"$STATE/broken_bot.py"
  tar czf "$STATE/broken.tgz" -C "$STATE" broken_bot.py
  python3 - "$STATE/broken.tgz" "$STATE/submit2.json" "$USER_ID" <<'PY'
import base64, json, sys
tgz, out, user = sys.argv[1:4]
json.dump({"userId": user, "botName": "broken", "language": "python", "entry": "broken_bot.py", "track": "ladder-v1", "hands": 600, "lanes": 2,
           "idempotencyKey": f"e2e-broken-{user}", "artifactTgzBase64": base64.b64encode(open(tgz, "rb").read()).decode()}, open(out, "w"))
PY
  RUNS_BEFORE=$(curl -sS "$ARENA/v1/benchmarks/$TRACK/runs?limit=1000" | python3 -c 'import json,sys;print(len(json.load(sys.stdin)))')
  api -o "$STATE/resp.json" -X POST --data-binary @"$STATE/submit2.json" "$RUNNER/v1/jobs"
  JOB2=$(jget id < "$STATE/resp.json"); JOB_IDS="$JOB_IDS $JOB2"
  for _ in $(seq 1 120); do
    api "$RUNNER/v1/jobs/$JOB2" >"$STATE/job2.json"
    [ "$(jget settled < "$STATE/job2.json")" = true ] && break
    sleep 1
  done
  J2="$STATE/job2.json"
  check "broken bot: job failed, blamed on the bot" "$(jget status < "$J2") $(jget error_class < "$J2")" "failed bot"
  check "broken bot: the user sees why (bot_log_tail has the SyntaxError)" "$(jget bot_log_tail < "$J2" | grep -c SyntaxError | sed 's/^[1-9].*/yes/')" "yes"
  check "broken bot: charge 0, whole hold released" "$(jget charge_micro < "$J2") $(jget released_micro < "$J2")" "0 30000"
  check "broken bot: balance unchanged" "$(api "$RUNNER/v1/credits/$USER_ID" | jget balance_micro)" "$(( 10000000 - CHARGE ))"
  check "broken bot: nothing posted to the leaderboard" "$(curl -sS "$ARENA/v1/benchmarks/$TRACK/runs?limit=1000" | python3 -c 'import json,sys;print(len(json.load(sys.stdin)))')" "$RUNS_BEFORE"
  check "broken bot: sandbox torn down" "$(jget sandbox_deleted < "$J2") $($ENGINE ps -a --filter "name=run-$JOB2" --format '{{.Names}}' | tr -d '\n')" "true "
fi

say "$PASS passed, $FAIL failed"
[ "$FAIL" = 0 ]
