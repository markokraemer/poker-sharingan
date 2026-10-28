//! nlhe `Action` (our chips) -> a legal Open Poker action (their chips).
//!
//! Open Poker's raise amount is a raise-to total on the current street, which is exactly
//! nlhe's `RaiseTo`, so mapping is: divide by [`crate::SCALE`], round, clamp into the advertised
//! `[min, max]`. Anything the server does not offer degrades to the closest passive action.

use crate::layout::unscale;
use crate::proto::ValidSet;
use nlhe::Action;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OpAction {
    pub action: String,
    pub amount: Option<i64>,
}

impl OpAction {
    fn simple(a: &str) -> Self {
        OpAction { action: a.into(), amount: None }
    }
    fn raise(x: i64) -> Self {
        OpAction { action: "raise".into(), amount: Some(x) }
    }
    pub fn label(&self) -> String {
        match self.amount {
            Some(a) => format!("{}:{a}", self.action),
            None => self.action.clone(),
        }
    }
}

/// How faithfully the nlhe action was realised (for the trace).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Fidelity {
    Exact,
    /// Raise size rounded/clamped: (wanted, sent) in Open Poker chips.
    Adjusted(i64, i64),
    /// The wanted action was not on offer; this is the substitute.
    Degraded(&'static str),
}

pub fn map_action(a: Action, v: &ValidSet) -> (OpAction, Fidelity) {
    match a {
        Action::Fold => {
            if v.fold {
                (OpAction::simple("fold"), Fidelity::Exact)
            } else {
                (passive(v), Fidelity::Degraded("fold not offered"))
            }
        }
        Action::Check => {
            if v.check {
                (OpAction::simple("check"), Fidelity::Exact)
            } else if v.call.is_some() {
                (OpAction::simple("call"), Fidelity::Degraded("check not offered"))
            } else {
                (passive(v), Fidelity::Degraded("check not offered"))
            }
        }
        Action::Call => {
            if v.call.is_some() {
                (OpAction::simple("call"), Fidelity::Exact)
            } else if v.check {
                (OpAction::simple("check"), Fidelity::Degraded("call not offered"))
            } else {
                (passive(v), Fidelity::Degraded("call not offered"))
            }
        }
        Action::RaiseTo(t) => {
            let want = unscale(t);
            if let Some((lo, hi)) = v.raise {
                let amt = want.clamp(lo, hi);
                let f = if amt == want { Fidelity::Exact } else { Fidelity::Adjusted(want, amt) };
                (OpAction::raise(amt), f)
            } else if v.all_in.is_some() {
                // Only a (short) all-in is possible; it is the nearest aggressive action.
                (OpAction::simple("all_in"), Fidelity::Degraded("raise not offered; all_in"))
            } else if v.call.is_some() {
                (OpAction::simple("call"), Fidelity::Degraded("raise not offered; call"))
            } else if v.check {
                (OpAction::simple("check"), Fidelity::Degraded("raise not offered; check"))
            } else {
                (passive(v), Fidelity::Degraded("raise not offered"))
            }
        }
    }
}

/// Safest legal action: check, else fold, else call, else all-in.
pub fn passive(v: &ValidSet) -> OpAction {
    if v.check {
        OpAction::simple("check")
    } else if v.fold {
        OpAction::simple("fold")
    } else if v.call.is_some() {
        OpAction::simple("call")
    } else if v.all_in.is_some() {
        OpAction::simple("all_in")
    } else if let Some((lo, _)) = v.raise {
        OpAction::raise(lo)
    } else {
        OpAction::simple("check")
    }
}

/// Effective-stack guard: never commit more than `cap` chips in total in one hand.
///
/// `prior` = chips committed on earlier streets, `allin_total` = total contribution if we
/// shoved now. A raise/all-in that would exceed `cap` becomes a raise to exactly `cap - prior`
/// on this street, or (if that is not a legal raise) a call, else a check. Folds, checks and
/// calls pass through untouched. Returns the replacement and a trace note.
pub fn cap_action(a: &OpAction, v: &ValidSet, cap: i64, prior: i64, allin_total: i64) -> Option<(OpAction, String)> {
    let exceeds = match a.action.as_str() {
        "raise" => a.amount.is_some_and(|x| prior + x > cap),
        "all_in" => allin_total > cap,
        _ => false,
    };
    if !exceeds {
        return None;
    }
    let r = cap - prior;
    let new = match v.raise {
        Some((lo, hi)) if r >= lo => OpAction::raise(r.min(hi)),
        _ if v.call.is_some() => OpAction::simple("call"),
        _ if v.check => OpAction::simple("check"),
        _ => return None,
    };
    let note = format!("{} -> {} (cap {cap}, committed on earlier streets {prior})", a.label(), new.label());
    Some((new, note))
}

