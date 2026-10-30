#!/usr/bin/env python3
"""Accuracy checker for the play page.

Drives the REAL page (headless Chromium) against the REAL arena, plays hands through the UI
(buttons, keyboard, typed amounts), and after EVERY hand compares what the page displays
(stacks, nets, board, shown cards, pot lines, uncalled bet, winners, hand names, hand-history
log lines) with the server's authoritative GET /v1/hands/{id}. At every decision it also
compares the rendered table and controls with the your_turn payload. Prints the mismatch count.

usage: check.py BASE_URL [--arena URL] [--scale N]
  BASE_URL e.g. http://127.0.0.1:18080/play/  or  https://sharingan.poker/play/
"""
import argparse, itertools, json, random, sys, time
from playwright.sync_api import sync_playwright

RANKS = "23456789TJQKA"
SING = {2: "Two", 3: "Three", 4: "Four", 5: "Five", 6: "Six", 7: "Seven", 8: "Eight", 9: "Nine", 10: "Ten", 11: "Jack", 12: "Queen", 13: "King", 14: "Ace"}
PLUR = {2: "Twos", 3: "Threes", 4: "Fours", 5: "Fives", 6: "Sixes", 7: "Sevens", 8: "Eights", 9: "Nines", 10: "Tens", 11: "Jacks", 12: "Queens", 13: "Kings", 14: "Aces"}


# ---------- an independent evaluator (does not share code with the page) ----------
def rk(c): return RANKS.index(c[0]) + 2

def score5(cs):
    r = sorted((rk(c) for c in cs), reverse=True)
    flush = len({c[1] for c in cs}) == 1
    cnt = {}
    for x in r: cnt[x] = cnt.get(x, 0) + 1
    g = sorted(cnt.items(), key=lambda kv: (-kv[1], -kv[0]))
    shape = [n for _, n in g]
    ranks_by_group = [x for x, _ in g]
    straight = 0
    if len(g) == 5:
        if r[0] - r[4] == 4: straight = r[0]
        elif r == [14, 5, 4, 3, 2]: straight = 5
    if straight and flush: return (8, [straight])
    if shape == [4, 1]: return (7, ranks_by_group)
    if shape == [3, 2]: return (6, ranks_by_group)
    if flush: return (5, r)
    if straight: return (4, [straight])
    if shape == [3, 1, 1]: return (3, ranks_by_group)
    if shape == [2, 2, 1]: return (2, ranks_by_group)
    if shape == [2, 1, 1, 1]: return (1, ranks_by_group)
    return (0, r)

def best7(cards):
    return max((score5(list(c)) for c in itertools.combinations(cards, 5)), key=lambda s: (s[0], s[1]))

def name_of(s):
    cat, t = s
    if cat == 0: return "High card, " + SING[t[0]]
    if cat == 1: return "Pair of " + PLUR[t[0]]
    if cat == 2: return "Two pair, %s and %s" % (PLUR[t[0]], PLUR[t[1]])
    if cat == 3: return "Three of a kind, " + PLUR[t[0]]
    if cat == 4: return "Straight, %s high" % SING[t[0]]
    if cat == 5: return "Flush, %s high" % SING[t[0]]
    if cat == 6: return "Full house, %s full of %s" % (PLUR[t[0]], PLUR[t[1]])
    if cat == 7: return "Four of a kind, " + PLUR[t[0]]
    return "Royal flush" if t[0] == 14 else "Straight flush, %s high" % SING[t[0]]

MADE = {1: 1, 2: 2, 3: 1, 7: 1}

def expected_names(hole, board, live):
    sc = {s: best7(hole[s] + board) for s in live}
    out = {}
    for s in live:
        nm = name_of(sc[s]); cat, t = sc[s]; k = 0
        n = MADE.get(cat, 0)
        if n:
            for o in live:
                if o == s or k: continue
                co, to = sc[o]
                if co == cat and t[:n] == to[:n]:
                    for i in range(n, len(t)):
                        if t[i] != to[i]: k = t[i]; break
        out[s] = nm + (", %s kicker" % SING[k] if k else "")
    return out


