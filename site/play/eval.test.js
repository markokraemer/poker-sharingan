// Unit tests for eval.js. Runs in a browser (Playwright: window.PokerEval) or Node (module.exports).
// run(E) returns { passed, failed, failures: [...] }.
(function (root) {
  "use strict";
  function run(E) {
    var failures = [], passed = 0;
    var C = function (s) { return s.split(" "); };
    function eq(label, got, want) { if (got === want) passed++; else failures.push(label + ": got " + JSON.stringify(got) + " want " + JSON.stringify(want)); }
    function nameOf(hole, board) { return E.name(E.best(C(hole).concat(C(board)))); }
    function sc(hole, board) { return E.best(C(hole).concat(C(board))).score; }

    // ---- naming ----
    eq("high card", nameOf("Ah 9d", "7c 5s 3h 2d Jc"), "High card, Ace");
    eq("pair", nameOf("Kh Kd", "7c 5s 3h 2d Jc"), "Pair of Kings");
    eq("two pair", nameOf("Ah 7d", "Ac 7s 3h 2d Jc"), "Two pair, Aces and Sevens");
    eq("two pair, 3 pairs picks top two", nameOf("Ah 7d", "Ac 7s 3h 3d Jc"), "Two pair, Aces and Sevens");
    eq("trips", nameOf("9h 9d", "9c 5s 3h 2d Jc"), "Three of a kind, Nines");
    eq("wheel straight", nameOf("Ah 2d", "3c 4s 5h Kd Kc"), "Straight, Five high");
    eq("wheel beats nothing: 6-high straight is higher", sc("6h 2d", "3c 4s 5h Kd Kc") > sc("Ah 2d", "3c 4s 5h Kd Kc"), true);
    eq("broadway", nameOf("Ah Kd", "Qc Js Th 2d 2c"), "Straight, Ace high");
    eq("not a wraparound straight (QKA23)", nameOf("Qh Kd", "Ac 2s 3h 9d 9c"), "Pair of Nines");
    eq("flush", nameOf("Ah 9h", "7h 5h 3h 2d Jc"), "Flush, Ace high");
    eq("flush beats straight", sc("Ah 9h", "7h 5h 3h 4d 6c") > sc("8c 9d", "7h 5h 3h 4d 6c"), true);
    eq("flush beats a 9-high straight in the same seven", nameOf("Ah 9h", "7h 8d 6h 5h Kc"), "Flush, Ace high");
    eq("straight flush needs five suited in sequence", nameOf("9h 8h", "7h 6h 5h 4h Kc"), "Straight flush, Nine high");
    eq("7-card flush picks the five highest", E.best(C("Ah Kh 9h 7h 5h 3h 2h")).cards.join(" "), "Ah Kh 9h 7h 5h");
    eq("full house", nameOf("Kh Kd", "Kc 5s 5h 2d Jc"), "Full house, Kings full of Fives");
    eq("full house from two trips picks the higher trips", nameOf("Kh Kd", "Kc 5s 5h 5d Jc"), "Full house, Kings full of Fives");
    eq("full house trips over pairs", nameOf("Kh Kd", "Kc 5s 5h 2d 2c"), "Full house, Kings full of Fives");
    eq("quads", nameOf("Qh Qd", "Qc Qs 5h 2d Jc"), "Four of a kind, Queens");
    eq("straight flush", nameOf("9h 8h", "7h 6h 5h 2d Jc"), "Straight flush, Nine high");
    eq("steel wheel", nameOf("Ah 2h", "3h 4h 5h Kd Kc"), "Straight flush, Five high");
    eq("royal flush", nameOf("Ah Kh", "Qh Jh Th 2d Jc"), "Royal flush");
    eq("rank order: quads > full house", sc("Qh Qd", "Qc Qs 5h 5d Jc") > sc("Kh Kd", "Kc 5s 5h 2d 2c"), true);
    eq("rank order: full house > flush", sc("Kh Kd", "Kc 5s 5h 2d 2c") > sc("Ah 9h", "7h 5h 3h 2d Jc"), true);
    eq("rank order: straight > trips", sc("9h 8d", "7c 6s 5h 2d Kc") > sc("Ah Ad", "Ac 6s 5h 2d Kc"), true);
    eq("rank order: two pair > pair", sc("Ah 7d", "Ac 7s 3h 2d Jc") > sc("Ah Ad", "Kc 7s 3h 2d Jc"), true);

    // ---- kickers and ties ----
    eq("kicker decides pair", sc("Ah Kd", "Kc 7s 3h 2d 9c") > sc("Qh Kh", "Ks 7s 3h 2d 9c") , true);
    eq("second kicker decides", sc("Ah Kd", "Ac 7s 3h 2d 9c") > sc("Ad Qh", "As 7s 3h 2d 9c"), true);
    eq("board plays: tie", sc("2h 3d", "Ac Kc Qc Jc Tc"), sc("4h 5d", "Ac Kc Qc Jc Tc"));
    eq("lower kicker on board plays: tie", sc("2h 3d", "Ac Ad Kc Qs 9h"), sc("4h 5d", "Ac Ad Kc Qs 9h"));
    eq("same straight, suits irrelevant", sc("9h 8d", "7c 6s 5h Kd Kc"), sc("9d 8c", "7c 6s 5h Kd Kc"));
    eq("two pair kicker", sc("Ah 7d", "Ac 7s Kh 2d Jc") > sc("Ad 7c", "As 7h Qh 2d Jc"), true);
    eq("trips kicker names", nameOf("9h 9d", "9c Ks 3h 2d Jc"), "Three of a kind, Nines");
    var a = E.best(C("Ah Kd Kc 7s 3h 2d 9c")), b = E.best(C("Qh Kh Ks 7s 3h 2d 9c"));
    eq("kickerVs ace vs queen", E.kickerVs(a, b), 14);
    eq("kickerVs equal hands", E.kickerVs(E.best(C("2h 3d Ac Kc Qc Jc Tc")), E.best(C("4h 5d Ac Kc Qc Jc Tc"))), 0);
    eq("withKicker", E.withKicker(a, 14), "Pair of Kings, Ace kicker");
    eq("different categories have no kicker", E.kickerVs(a, E.best(C("Ah Ad Ac 7s 3h 2d 9c"))), 0);

    // ---- reading order of the best five ----
    eq("wheel is shown 5 4 3 2 A", E.best(C("Ah 2d 3c 4s 5h Kd Kc")).cards.join(" "), "5h 4s 3c 2d Ah");
    eq("two pair order", E.best(C("Ah 7d Ac 7s 3h 2d Jc")).cards.map(function (c) { return c[0]; }).join(""), "AA77J");

    // ---- awards: side pots, split pots, odd chip ----
    var hole = { 0: C("Ah Ad"), 1: C("Kh Kd"), 2: C("Qh Qd") };
    var board = C("2c 7s 9d Jc 3h");
    var pots = [{ amount: 600, eligible_seats: [0, 1, 2] }, { amount: 400, eligible_seats: [1, 2] }];
    var aw = E.awards(pots, hole, board, 3, 6, null);
    eq("main pot to aces", JSON.stringify(aw[0].winners), "[0]");
    eq("side pot to kings", JSON.stringify(aw[1].winners) + JSON.stringify(aw[1].share), "[1][400]");
    var hole2 = { 1: C("2h 3d"), 3: C("4h 5d"), 5: C("6h 7d") };
    var aw2 = E.awards([{ amount: 1001, eligible_seats: [1, 3] }], { 1: hole2[1], 3: hole2[3] }, C("Ac Kc Qc Jc Tc"), 5, 6, null);
    eq("split + odd chip goes first clockwise from the button (button 5 -> seat 1 first)", JSON.stringify(aw2[0].winners) + JSON.stringify(aw2[0].share), "[1,3][501,500]");
    var aw3 = E.awards([{ amount: 1001, eligible_seats: [1, 3] }], { 1: hole2[1], 3: hole2[3] }, C("Ac Kc Qc Jc Tc"), 1, 6, null);
    eq("odd chip with the button on seat 1 goes to seat 3", JSON.stringify(aw3[0].winners) + JSON.stringify(aw3[0].share), "[3,1][501,500]");

    return { passed: passed, failed: failures.length, failures: failures };
  }
  var api = { run: run };
  if (typeof module !== "undefined" && module.exports) module.exports = api; else root.PokerEvalTest = api;
})(typeof window !== "undefined" ? window : globalThis);
