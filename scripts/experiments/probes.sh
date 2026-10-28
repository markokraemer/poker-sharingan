#!/usr/bin/env bash
# Translation probes (run ON THE DEV BOX): scripted bots that make one kind of off-menu bet,
# duplicate-matched (AIVAT) against 5 copies of a blueprint. Positive = the PROBE wins.
#   scripts/probes.sh TARGET.plcp SIZING [DEALS] [extra bench flags, e.g. --legacy --below smallest]
# Prints one line per probe: spec, AIVAT mbb/game +- 95% CI, target decisions after an off-menu
# action (miss rate / fold mass).
set -u
T="$1"; SZ="$2"; DEALS="${3:-8000}"; shift 3 2>/dev/null || shift $#
BIN="${BENCH_BIN:-target/xlate/release/bench}"
PROBES=(
  "probe:" "probe:pre=min" "probe:pre=2.5" "probe:pre=3" "probe:pre=4.5" "probe:pre=8" "probe:pre=allin"
  "probe:pre=3,pre3=3" "probe:pre3=allin"
  "probe:bet=0.5" "probe:bet=0.1" "probe:bet=0.2" "probe:bet=0.3" "probe:bet=0.4" "probe:bet=0.6" "probe:bet=0.75" "probe:bet=1" "probe:bet=1.25" "probe:bet=1.5" "probe:bet=2" "probe:bet=3" "probe:bet=5"
  "probe:bet=0.2,street=flop" "probe:bet=0.2,street=turn" "probe:bet=0.2,street=river"
  "probe:bet=3,street=river" "probe:bet=0.75,street=river" "probe:bet=0.75,street=turn"
  "probe:bet=0.5,raise=0.5" "probe:bet=0.5,raise=3"
)
[ -n "${PROBE_LIST:-}" ] && IFS='|' read -r -a PROBES <<< "$PROBE_LIST"
for P in "${PROBES[@]}"; do
  OUT=$(RAYON_NUM_THREADS="${THREADS:-32}" "$BIN" match --a "$P" --b "compact:$T" --bucketer tables:data/buckets-200 --sizing "$SZ" --players 6 --deals "$DEALS" --seed 11 "$@" 2>&1)
  L=$(echo "$OUT" | grep -E "^  $P " | head -1 | sed -E 's/.*aivat ([^ ]+ ± [^ ]+).*/\1/')
  R=$(echo "$OUT" | grep '^report' | sed -E 's/.*"after_off_menu":\{"decisions":([0-9]+),"miss":([0-9.]+),"desync":([0-9.]+),"why":(\[[0-9,]+\]),"p_fold":([0-9.]+).*/offmenu n=\1 miss=\2 desync=\3 why=\4 fold=\5/')
  printf '%-34s %-22s %s\n' "$P" "$L" "$R"
done