# ---------- checker ----------
class Stats:
    def __init__(self):
        self.hands = 0; self.checks = 0; self.mismatches = []; self.decisions = 0; self.turn_checks = 0
        self.allin_hands = 0; self.side_pot_hands = 0; self.split_hands = 0; self.showdowns = 0; self.uncalled_hands = 0
        self.raise_clamps = 0; self.reconnects = 0; self.timeouts = 0; self.folds_free = 0
    def eq(self, label, ui, srv, ctx=""):
        self.checks += 1
        if ui != srv:
            self.mismatches.append("%s: ui=%r server=%r %s" % (label, ui, srv, ctx))

ST = Stats()
STATE_JS = """() => { const s = window.__sharingan && window.__sharingan.state; if (!s || !s.S) return null;
  const t = s.turn; return { sent: s.sent, phase: document.body.dataset.phase, handId: s.S.handId, handNo: s.S.handNo, over: s.S.over,
    turn: t ? { did: t.decision_id, legal: t.legal, street: t.street, pot: t.pot, players: t.players, hand_id: t.hand_id } : null,
    anoms: s.anoms, errors: s.errors, tally: s.tally, hasToken: !!s.token, banner: document.getElementById('banner').innerText }; }"""

def ui_snapshot(pg):
    return pg.evaluate("""() => {
      const q = (sel, root) => Array.from((root || document).querySelectorAll(sel));
      const seats = {};
      q('#seats .seat').forEach(e => { const n = +e.dataset.seat; seats[n] = {
        stk: (e.querySelector('.stk') || {}).textContent, net: (e.querySelector('.net') || {}).textContent, hn: (e.querySelector('.hn') || {}).textContent,
        cards: q('.cards [data-c]', e).map(c => c.dataset.c), backs: q('.cards .back', e).length, win: e.classList.contains('win'), status: e.dataset.status, pos: (e.querySelector('.pos')||{}).textContent || '' }; });
      const bets = {}; q('#bets [data-bet]').forEach(e => { bets[e.dataset.bet] = e.textContent; });
      const d = q('#seats .btn-d').length;
      const H = window.__sharingan.state.S.handId;
      const det = document.querySelector('#log details[data-hand="' + H + '"]');
      const lines = det ? q('.ln', det).map(e => ({ ev: e.dataset.ev || null, st: e.dataset.st || null, sd: e.dataset.sd || null, lpot: e.dataset.lpot || null, lunc: e.dataset.lunc || null, txt: e.innerText })) : null;
      return { seats, bets, board: q('#board [data-c]').map(c => c.dataset.c), pots: q('#pots [data-pot]').map(e => e.dataset.pot), unc: q('#pots [data-unc]').map(e => e.dataset.unc),
               lvl: q('#pots [data-lvl]').length, buttons: d, potline: document.getElementById('potline').innerText, lines, banner: document.getElementById('banner').innerText,
               hist: window.__sharingan.state.S.lastEnd || null };
    }""")


def fmt_signed(n): return ("+%d" % n) if n > 0 else str(n)