/// Is `a` something `v` offers? (Used by tests and as a last assertion before sending.)
pub fn is_legal(a: &OpAction, v: &ValidSet) -> bool {
    match a.action.as_str() {
        "fold" => v.fold,
        "check" => v.check,
        "call" => v.call.is_some(),
        "all_in" => v.all_in.is_some(),
        "raise" => match (a.amount, v.raise) {
            (Some(x), Some((lo, hi))) => x >= lo && x <= hi,
            _ => false,
        },
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cards::Rng;

    fn vs(fold: bool, check: bool, call: Option<i64>, raise: Option<(i64, i64)>, all_in: Option<Option<i64>>) -> ValidSet {
        ValidSet { fold, check, call, raise, all_in }
    }

    #[test]
    fn raise_scaling_rounding_and_clamp() {
        let v = vs(true, false, Some(20), Some((40, 500)), None);
        // 5x chips: 300 -> 60
        assert_eq!(map_action(Action::RaiseTo(300), &v), (OpAction::raise(60), Fidelity::Exact));
        // 303 rounds to 61 (60.6), not exact
        assert_eq!(map_action(Action::RaiseTo(303), &v).0, OpAction::raise(61));
        // below min -> min, above max -> max
        assert_eq!(map_action(Action::RaiseTo(100), &v), (OpAction::raise(40), Fidelity::Adjusted(20, 40)));
        assert_eq!(map_action(Action::RaiseTo(10_000), &v), (OpAction::raise(500), Fidelity::Adjusted(2000, 500)));
    }

    #[test]
    fn degrade_when_not_offered() {
        let only_call = vs(true, false, Some(20), None, None);
        assert_eq!(map_action(Action::RaiseTo(500), &only_call).0, OpAction::simple("call"));
        let short_shove = vs(true, false, Some(20), None, Some(Some(30)));
        assert_eq!(map_action(Action::RaiseTo(500), &short_shove).0, OpAction::simple("all_in"));
        let free = vs(false, true, None, Some((20, 100)), None);
        assert_eq!(map_action(Action::Fold, &free).0, OpAction::simple("check"));
        assert_eq!(map_action(Action::Call, &free).0, OpAction::simple("check"));
        let facing = vs(true, false, Some(20), Some((40, 100)), None);
        assert_eq!(map_action(Action::Check, &facing).0, OpAction::simple("call"));
    }

    /// Fuzz: whatever the (random) offered set and the engine action, the output is offered.
    #[test]
    fn fuzz_always_legal() {
        let mut rng = Rng::new(42);
        let mut n = 0;
        for _ in 0..200_000 {
            let mut v = ValidSet::default();
            v.fold = rng.below(2) == 0;
            v.check = !v.fold && rng.below(4) != 0 || rng.below(8) == 0;
            if rng.below(2) == 0 {
                v.call = Some(rng.below(3000) as i64);
            }
            if rng.below(3) != 0 {
                let lo = 1 + rng.below(2000) as i64;
                v.raise = Some((lo, lo + rng.below(3000) as i64));
            }
            if rng.below(4) == 0 {
                v.all_in = Some(if rng.below(2) == 0 { Some(rng.below(900) as i64) } else { None });
            }
            if v.is_empty() {
                continue;
            }
            let a = match rng.below(4) {
                0 => Action::Fold,
                1 => Action::Check,
                2 => Action::Call,
                _ => Action::RaiseTo(rng.below(40_000) as i32),
            };
            let (m, _) = map_action(a, &v);
            assert!(is_legal(&m, &v), "{a:?} -> {m:?} not in {v:?}");
            assert!(is_legal(&passive(&v), &v), "passive illegal in {v:?}");
            n += 1;
        }
        assert!(n > 100_000);
    }
}
