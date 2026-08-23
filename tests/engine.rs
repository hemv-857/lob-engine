use lob_engine::{
    run_agent, schedule, AgentConfig, Algo, Book, CancelResult, Event, ExecSide, Lcg, Order, Side,
    TapeTick,
};

fn mk(id: u64, side: Side, price: u64, qty: u64) -> Order {
    Order {
        id,
        side,
        price,
        qty,
    }
}

#[test]
fn basic_cross_generates_trade_at_maker_price() {
    let mut book = Book::new();
    let _ = book.submit(mk(1, Side::Bid, 1000, 10));
    let events = book.submit(mk(2, Side::Ask, 990, 5));
    match &events[0] {
        Event::Trade(t) => {
            assert_eq!(t.price, 1000); // maker price, price-time priority
            assert_eq!(t.qty, 5);
            assert_eq!(t.maker_id, 1);
            assert_eq!(t.taker_id, 2);
        }
        e => panic!("expected trade, got {e:?}"),
    }
    assert_eq!(book.best_bid(), Some(1000));
    // remaining bid qty = 5
    assert_eq!(book.total_quantity(), 5);
}

#[test]
fn fifo_within_level_price_time_priority() {
    let mut book = Book::new();
    let _ = book.submit(mk(1, Side::Bid, 1000, 5)); // first in time
    let _ = book.submit(mk(2, Side::Bid, 1000, 7)); // second
    let _ = book.submit(mk(3, Side::Bid, 1001, 3)); // better price jumps queue
    assert_eq!(book.best_bid(), Some(1001));
    let events = book.submit(mk(9, Side::Ask, 999, 8));
    let trades: Vec<_> = events
        .iter()
        .filter_map(|e| match e {
            Event::Trade(t) => Some(*t),
            _ => None,
        })
        .collect();
    // ask for 8 eats best level (1001 x 3 = order 3), then FIFO head at 1000 (order 1, qty 5)
    assert_eq!(trades.len(), 2);
    assert_eq!(trades[0].maker_id, 3);
    assert_eq!(trades[0].qty, 3);
    assert_eq!(trades[1].maker_id, 1);
    assert_eq!(trades[1].qty, 5);
    assert_eq!(book.total_quantity(), 7); // order 2 remains
}

#[test]
fn cancel_removes_only_target_order() {
    let mut book = Book::new();
    let _ = book.submit(mk(1, Side::Bid, 1000, 10));
    let _ = book.submit(mk(2, Side::Bid, 1000, 20));
    assert_eq!(book.cancel(1), CancelResult::Canceled { remaining_qty: 0 });
    assert_eq!(book.cancel(1), CancelResult::NotFound); // double cancel
    assert_eq!(book.cancel(999), CancelResult::NotFound);
    assert_eq!(book.total_quantity(), 20);
    assert_eq!(book.open_orders(), 1);
}

#[test]
fn partial_fill_keeps_maker_in_queue_position() {
    let mut book = Book::new();
    let _ = book.submit(mk(1, Side::Ask, 1010, 10));
    let _ = book.submit(mk(2, Side::Ask, 1010, 10));
    let _ = book.submit(mk(3, Side::Bid, 1010, 4)); // hits order 1 partially
    let events = book.submit(mk(4, Side::Bid, 1010, 4));
    if let Some(Event::Trade(t)) = events.first() {
        assert_eq!(t.maker_id, 1); // order 1 keeps priority after partial fill
    } else {
        panic!("no trade");
    }
}

#[test]
fn determinism_same_stream_byte_identical_results() {
    fn run(seed: u64) -> (u64, u64) {
        let mut rng = Lcg::new(seed);
        let mut book = Book::new();
        let mut oid = 0;
        let mut trade_qty = 0;
        for _ in 0..4000 {
            oid += 1;
            let side = if rng.below(2) == 0 {
                Side::Bid
            } else {
                Side::Ask
            };
            let price = 1000 + rng.below(21);
            let qty = 1 + rng.below(50);
            let ev = book.submit(mk(oid, side, price, qty));
            for e in ev {
                if let Event::Trade(t) = e {
                    trade_qty += t.qty;
                }
            }
            if rng.below(4) == 0 && oid > 10 {
                let _ = book.cancel(rng.below(oid as _));
            }
        }
        let final_qty = book.total_quantity();
        (trade_qty, final_qty)
    }
    let a = run(42);
    let b = run(42);
    assert_eq!(a, b, "same seed must produce identical results");
}