def verify_hand(pg, hid, my_raises, ctx):
    st = pg.evaluate(STATE_JS)
    srv = pg.evaluate("(id) => window.__sharingan.api('GET', '/v1/hands/' + id)", hid)
    ui = ui_snapshot(pg)
    ST.hands += 1
    acts = srv["actions"]
    kind = lambda a: a.get("type") or a.get("kind")
    res = {r["seat"]: r for r in srv["results"]}
    hole = {int(k): v for k, v in srv["hole_cards"].items()}
    board = srv["board"]
    showdown = srv["ended_by"] == "showdown"
    c = "[%s]" % ctx
    # stacks and nets
    for seat, r in res.items():
        u = ui["seats"].get(str(seat)) or ui["seats"].get(seat)
        ST.eq("stack seat %d" % seat, u["stk"], str(r["stack_end"]), c + hid)
        ST.eq("net seat %d" % seat, u["net"], fmt_signed(r["net"]), c + hid)
    ST.eq("sum net", sum(r["net"] for r in srv["results"]), 0, c + hid)
    ST.eq("board", ui["board"], board, c + hid)
    ST.eq("pot line says hand over", "Hand over" in ui["potline"], True, c + hid)
    # contributions, uncalled
    contrib = {}
    for a in acts: contrib[a["seat"]] = contrib.get(a["seat"], 0) + a["added"]
    top = sorted(contrib.values(), reverse=True)
    unc = (top[0] - top[1]) if len(top) > 1 and top[0] > top[1] else (top[0] if len(top) == 1 else 0)
    uc_seat = max(contrib, key=lambda s: contrib[s]) if unc else None
    ST.eq("uncalled", ui["unc"], ["%d:%d" % (uc_seat, unc)] if unc else [], c + hid)
    if unc: ST.uncalled_hands += 1
    # pots
    spots = srv["pots"]
    exp = []
    for p in spots:
        u = unc if (unc and len(p["eligible_seats"]) == 1 and p["winners"] == [uc_seat]) else 0
        u = min(u, p["amount"])
        if p["amount"] - u > 0 or not u:
            exp.append("%d:%d:%s:%s" % (p["amount"] - u, u, ",".join(map(str, p["winners"])), ",".join(map(str, p["share"]))))
    ST.eq("pot lines", ui["pots"], exp, c + hid)
    ST.eq("sum of pots == sum of contributions", sum(p["amount"] for p in spots), sum(contrib.values()), c + hid)
    winners = sorted({w for p in spots for w in p["winners"]})
    ST.eq("winner highlight", sorted(int(s) for s, v in ui["seats"].items() if v["win"]), winners, c + hid)
    if any(len(p["winners"]) > 1 for p in spots): ST.split_hands += 1
    if len([p for p in spots if len(p["eligible_seats"]) > 1]) > 1: ST.side_pot_hands += 1
    if any(a["all_in"] for a in acts): ST.allin_hands += 1
    # folded seats / shown cards
    folded = {a["seat"] for a in acts if kind(a) == "fold"}
    live = [s for s in res if s not in folded]
    if showdown:
        ST.showdowns += 1
        names = expected_names(hole, board, live)
        for s in live:
            u = ui["seats"][str(s)]
            ST.eq("shown cards seat %d" % s, u["cards"], hole[s], c + hid)
            ST.eq("hand name seat %d" % s, u["hn"], names[s], c + hid)
        sd = [l["sd"] for l in ui["lines"] if l["sd"]]
        ST.eq("showdown lines seats", sorted(int(x.split(":")[0]) for x in sd), sorted(live), c + hid)
        # order: last aggressor of the last street with action, else first live left of the button
        last_street = acts[-1]["street"]
        for a in reversed(acts):
            if not a["forced"]: last_street = a["street"]; break
        aggr = [a["seat"] for a in acts if a["street"] == last_street and kind(a) in ("bet", "raise") and not a["forced"]]
        if aggr: first = aggr[-1]
        else: first = next((srv["button_seat"] + k) % 6 for k in range(1, 7) if (srv["button_seat"] + k) % 6 in live)
        order = [(first + k) % 6 for k in range(6) if (first + k) % 6 in live]
        ST.eq("showdown order", [int(x.split(":")[0]) for x in sd], order, c + hid)
    else:
        for s, u in ui["seats"].items():
            if int(s) != 0: ST.eq("no opponent cards shown after fold-out (seat %s)" % s, [x for x in u["cards"]], [], c + hid)
    # log lines vs server actions
    evs = [l["ev"] for l in ui["lines"] if l["ev"]]
    exp_ev = ["%d:%s:%d:%d:%d" % (a["seat"], kind(a), a["to"], a["added"], 1 if a["all_in"] else 0) for a in acts]
    ST.eq("log action lines", evs, exp_ev, c + hid)
    sts = [l["st"] for l in ui["lines"] if l["st"]]
    exp_st = [n + ":" + "".join(board[:k]) for n, k in (("flop", 3), ("turn", 4), ("river", 5)) if len(board) >= k]
    ST.eq("log street lines", sts, exp_st, c + hid)
    lp = [l["lpot"] for l in ui["lines"] if l["lpot"]]
    ST.eq("log pot lines", lp, exp, c + hid)
    # positions on the table
    btn = srv["button_seat"]
    ST.eq("dealer button seat", [s for s, u in ui["seats"].items() if u["pos"].startswith("BTN")], [str(btn)], c + hid)
    ST.eq("sb seat", [s for s, u in ui["seats"].items() if u["pos"] == "SB"], [str(srv["sb_seat"])], c + hid)
    ST.eq("bb seat", [s for s, u in ui["seats"].items() if u["pos"] == "BB"], [str(srv["bb_seat"])], c + hid)
    # my raises: the typed/clicked amount, clamped to the legal range, is what the server saw
    mine = [a for a in acts if a["seat"] == 0 and not a["forced"] and kind(a) in ("bet", "raise")]
    exp_to = [t for t in my_raises]
    ST.eq("my raise amounts (clamped to legal)", [a["to"] for a in mine], exp_to[:len(mine)], c + hid)
    sent = [d for d in st["sent"] if d["hand_id"] == hid and d["type"] in ("bet", "raise")]
    want_types = []
    for a in mine:
        prior = any(x["seq"] < a["seq"] and x["street"] == a["street"] and (x["forced"] or kind(x) in ("bet", "raise")) for x in acts)
        want_types.append("raise" if prior else "bet")
    ST.eq("sent bet/raise wording", [d["type"] for d in sent], want_types, c + hid)
    ST.eq("sent amounts == server amounts", [d["to"] for d in sent], [a["to"] for a in mine], c + hid)
    # engine bookkeeping inside the page
    ST.eq("page anomalies", st["anoms"], [], c + hid)
    ST.eq("server errors seen by the page", st["errors"], [], c + hid)
    ST.eq("anomalies in server history", srv.get("anomalies"), [], c + hid)


