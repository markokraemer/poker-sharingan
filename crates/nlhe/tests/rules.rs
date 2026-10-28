use cards::parse_cards;
use nlhe::*;

fn cfg(n: usize) -> GameConfig {
    GameConfig::new(n)
}

fn deal(holes: &[&str], board: &str) -> Deal {
    let mut d = Deal {
        holes: [[0; 2]; 6],
        board: [0; 5],
    };
    for (i, h) in holes.iter().enumerate() {
        let c = parse_cards(h).unwrap();
        d.holes[i] = [c[0], c[1]];
    }
    d.board.copy_from_slice(&parse_cards(board).unwrap());
    d
}

#[test]
fn heads_up_blind_order() {
    let mut s = State::new(&cfg(2));
    assert_eq!(s.to_act(), Some(0)); // SB/button acts first preflop
    assert_eq!((s.street_bet(0), s.street_bet(1), s.pot()), (50, 100, 150));
    let l = s.legal();
    assert!(l.can_fold && !l.can_check && l.call_amount == 50);
    assert_eq!((l.min_raise_to, l.max_raise_to), (200, 10_000));
    assert_eq!(s.apply(Action::Call), Step::Continue);
    assert_eq!(s.to_act(), Some(1));
    assert!(s.legal().can_check);
    assert_eq!(s.apply(Action::Check), Step::NewStreet(Street::Flop));
    assert_eq!(s.to_act(), Some(1)); // BB first postflop
    assert_eq!(s.legal().min_raise_to, 100);
}

#[test]
fn six_max_order_and_fold_to_bb() {
    let mut s = State::new(&cfg(6));
    assert_eq!(s.to_act(), Some(2));
    for seat in [2, 3, 4, 5] {
        assert_eq!(s.to_act(), Some(seat));
        assert_eq!(s.apply(Action::Fold), Step::Continue);
    }
    assert_eq!(s.to_act(), Some(0));
    assert_eq!(s.apply(Action::Fold), Step::Terminal);
    assert!(s.is_terminal() && !s.showdown() && s.to_act().is_none());
    let d = deal(
        &["2c3c", "4c5c", "6c7c", "8c9c", "TcJc", "QcKc"],
        "2d3d4d5d6d",
    );
    assert_eq!(s.payoffs(&d), [-50, 50, 0, 0, 0, 0]);
}

#[test]
fn min_raise_tracks_increment() {
    let mut s = State::new(&cfg(6));
    s.apply(Action::RaiseTo(300)); // seat 2, +200
    assert_eq!(s.legal().min_raise_to, 500);
    s.apply(Action::RaiseTo(1000)); // seat 3, +700
    assert_eq!(s.legal().min_raise_to, 1700);
    assert_eq!(s.legal().call_amount, 1000);
    assert_eq!(s.raises_this_street(), 2);
    for _ in 0..4 {
        s.apply(Action::Fold);
    }
    // seats 4, 5, 0, 1 fold; seat 2 to act facing 700 more
    assert_eq!(s.to_act(), Some(2));
    assert_eq!(s.legal().call_amount, 700);
    assert_eq!(s.apply(Action::Call), Step::NewStreet(Street::Flop));
    assert_eq!(s.to_act(), Some(2));
    assert_eq!(s.legal().min_raise_to, 100); // reset to BB each street
}

#[test]
fn short_all_in_does_not_reopen() {
    let c = cfg(3).with_stacks(&[10_000, 1_100, 10_000]);
    let mut s = State::new(&c);
    s.apply(Action::RaiseTo(1000)); // seat 2, inc 900
    s.apply(Action::Call); // seat 0 calls
    assert_eq!(s.to_act(), Some(1));
    let l = s.legal();
    assert_eq!((l.min_raise_to, l.max_raise_to), (1100, 1100)); // short all-in only
    assert_eq!(s.apply(Action::RaiseTo(1100)), Step::Continue);
    // seat 2 already acted at full raise: call/fold only
    assert_eq!(s.to_act(), Some(2));
    let l = s.legal();
    assert!(!l.can_raise && l.can_fold && l.call_amount == 100);
    s.apply(Action::Call);
    assert_eq!(s.to_act(), Some(0));
    assert!(!s.legal().can_raise);
    // seat 1 is all-in, seats 0 and 2 still have chips: play continues on the flop
    assert_eq!(s.apply(Action::Call), Step::NewStreet(Street::Flop));
    assert_eq!(s.to_act(), Some(0));
    assert_eq!(s.pot(), 3300);
}

#[test]
fn short_all_in_still_open_to_unacted() {
    let c = cfg(4).with_stacks(&[10_000, 10_000, 10_000, 1_100]);
    let mut s = State::new(&c);
    s.apply(Action::RaiseTo(1000)); // seat 2
    s.apply(Action::RaiseTo(1100)); // seat 3 short all-in (inc 100 < 900)
    assert_eq!(s.to_act(), Some(0)); // not yet acted: may raise, min = 1100 + 900
    let l = s.legal();
    assert!(l.can_raise);
    assert_eq!(l.min_raise_to, 2000);
    s.apply(Action::Call);
    assert_eq!(s.to_act(), Some(1));
    s.apply(Action::Fold);
    assert_eq!(s.to_act(), Some(2));
    assert!(!s.legal().can_raise);
}

