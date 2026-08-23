//! Micro-benchmark: order submissions + cancels per second on one book.

use lob_engine::{Book, Lcg, Order, Side};
use std::time::Instant;

fn main() {
    let n: u64 = 1_000_000;
    let mut rng = Lcg::new(7);
    let mut book = Book::new();
    let mut oid = 0u64;
    let live: Vec<u64> = Vec::new();
    let mut _live = live;

    let start = Instant::now();
    for i in 0..n {
        oid += 1;
        let side = if rng.below(2) == 0 { Side::Bid } else { Side::Ask };
        let price = 1000 + rng.below(200);
        book.submit(Order { id: oid, side, price, qty: 10 });
        if i % 3 == 0 && oid > 5 {
            let k = (rng.below(_live.len().max(1) as u64)) as usize;
            if !_live.is_empty() {
                let id = _live.swap_remove(k);
                let _ = book.cancel(id);
            }
        } else {
            _live.push(oid);
        }
    }
    let dt = start.elapsed();
    println!(
        "{} ops in {:?} -> {:.0} ops/sec | final open orders: {}",
        n,
        dt,
        n as f64 / dt.as_secs_f64(),
        book.open_orders()
    );
}