def turn_checks(pg, t, ctx):
    ST.turn_checks += 1
    ui = ui_snapshot_light(pg)
    L = t["legal"]; c = "[%s turn %s]" % (ctx, t["did"])
    for p in t["players"]:
        u = ui["seats"].get(str(p["seat"]))
        ST.eq("turn stack seat %d" % p["seat"], u["stk"], str(p["stack"]), c)
        want = str(p["street_contrib"]) if p["street_contrib"] else None
        ST.eq("turn bet seat %d" % p["seat"], ui["bets"].get(str(p["seat"])), want, c)
    collected = t["pot"] - sum(p["street_contrib"] for p in t["players"])
    ST.eq("collected pot", ui["potB"], str(collected), c)
    call = ui["callText"].strip().lower()
    if L["check"]: ST.eq("call button", call.startswith("check"), True, c)
    else: ST.eq("call button", call.startswith("call %d" % L["call"]["added"]) and (("all-in" in call) == bool(L["call"]["all_in"])), True, c + call)
    ST.eq("fold enabled", not ui["foldDisabled"], bool(L["fold"]), c)
    ST.eq("raise enabled", not ui["raiseDisabled"], L["raise"] is not None, c)
    if L["raise"]:
        ST.eq("slider min", ui["sliderMin"], L["raise"]["min_to"], c)
        ST.eq("slider max", ui["sliderMax"], L["raise"]["max_to"], c)
        ST.eq("preset all-in present", ui["presets"][-1] == str(L["raise"]["max_to"]), True, c)
        ST.eq("presets within legal range", all(L["raise"]["min_to"] <= int(x) <= L["raise"]["max_to"] for x in ui["presets"]), True, c)
        cb = max(p["street_contrib"] for p in t["players"])
        word = ui["raiseText"].strip().lower()
        ST.eq("bet/raise wording", ("bet" in word.split()[:1]) or ("all-in" in word) if cb == 0 else ("raise" in word.split()[:1]) or ("all-in" in word), True, c + word)
    else:
        ST.eq("no raise controls", ui["sizingHidden"], True, c)
        ST.eq("reason shown", "red" in ui["infoHtml"] or "short" in ui["infoHtml"] or "all-in" in ui["infoHtml"], True, c)
    me = next(p for p in t["players"] if p["seat"] == 0)
    ST.eq("to call shown", ("To call %d" % (L["call"]["added"] if L["call"] else 0)) in ui["infoText"].replace("\n", " "), True, c + ui["infoText"])
    ST.eq("timer shown", "s" in ui["timer"], True, c)


