//! Execution algorithms over a historical trade tape.
//!
//! Agents produce child-order schedules (TWAP / VWAP / POV); fills are obtained
//! by walking forward through subsequent tape volume at encountered prices.
//! Slippage is measured against the arrival price of each parent order.

#[derive(Clone, Copy, Debug)]
pub struct TapeTick {
    pub qty: u64,
    pub price: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Algo {
    Twap,
    Vwap,
    Pov,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExecSide {
    Buy,
    Sell,
}

#[derive(Clone, Copy, Debug)]
pub struct AgentConfig {
    pub algo: Algo,
    pub side: ExecSide,
    pub parent_qty: u64,
    pub n_slices: usize,
    /// POV participation rate, (0, 1]; ignored by other algos.
    pub pov_rate: f64,
}

/// Child orders as (start_index_in_tape, qty). Execution of each child walks
/// forward through tape volume from its start index until filled.
pub fn schedule(tape: &[TapeTick], cfg: &AgentConfig) -> Vec<(usize, u64)> {
    let n = tape.len().max(1);
    let mut out = Vec::new();
    match cfg.algo {
        Algo::Twap => {
            let k = cfg.n_slices.max(1);
            let base = cfg.parent_qty / k as u64;
            let rem = cfg.parent_qty % k as u64;
            for i in 0..k {
                let idx = i * n / k;
                let extra = u64::from((i as u64) < rem);
                out.push((idx, base + extra));
            }
        }
        Algo::Vwap => {
            // weight slices by realized volume share of their bucket (+1 to avoid 0)
            let k = cfg.n_slices.max(1);
            let w: Vec<f64> = (0..k)
                .map(|i| {
                    let lo = i * n / k;
                    let hi = ((i + 1) * n / k).max(lo + 1);
                    tape[lo.min(n)..hi.min(n)]
                        .iter()
                        .map(|t| t.qty as f64)
                        .sum::<f64>()
                        + 1.0
                })
                .collect();
            let wsum: f64 = w.iter().sum();
            let mut assigned = 0u64;
            for i in 0..k {
                let idx = i * n / k;
                if i == k - 1 {
                    out.push((idx, cfg.parent_qty - assigned));
                } else {
                    let q = ((w[i] / wsum) * cfg.parent_qty as f64).round() as u64;
                    assigned += q;
                    out.push((idx, q));
                }
            }
        }
        Algo::Pov => {
            let total: f64 = tape.iter().map(|t| t.qty as f64).sum();
            let per_slice = ((total * cfg.pov_rate) as u64)
                .min(cfg.parent_qty)
                .div_euclid(cfg.n_slices.max(1) as u64)
                .max(1);
            let stride = (n / cfg.n_slices.max(1)).max(1);
            let mut remaining = cfg.parent_qty;
            let mut i = 0usize;
            while remaining > 0 && i < n {
                let q = per_slice.min(remaining);
                out.push((i, q));
                remaining -= q;
                i += stride;
            }
        }
    }
    out.retain(|(_, q)| *q > 0);
    out
}

/// Execute one child order by consuming forward tape volume.
/// Returns (filled_qty, vwap_price). Market impact is not modeled; the tape
/// moves on its own and we take liquidity passively at traded prices.
fn fill_child(tape: &[TapeTick], start: usize, mut qty: u64, _side: ExecSide) -> (u64, f64) {
    let mut filled = 0u64;
    let mut notional = 0.0;
    for t in &tape[start..] {
        if qty == 0 {
            break;
        }
        let take = t.qty.min(qty);
        notional += take as f64 * t.price;
        filled += take;
        qty -= take;
    }
    let px = if filled > 0 {
        notional / filled as f64
    } else {
        0.0
    };
    (filled, px)
}

pub struct FillStats {
    pub filled_qty: u64,
    /// bps vs arrival mid at schedule time; positive = worse than arrival
    pub slippage_bps: f64,
}

pub fn run_agent(tape: &[TapeTick], cfg: &AgentConfig) -> FillStats {
    let children = schedule(tape, cfg);
    let arrival = tape
        .first()
        .map(|t| t.price)
        .unwrap_or_else(|| panic!("empty tape"));
    let mut filled_total = 0u64;
    let mut notional = 0.0;
    for (idx, qty) in children {
        let (f, px) = fill_child(tape, idx, qty, cfg.side);
        filled_total += f;
        notional += f as f64 * px;
    }
    let avg_px = if filled_total > 0 {
        notional / filled_total as f64
    } else {
        arrival
    };
    // buys pay more than arrival -> positive slippage; sells the mirror
    let dir = if cfg.side == ExecSide::Buy { 1.0 } else { -1.0 };
    FillStats {
        filled_qty: filled_total,
        slippage_bps: dir * (avg_px / arrival - 1.0) * 10_000.0,
    }
}
