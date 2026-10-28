// Poker Sharingan: play a 6-max no-limit hold'em table in the browser.
// REST: register (kind human) -> create table -> sit -> WebSocket /v1/play -> fill with house bots.
// Amounts on the wire are chips; everything shown is in big blinds.
(function () {
  "use strict";
  const $ = id => document.getElementById(id);
  const params = new URLSearchParams(location.search);
  const FAST = params.has("fast");                       // testing: no pacing between bot actions
  const HAND_DELAY = parseInt(params.get("delay") || (FAST ? "150" : "2500"), 10);
  const BB_DEFAULT = 100;

  const ROOMS = {
    sharingan: { label: "vs SHARINGAN-1", desc: "You and five SHARINGAN-1. The hardest table we have.",
                 seats: ["house:sharingan-1", "house:sharingan-1", "house:sharingan-1", "house:sharingan-1", "house:sharingan-1"] },
    practice:  { label: "practice", desc: "You and five weaker house bots: random, calling station, tight-aggressive, nit, LAG.",
                 seats: ["house:random", "house:calling_station", "house:tight_aggressive", "house:nit", "house:lag"] },
    mixed:     { label: "mixed", desc: "You, two SHARINGAN-1 and three house bots.",
                 seats: ["house:sharingan-1", "house:tight_aggressive", "house:lag", "house:sharingan-1", "house:calling_station"] },
  };

  // ---------- state ----------
  let room = "sharingan";
  try { const r = localStorage.getItem("sharingan.room"); if (r && ROOMS[r]) room = r; } catch (e) {}
  if (params.get("room") && ROOMS[params.get("room")]) room = params.get("room");

  let gen = 0;                 // bumps on every (re)start so stale sockets/timers die
  let ws = null, tableId = null, me = null, mySeat = 0;
  let bb = BB_DEFAULT;
  let S = null;                // per-table view model
  let turn = null;             // pending your_turn data
  let queue = [], pumping = false;
  let tally = loadTally();
  let logRows = [];            // newest first
  let cur = null;              // hand being recorded

  function loadTally() {
    try { const t = JSON.parse(localStorage.getItem("sharingan.tally." + room) || "null"); if (t && typeof t.hands === "number") return t; } catch (e) {}
    return { hands: 0, net: 0 };
  }
  function saveTally() { try { localStorage.setItem("sharingan.tally." + room, JSON.stringify(tally)); } catch (e) {} }

  function freshView() {
    return { seats: Array.from({ length: 6 }, (_, i) => ({ seat: i, name: null, house: false, stack: 0, bet: 0, status: "empty", last: "", cards: null, net: null })),
             board: [], pot: 0, button: null, sb: null, bb: null, street: "preflop", toAct: null, handNo: 0, over: true, myCards: null, ended: null };
  }

  // ---------- formatting ----------
  function f(chips) { const v = Math.round((chips / bb) * 10) / 10; return String(v); }
  function fs(chips) { const v = Math.round((chips / bb) * 10) / 10; return (v > 0 ? "+" : "") + v; }
  const SUIT = { h: "♥", d: "♦", s: "♠", c: "♣" };
  function card(c, cls) {
    const r = c[0] === "T" ? "10" : c[0], s = c[1];
    return '<span class="cd' + (s === "h" || s === "d" ? " r" : "") + (cls ? " " + cls : "") + '">' + r + SUIT[s] + "</span>";
  }
  function cardTxt(c) { return (c[0] === "T" ? "10" : c[0]) + SUIT[c[1]]; }
  function cards(list) { return (list || []).map(c => card(c)).join(""); }
  function clean(n) { return String(n || "").replace(/^house:/, ""); }
  const NAMES = { "sharingan-1": "SHARINGAN-1", tight_aggressive: "TAG", lag: "LAG", calling_station: "CALLING STATION", nit: "NIT", random: "RANDOM" };
  function pretty(n) { const c = clean(n).toLowerCase(); return NAMES[c] || c.replace(/_/g, " ").toUpperCase(); }
  function nameOf(seat) { return seat === mySeat ? "you" : "S" + seat; }       // short name for the log
  const POS = ["BTN", "SB", "BB", "UTG", "HJ", "CO"];
  function posOf(seat) { return S.button == null ? "" : POS[(seat - S.button + 6) % 6]; }
  function say(msg, red) { const el = $("status"); el.textContent = msg; el.className = "p" + (red ? " red" : ""); }

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
    try { localStorage.setItem(key, JSON.stringify(rec)); } catch (e) {}
    return rec;
  }

  // ---------- start / stop ----------
  function teardown() {
    gen++;
    queue = []; pumping = false; turn = null;
    if (ws) { try { ws.onclose = null; ws.send(JSON.stringify({ v: 1, type: "leave", t_ms: Date.now(), table_id: tableId, data: { table_id: tableId } })); ws.close(); } catch (e) {} ws = null; }
    if (tableId && me) { fetch(ARENA_API + "/v1/tables/" + tableId + "/seats/" + mySeat, { method: "DELETE", headers: { authorization: "Bearer " + me.token }, keepalive: true }).catch(() => {}); }
    tableId = null;
  }

  async function start() {
    teardown();
    const g = gen;
    tally = loadTally(); logRows = []; cur = null;
    S = freshView(); bb = BB_DEFAULT;
    renderAll();
    say("CONNECTING...");
    try {
      me = await ensurePlayer(false);
      const cfg = { variant: "nlhe", max_seats: 6, small_blind: 50, big_blind: 100, starting_stack: 10000, stack_mode: "reset",
                    decision_time_ms: 30000, time_bank_ms: 30000, hand_delay_ms: HAND_DELAY, name: "play-" + room };
      let t;
      try { t = await api("POST", "/v1/tables", cfg, me.token); }
      catch (e) { if (e.status === 401) { me = await ensurePlayer(true); t = await api("POST", "/v1/tables", cfg, me.token); } else throw e; }
      if (g !== gen) return;
      tableId = t.table_id; bb = (t.config && t.config.big_blind) || BB_DEFAULT;
      await api("POST", "/v1/tables/" + tableId + "/seats/" + mySeat, {}, me.token);
      await connect(g, false);
      // Bots sit last: the table deals as soon as it is full and we are connected.
      const bots = ROOMS[room].seats;
      for (let i = 0; i < bots.length; i++) {
        if (g !== gen) return;
        await api("POST", "/v1/tables/" + tableId + "/seats/" + (i + 1), { house: bots[i] }, me.token);
      }
      say("WAITING FOR THE DEAL...");
    } catch (e) {
      if (g !== gen) return;
      let m = e.message;
      if (e.status === 400 && /sharingan-1|house/i.test(m) && room !== "practice") m += " Try the practice room.";
      say(m, true);
      renderControls();
    }
  }

  function connect(g, resume) {
    return new Promise((resolve, reject) => {
      const url = ARENA_API.replace(/^http/, "ws") + "/v1/play?token=" + encodeURIComponent(me.token);
      const sock = new WebSocket(url);
      let opened = false;
      sock.onopen = () => { opened = true; ws = sock; send("hello", { client: "sharingan-web/1" }); resolve(); };
      sock.onerror = () => { if (!opened) reject(new Error("Cannot open the game socket at " + url.replace(/\?.*/, "") + ".")); };
      sock.onmessage = ev => {
        if (g !== gen) return;
        let m; try { m = JSON.parse(ev.data); } catch (e) { return; }
        queue.push(m); pump(g);
      };
      sock.onclose = () => {
        if (g !== gen || !opened) return;
        say("DISCONNECTED. RECONNECTING...", true);
        turn = null; renderControls();
        setTimeout(() => { if (g === gen && tableId) connect(g, true).catch(() => say("Connection lost. Press NEW TABLE.", true)); }, 1500);
      };
    });
  }

  function send(type, data) {
    if (!ws || ws.readyState !== 1) return;
    ws.send(JSON.stringify({ v: 1, type, table_id: tableId, t_ms: Date.now(), data }));
  }
  setInterval(() => send("ping", {}), 15000);

  // ---------- event pump (paces bot actions so a human can follow) ----------
  async function pump(g) {
    if (pumping) return;
    pumping = true;
    while (queue.length && g === gen) {
      const m = queue.shift();
      try { handle(m); } catch (e) { console.warn("event error", e, m); }
      renderAll();
      if (!FAST && queue.length) {
        const t = m.type;
        await new Promise(r => setTimeout(r, t === "action" ? 520 : t === "street" ? 420 : t === "hand_start" ? 350 : 0));
      }
    }
    pumping = false;
  }

  // ---------- protocol handlers ----------
  function handle(m) {
    const d = m.data || {};
    switch (m.type) {
      case "welcome": break;
      case "table_snapshot": snapshot(d); break;
      case "seat_update": { const s = S.seats[d.seat]; if (s) { s.name = d.name; s.house = !!d.house; s.stack = d.stack; s.status = d.status === "empty" ? "empty" : (s.status === "empty" ? "seated" : s.status); } break; }
      case "hand_start": handStart(d); break;
      case "hole_cards": if (d.seat === mySeat) { S.myCards = d.cards; if (cur) cur.cards = d.cards; } break;
      case "action": action(d); break;
      case "street": S.board = d.board || []; S.pot = d.pot; S.street = d.street; S.seats.forEach(s => { s.bet = 0; if (s.status === "active") s.last = ""; });
        if (cur) cur.streets.push({ n: d.street, cards: (d.new_cards || []).map(cardTxt).join(" "), acts: [] }); break;
      case "your_turn": yourTurn(d); break;
      case "hand_end": handEnd(d); break;
      case "error": {
        const code = d.code || "error";
        if (code === "illegal_action" || code === "bad_amount") say("ILLEGAL ACTION: " + (d.message || code), true);
        else if (code !== "not_your_turn") say((d.message || code).toUpperCase(), true);
        break;
      }
      default: break;
    }
  }

  function snapshot(d) {
    if (d.config && d.config.big_blind) bb = d.config.big_blind;
    (d.seats || []).forEach(x => { const s = S.seats[x.seat]; s.name = x.name; s.house = !!x.house; s.stack = x.stack; s.status = x.player_id ? "seated" : "empty"; });
    S.button = d.button_seat;
    const h = d.hand;
    if (h && h.hand_id && h.street !== "showdown") {
      S.handNo = h.hand_no; S.over = false; S.board = h.board || []; S.pot = h.pot; S.street = h.street; S.toAct = h.to_act;
      S.button = h.button_seat; S.sb = h.sb_seat; S.bb = h.bb_seat;
      (h.players || []).forEach(p => { const s = S.seats[p.seat]; s.stack = p.stack; s.bet = p.street_contrib; s.status = p.status; });
      const mine = d.hole_cards && d.hole_cards[String(mySeat)];
      if (mine) S.myCards = mine;
      if (!cur) cur = { no: h.hand_no, cards: mine || null, streets: [{ n: h.street, cards: "", acts: [] }] };
    }
  }

  function handStart(d) {
    bb = d.big_blind || bb;
    S.over = false; S.ended = null; S.board = []; S.pot = 0; S.street = "preflop"; S.myCards = null; S.toAct = null;
    S.handNo = d.hand_no; S.button = d.button_seat; S.sb = d.sb_seat; S.bb = d.bb_seat;
    S.seats.forEach(s => { s.bet = 0; s.last = ""; s.cards = null; s.net = null; s.status = "empty"; });
    (d.players || []).forEach(p => { const s = S.seats[p.seat]; s.name = p.name; s.stack = p.stack_start; s.status = p.in_hand ? "active" : "out"; });
    cur = { no: d.hand_no, cards: null, streets: [{ n: "preflop", cards: "", acts: [] }] };
    $("lasthand").textContent = "";
    say("HAND " + d.hand_no);
  }

  function action(d) {
    const s = S.seats[d.seat];
    s.stack = d.stack; S.pot = d.pot;
    if (d.type !== "fold" && d.type !== "check") s.bet = d.to;
    let t = d.type;
    if (d.forced) t = d.type === "post_sb" ? "SB " + f(d.added) : d.type === "post_bb" ? "BB " + f(d.added) : "ANTE " + f(d.added);
    else if (d.type === "call") t = "call " + f(d.added);
    else if (d.type === "bet") t = "bet " + f(d.to);
    else if (d.type === "raise") t = "raise " + f(d.to);
    if (d.type === "fold") s.status = "folded";
    if (d.all_in && d.type !== "fold") { s.status = "all_in"; t += " all-in"; }
    if (d.timeout) t += " (time)";
    s.last = t;
    S.toAct = d.next_to_act;
    if (cur && !d.forced) cur.streets[cur.streets.length - 1].acts.push({ s: d.seat, t: t.replace(" all-in", " AI").replace(" (time)", "") });
  }

  function yourTurn(d) {
    turn = d;
    S.toAct = d.seat; S.board = d.board || S.board; S.pot = d.pot;
    S.myCards = d.hole_cards;
    (d.players || []).forEach(p => { const s = S.seats[p.seat]; s.stack = p.stack; s.bet = p.street_contrib; });
    setupControls();
    say("YOUR TURN");
  }

  function handEnd(d) {
    turn = null; S.over = true; S.toAct = null;
    S.board = d.board || S.board;
    const shown = {};
    (d.showdown || []).forEach(x => { if (x.shown) { shown[x.seat] = x; S.seats[x.seat].cards = x.cards; } });
    const rankBySeat = {};
    (d.showdown || []).forEach(x => { rankBySeat[x.seat] = x.hand_rank; });
    let mine = 0;
    (d.results || []).forEach(r => {
      const s = S.seats[r.seat]; s.net = r.net; s.stack = r.stack_end; s.bet = 0;
      if (r.seat === mySeat) mine = r.net;
    });
    tally.hands += 1; tally.net += mine; saveTally();
    const winners = new Set(); (d.pots || []).forEach(p => (p.winners || []).forEach(w => winners.add(w)));
    const wl = [...winners].map(w => nameOf(w)).join(", ");
    const how = d.ended_by === "showdown" ? " at showdown" + (rankBySeat[[...winners][0]] ? " with " + String(rankBySeat[[...winners][0]]).replace(/_/g, " ") : "") : "";
    const msg = mine > 0 ? "YOU WIN " + f(mine) + " bb" + how : mine < 0 ? "YOU LOSE " + f(-mine) + " bb" + (winners.has(mySeat) ? "" : "; " + wl + " wins" + how) : "YOU BREAK EVEN; " + wl + " wins" + how;
    $("lasthand").textContent = "";
    say(msg.toUpperCase());
    S.ended = msg;
    const row = { no: S.handNo, cards: S.myCards || (cur && cur.cards), board: S.board.slice(), net: mine, streets: cur ? cur.streets : [],
                  shown: Object.keys(shown).filter(k => +k !== mySeat).map(k => nameOf(+k) + " " + shown[k].cards.map(cardTxt).join(" ")).join(" · ") };
    logRows.unshift(row); if (logRows.length > 40) logRows.pop();
    cur = null;
    renderLog();
  }

  // ---------- render ----------
  function renderAll() { renderRoom(); renderTable(); renderControls(); renderResult(); }

  function renderRoom() {
    const el = $("rooms");
    if (!el.firstChild) {
      Object.keys(ROOMS).forEach(k => {
        const b = document.createElement("button"); b.dataset.room = k; b.textContent = ROOMS[k].label;
        b.onclick = () => { if (room === k) return; room = k; try { localStorage.setItem("sharingan.room", k); } catch (e) {} start(); };
        el.appendChild(b);
      });
    }
    el.querySelectorAll("button").forEach(b => { b.className = b.dataset.room === room ? "on" : ""; b.setAttribute("aria-pressed", b.dataset.room === room); });
    $("roomdesc").textContent = ROOMS[room].desc;
  }

  function renderTable() {
    $("board").innerHTML = [0, 1, 2, 3, 4].map(i => S.board[i] ? card(S.board[i]) : '<span class="cd ph">&nbsp;&nbsp;</span>').join("");
    $("pot").textContent = S.handNo ? f(S.pot) + " bb" : "-";
    $("mine").innerHTML = S.myCards ? cards(S.myCards) : '<span class="muted">-</span>';
    let h = '<div class="l"><div class="seat hd"><span></span><span>SEAT</span><span></span><span>STACK</span><span>BET</span></div></div>';
    S.seats.forEach(s => {
      if (s.status === "empty" && !s.name) { h += '<div class="l"><div class="seat folded"><span></span><span class="nm">empty</span><span></span><span></span><span></span></div></div>'; return; }
      const me = s.seat === mySeat;
      const folded = s.status === "folded" || s.status === "out";
      const turnNow = !S.over && S.toAct === s.seat;
      const pos = s.status === "out" ? "" : posOf(s.seat);
      let right = s.bet > 0 ? f(s.bet) + (s.status === "all_in" ? " AI" : "") : (s.status === "all_in" ? "AI" : "");
      if (S.over && s.net !== null) right = fs(s.net);
      else if (s.status === "folded") right = "fold";
      else if (s.status === "out") right = "out";
      const nm = me ? "YOU" : "S" + s.seat + " " + pretty(s.name);
      h += '<div class="l"><div class="seat' + (me ? " me" : "") + (folded ? " folded" : "") + (turnNow ? " turn" : "") + '">' +
        '<span>' + (turnNow ? "&gt;" : "") + '</span>' +
        '<span class="nm">' + esc(nm) + "</span>" +
        '<span class="pos">' + pos + "</span>" +
        "<span>" + f(s.stack) + "</span>" +
        '<span class="' + (S.over && s.net > 0 ? "win" : "") + '">' + esc(right) + "</span>" +
        (s.cards && !me && !folded ? '<span class="sub">' + cards(s.cards) + "</span>" : "") + "</div></div>";
    });
    $("seats").innerHTML = h;
  }

  function renderResult() {
    const n = tally.hands;
    $("result").innerHTML = (tally.net > 0 ? '<span class="win">' : "<span>") + esc(fs(tally.net)) + " bb</span> in " + n + " hand" + (n === 1 ? "" : "s") +
      (n ? ' <span class="muted">(' + (Math.round((tally.net / bb / n) * 100) / 100 > 0 ? "+" : "") + Math.round((tally.net / bb / n) * 100) / 100 + " bb/hand)</span>" : "");
  }

  const STREET = { preflop: "PRE", flop: "FLOP", turn: "TURN", river: "RIVER" };
  function streetLine(st) {
    const out = []; let run = [];
    const flush = () => { if (run.length) out.push(run.length > 1 ? run.length + " fold" : nameOf(run[0]) + " fold"); run = []; };
    st.acts.forEach(a => {
      if (a.t === "fold" && a.s !== mySeat) { run.push(a.s); return; }
      flush(); out.push(nameOf(a.s) + " " + a.t);
    });
    flush();
    return out;
  }
  function renderLog() {
    $("log").innerHTML = logRows.length ? logRows.map(r => {
      let h = '<div class="l"><div class="lh"><span>#' + r.no + " " + (r.cards ? r.cards.map(cardTxt).join(" ") : "") + "</span>" +
        '<span class="' + (r.net > 0 ? "win" : "") + '">' + fs(r.net) + " bb</span></div></div>";
      r.streets.forEach(st => {
        const parts = streetLine(st);
        if (!parts.length && !st.cards) return;
        h += '<div class="l"><div class="st"><span class="muted">' + STREET[st.n] + '</span><span>' + (st.cards ? '<span class="muted">' + esc(st.cards) + "</span> " : "") + esc(parts.join(" · ")) + "</span></div></div>";
      });
      if (r.shown) h += '<div class="l"><div class="st"><span class="muted">SHOW</span><span>' + esc(r.shown) + "</span></div></div>";
      return h;
    }).join("") : '<div class="l muted">No hands yet.</div>';
  }

  // ---------- controls ----------
  let tickTimer = null;
  function setupControls() {
    const L = turn.legal, sl = $("slider");
    if (L.raise) { sl.min = 0; sl.max = 1000; sl.value = 0; }
    renderControls();
    clearInterval(tickTimer);
    tickTimer = setInterval(() => { const el = $("timer"); if (!turn) { el.textContent = ""; clearInterval(tickTimer); return; } const s = Math.max(0, Math.round((turn.deadline_ms - Date.now()) / 1000)); el.textContent = s + "s"; el.className = "tm" + (s <= 5 ? " red" : ""); }, 250);
  }

  function raiseTo() {
    const L = turn && turn.legal && turn.legal.raise; if (!L) return null;
    const v = +$("slider").value / 1000;
    let to = Math.round((L.min_to + (L.max_to - L.min_to) * v) / 50) * 50;
    if (to >= L.max_to - 25 || v >= 0.999) to = L.max_to;
    return Math.min(L.max_to, Math.max(L.min_to, to));
  }
  function setRaiseTo(to) {
    const L = turn.legal.raise; if (!L) return;
    to = Math.min(L.max_to, Math.max(L.min_to, Math.round(to)));
    $("slider").value = L.max_to === L.min_to ? 1000 : Math.round(((to - L.min_to) / (L.max_to - L.min_to)) * 1000);
    renderControls();
  }
  function potSize(frac) {
    const my = S.seats[mySeat].bet, add = turn.legal.call ? turn.legal.call.added : 0;
    return my + add + frac * (turn.pot + add);
  }

  const K = k => ' <span class="k">' + k + "</span>";
  function renderControls() {
    const bF = $("bFold"), bC = $("bCall"), bR = $("bRaise"), sl = $("slider");
    const q = [$("qHalf"), $("qPot"), $("qAll")];
    if (!turn) {
      [bF, bC, bR, sl].concat(q).forEach(b => b.disabled = true);
      bF.innerHTML = "Fold" + K("f"); bC.innerHTML = "Check" + K("c"); bR.innerHTML = "Raise" + K("r"); $("timer").textContent = "";
      return;
    }
    const L = turn.legal;
    bF.disabled = !(L.fold && !L.check);
    bF.innerHTML = "Fold" + K("f");
    bC.disabled = false;
    bC.innerHTML = (L.check ? "Check" : "Call " + f(L.call.added) + (L.call.all_in ? " all-in" : "")) + K("c");
    const can = !!L.raise;
    [bR, sl].concat(q).forEach(b => b.disabled = !can);
    if (can) {
      const to = raiseTo();
      const bet = !L.call && S.seats[mySeat].bet === 0 && turn.street !== "preflop";
      bR.innerHTML = (to === L.max_to ? "All-in " : bet ? "Bet " : "Raise to ") + f(to) + K("r");
    } else bR.innerHTML = "Raise" + K("r");
  }

  function act(type, to) {
    if (!turn) return;
    const d = { hand_id: turn.hand_id, decision_id: turn.decision_id, type };
    if (to !== undefined) d.to = to;
    send("act", d);
    turn = null; clearInterval(tickTimer);
    say("...");
    renderControls();
  }

  $("bFold").onclick = () => act("fold");
  $("bCall").onclick = () => { if (turn) act(turn.legal.check ? "check" : "call"); };
  $("bRaise").onclick = () => { if (turn && turn.legal.raise) act("raise", raiseTo()); };
  $("slider").oninput = renderControls;
  $("qHalf").onclick = () => { if (turn) setRaiseTo(potSize(0.5)); };
  $("qPot").onclick = () => { if (turn) setRaiseTo(potSize(1)); };
  $("qAll").onclick = () => { if (turn) setRaiseTo(turn.legal.raise.max_to); };
  $("bReset").onclick = () => { tally = { hands: 0, net: 0 }; saveTally(); logRows = []; renderResult(); renderLog(); };
  $("bNew").onclick = () => start();
  document.addEventListener("keydown", e => {
    if (e.ctrlKey || e.metaKey || e.altKey || !turn) return;
    const k = e.key.toLowerCase(), b = k === "f" ? $("bFold") : k === "c" ? $("bCall") : k === "r" ? $("bRaise") : null;
    if (b && !b.disabled) { e.preventDefault(); b.click(); }
  });
  window.addEventListener("pagehide", () => { gen++; if (ws) try { ws.onclose = null; ws.close(); } catch (e) {} });
  window.__sharingan = { get state() { return { S, turn, tally, tableId }; } };   // read-only hook for tests

  renderLog();
  start();
})();