def ui_snapshot_light(pg):
    return pg.evaluate("""() => {
      const q = (sel, root) => Array.from((root || document).querySelectorAll(sel));
      const seats = {}; q('#seats .seat').forEach(e => { seats[e.dataset.seat] = { stk: (e.querySelector('.stk') || {}).textContent }; });
      const bets = {}; q('#bets [data-bet]').forEach(e => { bets[e.dataset.bet] = e.textContent; });
      const pb = document.querySelector('#potline b');
      const sl = document.getElementById('slider');
      return { seats, bets, potB: pb ? pb.textContent : null, callText: document.getElementById('bCall').innerText, foldDisabled: document.getElementById('bFold').disabled,
        raiseDisabled: document.getElementById('bRaise').disabled, raiseText: document.getElementById('bRaise').innerText, sliderMin: +sl.min, sliderMax: +sl.max,
        presets: q('#presets button').map(b => b.dataset.to), sizingHidden: document.getElementById('sizing').hidden, infoHtml: document.getElementById('info').innerHTML,
        infoText: document.getElementById('info').innerText, timer: (document.querySelector('[data-timer]') || {}).textContent || '' };
    }""")


def play(pg, scenario, hands_target, rng, deadline):
    name = scenario["name"]
    url = scenario["url"]
    pg.goto(url, wait_until="domcontentloaded")
    seen = set(); done = 0; my_raises = []; handled_turn = set(); last_progress = time.time(); dropped = False; timeout_test = scenario.get("timeout")
    cur_hand = None
    while done < hands_target and time.time() < deadline:
        st = pg.evaluate(STATE_JS)
        if not st: time.sleep(0.1); continue
        if time.time() - last_progress > 90:
            ST.mismatches.append("[%s] stalled: %s" % (name, json.dumps({k: st[k] for k in ("phase", "handNo", "banner")}))); return done
        if st["handId"] != cur_hand and st["phase"] == "playing":
            cur_hand = st["handId"]; my_raises = []
        if st["phase"] == "settled" and st["handId"] not in seen:
            seen.add(st["handId"])
            time.sleep(0.4)
            verify_hand(pg, st["handId"], my_raises, name)
            done += 1; last_progress = time.time()
            if rng.random() < 0.5: pg.evaluate("document.getElementById('bNext').click()")
            else: pg.keyboard.press("Space")
            if done % 10 == 0: print("  [%s] %d hands, mismatches so far %d" % (name, done, len(ST.mismatches)), flush=True)
            continue
        t = st["turn"]
        if t and t["did"] not in handled_turn:
            last_progress = time.time()
            time.sleep(0.12)
            st2 = pg.evaluate(STATE_JS)
            if not st2["turn"] or st2["turn"]["did"] != t["did"]: continue
            handled_turn.add(t["did"]); ST.decisions += 1
            turn_checks(pg, t, name)
            L = t["legal"]
            if scenario.get("timeout") and not ST.timeouts and rng.random() < 0.3 and L["fold"]:
                ST.timeouts += 1
                time.sleep(scenario["dtime"] / 1000 + 1.5)       # let the server time us out
                b = pg.evaluate(STATE_JS)["banner"]
                ST.eq("timeout explained", "Time ran out" in b, True, "[%s] banner=%r" % (name, b))
                continue
            if scenario.get("drop") and not dropped and rng.random() < 0.25:
                dropped = True; ST.reconnects += 1
                pg.evaluate("window.__sharingan.drop()")
                for _ in range(100):
                    time.sleep(0.2)
                    s3 = pg.evaluate(STATE_JS)
                    if s3 and s3["turn"] and s3["turn"]["did"] == t["did"]: break
                else:
                    ST.mismatches.append("[%s] no resumed your_turn after reconnect" % name); return done
                time.sleep(0.2)
                ST.eq("same decision after reconnect", pg.evaluate(STATE_JS)["turn"]["did"], t["did"], "[%s]" % name)
                turn_checks(pg, pg.evaluate(STATE_JS)["turn"], name + "/resumed")
                handled_turn.add(t["did"])
            decide(pg, t, scenario, rng, my_raises)
            continue
        time.sleep(0.04)
    return done


