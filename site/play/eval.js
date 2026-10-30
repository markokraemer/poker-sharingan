// Poker hand evaluator and namer for the play page (no dependencies, runs in browsers and Node).
// Cards are two characters: rank 23456789TJQKA, suit cdhs ("Ah", "Td").
(function (root) {
  "use strict";
  var RANKS = "23456789TJQKA";
  var CAT = ["high card", "pair", "two pair", "three of a kind", "straight", "flush", "full house", "four of a kind", "straight flush"];
  var SING = { 2: "Two", 3: "Three", 4: "Four", 5: "Five", 6: "Six", 7: "Seven", 8: "Eight", 9: "Nine", 10: "Ten", 11: "Jack", 12: "Queen", 13: "King", 14: "Ace" };
  var PLUR = { 2: "Twos", 3: "Threes", 4: "Fours", 5: "Fives", 6: "Sixes", 7: "Sevens", 8: "Eights", 9: "Nines", 10: "Tens", 11: "Jacks", 12: "Queens", 13: "Kings", 14: "Aces" };

  function rankOf(c) { return RANKS.indexOf(c[0]) + 2; }

  // Score one 5-card hand: { cat, tb (tie-break ranks, most significant first), score }.
  function eval5(cs) {
    var r = cs.map(rankOf).sort(function (a, b) { return b - a; });
    var flush = cs.every(function (c) { return c[1] === cs[0][1]; });
    var cnt = {};
    r.forEach(function (x) { cnt[x] = (cnt[x] || 0) + 1; });
    // groups: by count desc, then rank desc
    var groups = Object.keys(cnt).map(function (k) { return [cnt[k], +k]; })
      .sort(function (a, b) { return b[0] - a[0] || b[1] - a[1]; });
    var uniq = groups.length === 5;
    var straightHigh = 0;
    if (uniq) {
      if (r[0] - r[4] === 4) straightHigh = r[0];
      else if (r[0] === 14 && r[1] === 5 && r[2] === 4 && r[3] === 3 && r[4] === 2) straightHigh = 5;   // wheel
    }
    var cat, tb;
    if (straightHigh && flush) { cat = 8; tb = [straightHigh]; }
    else if (groups[0][0] === 4) { cat = 7; tb = [groups[0][1], groups[1][1]]; }
    else if (groups[0][0] === 3 && groups[1][0] === 2) { cat = 6; tb = [groups[0][1], groups[1][1]]; }
    else if (flush) { cat = 5; tb = r; }
    else if (straightHigh) { cat = 4; tb = [straightHigh]; }
    else if (groups[0][0] === 3) { cat = 3; tb = groups.map(function (g) { return g[1]; }); }
    else if (groups[0][0] === 2 && groups[1][0] === 2) { cat = 2; tb = groups.map(function (g) { return g[1]; }); }
    else if (groups[0][0] === 2) { cat = 1; tb = groups.map(function (g) { return g[1]; }); }
    else { cat = 0; tb = r; }
    var score = cat;
    for (var i = 0; i < 5; i++) score = score * 15 + (tb[i] || 0);
    return { cat: cat, tb: tb, score: score };
  }

  // Best 5 of any 5..7 cards. Returns { cat, tb, score, cards (the 5 chosen, strongest first) }.
  function best(cards) {
    var n = cards.length, top = null;
    if (n < 5) return null;
    var idx = [0, 1, 2, 3, 4];
    for (;;) {
      var pick = idx.map(function (i) { return cards[i]; });
      var e = eval5(pick);
      if (!top || e.score > top.score) { top = e; top.cards = pick; }
      var k = 4;
      while (k >= 0 && idx[k] === n - 5 + k) k--;
      if (k < 0) break;
      idx[k]++;
      for (var j = k + 1; j < 5; j++) idx[j] = idx[j - 1] + 1;
    }
    top.cards = order(top);
    return top;
  }

  // The 5 cards in reading order (pair first, straights high to low, wheel ends with the ace).
  function order(h) {
    var cs = h.cards.slice();
    var rk = function (c) { return rankOf(c); };
    if (h.cat === 4 || h.cat === 8) {
      cs.sort(function (a, b) { return rk(b) - rk(a); });
      if (h.tb[0] === 5) { var ace = cs.shift(); cs.push(ace); }
      return cs;
    }
    var cnt = {};
    cs.forEach(function (c) { cnt[rk(c)] = (cnt[rk(c)] || 0) + 1; });
    return cs.sort(function (a, b) { return (cnt[rk(b)] - cnt[rk(a)]) || (rk(b) - rk(a)); });
  }

  // "Two pair, Aces and Sevens". Royal flush is its own name.
  function name(h) {
    var t = h.tb;
    switch (h.cat) {
      case 0: return "High card, " + SING[t[0]];
      case 1: return "Pair of " + PLUR[t[0]];
      case 2: return "Two pair, " + PLUR[t[0]] + " and " + PLUR[t[1]];
      case 3: return "Three of a kind, " + PLUR[t[0]];
      case 4: return "Straight, " + SING[t[0]] + " high";
      case 5: return "Flush, " + SING[t[0]] + " high";
      case 6: return "Full house, " + PLUR[t[0]] + " full of " + PLUR[t[1]];
      case 7: return "Four of a kind, " + PLUR[t[0]];
      default: return t[0] === 14 ? "Royal flush" : "Straight flush, " + SING[t[0]] + " high";
    }
  }
  function catName(h) { return CAT[h.cat]; }

  // The first kicker that separates two hands of the same category and same made ranks, or 0.
  // Used to say "Pair of Kings, Ace kicker" only when it decides something.
  function kickerVs(a, b) {
    if (a.cat !== b.cat) return 0;
    var made = { 1: 1, 2: 2, 3: 1, 7: 1 }[a.cat];
    if (!made) return 0;
    for (var i = 0; i < made; i++) if (a.tb[i] !== b.tb[i]) return 0;
    for (var j = made; j < a.tb.length; j++) if (a.tb[j] !== b.tb[j]) return a.tb[j];
    return 0;
  }
  function withKicker(h, k) { return name(h) + ", " + SING[k] + " kicker"; }

  function cmp(a, b) { return a.score - b.score; }

  // Split every pot between the best eligible hands. holes: {seat: [c,c]}, live: seats still in the hand,
  // pots: [{amount, eligible_seats}], button: seat index. Odd chips go to the first winner clockwise
  // from the button. Returns [{amount, eligible_seats, winners, share}].
  function awards(pots, holes, board, button, maxSeats, folded) {
    maxSeats = maxSeats || 6;
    var hands = {};
    Object.keys(holes).forEach(function (s) {
      if (folded && folded[s]) return;
      if (board.length === 5) hands[s] = best(holes[s].concat(board));
    });
    return pots.map(function (p) {
      var el = p.eligible_seats;
      var win;
      if (board.length === 5) {
        var top = Math.max.apply(null, el.map(function (s) { return hands[s].score; }));
        win = el.filter(function (s) { return hands[s].score === top; });
      } else win = el.slice();   // everyone else folded: a single eligible seat
      var order = [];
      for (var k = 1; k <= maxSeats; k++) { var s = (button + k) % maxSeats; if (win.indexOf(s) >= 0) order.push(s); }
      var q = Math.floor(p.amount / order.length), odd = p.amount % order.length;
      return { amount: p.amount, eligible_seats: el, winners: order, share: order.map(function (_, i) { return q + (i < odd ? 1 : 0); }) };
    });
  }

  var api = { RANKS: RANKS, rankOf: rankOf, eval5: eval5, best: best, name: name, catName: catName, kickerVs: kickerVs, withKicker: withKicker, cmp: cmp, awards: awards, SING: SING, PLUR: PLUR };
  if (typeof module !== "undefined" && module.exports) module.exports = api; else root.PokerEval = api;
})(typeof window !== "undefined" ? window : globalThis);
