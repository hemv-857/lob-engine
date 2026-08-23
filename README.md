# lob-engine

Price-time priority **limit order book** in Rust (std-only, zero dependencies)
plus an **execution-algorithm study** harness (TWAP / VWAP / POV) over
historical trade tapes.

```
submit(order) -> Vec<Event>          // event-sourced matching: Trades + BookUpdates
cancel(id)    -> CancelResult
BTreeMap<price_ticks, VecDeque<Order>>   // FIFO inside level, O(log L) level ops,
                                         // O(1) cached best bid/ask via map ends
```

## Quickstart

```bash
cargo test --release        # 9 tests incl. determinism + op-storm invariants
cargo run --release --bin bench   # ~2.5M ops/sec submit+match+cancel on this laptop
cargo run --release --bin study   # TWAP vs VWAP vs POV slippage over a synthetic tape
```

## Correctness gates (enforced in tests)

- Crossing fills at the **maker's** price; FIFO within a level; better prices jump queues
- Partial fill preserves queue position of the resting order
- Double-cancel returns `NotFound`; cancels remove exactly the target order
- **Determinism**: same input stream -> identical trade count and final book state
- Op-storm invariants: book never crosses while both sides populated; open-order
  count consistent with cancel accounting
- Agent schedules allocate exactly the parent quantity (TWAP/VWAP)

## Architecture note

The engine and the execution study are deliberately decoupled: agents consume a
trade tape and walk it forward for fills, so the same harness replays real
recorded trades (`TapeTick` from CSV) without a full LOB feedback loop.
`ponytail:` no market-impact model — add one only when measuring against your
own live fills shows you need it.

## Honest scope

- Single instrument, single book; port/fee tiers are out of scope.
- Integer tick prices only (floats are never compared).
- Bench is wall-clock on one core; CI runs tests only, benches locally.