#[test]
fn full_raise_reopens() {
    let mut s = State::new(&cfg(3));
    s.apply(Action::RaiseTo(300)); // seat 2
    s.apply(Action::Call); // seat 0
    s.apply(Action::RaiseTo(700)); // seat 1 full raise (inc 400)
    assert_eq!(s.to_act(), Some(2));
    assert!(s.legal().can_raise);
    assert_eq!(s.legal().min_raise_to, 1100);
}

fn three_way_all_in() -> State {
    let mut s = State::new(&cfg(3).with_stacks(&[1000, 2000, 3000]));
    assert_eq!(s.to_act(), Some(2));
    s.apply(Action::RaiseTo(3000));
    s.apply(Action::Call); // seat 0 all-in for 1000
    assert_eq!(s.apply(Action::Call), Step::Terminal); // seat 1 all-in for 2000
    assert!(s.showdown());
    s
}

#[test]
fn three_way_side_pots() {
    let s = three_way_all_in();
    let d = deal(&["AhAd", "KhKd", "QhQd"], "2c7d9hJs3c");
    assert_eq!(s.payoffs(&d)[..3], [2000, 0, -2000]);
    // seat 2 (short of nothing) wins everything it covers; uncalled 1000 returned
    let d = deal(&["QhQd", "KhKd", "AhAd"], "2c7d9hJs3c");
    assert_eq!(s.payoffs(&d)[..3], [-1000, -2000, 3000]);
}

#[test]
fn three_way_side_pot_with_split() {
    let s = three_way_all_in();
    let d = deal(&["AhAd", "AcAs", "QhQd"], "2c7d9hJs3c");
    // main 3000 split between seats 0,1; side 2000 to seat 1; 1000 uncalled back to seat 2
    assert_eq!(s.payoffs(&d)[..3], [500, 1500, -2000]);
    let all_tie = deal(&["2h2d", "3h3d", "4h4d"], "TsJsQsKsAs");
    assert_eq!(s.payoffs(&all_tie)[..3], [0, 0, 0]);
}

#[test]
fn odd_chip_goes_left_of_button() {
    let mut c = cfg(3);
    c.small_blind = 25;
    c.big_blind = 50;
    let mut s = State::new(&c);
    s.apply(Action::Call); // seat 2
    s.apply(Action::Fold); // seat 0, 25 dead
    assert_eq!(s.apply(Action::Check), Step::NewStreet(Street::Flop)); // seat 1
                                                                       // postflop: seats 1 and 2 remain; first left of button (seat 2) among live is seat 1
    assert_eq!(s.to_act(), Some(1));
    s.apply(Action::Check);
    s.apply(Action::Check);
    s.apply(Action::Check);
    s.apply(Action::Check);
    s.apply(Action::Check);
    assert_eq!(s.apply(Action::Check), Step::Terminal);
    let d = deal(&["2h2d", "3h3d", "4h4d"], "TsJsQsKsAs");
    // pot 125 split 62/63, odd chip to seat 1 (first live seat left of the button)
    assert_eq!(s.payoffs(&d)[..3], [-25, 13, 12]);
}

#[test]
fn heads_up_check_through_showdown() {
    let mut s = State::new(&cfg(2));
    s.apply(Action::Call);
    assert_eq!(s.apply(Action::Check), Step::NewStreet(Street::Flop));
    assert_eq!(s.apply(Action::Check), Step::Continue);
    assert_eq!(s.apply(Action::Check), Step::NewStreet(Street::Turn));
    s.apply(Action::Check);
    assert_eq!(s.apply(Action::Check), Step::NewStreet(Street::River));
    s.apply(Action::Check);
    assert_eq!(s.apply(Action::Check), Step::Terminal);
    assert!(s.showdown());
    let d = deal(&["AhAd", "KhKd"], "2c7d9hJs3c");
    assert_eq!(s.payoffs(&d)[..2], [100, -100]);
}

#[test]
fn all_in_runout_and_uncalled_bet() {
    let mut s = State::new(&cfg(2));
    s.apply(Action::RaiseTo(10_000));
    assert_eq!(s.apply(Action::Call), Step::Terminal);
    assert!(s.showdown());
    let d = deal(&["AhAd", "KhKd"], "2c7d9hJs3c");
    assert_eq!(s.payoffs(&d)[..2], [10_000, -10_000]);

    // bet larger than the caller's stack: excess is returned
    let mut s = State::new(&cfg(2).with_stacks(&[10_000, 500]));
    assert_eq!(s.legal().max_raise_to, 10_000);
    s.apply(Action::RaiseTo(2000));
    assert_eq!(s.legal().call_amount, 400);
    assert_eq!(s.apply(Action::Call), Step::Terminal);
    let d = deal(&["KhKd", "AhAd"], "2c7d9hJs3c");
    assert_eq!(s.payoffs(&d)[..2], [-500, 500]);
}

#[test]
fn short_blinds() {
    let s = State::new(&cfg(3).with_stacks(&[30, 60, 10_000]));
    assert_eq!((s.street_bet(0), s.street_bet(1)), (30, 60));
    assert!(s.is_all_in(0));
    assert_eq!(s.to_act(), Some(2));
}