def decide(pg, t, scenario, rng, my_raises):
    L = t["legal"]; r = rng.random(); jam = scenario.get("jam", 0.2)
    def click(i): pg.evaluate("(i) => document.getElementById(i).click()", i)
    if L["raise"] and r < jam:
        mn, mx = L["raise"]["min_to"], L["raise"]["max_to"]
        how = rng.choice(["allin_key", "allin_preset", "allin_type"])
        if how == "allin_key":
            pg.keyboard.press("a"); time.sleep(0.05); pg.keyboard.press("Enter")
        elif how == "allin_preset":
            pg.evaluate("() => { const b = Array.from(document.querySelectorAll('#presets button')).pop(); b.click(); }"); time.sleep(0.05); click("bRaise")
        else:
            pg.keyboard.press("r"); pg.keyboard.type(str(mx + 5000)); pg.keyboard.press("Enter"); ST.raise_clamps += 1
        my_raises.append(mx); return
    r2 = rng.random()
    if L["raise"] and r2 < 0.38:
        mn, mx = L["raise"]["min_to"], L["raise"]["max_to"]
        way = rng.choice(["type", "type", "preset", "slider", "under", "arrows"])
        if way == "type":
            v = rng.randint(mn, mx)
            pg.keyboard.press("r"); pg.keyboard.type(str(v)); pg.keyboard.press("Enter"); my_raises.append(v)
        elif way == "under":
            v = max(1, mn - rng.randint(1, 80))
            pg.keyboard.press("r"); pg.keyboard.type(str(v)); pg.keyboard.press("Enter"); my_raises.append(mn); ST.raise_clamps += 1
        elif way == "preset":
            btns = pg.evaluate("() => Array.from(document.querySelectorAll('#presets button')).map(b => +b.dataset.to)")
            i = rng.randrange(len(btns))
            pg.evaluate("(i) => document.querySelectorAll('#presets button')[i].click()", i); time.sleep(0.05); click("bRaise"); my_raises.append(btns[i])
        elif way == "arrows":
            n = rng.randint(1, 4)
            pg.keyboard.press("r")
            for _ in range(n): pg.keyboard.press("ArrowUp")
            time.sleep(0.05)
            v = pg.evaluate("() => +document.getElementById('slider').value")
            pg.keyboard.press("Enter"); my_raises.append(v)
            if not (mn <= v <= mx): ST.mismatches.append("arrow-key amount out of range %s" % v)
        else:
            v = rng.randint(mn, mx)
            pg.evaluate("(v) => { const s = document.getElementById('slider'); s.value = v; s.dispatchEvent(new Event('input')); }", v)
            time.sleep(0.05)
            shown = pg.evaluate("() => +document.getElementById('slider').value")
            click("bRaise"); my_raises.append(shown)
        return
    if L["fold"] and r2 < 0.38 + 0.22: click("bFold"); return
    if L["check"] and rng.random() < 0.05:
        ST.folds_free += 1
        pg.keyboard.press("f"); time.sleep(0.05); pg.keyboard.press("f"); return    # fold-when-free: hint, then a check
    if L["check"] or L["call"]: click("bCall"); return
    click("bFold")


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("base"); ap.add_argument("--arena", default="https://arena.sharingan.poker"); ap.add_argument("--scale", type=float, default=1.0)
    ap.add_argument("--seed", type=int, default=7)
    a = ap.parse_args()
    q = "?api=%s&fast=1&unit=chips&auto=0" % a.arena
    sc = lambda n: max(1, int(n * a.scale))
    scenarios = [
        dict(name="sharingan-reset", url=a.base + q + "&room=sharingan", n=sc(40), jam=0.12),
        dict(name="practice-reset", url=a.base + q + "&room=practice", n=sc(40), jam=0.15),
        dict(name="action-short-stacks", url=a.base + q + "&room=action&carry=1&stacks=100,35,12,60,150,20", n=sc(60), jam=0.35),
        dict(name="mixed-short-stacks", url=a.base + q + "&room=mixed&carry=1&stacks=150,100,45,100,25,80", n=sc(50), jam=0.3),
        dict(name="action-carry-jam", url=a.base + q + "&room=action&carry=1&stacks=40,100,100,70,15,100", n=sc(45), jam=0.5, drop=True),
        dict(name="practice-timeout", url=a.base + q + "&room=practice&dtime=3000", n=sc(6), jam=0.1, timeout=True, dtime=3000),
        dict(name="sharingan-jam", url=a.base + q + "&room=sharingan&carry=1&stacks=60,100,100,100,100,100", n=sc(20), jam=0.45),
    ]
    rng = random.Random(a.seed); total = 0; t0 = time.time()
    with sync_playwright() as p:
        b = p.chromium.launch()
        for s in scenarios:
            ctx = b.new_context(viewport={"width": 1280, "height": 900})
            pg = ctx.new_page()
            errs = []
            pg.on("pageerror", lambda e, errs=errs: errs.append(str(e)))
            pg.on("console", lambda m, errs=errs: errs.append(m.text) if m.type == "error" and "fonts" not in m.text else None)
            print("scenario", s["name"], flush=True)
            try:
                total += play(pg, s, s["n"], rng, time.time() + 1500)
            except Exception as e:
                ST.mismatches.append("[%s] exception %r" % (s["name"], e))
            ST.eq("[%s] page JS errors" % s["name"], errs, [], "")
            ctx.close()
        b.close()
    print("\n=== RESULT ===")
    print("hands verified:", ST.hands, " decisions:", ST.decisions, " turn-state checks:", ST.turn_checks, " field comparisons:", ST.checks)
    print("coverage: showdowns=%d all-in hands=%d side-pot hands=%d split-pot hands=%d uncalled-bet hands=%d raise clamps=%d reconnects=%d timeouts=%d fold-when-free=%d" % (
        ST.showdowns, ST.allin_hands, ST.side_pot_hands, ST.split_hands, ST.uncalled_hands, ST.raise_clamps, ST.reconnects, ST.timeouts, ST.folds_free))
    print("MISMATCHES:", len(ST.mismatches))
    for m in ST.mismatches[:40]: print("  -", m)
    print("elapsed %.0fs" % (time.time() - t0))
    sys.exit(1 if ST.mismatches or ST.hands < 200 and a.scale >= 1 else 0)

if __name__ == "__main__":
    main()
