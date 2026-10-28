#!/usr/bin/env bash
# usage: xlate-wait-then.sh LOGFILE PATTERN cmd...   (run on the dev box; waits until PATTERN is in LOGFILE)
L="$1"; P="$2"; shift 2
until grep -q "$P" "$L"; do sleep 10; done
exec "$@"
