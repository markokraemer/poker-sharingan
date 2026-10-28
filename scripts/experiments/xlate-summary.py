#!/usr/bin/env python3
"""Print exploiter result JSONs (scripts/xlate-exploit.sh) one line each."""
import json, sys
for p in sys.argv[1:]:
    j = json.load(open(p))
    e = j["exploitation"]
    a = e["avg_mbb"]
    print(f'{p}: {a["mbb"]:+.0f} ± {a["ci95"]:.0f} (raw {e["raw_avg_mbb"]["mbb"]:+.0f}) seats {[round(s["mbb"]) for s in e["by_seat"]]} learner-miss {j["learner"]["miss_rate"]:.4f} track {j.get("track")}')
    r = j.get("target_report")
    if r:
        o = json.loads(r)["after_off_menu"]
        print(f'    target after off-menu: n={o["decisions"]} miss={o["miss"]} desync={o["desync"]} fold={o["p_fold"]} call={o["p_call"]} raise={o["p_raise"]}')
