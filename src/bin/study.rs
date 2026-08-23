//! Slippage study: TWAP vs VWAP vs POV over a synthetic GBM tape.
//! Swap `load_tape_csv` input for real recorded trades to rerun on history.

use lob_engine::{run_agent, AgentConfig, Algo, ExecSide, TapeTick};

fn main() {
    let n = 20_000usize;
    // synthetic GBM-ish tape with intraday U-shaped volume
    let mut price = 100.0;
    let mut seed = 12345u64;
    let tape: Vec<TapeTick> = (0..n)
        .map(|i| {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
            let r = ((seed >> 32) as f64 / 4294967296.0) - 0.5; // uniform [-0.5, 0.5)
            price *= 1.0 + r * 0.0008;
            let tod = (i as f64 / n as f64 * std::f64::consts::PI).sin();
            TapeTick {
                qty: (50.0 + 450.0 * (tod * 2.0).abs()) as u64,
                price,
            }
        })
        .collect();

    println!("parent: buy 50_000 over {} ticks\n", n);
    println!("{:<6} {:>12} {:>14}", "algo", "filled", "slippage_bps");
    for (algo, pov) in [
        (Algo::Twap, 0.2),
        (Algo::Vwap, 0.2),
        (Algo::Pov, 0.1),
        (Algo::Pov, 0.3),
    ] {
        let cfg = AgentConfig {
            algo,
            side: ExecSide::Buy,
            parent_qty: 50_000,
            n_slices: 40,
            pov_rate: pov,
        };
        let s = run_agent(&tape, &cfg);
        let label = if algo == Algo::Pov {
            format!("{:?}-{}", algo, pov)
        } else {
            format!("{algo:?}")
        };
        println!("{:<6} {:>12} {:>14.2}", label, s.filled_qty, s.slippage_bps);
    }
}
