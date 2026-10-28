#!/usr/bin/env bash
# show results of all arms on box B (both work dirs). usage: hc-show.sh [grep-regex]
BOX=B "$(dirname "$0")/../remote.sh" ssh 'for d in pluribus-search ps2; do cd ~/$d/logs/hc 2>/dev/null || continue; for f in *.log; do [ "$f" = "batch*" ] && continue; printf "%-14s %-9s" ${f%.log} $d; grep -h "RESULT" $f | sed "s/.*| aivat//;s/mbb\/game//" | tr "\n" " "; grep -h "timing" $f | sed "s/search timing: //;s/ searches//;s/mean iters.*//" | cut -c1-60; echo; done; done' | grep -v "^$" | grep -E "${1:-.}"
