// Poker Sharingan: play a 6-max no-limit hold'em table in the browser.
// REST: register (kind human) -> create table -> sit -> WebSocket /v1/play -> fill with house bots.
// Amounts on the wire are chips and always "raise TO". The UI shows big blinds (toggle: chips).
// The server is the single source of truth: this page only renders its events and sends legal actions.
(function () {
  "use strict";
  const $ = id => document.getElementById(id);
  const E = window.PokerEval;
  const params = new URLSearchParams(location.search);
  const FAST = params.has("fast");                       // testing: no pacing
  const BB_DEFAULT = 100;
  const HOLD_MS = 120000;                                // the server holds the table between hands until next_hand
  const DECISION_MS = 45000;                             // flat decision clock: no time bank, so every decision has the same, visible deadline
  const AWAY_HANDS = 3;                                  // this many hands in a row timed out = you are away: close the table
  const mySeat = 0;

  const ROOMS = {
    sharingan: { label: "vs SHARINGAN-1", desc: "You and five SHARINGAN-1. The hardest table we have.",
                 seats: ["house:sharingan-1", "house:sharingan-1", "house:sharingan-1", "house:sharingan-1", "house:sharingan-1"] },
    practice:  { label: "practice", desc: "You and five weaker house bots: random, calling station, tight-aggressive, nit, LAG.",
                 seats: ["house:random", "house:calling_station", "house:tight_aggressive", "house:nit", "house:lag"] },
    mixed:     { label: "mixed", desc: "You, two SHARINGAN-1 and three house bots.",
                 seats: ["house:sharingan-1", "house:tight_aggressive", "house:lag", "house:sharingan-1", "house:calling_station"] },
    action:    { label: "action", desc: "You against maniacs and LAGs. Big pots, all-ins and side pots.",
                 seats: ["house:maniac", "house:lag", "house:maniac", "house:calling_station", "house:lag"] },
  };

  // ---------- settings / session ----------
  const ls = {
    get(k, d) { try { const v = localStorage.getItem(k); return v === null ? d : v; } catch (e) { return d; } },
    set(k, v) { try { localStorage.setItem(k, v); } catch (e) {} },
  };
  let room = ls.get("sharingan.room", "sharingan"); if (!ROOMS[room]) room = "sharingan";
  if (params.get("room") && ROOMS[params.get("room")]) room = params.get("room");
  let unit = params.get("unit") || ls.get("sharingan.unit", "bb"); if (unit !== "chips") unit = "bb";
  let auto = params.has("auto") ? params.get("auto") !== "0" : ls.get("sharingan.auto", "1") === "1";
  let stackMode = params.has("carry") ? "carry" : ls.get("sharingan.stackmode", "reset");
  const AUTO_MS = parseInt(params.get("automs") || "0", 10);

  let gen = 0;                     // bumps on every (re)start so stale sockets/timers die
  let ws = null, tableId = null, me = null;
  let bb = BB_DEFAULT, sb = 50, startStack = 10000;
  let S = null;                    // per-table view model
  let turn = null;                 // pending your_turn data (+ skew)
  let queue = [], pumping = false;
  let tally = loadTally();
  let logRows = [];                // newest first
  let cur = null;                  // hand being recorded
  let anoms = [];                  // UI-vs-server consistency problems (tests require none)
  let errors = [];                 // server errors received
  let skipping = false, sleepers = [];
  let answered = new Set(), sent = [];
  let rt = 0, armed = false, foldHint = 0, allinHint = 0;
  let reconnects = 0, left = false, nextTimer = null, decisionMs = DECISION_MS;
  let clockSkew = 0;               // server clock minus ours, from welcome / snapshot / pong (never from a replayed your_turn)
  let resumed = false, ephemeral = false, tmoStreak = 0, tmoHand = null;
  const TAB = (function () { try { let t = sessionStorage.getItem("sharingan.tab"); if (!t) { t = Math.random().toString(36).slice(2); sessionStorage.setItem("sharingan.tab", t); } return t; } catch (e) { return Math.random().toString(36).slice(2); } })();

  function loadTally() {
    try { const t = JSON.parse(ls.get("sharingan.tally." + room, "null")); if (t && typeof t.hands === "number") return t; } catch (e) {}
    return { hands: 0, net: 0 };
  }
  function saveTally() { ls.set("sharingan.tally." + room, JSON.stringify(tally)); }
  // Hand history is kept per room next to the tally, so a reload (or a new table) never splits them.
  function loadHist() {
    try { const r = JSON.parse(ls.get("sharingan.hist." + room, "null")); if (Array.isArray(r)) { const seen = new Set(); return r.filter(x => x && x.id && !seen.has(x.id) && seen.add(x.id)).slice(0, 40); } } catch (e) {}
    return [];
  }
  function saveHist() { ls.set("sharingan.hist." + room, JSON.stringify(logRows.map(r => Object.assign({}, r, { cards: r.cards || null })))); }

  // The running table survives a reload: its id (and the hand in progress) is kept in localStorage.
  const SESS_KEY = "sharingan.sess." + ARENA_API, LOCK_KEY = "sharingan.lock." + ARENA_API;
  function loadSess() { try { return JSON.parse(ls.get(SESS_KEY, "null")); } catch (e) { return null; } }
  function saveSess(patch) {
    if (ephemeral) return;
    const s = Object.assign(loadSess() || {}, patch);
    ls.set(SESS_KEY, JSON.stringify(s));
  }
  function clearSess() { try { localStorage.removeItem(SESS_KEY); } catch (e) {} }
  function otherTabLive() {
    try { const l = JSON.parse(ls.get(LOCK_KEY, "null")); return !!(l && l.tab !== TAB && Date.now() - l.ts < 6000); } catch (e) { return false; }
  }
  setInterval(() => { if (tableId && !ephemeral && !left) ls.set(LOCK_KEY, JSON.stringify({ tab: TAB, ts: Date.now() })); }, 2000);

  function freshView() {
    return { seats: Array.from({ length: 6 }, (_, i) => ({ seat: i, name: null, house: false, stack: 0, bet: 0, contrib: 0, status: "empty", last: "", cards: null, shown: false, hn: "", net: null, win: false, pos: "", hl: null, ai: false })),
             board: [], street: "preflop", button: null, sb: null, bb: null, toAct: null, handNo: 0, handId: null, over: true, myCards: null,
             sweep: false, result: null, hlBoard: null, phase: "init", dealt: false, uncalled: null };
  }

  // ---------- formatting ----------
  function num(chips) {
    if (unit === "chips") return String(chips);
    return String(Math.round((chips * 100) / bb) / 100);
  }
  const f = num;
  function fs(chips) { return (chips > 0 ? "+" : "") + num(chips); }
  function U() { return unit === "chips" ? "" : " bb"; }
  // Stored texts (history lines, seat badges, headlines) keep amounts as tokens in chips and are
  // formatted when drawn, so switching bb/chips re-labels everything, history included.
  const C = n => "{c:" + n + "}", P = n => "{p:" + n + "}", SG = n => "{s:" + n + "}";
  function X(t) { return String(t == null ? "" : t).replace(/\{([cps]):(-?\d+)\}/g, (_, k, n) => k === "c" ? num(+n) + U() : k === "p" ? num(+n) : fs(+n) + U()); }
  const SUIT = { h: "♥", d: "♦", s: "♠", c: "♣" };
  const animated = new Set();
  function card(c, cls, key) {
    const r = c[0] === "T" ? "10" : c[0], s = c[1];
    let a = "";
    if (key && !animated.has(key)) { animated.add(key); a = " deal"; }
    return '<span class="cd' + (s === "h" || s === "d" ? " r" : "") + (cls ? " " + cls : "") + a + '" data-c="' + c + '">' + r + SUIT[s] + "</span>";
  }
  function cardTxt(c) { return (c[0] === "T" ? "10" : c[0]) + SUIT[c[1]]; }
  const back = key => { const a = key && !animated.has(key) ? (animated.add(key), " deal") : ""; return '<span class="cd back' + a + '">&nbsp;&nbsp;</span>'; };
  function clean(n) { return String(n || "").replace(/^house:/, ""); }
  const NAMES = { "sharingan-1": "SHARINGAN-1", tight_aggressive: "TAG", lag: "LAG", calling_station: "CALLING STN", nit: "NIT", random: "RANDOM", maniac: "MANIAC" };
  function pretty(n) { const c = clean(n).toLowerCase(); return NAMES[c] || c.replace(/_/g, " ").toUpperCase(); }
  const STREETS = { preflop: "Preflop", flop: "Flop", turn: "Turn", river: "River" };
  const SRV_CAT = { highcard: "high card", pair: "pair", twopair: "two pair", trips: "three of a kind", straight: "straight", flush: "flush", fullhouse: "full house", quads: "four of a kind", straightflush: "straight flush" };
  const norm = s => String(s || "").toLowerCase().replace(/[^a-z]/g, "");

  // ---------- positions ----------
  function computePositions(dealtSeats, button) {
    const n = dealtSeats.length, out = {};
    const order = [];
    for (let k = 0; k < 6; k++) { const s = (button + k) % 6; if (dealtSeats.indexOf(s) >= 0) order.push(s); }   // button first, clockwise
    if (n === 2) { out[order[0]] = "BTN/SB"; out[order[1]] = "BB"; return out; }
    out[order[0]] = "BTN"; out[order[1]] = "SB"; out[order[2]] = "BB";
    const rest = order.slice(3);
    const names = { 0: [], 1: ["UTG"], 2: ["UTG", "CO"], 3: ["UTG", "HJ", "CO"], 4: ["UTG", "LJ", "HJ", "CO"] }[rest.length] || [];
    rest.forEach((s, i) => { out[s] = names[i] || ""; });
    return out;
  }
  const slotOf = seat => (seat - mySeat + 6) % 6;
  function label(seat) {
    const s = S.seats[seat];
    return (seat === mySeat ? "You" : "S" + seat) + (s.pos ? " " + s.pos : "");
  }

  // ---------- pots (client side, cross-checked against the server) ----------
  // Pots split only at the contribution of an all-in player. Without an all-in there is one pot, however
  // uneven the contributions of the current street look while players have not yet acted.
  function potLevels() {
    const seats = S.seats.filter(s => s.status !== "empty" && s.status !== "out");
    const live = seats.filter(s => s.status !== "folded");
    const cut = [...new Set(live.filter(s => s.status === "all_in" && s.contrib > 0).map(s => s.contrib))].sort((a, b) => a - b);
    if (!cut.length) return [];
    const top = Math.max.apply(null, live.map(s => s.contrib));
    if (top > cut[cut.length - 1]) cut.push(top);
    let prev = 0; const out = [];
    cut.forEach(L => {
      const amount = seats.reduce((t, s) => t + Math.min(s.contrib, L) - Math.min(s.contrib, prev), 0);
      const eligible = live.filter(s => s.contrib > prev).map(s => s.seat);
      if (eligible.length >= 2) out.push({ amount, eligible });
      prev = L;
    });
    return out;
  }
  const sumContrib = () => S.seats.reduce((t, s) => t + s.contrib, 0);
  const sumBets = () => S.seats.reduce((t, s) => t + s.bet, 0);

  // ---------- REST ----------
  async function api(method, path, body, token) {
    const h = {};
    if (body !== undefined) h["content-type"] = "application/json";
    if (token) h["authorization"] = "Bearer " + token;
    let res;
    try { res = await fetch(ARENA_API + path, { method, headers: h, body: body !== undefined ? JSON.stringify(body) : undefined }); }
    catch (e) { const err = new Error("Cannot reach the arena at " + ARENA_API + "."); err.net = true; throw err; }
    let j = null;
    try { j = await res.json(); } catch (e) {}
    if (!res.ok) { const err = new Error((j && j.error && j.error.message) || ("HTTP " + res.status)); err.status = res.status; err.code = j && j.error && j.error.code; throw err; }
    return j;
  }

  async function ensurePlayer(force) {
    const key = "sharingan.player." + ARENA_API;
    if (!force) { try { const p = JSON.parse(localStorage.getItem(key) || "null"); if (p && p.token) return p; } catch (e) {} }
    const p = await api("POST", "/v1/players", { name: params.get("name") || "human", kind: "human" });
    const rec = { player_id: p.player_id, token: p.token, name: p.name };
    ls.set(key, JSON.stringify(rec));
    return rec;
  }

  // ---------- banner ----------
  function banner(msg, err, btn, fn) {
    const el = $("banner");
    el.className = "pk-banner" + (err ? " err" : "");
    el.textContent = msg || "";
    if (btn) { const b = document.createElement("button"); b.textContent = btn; b.onclick = fn; el.appendChild(b); }
  }

  // ---------- start / stop ----------
  function closeTable(id) {
    if (id && me) fetch(ARENA_API + "/v1/tables/" + id, { method: "DELETE", headers: { authorization: "Bearer " + me.token }, keepalive: true }).catch(() => {});
  }
  // Stop this page's view of the table. keep=false (leave, new table, room switch) also closes it on the server;
  // a page reload keeps it so the table, and the hand in progress, can be resumed.
  function teardown(keep) {
    gen++;
    queue = []; pumping = false; turn = null; clearTimeout(nextTimer); clearInterval(tickTimer); wakeSleepers();
    if (ws) { try { ws.onclose = null; if (!keep) ws.send(JSON.stringify({ v: 1, type: "leave", t_ms: Date.now(), table_id: tableId, data: { table_id: tableId } })); ws.close(); } catch (e) {} ws = null; }
    if (tableId && !keep) { closeTable(tableId); if (!ephemeral) clearSess(); }
    tableId = null;
  }

  // A player's tables that nobody is connected to (a closed tab, a crash) would live on the server forever.
  // Close them before opening another one. A table with a live socket (another tab) is left alone.
  async function cleanupStale(keepId) {
    try {
      const all = await api("GET", "/v1/tables");
      const dead = all.filter(x => x.owner === me.player_id && x.status !== "closed" && !x.match_id && x.table_id !== keepId &&
        (x.seats || []).every(z => z.player_id !== me.player_id || z.status === "disconnected"));
      await Promise.all(dead.map(x => api("DELETE", "/v1/tables/" + x.table_id, undefined, me.token).catch(() => {})));
    } catch (e) {}
  }

  async function resumableTable(sess) {
    if (!sess || !sess.table_id || sess.room !== room || sess.stack_mode !== stackMode || sess.player_id !== me.player_id) return null;
    if (params.get("stacks") || params.has("fresh")) return null;
    try {
      const t = await api("GET", "/v1/tables/" + sess.table_id);
      if (t.status === "closed" || !t.seats[mySeat] || t.seats[mySeat].player_id !== me.player_id) return null;
      return t;
    } catch (e) { if (e.status === 404) return null; throw e; }
  }

  async function start(tryResume) {
    teardown();
    const g = gen;
    left = false; reconnects = 0; resumed = false; tmoStreak = 0; tmoHand = null;
    ephemeral = otherTabLive();                       // another tab is playing the saved table: this one plays its own, unsaved
    tally = loadTally(); logRows = loadHist(); cur = null; anoms = []; errors = []; answered = new Set();
    S = freshView(); bb = BB_DEFAULT;
    S.phase = "init"; document.body.dataset.phase = "init";
    renderAll(); renderLog(true);
    banner("Connecting...");
    try {
      me = await ensurePlayer(false);
      const sess = loadSess();
      let t = tryResume && !ephemeral ? await resumableTable(sess) : null;
      if (g !== gen) return;
      if (!t) {
        await cleanupStale(ephemeral && sess ? sess.table_id : null);
        const cfg = { variant: "nlhe", max_seats: 6, small_blind: 50, big_blind: 100, starting_stack: startStack, stack_mode: stackMode,
                      decision_time_ms: parseInt(params.get("dtime") || String(DECISION_MS), 10), time_bank_ms: 0, hand_delay_ms: HOLD_MS, name: "play-" + room };
        const st = params.get("stacks");                // testing: first-hand stacks in bb, e.g. stacks=100,40,12,100,250,60 (needs carry)
        if (st) { cfg.mode = "sandbox"; cfg.sandbox = { stacks: st.split(",").map(x => Math.round(parseFloat(x) * 100)) }; cfg.stack_mode = "carry"; }
        try { t = await api("POST", "/v1/tables", cfg, me.token); }
        catch (e) { if (e.status === 401) { me = await ensurePlayer(true); t = await api("POST", "/v1/tables", cfg, me.token); } else throw e; }
        if (g !== gen) return;
        tableId = t.table_id;
        saveSess({ table_id: tableId, room, stack_mode: stackMode, player_id: me.player_id, open: null });
        await api("POST", "/v1/tables/" + tableId + "/seats/" + mySeat, {}, me.token);
      } else { resumed = true; tableId = t.table_id; }
      bb = (t.config && t.config.big_blind) || BB_DEFAULT; sb = (t.config && t.config.small_blind) || 50; startStack = (t.config && t.config.starting_stack) || 10000;
      await connect(g);
      const bots = ROOMS[room].seats;
      for (let i = 0; i < bots.length; i++) {
        if (g !== gen) return;
        if (resumed && t.seats[i + 1] && t.seats[i + 1].player_id) continue;
        await api("POST", "/v1/tables/" + tableId + "/seats/" + (i + 1), { house: bots[i] }, me.token);
      }
      if (S.phase === "init") banner(resumed ? "Resuming your table..." : "Waiting for the deal...");
      else if (!S.toAct && !turn) banner("");
    } catch (e) {
      if (g !== gen) return;
      let m = e.message;
      if (e.status === 400 && /sharingan-1|house/i.test(m) && room !== "practice") m += " Try the practice room.";
      if (e.status === 429) m += " Close other tabs or wait a minute.";
      banner(m, true, "Retry", () => start(true));
      renderControls();
    }
  }

  function connect(g) {
    return new Promise((resolve, reject) => {
      const url = ARENA_API.replace(/^http/, "ws") + "/v1/play?token=" + encodeURIComponent(me.token);
      const sock = new WebSocket(url);
      let opened = false;
      sock.onopen = () => { opened = true; ws = sock; send("hello", { client: "sharingan-web/2" }); resolve(); };
      sock.onerror = () => { if (!opened) reject(new Error("Cannot open the game socket at " + url.replace(/\?.*/, "") + ".")); };
      sock.onmessage = ev => {
        if (g !== gen) return;
        let m; try { m = JSON.parse(ev.data); } catch (e) { return; }
        // The socket carries every table this player sits at (a leaked or just-closed one too): only ours counts.
        if (m.table_id && tableId && m.table_id !== tableId) return;
        if (m.type === "welcome" && m.data && m.data.server_time) clockSkew = m.data.server_time - Date.now();
        else if ((m.type === "table_snapshot" || m.type === "pong") && m.t_ms) clockSkew = m.t_ms - Date.now();
        queue.push(m); pump(g);
      };
      sock.onclose = () => {
        if (g !== gen || !opened || left) return;
        turn = null; renderControls();
        reconnect(g);
      };
    });
  }

  async function reconnect(g) {
    // Mid-hand the seat keeps playing on the timeout policy; the snapshot we get back is authoritative.
    for (let i = 0; i < 8; i++) {
      if (g !== gen) return;
      banner("Disconnected. Reconnecting" + (i ? " (try " + (i + 1) + ")" : "") + "... the seat auto-folds if a decision times out.", true);
      await new Promise(r => setTimeout(r, i === 0 ? 250 : Math.min(8000, 1000 * i)));
      if (g !== gen) return;
      try {
        await connect(g);
        reconnects++;
        banner("Reconnected.");
        return;
      } catch (e) {
        let t2 = null;
        try { t2 = await api("GET", "/v1/tables/" + tableId); }
        catch (e2) { if (e2.status === 404) { t2 = { status: "closed" }; } }
        if (t2 && t2.status === "closed") { clearSess(); banner("The table is gone (it closed while you were away).", true, "New table", () => start()); return; }
      }
    }
    banner("The arena is unreachable. Check your connection.", true, "Try again", () => { reconnect(g); });
  }

  function send(type, data) {
    if (!ws || ws.readyState !== 1) return false;
    ws.send(JSON.stringify({ v: 1, type, table_id: tableId, t_ms: Date.now(), data }));
    return true;
  }
  setInterval(() => send("ping", {}), 15000);

  // ---------- pacing ----------
  function sleep(ms) {
    if (FAST || skipping || ms <= 0) return Promise.resolve();
    return new Promise(r => { const t = setTimeout(() => { sleepers = sleepers.filter(x => x !== done); r(); }, ms); const done = () => { clearTimeout(t); r(); }; sleepers.push(done); });
  }
  function wakeSleepers() { const l = sleepers; sleepers = []; l.forEach(x => x()); }

  async function pump(g) {
    if (pumping) return;
    pumping = true;
    while (queue.length && g === gen) {
      const m = queue.shift();
      try { await handle(m); } catch (e) { console.warn("event error", e, m); anoms.push("event error " + m.type + ": " + e.message); }
      renderAll();
      if (!FAST && queue.length) {
        const t = m.type, d = m.data || {};
        await sleep(t === "action" ? (d.forced ? 120 : 520) : t === "street" ? 380 : t === "hand_start" ? 450 : 0);
      }
    }
    pumping = false;
  }

  // ---------- protocol handlers ----------
  async function handle(m) {
    const d = m.data || {};
    switch (m.type) {
      case "welcome": break;
      case "table_snapshot": await snapshot(d); break;
      case "seat_update": { const s = S.seats[d.seat]; if (s && S.over) { s.name = d.name; s.house = !!d.house; s.stack = d.stack; if (!d.player_id) s.status = "empty"; else if (s.status === "empty") s.status = "seated"; } break; }
      case "hand_start": handStart(d); break;
      case "hole_cards": if (d.seat === mySeat) { S.myCards = d.cards; if (cur) cur.cards = d.cards; } S.dealt = true; break;
      case "action": action(d); break;
      case "street": await street(d); break;
      case "your_turn": yourTurn(d); break;
      case "hand_end": await handEnd(d); break;
      case "error": {
        const code = d.code || "error";
        errors.push(code + ": " + (d.message || ""));
        if (code === "illegal_action" || code === "bad_amount") banner("The server refused that action: " + (d.message || code), true);
        else if (code !== "not_your_turn" && code !== "stale_decision") banner(d.message || code, true);
        break;
      }
      default: break;
    }
  }

  // Authoritative state after (re)connecting: replaces the model of the hand in progress.
  async function snapshot(d) {
    if (d.config && d.config.big_blind) { bb = d.config.big_blind; sb = d.config.small_blind; startStack = d.config.starting_stack; decisionMs = d.config.decision_time_ms || decisionMs; }
    const h = d.hand;
    const running = !!(h && h.hand_id && h.street !== "showdown");
    // A hand we were in may have ended while we were away (reload, dropped socket): settle it from the stored history first.
    const sess = loadSess();
    const pendingId = !S.over && S.handId ? S.handId : (sess && sess.open && sess.open.id) || null;
    if (pendingId && (!running || h.hand_id !== pendingId) && !logRows.some(r => r.id === pendingId)) { await finishFromHistory(pendingId); }
    (d.seats || []).forEach(x => { const s = S.seats[x.seat]; s.name = x.name; s.house = !!x.house; if (S.over || s.status === "empty") { s.stack = x.stack; s.status = x.player_id ? (S.over ? "seated" : s.status) : "empty"; } });
    if (running) {
      // A hand is running: replace the model from the snapshot (authoritative).
      const same = S.handId === h.hand_id && cur;
      S.handNo = h.hand_no; S.handId = h.hand_id; S.over = false; S.result = null; S.board = h.board || []; S.street = h.street; S.toAct = h.to_act;
      S.button = h.button_seat; S.sb = h.sb_seat; S.bb = h.bb_seat; S.dealt = true; S.sweep = false; S.uncalled = null; S.hlBoard = null;
      const dealt = (h.players || []).map(p => p.seat);
      const pos = computePositions(dealt, h.button_seat);
      S.seats.forEach(s => { if (dealt.indexOf(s.seat) < 0 && s.status !== "empty") s.status = "out"; s.cards = null; s.shown = false; s.hn = ""; s.net = null; s.win = false; s.hl = null; });
      (h.players || []).forEach(p => { const s = S.seats[p.seat]; s.stack = p.stack; s.bet = p.street_contrib; s.contrib = p.hand_contrib; s.status = p.status === "out" ? "out" : p.status; s.pos = pos[p.seat] || ""; s.name = p.name || s.name; s.ai = p.status === "all_in"; if (!same) s.last = ""; });
      const mine = d.hole_cards && d.hole_cards[String(mySeat)];
      if (mine) S.myCards = mine;
      if (!same) cur = { id: h.hand_id, cards: mine || null, lines: [], acts: [], reconn: true };
      else cur.reconn = true;
      cur.lines.push({ cls: "n", s: "", t: "Reconnected: actions before this point may be missing here." });
      saveSess({ open: { id: h.hand_id } });
      skipping = false; clearTimeout(nextTimer); $("nextrow").hidden = true;
      S.phase = "playing"; document.body.dataset.phase = "playing";
      banner("");
    } else if (resumed && S.phase === "init") {
      banner("");
      // Resumed between hands: show the last result and the next-hand controls.
      const last = logRows[0];
      if (last) {
        S.handNo = last.no; S.handId = last.id; S.msg = last.msg; S.over = true; S.phase = "settled";
        document.body.dataset.phase = "settled";
        banner("");
        promptNext(false);
      }
    }
    if (resumed) banner("");
  }

  // The hand ended while we were not listening: fetch it from the hand history and finish it the normal way.
  async function finishFromHistory(handId) {
    try {
      const h = await api("GET", "/v1/hands/" + handId, undefined, me.token);
      const dealt = h.players.map(p => p.seat);
      const pos = computePositions(dealt, h.button_seat);
      S.handId = h.hand_id; S.handNo = h.hand_no; S.over = false; S.result = null; S.button = h.button_seat; S.sb = h.sb_seat; S.bb = h.bb_seat;
      S.dealt = true; S.board = h.board; S.myCards = (h.hole_cards || {})[mySeat] || S.myCards; S.toAct = null; S.uncalled = null;
      S.seats.forEach(s => { s.bet = 0; s.contrib = 0; s.cards = null; s.shown = false; s.hn = ""; s.net = null; s.win = false; s.hl = null; s.last = ""; s.ai = false; s.pos = pos[s.seat] || ""; if (dealt.indexOf(s.seat) < 0 && s.status !== "empty") s.status = "out"; });
      h.players.forEach(p => { const s = S.seats[p.seat]; s.stack = p.stack_start; s.status = "active"; s.name = p.name || s.name; });
      h.actions.forEach(a => {
        const s = S.seats[a.seat], k = a.type || a.kind;
        s.contrib += a.added; s.stack -= a.added;
        if (k === "fold") s.status = "folded"; else if (a.all_in) { s.status = "all_in"; s.ai = true; }
      });
      cur = { id: h.hand_id, cards: S.myCards, lines: [], acts: [], reconn: true };
      rebuildLog(h);
      skipping = true;                                   // no showdown theatre for a hand that is already over
      const live = S.seats.filter(s => s.status === "active" || s.status === "all_in").map(s => s.seat);
      const showdown = h.ended_by === "showdown" ? live.map(seat => ({ seat, cards: h.hole_cards[seat], shown: true, hand_rank: "", best5: [] })) : [];
      await handEnd({ hand_id: h.hand_id, board: h.board, hole_cards: h.hole_cards, pots: h.pots, results: h.results, showdown, ended_by: h.ended_by, _hist: true });
    } catch (e) {
      anoms.push("history fetch failed: " + e.message);
      cur = null; S.over = true; S.toAct = null; S.handId = null; saveSess({ open: null }); renderAll();
    }
  }

  function handStart(d) {
    clearTimeout(nextTimer);
    bb = d.big_blind || bb; sb = d.small_blind || sb;
    skipping = false; armed = false; turn = null;
    const prev = S.seats.map(s => s.stack);
    S.over = false; S.result = null; S.board = []; S.street = "preflop"; S.myCards = null; S.toAct = null; S.sweep = false; S.dealt = false;
    S.uncalled = null; S.hlBoard = null; S.handNo = d.hand_no; S.handId = d.hand_id; S.button = d.button_seat; S.sb = d.sb_seat; S.bb = d.bb_seat;
    const dealt = (d.players || []).filter(p => p.in_hand).map(p => p.seat);
    const pos = computePositions(dealt, d.button_seat);
    S.seats.forEach(s => { s.bet = 0; s.contrib = 0; s.last = ""; s.cards = null; s.shown = false; s.hn = ""; s.net = null; s.win = false; s.hl = null; s.pos = ""; s.ai = false; if (!s.name) s.status = "empty"; });
    const rebuys = [];
    (d.players || []).forEach(p => {
      const s = S.seats[p.seat]; s.name = p.name; s.stack = p.stack_start; s.status = p.in_hand ? "active" : "out"; s.pos = p.in_hand ? (pos[p.seat] || "") : "";
      if (stackMode === "carry" && p.in_hand && prev[p.seat] === 0 && p.stack_start > 0) rebuys.push(p.seat);   // reset mode refills everyone every hand: not a rebuy
    });
    cur = { id: d.hand_id, cards: null, lines: [], acts: [], reconn: false, start: (d.players || []).map(p => ({ seat: p.seat, stack: p.stack_start })) };
    saveSess({ open: { id: d.hand_id } });
    rebuys.forEach(r => cur.lines.push({ cls: "n", s: "", t: (r === mySeat ? "You rebuy" : "S" + r + " rebuys") + " to " + C(startStack) }));
    S.phase = "playing"; document.body.dataset.phase = "playing"; document.body.dataset.handId = d.hand_id;
    banner("");
  }

  function actText(d) {
    const ai = d.all_in ? " (all-in)" : "";
    switch (d.type) {
      case "post_sb": return "posts small blind " + C(d.added) + ai;
      case "post_bb": return "posts big blind " + C(d.added) + ai;
      case "post_ante": return "posts ante " + C(d.added);
      case "fold": return "folds";
      case "check": return "checks";
      case "call": return "calls " + C(d.added) + (d.to !== d.added ? " (total " + P(d.to) + ")" : "") + ai;
      case "bet": return "bets " + C(d.to) + ai;
      case "raise": return "raises to " + C(d.to) + ai;
      default: return d.type;
    }
  }
  function shortText(d) {
    if (d.forced) return d.type === "post_sb" || d.type === "post_bb" ? "post " + P(d.added) : "ante";
    switch (d.type) {
      case "fold": return "fold"; case "check": return "check";
      case "call": return "call " + P(d.added);
      case "bet": return "bet " + P(d.to);
      case "raise": return "raise " + P(d.to);
      default: return d.type;
    }
  }

  function action(d) {
    const s = S.seats[d.seat];
    const expectStack = s.stack - d.added;
    if (d.stack !== expectStack) anoms.push("stack mismatch seat " + d.seat + " seq " + d.seq + ": model " + expectStack + " server " + d.stack);
    s.stack = d.stack; s.contrib += d.added;
    if (d.type !== "fold" && d.type !== "check") s.bet = d.to;
    if (d.type === "fold") s.status = "folded";
    if (d.all_in && d.type !== "fold") { s.status = "all_in"; s.ai = true; }
    s.last = shortText(d) + (d.all_in && d.type !== "fold" ? " AI" : "") + (d.timeout ? " (time)" : "");
    S.toAct = d.next_to_act;
    if (sumContrib() !== d.pot) anoms.push("pot mismatch seq " + d.seq + ": model " + sumContrib() + " server " + d.pot);
    if (cur) {
      if (!d.forced) cur.acts.push({ street: d.street, seat: d.seat, type: d.type });
      cur.lines.push({ cls: "a", ev: [d.seat, d.type, d.to, d.added, d.all_in ? 1 : 0].join(":"), s: "", t: label(d.seat) + " " + actText(d) + (d.timeout ? " (timed out)" : ""), street: d.street, seat: d.seat });
    }
    if (d.seat === mySeat && !d.forced) {
      if (turn) { turn = null; clearInterval(tickTimer); }   // the server acted for us (timeout): the decision is over
      if (!d.timeout) tmoStreak = 0;
      else { if (tmoHand !== d.hand_id) { tmoHand = d.hand_id; tmoStreak++; } banner("Time ran out: the server " + (d.type === "fold" ? "folded" : "checked") + " for you.", true); }
    }
  }

  async function street(d) {
    if (sumBets() > 0 && !FAST && !skipping) { S.sweep = true; renderAll(); await sleep(300); S.sweep = false; }
    S.board = d.board || []; S.street = d.street;
    S.seats.forEach(s => { s.bet = 0; if (s.status === "active" || s.status === "all_in") s.last = s.status === "all_in" ? "all-in" : ""; });
    if (sumContrib() !== d.pot) anoms.push("street pot mismatch " + d.street + ": model " + sumContrib() + " server " + d.pot);
    if (cur) cur.lines.push({ cls: "st", st: d.street + ":" + (d.board || []).join(""), s: STREETS[d.street], t: (d.new_cards || []).map(cardTxt).join(" ") + "   pot " + C(d.pot) });
  }

  function yourTurn(d) {
    turn = d;
    S.toAct = d.seat; S.board = d.board || S.board; S.street = d.street || S.street;
    S.myCards = d.hole_cards; S.dealt = true;
    if (cur && cur.reconn && d.history && d.history.length) {
      // Resumed mid-hand: your_turn carries the full public history of the hand, so the log and the seat badges are exact.
      rebuildLog({ players: d.players, button_seat: d.button_seat, board: d.board, actions: d.history });
      S.seats.forEach(s => { s.last = ""; });
      d.history.forEach(a => {
        const s = S.seats[a.seat];
        if (a.type === "fold" || a.street !== d.street) return;
        s.last = shortText(a) + (a.all_in ? " AI" : "") + (a.timeout ? " (time)" : "");
      });
      S.seats.forEach(s => { if (s.status === "all_in" && !s.last) s.last = "all-in"; });
    }
    (d.players || []).forEach(p => {
      const s = S.seats[p.seat];
      if (s.stack !== p.stack || s.bet !== p.street_contrib || s.contrib !== p.hand_contrib) anoms.push("your_turn state mismatch seat " + p.seat + ": model " + [s.stack, s.bet, s.contrib] + " server " + [p.stack, p.street_contrib, p.hand_contrib]);
      s.stack = p.stack; s.bet = p.street_contrib; s.contrib = p.hand_contrib;
    });
    armed = false; foldHint = 0; allinHint = 0;
    const L = d.legal;
    if (L.raise) rt = L.raise.min_to;
    setupControls();
    banner("");
    if (!FAST) fitTable();
  }
  // Phone: the dock is pinned to the bottom of the screen. On your turn scroll just enough that the whole table,
  // your own seat included, sits above it (never the other way round: nothing is ever hidden under the dock).
  function fitTable() {
    if (window.innerWidth > 640) return;
    try {
      const hero = document.querySelector(".seat.me"); if (!hero) return;
      const t = $("tbl").getBoundingClientRect(), hb = hero.getBoundingClientRect().bottom;
      const limit = window.innerHeight - $("dock").offsetHeight - 4;
      let dy = 0;
      if (hb > limit) dy = hb - limit;
      else if (t.top < 0) dy = -Math.min(-t.top, limit - hb);
      if (Math.abs(dy) > 2) window.scrollBy({ top: dy, behavior: "smooth" });
    } catch (e) {}
  }

  // ---------- hand end ----------
  function lastAggressor(live, button) {
    const acts = (cur && cur.acts) || [];
    let st = null;
    acts.forEach(a => { st = a.street; });
    const of = acts.filter(a => a.street === st && (a.type === "bet" || a.type === "raise"));
    if (of.length) return of[of.length - 1].seat;
    for (let k = 1; k <= 6; k++) { const s = (button + k) % 6; if (live.indexOf(s) >= 0) return s; }
    return live[0];
  }

  async function handEnd(d) {
    turn = null; S.toAct = null; clearInterval(tickTimer);
    S.board = d.board || S.board;
    if (cur && cur.reconn && !d._hist && cur.id) { try { rebuildLog(await api("GET", "/v1/hands/" + cur.id, undefined, me.token)); } catch (e) {} }
    const hole = d.hole_cards || {};
    const showdown = d.ended_by === "showdown";
    const results = d.results || [];
    const pots = d.pots || [];
    const live = S.seats.filter(s => s.status === "active" || s.status === "all_in").map(s => s.seat);
    // uncalled bet: the top contributor's excess over the second highest
    const contribs = S.seats.filter(s => s.contrib > 0).sort((a, b) => b.contrib - a.contrib);
    let unc = null;
    if (contribs.length >= 2 && contribs[0].contrib > contribs[1].contrib) unc = { seat: contribs[0].seat, amount: contribs[0].contrib - contribs[1].contrib };
    else if (contribs.length === 1) unc = { seat: contribs[0].seat, amount: contribs[0].contrib };
    // 1. sweep the last street's bets (and give back the uncalled part)
    if (sumBets() > 0 && !FAST && !skipping) { S.sweep = true; renderAll(); await sleep(300); S.sweep = false; }
    if (unc && unc.amount > 0) {
      S.uncalled = unc;
      const us = S.seats[unc.seat]; us.stack += unc.amount; us.contrib -= unc.amount; us.bet = Math.max(0, us.bet - unc.amount);
      if (cur) cur.lines.push({ cls: "r", unc: unc.seat + ":" + unc.amount, s: "", t: "Uncalled bet " + C(unc.amount) + " returned to " + label(unc.seat) });
      renderAll(); await sleep(700);
    }
    S.seats.forEach(s => { s.bet = 0; });
    // 2. client-side evaluation (hand names, best five), cross-checked against the server
    const hands = {};
    const rows = {};
    (d.showdown || []).forEach(x => { rows[x.seat] = x; });
    let order = [];
    if (showdown) {
      const liveSeats = (d.showdown || []).map(x => x.seat);
      liveSeats.forEach(sn => { hands[sn] = E.best((hole[sn] || rows[sn].cards).concat(S.board)); });
      liveSeats.forEach(sn => {
        const h = hands[sn]; let k = 0;
        liveSeats.forEach(o => { if (!k && o !== sn) k = E.kickerVs(h, hands[o]); });
        h.label = k ? E.withKicker(h, k) : E.name(h);
        const r = rows[sn];
        if (r && r.hand_rank && SRV_CAT[norm(r.hand_rank)] !== E.catName(h)) anoms.push("hand rank mismatch seat " + sn + ": client " + E.catName(h) + " server " + r.hand_rank);
        if (r && r.best5 && r.best5.length === 5 && E.best(r.best5).score !== h.score) anoms.push("best5 mismatch seat " + sn);
      });
      const first = lastAggressor(liveSeats, S.button);
      for (let k = 0; k < 6; k++) { const sn = (first + k) % 6; if (liveSeats.indexOf(sn) >= 0) order.push(sn); }
      // server pots vs client awards
      const ca = E.awards(pots.map(p => ({ amount: p.amount, eligible_seats: p.eligible_seats })), hole, S.board, S.button, 6, null);
      ca.forEach((c, i) => {
        const sp = pots[i];
        if (JSON.stringify(c.winners) !== JSON.stringify(sp.winners) || JSON.stringify(c.share) !== JSON.stringify(sp.share)) anoms.push("pot " + i + " award mismatch: client " + JSON.stringify([c.winners, c.share]) + " server " + JSON.stringify([sp.winners, sp.share]));
      });
    }
    // 3. reveal in showdown order
    if (showdown) {
      for (const sn of order) {
        const s = S.seats[sn];
        s.cards = hole[sn] || rows[sn].cards; s.shown = true; s.hn = hands[sn].label; s.last = "";
        if (cur) cur.lines.push({ cls: "sd", sd: sn + ":" + s.cards.join(""), s: "", t: label(sn) + " shows " + s.cards.map(cardTxt).join(" ") + ": " + hands[sn].label });
        renderAll();
        if (sn !== order[order.length - 1]) await sleep(900);
      }
      await sleep(700);
    }
    // 4. payout
    const net = {}, endStack = {};
    results.forEach(r => { net[r.seat] = r.net; endStack[r.seat] = r.stack_end; });
    const lines = [];
    const labelFor = (i, n) => (n === 1 ? "Pot" : i === 0 ? "Main pot" : "Side pot" + (n > 2 ? " " + i : ""));
    const shown = pots.map(p => {
      let y = p.amount, u = 0;
      if (unc && unc.amount > 0 && p.eligible_seats.length === 1 && p.winners.length === 1 && p.winners[0] === unc.seat) { u = Math.min(unc.amount, p.amount); y = p.amount - u; }
      return { p, y, u };
    });
    const real = shown.filter(x => x.y > 0);
    real.forEach((x, i) => {
      const p = x.p, n = real.length;
      const winTxt = p.winners.length === 1
        ? label(p.winners[0]) + " wins " + C(p.share[0] - x.u)
        : "split: " + p.winners.map((w, k) => label(w) + " " + C(p.share[k] - (w === unc?.seat ? x.u : 0))).join(", ");
      lines.push({ amount: x.y, u: x.u, winners: p.winners, share: p.share, text: labelFor(i, n) + " " + C(x.y) + ": " + winTxt, eligible: p.eligible_seats });
    });
    S.result = lines;
    // Who won what, strictly from the hand_end pots: shares per seat, minus the uncalled bet that went back to its owner
    // (the server lists it as a one-seat pot; giving a bet back is not winning).
    const won = {};
    pots.forEach(p => p.winners.forEach((w, k) => { won[w] = (won[w] || 0) + p.share[k]; }));
    if (unc && unc.amount > 0) won[unc.seat] = (won[unc.seat] || 0) - unc.amount;
    const winners = new Set(Object.keys(won).filter(k => won[k] > 0).map(Number));
    S.seats.forEach(s => { s.win = winners.has(s.seat); });
    // highlight the winning five-card hands
    if (showdown) {
      const hb = new Set();
      winners.forEach(w => {
        const h = hands[w]; if (!h) return;
        const s = S.seats[w]; s.hl = new Set(h.cards.filter(c => (s.cards || []).indexOf(c) >= 0));
        h.cards.forEach(c => { if (S.board.indexOf(c) >= 0) hb.add(c); });
      });
      S.hlBoard = hb;
      S.seats.forEach(s => { if (s.shown && !winners.has(s.seat)) s.hl = new Set(); });
    }
    S.seats.forEach(s => {
      if (endStack[s.seat] !== undefined) s.stack = endStack[s.seat];
      s.net = net[s.seat] === undefined ? null : net[s.seat]; s.last = "";
    });
    // verify the payout arithmetic against the model
    S.seats.forEach(s => {
      const r = results.find(x => x.seat === s.seat); if (!r) return;
      let won = 0; pots.forEach(p => p.winners.forEach((w, k) => { if (w === s.seat) won += p.share[k]; }));
      const startCh = (cur && cur.start && (cur.start.find(x => x.seat === s.seat) || {}).stack);
      if (r.net !== won - (s.contrib + (unc && unc.seat === s.seat ? unc.amount : 0))) anoms.push("net mismatch seat " + s.seat + ": server " + r.net + " vs pots " + (won - s.contrib));
      if (startCh !== undefined && startCh + r.net !== r.stack_end) anoms.push("stack_end mismatch seat " + s.seat);
    });
    const mine = net[mySeat] || 0;
    const nm = w => (w === mySeat ? "You" : "S" + w);
    const ws_ = [...winners].sort((a, b) => a - b);
    const chop = pots.some(p => p.winners.length > 1 && !(unc && p.eligible_seats.length === 1));
    const withHand = w => (showdown && hands[w] ? " with " + hands[w].label : "");
    let msg;
    if (mine > 0) msg = "You " + (chop && won[mySeat] > 0 && ws_.length > 1 ? "split the pot and win " : "win ") + C(mine) + (ws_.length === 1 ? withHand(mySeat) : "");
    else {
      const names = ws_.map(nm).sort((a, b) => (a === "You") - (b === "You"));   // "S2, S5 and You"
      const who = names.length > 1 ? [names.slice(0, -1).join(", ") + " and " + names[names.length - 1]] : names;
      const lead = mine < 0 ? "You lose " + C(-mine) + ". " : "You break even. ";
      const verb = ws_.length === 1 ? (ws_[0] === mySeat ? "win" : "wins") : chop ? "split" : "win";
      msg = lead + (who.length ? who.join(" and ") + " " + verb + (ws_.length === 1 ? withHand(ws_[0]) : "") : "");
    }
    S.msg = msg; S.lastEnd = d;
    if (cur) {
      lines.forEach(l => cur.lines.push({ cls: "r", pot: l.amount + ":" + l.u + ":" + l.winners.join(",") + ":" + l.share.join(","), s: "", t: l.text }));
      cur.lines.push({ cls: "r", s: "", t: "Result: " + results.map(r => nm(r.seat) + " " + SG(r.net)).join(", ") });
    }
    // One row per hand_id, ever: a hand is counted once even if its end reaches us twice (reconnect, replay).
    if (!logRows.some(r => r.id === S.handId)) {
      tally.hands += 1; tally.net += mine; saveTally();
      logRows.unshift({ no: tally.hands, id: S.handId, cards: S.myCards || (cur && cur.cards) || null, board: S.board.slice(), net: mine, lines: cur ? cur.lines : [], msg });
      if (logRows.length > 40) logRows.pop();
      saveHist();
    }
    cur = null; saveSess({ open: null });
    S.over = true; S.phase = "settled";
    S.handNo = logRows[0] && logRows[0].id === S.handId ? logRows[0].no : S.handNo;
    document.body.dataset.phase = "settled"; document.body.dataset.handId = S.handId;
    renderAll(); renderLog(true);
    promptNext(showdown);
  }

  function rebuildLog(h) {
    if (!cur) return;
    const lines = []; let shownTo = 0;
    const dealt = h.players.map(p => p.seat);
    const pos = computePositions(dealt, h.button_seat);
    const lab = s => (s === mySeat ? "You" : "S" + s) + (pos[s] ? " " + pos[s] : "");
    const order = ["preflop", "flop", "turn", "river"], lens = [0, 3, 4, 5];
    let pot = 0;
    const emit = k => {   // street marker for order[k], once
      if (k <= shownTo || h.board.length < lens[k]) return;
      for (let j = shownTo + 1; j <= k; j++) {
        if (h.board.length < lens[j]) break;
        lines.push({ cls: "st", st: order[j] + ":" + h.board.slice(0, lens[j]).join(""), s: STREETS[order[j]], t: h.board.slice(j === 1 ? 0 : lens[j] - 1, lens[j]).map(cardTxt).join(" ") + "   pot " + C(pot) });
        shownTo = j;
      }
    };
    cur.acts = [];
    h.actions.forEach(a => {
      const kind = a.type || a.kind;
      const k = order.indexOf(a.street);
      if (k > shownTo) { pot = a.pot - a.added; emit(k); }
      pot = a.pot;
      const d = Object.assign({}, a, { type: kind });
      if (!a.forced) cur.acts.push({ street: a.street, seat: a.seat, type: kind });
      lines.push({ cls: "a", ev: [a.seat, kind, a.to, a.added, a.all_in ? 1 : 0].join(":"), s: "", t: lab(a.seat) + " " + actText(d) + (a.timeout ? " (timed out)" : ""), street: a.street, seat: a.seat });
    });
    emit(3);
    cur.lines = lines;
    cur.reconn = false;
  }

  function leaveAway() {
    // Several hands in a row timed out: nobody is playing. Close the table instead of folding forever.
    left = true; teardown(); S.toAct = null; turn = null; renderAll();
    $("nextrow").hidden = true; document.body.dataset.phase = "left";
    banner("You were away for " + AWAY_HANDS + " hands, so the table was closed. Your result stays in the session.", false, "Sit down", () => start());
  }
  function promptNext(showdown) {
    if (tmoStreak >= AWAY_HANDS) { leaveAway(); return; }
    $("nextrow").hidden = left;
    if (left) return;
    clearTimeout(nextTimer);
    const ms = AUTO_MS || (showdown ? 6000 : 4000);
    if (!auto) { $("nextnote").textContent = "Take your time. Press space or Next hand to continue."; return; }
    if (FAST) { nextHand(); return; }
    const t0 = Date.now();
    const tick = () => {
      const left_ = Math.max(0, Math.ceil((ms - (Date.now() - t0)) / 1000));
      $("nextnote").textContent = "Next hand in " + left_ + "s (auto-deal on)";
      if (Date.now() - t0 >= ms) nextHand(); else nextTimer = setTimeout(tick, 250);
    };
    tick();
  }
  function nextHand() {
    clearTimeout(nextTimer);
    if (!S.over || left) return;
    $("nextrow").hidden = true;
    $("nextnote").textContent = "";
    send("next_hand", {});
    banner("Dealing...");
  }

  // ---------- render ----------
  function renderAll() { renderRoom(); renderTable(); renderControls(); renderResult(); renderOpts(); scheduleLog(); }

  function renderRoom() {
    const el = $("rooms");
    if (!el.firstChild) {
      Object.keys(ROOMS).forEach(k => {
        const b = document.createElement("button"); b.dataset.room = k; b.textContent = ROOMS[k].label;
        b.onclick = () => { if (room === k) return; room = k; ls.set("sharingan.room", k); start(); };
        el.appendChild(b);
      });
    }
    el.querySelectorAll("button").forEach(b => { b.className = b.dataset.room === room ? "on" : ""; b.setAttribute("aria-pressed", b.dataset.room === room); });
    $("roomdesc").textContent = ROOMS[room].desc;
  }
  function renderOpts() {
    $("oUnit").textContent = "units: " + unit;
    $("oAuto").textContent = "auto-deal: " + (auto ? "on" : "off");
    $("oStack").textContent = "stacks: " + stackMode;
    $("blinds").textContent = unit === "chips" ? sb + "/" + bb + " chips" : num(sb) + "/" + num(bb) + " bb";
    $("oStack").title = stackMode === "reset" ? "Every hand starts at 100 bb" : "Stacks carry over; a busted seat rebuys to 100 bb";
  }

  function seatHtml(s) {
    const me = s.seat === mySeat, slot = slotOf(s.seat);
    if (s.status === "empty") return '<div class="seat p' + slot + ' empty" data-seat="' + s.seat + '"><span class="nm">empty seat</span></div>';
    const inHand = s.status === "active" || s.status === "all_in";
    const folded = s.status === "folded", out = s.status === "out";
    const turnNow = !S.over && S.toAct === s.seat;
    let cards = "";
    const key = S.handId + ":" + s.seat;
    if (s.cards && (s.shown || me)) cards = s.cards.map((c, i) => card(c, (s.hl && !s.hl.has(c) ? "dim" : "") + (s.hl && s.hl.has(c) ? " hl" : ""), key + ":" + i)).join("");
    else if (me && S.myCards && !folded) cards = S.myCards.map((c, i) => card(c, "", key + ":" + i)).join("");
    else if (!folded && !out && S.dealt && S.handNo) cards = back(key + ":0") + back(key + ":1");
    const dim = folded ? "" : "";
    let bdg = s.last ? esc(X(s.last)).replace(" AI", ' <span class="ai">all-in</span>') : "";
    if (!bdg && s.status === "all_in") bdg = '<span class="ai">all-in</span>';
    if (folded && !bdg) bdg = "fold";
    if (out) bdg = "sitting out";
    const cls = "seat p" + slot + (me ? " me" : "") + (folded ? " folded" : "") + (out ? " out" : "") + (turnNow ? " turn" : "") + (s.win ? " win" : "");
    const nm = me ? "YOU" : "S" + s.seat + " " + pretty(s.name);
    return '<div class="' + cls + '" data-seat="' + s.seat + '" data-status="' + s.status + '">' +
      (S.button === s.seat && !out ? '<span class="btn-d" title="Dealer button">D</span>' : "") +
      '<div class="cards">' + cards + "</div>" +
      '<span class="nm">' + esc(nm) + "</span>" +
      '<span class="stk">' + f(s.stack) + "</span>" +
      '<span class="bdg">' + (s.pos && !out ? '<span class="pos">' + s.pos + "</span>" : "") + bdg + "</span>" +
      '<span class="res"><span class="hn">' + esc(s.hn || "") + "</span>" +
      '<span class="net' + (s.net > 0 ? " plus" : "") + '">' + (S.over && s.net !== null && s.net !== undefined ? fs(s.net) : "") + "</span></span>" +
      (turnNow && s.seat === mySeat ? '<span class="bar" id="tbar"></span>' : "") + "</div>";
  }

  function renderTable() {
    $("seats").innerHTML = S.seats.map(seatHtml).join("");
    $("bets").innerHTML = S.seats.map(s => s.bet > 0 ? '<div class="bet b' + slotOf(s.seat) + (s.ai ? " ai" : "") + (S.sweep ? " sweep" : "") + '" data-bet="' + s.seat + '">' + f(s.bet) + "</div>" : "").join("");
    $("board").innerHTML = [0, 1, 2, 3, 4].map(i => {
      const c = S.board[i];
      if (!c) return '<span class="cd ph">&nbsp;&nbsp;</span>';
      const hl = S.hlBoard ? (S.hlBoard.has(c) ? " hl" : " dim") : "";
      return card(c, hl.trim(), S.handId + ":b" + i);
    }).join("");
    const bets = sumBets(), tot = sumContrib();
    const collected = tot - bets;
    let pl;
    // "Pot" is what is already in the middle (finished streets). Bets of the street in progress are shown in front of
    // their players and added on the side; they are not part of the pot until the street is over (and an uncalled bet never is).
    if (!S.handNo || (S.over && !S.result)) pl = '<span class="muted">-</span>';
    else if (S.over && S.result) pl = '<span class="muted">Hand over</span>';
    else pl = "Pot <b>" + f(collected) + "</b>" + (bets > 0 ? ' <span class="muted">+ ' + f(bets) + " bets = " + f(tot) + "</span>" : "");
    $("potline").innerHTML = pl;
    let pt = "";
    if (S.over && S.result) {
      pt = S.result.map(l => '<div data-pot="' + l.amount + ":" + l.u + ":" + l.winners.join(",") + ":" + l.share.join(",") + '"><span class="w">' + esc(X(l.text)) + "</span></div>").join("");
      if (S.uncalled && S.uncalled.amount > 0) pt = '<div data-unc="' + S.uncalled.seat + ":" + S.uncalled.amount + '">Uncalled bet ' + f(S.uncalled.amount) + U() + " returned to " + esc(S.seats[S.uncalled.seat].pos ? label(S.uncalled.seat) : "S" + S.uncalled.seat) + "</div>" + pt;
    } else if (!S.over) {
      const lv = potLevels().filter(l => l.eligible.length >= 2);
      if (lv.length >= 2) pt = lv.map((l, i) => '<div data-lvl="' + l.amount + '">' + (i === 0 ? "Main " : "Side " + (lv.length > 2 ? i + " " : "")) + f(l.amount) + U() + " (" + l.eligible.map(e => (e === mySeat ? "you" : "S" + e)).join(", ") + ")</div>").join("");
    }
    $("pots").innerHTML = pt;
    $("tblmsg").textContent = "";
  }

  function renderResult() {
    const n = tally.hands, net = tally.net;
    const per100 = n ? Math.round((net / bb / n) * 100 * 10) / 10 : 0;
    $("result").innerHTML = (net > 0 ? '<span class="win">' : "<span>") + esc(fs(net)) + (unit === "chips" ? "" : " bb") + "</span>, " + n + " hand" + (n === 1 ? "" : "s") +
      (n >= 50 ? ' <span class="muted">(' + (per100 > 0 ? "+" : "") + per100 + " bb/100)</span>" : "");
    const lh = $("lasthand"), last = logRows[0];
    lh.textContent = last ? "#" + last.no + " " + X(last.msg) : "-";
    lh.className = last && S.over ? "" : "muted";
  }

  let logTimer = null;
  function scheduleLog() { if (logTimer) return; logTimer = setTimeout(() => { logTimer = null; renderLog(); }, FAST ? 250 : 60); }
  function lineHtml(l) {
    const attr = l.ev ? ' data-ev="' + l.ev + '"' : l.st ? ' data-st="' + l.st + '"' : l.sd ? ' data-sd="' + l.sd + '"' : l.pot ? ' data-lpot="' + l.pot + '"' : l.unc ? ' data-lunc="' + l.unc + '"' : "";
    return '<div class="ln ' + (l.cls || "") + '"' + attr + '><span class="s">' + esc(l.s || "") + '</span><span class="t">' + esc(X(l.t)) + "</span></div>";
  }
  function rowHtml(r, open) {
    return '<details data-hand="' + esc(r.id) + '"' + (open ? " open" : "") + "><summary><span>#" + r.no + " " + (r.cards ? r.cards.map(cardTxt).join(" ") : "") + ' <span class="muted">' + r.board.map(cardTxt).join(" ") + "</span></span>" +
      '<span class="' + (r.net > 0 ? "win" : "") + '">' + fs(r.net) + U() + "</span></summary>" + r.lines.map(lineHtml).join("") + "</details>";
  }
  function renderLog(force) {
    const el = $("log");
    let h = "";
    if (cur && cur.lines.length) h += '<details open data-hand="' + esc(cur.id || "") + '" data-live="1"><summary><span>#' + (tally.hands + 1) + " " + (cur.cards ? cur.cards.map(cardTxt).join(" ") : "") + ' <span class="muted">in progress</span></span></summary>' + cur.lines.map(lineHtml).join("") + "</details>";
    const seen = new Set();
    h += logRows.filter(r => !seen.has(r.id) && seen.add(r.id)).map((r, i) => rowHtml(r, i === 0 && !cur)).join("");
    el.innerHTML = h || '<div class="muted">No hands yet.</div>';
  }

  // ---------- controls ----------
  let tickTimer = null;
  function setupControls() {
    renderControls();
    clearInterval(tickTimer);
    tickTimer = setInterval(tick, 200);
    tick();
  }
  // The server's deadline_ms is absolute and already includes the time bank; time_bank_ms is what is left of the bank,
  // so the regular clock ends at deadline_ms - time_bank_ms. A replayed your_turn (reload) keeps the original deadline.
  function remainingMs() { return turn ? turn.deadline_ms - (Date.now() + clockSkew) : 0; }
  function clockText() {
    const rem = Math.max(0, remainingMs()), bank = turn.time_bank_ms || 0;
    const reg = Math.max(0, rem - bank);
    if (!bank) return { t: Math.ceil(rem / 1000) + "s", sec: Math.ceil(rem / 1000) };
    if (reg > 0) return { t: Math.ceil(reg / 1000) + "s + " + Math.round(bank / 1000) + "s bank", sec: Math.ceil(rem / 1000) };
    return { t: "bank " + Math.ceil(rem / 1000) + "s", sec: Math.ceil(rem / 1000) };
  }
  function tick() {
    const el = $("info");
    if (!turn) { clearInterval(tickTimer); return; }
    const rem = Math.max(0, remainingMs()), total = Math.max(1, decisionMs + (turn.time_bank_ms || 0));
    const bar = $("tbar"); if (bar) bar.style.transform = "scaleX(" + Math.min(1, rem / total) + ")";
    const t = el.querySelector("[data-timer]");
    if (t) { const c = clockText(); t.textContent = c.t; t.style.color = c.sec <= 10 ? "var(--red)" : ""; }
  }

  function curBet() { return turn ? Math.max.apply(null, turn.players.map(p => p.street_contrib)) : 0; }
  function clampTo(to) { const R = turn.legal.raise; return Math.min(R.max_to, Math.max(R.min_to, Math.round(to))); }
  function fracTo(frac) {
    const A = turn.legal.call ? turn.legal.call.added : 0;
    return clampTo(curBet() + frac * (turn.pot + A));
  }
  function whyNoRaise() {
    const L = turn.legal, me = turn.players.find(p => p.seat === mySeat);
    if (L.call && me && L.call.added >= me.stack) return "Calling puts you all-in; there is nothing left to raise.";
    const others = turn.players.filter(p => p.seat !== mySeat && p.status === "active");
    if (!others.length) return "Everyone else is all-in or folded: you can only call or fold.";
    return "Betting is not reopened: the last raise was a short all-in. You can call or fold.";
  }

  function renderControls() {
    const bF = $("bFold"), bC = $("bCall"), bR = $("bRaise"), siz = $("sizing"), info = $("info");
    if (!turn) {
      [bF, bC, bR].forEach(b => { b.disabled = true; });
      bF.innerHTML = 'Fold <span class="k">f</span>'; bC.innerHTML = 'Check <span class="k">c</span>'; bR.innerHTML = 'Raise <span class="k">r</span>';
      siz.hidden = true;
      if (!S || !S.handNo) info.textContent = "";
      else if (S.over) info.innerHTML = '<span>' + esc(X(S.msg || "")) + "</span>";
      else info.innerHTML = '<span>Waiting for ' + (S.toAct != null && S.toAct !== mySeat ? "S" + S.toAct : "the table") + "...</span>";
      return;
    }
    const L = turn.legal, me = turn.players.find(p => p.seat === mySeat), cb = curBet();
    bF.disabled = !L.fold;
    bF.title = L.fold ? "" : "Checking is free, there is nothing to fold";
    bC.disabled = !(L.check || L.call);
    bC.innerHTML = (L.check ? "Check" : "Call " + f(L.call.added) + (L.call.all_in ? " all-in" : "")) + ' <span class="k">c</span>';
    const R = L.raise;
    bR.disabled = !R;
    siz.hidden = !R;
    const opps = turn.players.filter(p => p.seat !== mySeat && p.status !== "folded" && p.status !== "out");
    const eff = Math.min(me.stack + me.street_contrib, Math.max.apply(null, opps.map(p => p.stack + p.street_contrib).concat([0])));
    const inFront = turn.players.reduce((t, p) => t + p.street_contrib, 0);
    let bits = '<span>To call <b>' + (L.call ? f(L.call.added) : "0") + '</b></span><span title="Pot = finished streets. Bets of this street are in front of their players (total ' + f(turn.pot) + ')">Pot <b>' + f(turn.pot - inFront) + "</b>" + (inFront > 0 ? " + " + f(inFront) + " bets" : "") + "</span><span>Stack <b>" + f(me.stack) + "</b> (eff " + f(eff) + ")</span>";
    if (R) bits += "<span>" + (cb === 0 ? "Bet" : "Raise to") + " <b>" + f(R.min_to) + "</b> to <b>" + f(R.max_to) + "</b></span>";
    bits += '<span class="clk" title="Server decision clock. The server folds (or checks) for you when it runs out."><b data-timer>' + clockText().t + "</b></span>";
    if (!R) bits += "<span class='red'>" + esc(whyNoRaise()) + "</span>";
    info.innerHTML = bits;
    if (R) {
      rt = Math.min(R.max_to, Math.max(R.min_to, rt || R.min_to));
      const sl = $("slider"); sl.min = R.min_to; sl.max = R.max_to; sl.step = 1; sl.value = rt; sl.disabled = R.min_to === R.max_to;
      sl.setAttribute("aria-valuetext", f(rt) + (unit === "chips" ? " chips" : " big blinds") + (rt === R.max_to ? ", all-in" : ""));
      const am = $("amt"); if (document.activeElement !== am) am.value = num(rt);
      const verb = cb === 0 ? "Bet" : "Raise to";
      bR.innerHTML = (rt === R.max_to ? "All-in " : verb + " ") + f(rt) + ' <span class="k">r</span>';
      renderPresets(R, cb);
    } else bR.innerHTML = 'Raise <span class="k">r</span>';
  }

  function renderPresets(R, cb) {
    const pre = turn.street === "preflop";
    const list = [{ l: "min", to: R.min_to }];
    if (pre) [["2.5x", 2.5], ["3x", 3], ["4x", 4]].forEach(([l, x]) => list.push({ l, to: Math.round(x * cb), raw: true }));
    [["½ pot", .5], ["¾ pot", .75], ["pot", 1]].forEach(([l, fr]) => { if (!pre || fr === 1 || true) list.push({ l, to: fracTo(fr), pot: true }); });
    const seen = new Set(); const out = [];
    list.forEach((p, i) => {
      if (p.raw && (p.to < R.min_to || p.to >= R.max_to)) return;
      if (p.pot && (p.to <= R.min_to || p.to >= R.max_to) && i > 0) return;
      if (pre && p.pot && p.l !== "pot") return;
      if (seen.has(p.to)) return; seen.add(p.to); out.push(p);
    });
    if (!seen.has(R.max_to)) out.push({ l: "all-in", to: R.max_to }); else { const o = out.find(x => x.to === R.max_to); if (o) o.l = "all-in"; }
    $("presets").innerHTML = out.map(p => '<button data-to="' + p.to + '"' + (p.to === rt ? ' class="on"' : "") + ">" + p.l + (p.l === "all-in" || p.l === "min" ? "" : " " + f(p.to)) + "</button>").join("");
    $("presets").querySelectorAll("button").forEach(b => { b.onclick = () => { setRt(+b.dataset.to); }; });
  }

  function setRt(to) {
    if (!turn || !turn.legal.raise) return;
    rt = clampTo(to);
    armed = true;
    renderControls();
    $("amt").value = num(rt);
  }
  function parseAmt(txt) {
    txt = String(txt).replace(",", ".").replace(/[^0-9.]/g, "");
    const v = parseFloat(txt); if (!isFinite(v)) return null;
    return unit === "chips" ? Math.round(v) : Math.round(v * bb);
  }

  function act(type, to) {
    if (!turn) return;
    const L = turn.legal;
    if (answered.has(turn.decision_id)) return;
    // Never send an illegal action: re-check against the legal set.
    if (type === "fold" && !L.fold) return;
    if (type === "check" && !L.check) return;
    if (type === "call" && !L.call) return;
    const d = { hand_id: turn.hand_id, decision_id: turn.decision_id, type };
    if (type === "raise" || type === "bet") {
      if (!L.raise) return;
      d.to = clampTo(to);
      d.type = curBet() === 0 ? "bet" : "raise";
    }
    if (!send("act", d)) { banner("Not connected: your action was not sent. Reconnecting...", true); return; }
    answered.add(turn.decision_id); sent.push(d);
    turn = null; armed = false; clearInterval(tickTimer);
    banner("");
    renderAll();
  }
  function confirmRaise() {
    if (!turn || !turn.legal.raise) return;
    const v = parseAmt($("amt").value);
    if (document.activeElement === $("amt") && v !== null) rt = clampTo(v);
    act("raise", rt);
  }

  $("bFold").onclick = () => act("fold");
  $("bCall").onclick = () => { if (turn) act(turn.legal.check ? "check" : "call"); };
  $("bRaise").onclick = () => confirmRaise();
  $("slider").oninput = e => {
    const R = turn && turn.legal.raise; if (!R) return;
    let v = +e.target.value;
    if (v >= R.max_to - sb / 2) v = R.max_to; else v = Math.max(R.min_to, Math.round(v / sb) * sb);
    rt = Math.min(R.max_to, v); armed = true; renderControls();
  };
  $("amt").oninput = e => { const v = parseAmt(e.target.value); const R = turn && turn.legal.raise; if (v !== null && R) { rt = clampTo(v); $("slider").value = rt; armed = true; } };
  $("amt").onblur = () => { if (turn && turn.legal.raise) { const v = parseAmt($("amt").value); if (v !== null) { const c = clampTo(v); if (c !== v) banner("Raise to must be between " + f(turn.legal.raise.min_to) + " and " + f(turn.legal.raise.max_to) + ": set to " + f(c) + ".", false); rt = c; } renderControls(); } };
  $("amt").onfocus = () => { $("amt").select(); };
  $("bNext").onclick = () => nextHand();
  $("bReset").onclick = () => { tally = { hands: 0, net: 0 }; saveTally(); logRows = []; saveHist(); renderResult(); renderLog(true); };
  $("bNew").onclick = () => start();
  $("bLeave").onclick = () => {
    left = true; teardown(); S.toAct = null; turn = null; renderAll();
    $("nextrow").hidden = true; document.body.dataset.phase = "left";
    banner("You left the table. Your result stays in the session.", false, "Sit down", () => start());
  };
  $("oUnit").onclick = () => { unit = unit === "bb" ? "chips" : "bb"; ls.set("sharingan.unit", unit); renderAll(); renderLog(true); };
  $("oAuto").onclick = () => { auto = !auto; ls.set("sharingan.auto", auto ? "1" : "0"); renderOpts(); if (S.over && S.phase === "settled" && !$("nextrow").hidden) promptNext(false); };
  $("oStack").onclick = () => { stackMode = stackMode === "reset" ? "carry" : "reset"; ls.set("sharingan.stackmode", stackMode); start(); };

  document.addEventListener("keydown", e => {
    if (e.ctrlKey || e.metaKey || e.altKey) return;
    const inAmt = e.target === $("amt");
    if (inAmt) {
      if (e.key === "Enter") { e.preventDefault(); confirmRaise(); }
      else if (e.key === "Escape") { e.preventDefault(); $("amt").blur(); armed = false; renderControls(); }
      else if (e.key === "ArrowUp" || e.key === "ArrowDown") { e.preventDefault(); stepRt(e.key === "ArrowUp" ? 1 : -1, e.shiftKey); }
      return;
    }
    if (e.target && /^(INPUT|TEXTAREA|SELECT)$/.test(e.target.tagName) && e.target.type !== "range") return;
    const k = e.key.toLowerCase();
    if (S && S.over && S.phase === "settled" && (k === " " || k === "enter") && !$("nextrow").hidden) { e.preventDefault(); if (!skipSleepers()) nextHand(); return; }
    if (S && !S.over && (k === " " || k === "enter") && !turn) { skipSleepers(); return; }
    if (!turn) return;
    const L = turn.legal;
    if (k === "f") {
      e.preventDefault();
      if (L.fold) act("fold");
      else if (Date.now() - foldHint < 2500) { foldHint = 0; act("check"); }
      else { foldHint = Date.now(); banner("Checking is free, so there is nothing to fold. Press f again to check.", false); }
    } else if (k === "c") { e.preventDefault(); act(L.check ? "check" : "call"); }
    else if (k === "r" && L.raise) { e.preventDefault(); armed = true; $("amt").focus(); }
    else if (k === "a" && L.raise) {
      // a: select all-in; a second press (or Enter) sends it.
      e.preventDefault();
      if (allinHint && Date.now() - allinHint < 5000 && rt === L.raise.max_to) { allinHint = 0; confirmRaise(); }
      else if (!e.repeat) { setRt(L.raise.max_to); allinHint = Date.now(); banner("All-in selected: press a again or Enter to send it, Esc to cancel.", false); }
    }
    else if (k === "enter" && armed && L.raise) { e.preventDefault(); confirmRaise(); }
    else if (k === "escape") { armed = false; renderControls(); }
    else if ((k === "arrowup" || k === "arrowdown" || k === "arrowright" || k === "arrowleft") && L.raise && e.target.type !== "range") { e.preventDefault(); stepRt(k === "arrowup" || k === "arrowright" ? 1 : -1, e.shiftKey); }
  });
  function stepRt(dir, big) { if (!turn || !turn.legal.raise) return; setRt(rt + dir * (big ? 5 : 1) * bb); }
  function skipSleepers() { if (sleepers.length) { skipping = true; wakeSleepers(); return true; } return false; }

  // Leaving the page (reload, close, navigate) only drops the socket. The table stays on the server (it holds between
  // hands, the seat plays on the timeout policy mid-hand) and the next load of this page resumes it. Only a tab that
  // could not save its table for resuming (another tab owns the saved one) closes its table here.
  window.addEventListener("pagehide", () => {
    gen++; clearTimeout(nextTimer);
    if (ws) try { ws.onclose = null; ws.close(); } catch (e) {}
    if (ephemeral && tableId) closeTable(tableId);
  });
  window.addEventListener("pageshow", e => { if (e.persisted) location.reload(); });   // back/forward cache: the old page state is dead
  window.__sharingan = {
    get state() { return { sent, S, turn, tally, tableId, anoms, errors, logRows, cur, unit, bb, reconnects, token: me && me.token }; },
    api: (m, p, b) => api(m, p, b, me && me.token),
    drop() { if (ws) ws.close(); },                // test hook: simulate a dropped socket
  };

  renderLog(true);
  start(true);
})();
