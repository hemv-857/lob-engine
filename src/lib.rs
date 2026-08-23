mod agents;
mod orderbook;

pub use agents::{run_agent, schedule, AgentConfig, Algo, ExecSide, FillStats, TapeTick};
pub use orderbook::{Book, CancelResult, Event, Order, Side, Trade};

/// Deterministic 64-bit LCG for property-style tests (no external crates).
pub struct Lcg(u64);

impl Lcg {
    pub fn new(seed: u64) -> Self {
        Lcg(seed | 1)
    }
    pub fn next_u64(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        self.0
    }
    pub fn below(&mut self, n: u64) -> u64 {
        self.next_u64() % n.max(1)
    }
}
