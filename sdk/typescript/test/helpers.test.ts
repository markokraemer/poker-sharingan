import { describe, expect, it } from "vitest";
import { call, check, fold, isLegal, legalize, legalTypes, madeHandCategory, parseActionText, potOdds, potRaiseTo, preflopStrength, raiseTo, renderState, safeAction, allInAction, bbMultipleTo } from "../src/index.js";
import { DOC_YOUR_TURN, docState, facingBetState } from "./fixtures.js";

describe("helpers", () => {
  it("parses the doc example", () => {
    const s = docState();
    expect(s.decision_id).toBe("d-77-9");
    expect(s.hand_id).toBe("h-77"); // from the envelope
    expect(legalTypes(s)).toEqual(["fold", "check", "raise"]);
    expect(potOdds(s)).toBe(0);
  });
  it("pot odds and sizes", () => {
    const s = facingBetState();
    expect(potOdds(s)).toBeCloseTo(200 / 950);
    expect(potRaiseTo(s, 1)).toBe(1150);
    expect(potRaiseTo(s, 0)).toBe(400);
    expect(bbMultipleTo(s, 1000)).toBe(9750);
    expect(allInAction(s)).toEqual(raiseTo(9750));
    expect(potRaiseTo(docState(), 0.5)).toBe(275);
  });
  it("legalize", () => {
    const s = docState();
    expect(legalize(s, fold())).toEqual(check());
    expect(legalize(s, call())).toEqual(check());
    expect(legalize(s, raiseTo(1))).toEqual(raiseTo(100));
    expect(legalize(s, raiseTo(1e9))).toEqual(raiseTo(9750));
    expect(legalize(s, { type: "all_in" })).toEqual(raiseTo(9750));
    expect(legalize(s, { type: "bet", to: 300 })).toEqual(raiseTo(300));
    const f = facingBetState();
    expect(legalize(f, check())).toEqual(fold());
    f.legal.raise = null;
    expect(legalize(f, raiseTo(500))).toEqual(call());
    expect(isLegal(s, raiseTo(100))).toBe(true);
    expect(isLegal(s, raiseTo(99))).toBe(false);
    expect(isLegal(s, call())).toBe(false);
    expect(safeAction(f)).toEqual(fold());
  });
  it("parses action text", () => {
    expect(parseActionText("I think we should RAISE TO 450.")).toEqual({ type: "raise", to: 450 });
    expect(parseActionText("ACTION: call")).toEqual(call());
    expect(parseActionText("all-in!")).toEqual({ type: "all_in" });
    expect(parseActionText("no idea")).toBeNull();
    expect(parseActionText("call... actually raise 300")).toEqual({ type: "raise", to: 300 });
  });
  it("strength", () => {
    expect(preflopStrength(["Ah", "Ad"])).toBeGreaterThanOrEqual(19);
    expect(preflopStrength(["7h", "2d"])).toBeLessThan(3);
    expect(madeHandCategory(["Ah", "Ad"], ["7c", "8d", "2s"])).toBe(1);
    expect(madeHandCategory(["Ah", "Kd"], ["7c", "8d", "2s"])).toBe(0);
    expect(madeHandCategory(["Ah", "Kd"], ["Ac", "Kc", "2s"])).toBe(2);
    expect(madeHandCategory(["Ah", "Kd"], ["Qc", "Jc", "Ts"])).toBe(3);
  });
  it("renders state", () => {
    const t = renderState(facingBetState());
    for (const n of ["Ah Kd", "7c 8d 2s", "To call: 200", "raise (to an amount between 400 and 9750", "post_sb to 50"]) expect(t).toContain(n);
    expect(DOC_YOUR_TURN.type).toBe("your_turn");
  });
});