#[test]
fn invariants_hold_over_random_op_storms() {
    for seed in [1u64, 7, 123] {
        let mut rng = Lcg::new(seed);
        let mut book = Book::new();
        let mut live: std::collections::HashMap<u64, u64> = std::collections::HashMap::new();
        let mut oid = 0;
        for _ in 0..5000 {
            oid += 1;
            let side = if rng.below(2) == 0 {
                Side::Bid
            } else {
                Side::Ask
            };
            let price = 1000 + rng.below(11);
            let qty = 1 + rng.below(30);
            book.submit(mk(oid, side, price, qty));
            live.insert(oid, qty); // ponytail: ignores partial fills against this order later; tracks resting intent
            if live.len() > 40 || rng.below(5) == 0 {
                let k = rng.below(live.len() as u64) as usize;
                let keys: Vec<u64> = live.keys().copied().collect();
                let id = keys[k];
                if book.cancel(id) != CancelResult::NotFound {
                    live.remove(&id);
                }
            }
            // invariant: never crossed while both sides populated
            if let (Some(b), Some(a)) = (book.best_bid(), book.best_ask()) {
                assert!(b < a, "crossed book: best bid {b} >= best ask {a}");
            }
        }
        // fills can consume tracked resting intents, so book count only shrinks
        assert!(book.open_orders() <= live.len());
    }
}

// ------------------------------------------------------------------ agents
fn synthetic_tape(n: usize) -> Vec<TapeTick> {
    (0..n)
        .map(|i| TapeTick {
            qty: 100 + ((i * 37) % 200) as u64,
            price: 100.0 + (i as f64 * 0.01).sin() * 2.0,
        })
        .collect()
}

#[test]
fn schedules_sum_to_parent_qty_for_twap_vwap() {
    let tape = synthetic_tape(500);
    for algo in [Algo::Twap, Algo::Vwap] {
        let cfg = AgentConfig {
            algo,
            side: ExecSide::Buy,
            parent_qty: 10_000,
            n_slices: 10,
            pov_rate: 0.2,
        };
        let total: u64 = schedule(&tape, &cfg).iter().map(|(_, q)| q).sum();
        assert_eq!(
            total, 10_000,
            "{algo:?} must allocate exactly the parent qty"
        );
    }
}

#[test]
fn agent_fills_full_parent_on_deep_tape() {
    let tape = synthetic_tape(500);
    let cfg = AgentConfig {
        algo: Algo::Twap,
        side: ExecSide::Buy,
        parent_qty: 5_000,
        n_slices: 10,
        pov_rate: 0.2,
    };
    let stats = run_agent(&tape, &cfg);
    assert_eq!(stats.filled_qty, 5_000);
}

#[test]
fn slippage_sign_follows_direction_of_tape_drift() {
    // rising tape: buys slip positive, sells negative
    let tape: Vec<TapeTick> = (0..400)
        .map(|i| TapeTick {
            qty: 100,
            price: 100.0 + i as f64 * 0.05,
        })
        .collect();
    let buy = run_agent(
        &tape,
        &AgentConfig {
            algo: Algo::Twap,
            side: ExecSide::Buy,
            parent_qty: 4_000,
            n_slices: 8,
            pov_rate: 0.2,
        },
    );
    let sell = run_agent(
        &tape,
        &AgentConfig {
            algo: Algo::Twap,
            side: ExecSide::Sell,
            parent_qty: 4_000,
            n_slices: 8,
            pov_rate: 0.2,
        },
    );
    assert!(
        buy.slippage_bps > 10.0,
        "rising tape should penalize buyers"
    );
    assert!(sell.slippage_bps < -10.0, "rising tape rewards sellers");
}
