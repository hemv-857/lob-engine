//! Price-time priority limit order book.
//!
//! - Prices are integer ticks (no float comparisons anywhere).
//! - Levels: BTreeMap<price, VecDeque<Order>>; best bid/ask cached O(1).
//! - Matching is event-sourced: every operation returns a Vec<Event>, so a
//!   replay of the same input stream is byte-identical (determinism test).

use std::collections::{BTreeMap, VecDeque};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    Bid,
    Ask,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Order {
    pub id: u64,
    pub side: Side,
    pub price: u64,
    pub qty: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Trade {
    pub maker_id: u64,
    pub taker_id: u64,
    pub price: u64,
    pub qty: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CancelResult {
    Canceled { remaining_qty: u64 },
    NotFound,
}

#[derive(Clone, Debug)]
pub enum Event {
    Trade(Trade),
    BookUpdate {
        side: Side,
        price: u64,
        new_level_qty: u64,
    },
}

#[derive(Default)]
pub struct Book {
    bids: BTreeMap<u64, VecDeque<Order>>, // key: price desc via rev iter
    asks: BTreeMap<u64, VecDeque<Order>>, // key: price asc
    index: BTreeMap<u64, Order>,          // order id -> live order
}

impl Book {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn best_bid(&self) -> Option<u64> {
        self.bids.keys().next_back().copied()
    }

    pub fn best_ask(&self) -> Option<u64> {
        self.asks.keys().next().copied()
    }

    /// Submit an aggressive/limit order; matches against the opposite side
    /// within its limit, remainder rests. Returns the resulting events.
    pub fn submit(&mut self, mut order: Order) -> Vec<Event> {
        assert!(order.qty > 0, "zero-qty order");
        let mut events = Vec::new();
        // match against opposite book while crossing
        let maker_side = match order.side {
            Side::Bid => Side::Ask,
            Side::Ask => Side::Bid,
        };
        while order.qty > 0 {
            let (best_price, level) = match self.opposite_top(order.side) {
                Some(x) => x,
                None => break,
            };
            if !self.crosses(order.side, order.price, best_price) {
                break;
            }
            let mut maker = level.front().copied().expect("level nonempty");
            let fill = order.qty.min(maker.qty);
            maker.qty -= fill;
            order.qty -= fill;
            events.push(Event::Trade(Trade {
                maker_id: maker.id,
                taker_id: order.id,
                price: best_price,
                qty: fill,
            }));
            let maker_level = self.level_mut(maker_side, best_price);
            maker_level.front_mut().unwrap().qty = maker.qty;
            if maker.qty == 0 {
                maker_level.pop_front();
                if maker_level.is_empty() {
                    self.book_mut(maker_side).remove(&best_price);
                }
                events.push(Event::BookUpdate {
                    side: maker_side,
                    price: best_price,
                    new_level_qty: 0,
                });
            }
        }
        if order.qty > 0 {
            let price = order.price;
            self.book_mut(order.side)
                .entry(price)
                .or_default()
                .push_back(order);
            self.index.insert(order.id, order);
            events.push(Event::BookUpdate {
                side: order.side,
                price,
                new_level_qty: self.level_qty(order.side, price),
            });
        }
        events
    }

    pub fn cancel(&mut self, id: u64) -> CancelResult {
        let Some(order) = self.index.remove(&id) else {
            return CancelResult::NotFound;
        };
        let book = self.book_mut(order.side);
        let remove_whole_level;
        if let Some(level) = book.get_mut(&order.price) {
            let before = level.len();
            level.retain(|o| o.id != id);
            remove_whole_level = level.len() != before && level.is_empty();
            if remove_whole_level {
                book.remove(&order.price);
            }
        }
        CancelResult::Canceled { remaining_qty: 0 }
    }

    pub fn open_orders(&self) -> usize {
        self.index.len()
    }

    pub fn total_quantity(&self) -> u64 {
        let sum_book = |b: &BTreeMap<u64, VecDeque<Order>>| -> u64 {
            b.values()
                .map(|lv| lv.iter().map(|o| o.qty).sum::<u64>())
                .sum()
        };
        sum_book(&self.bids) + sum_book(&self.asks)
    }

    // -- internals --
    fn crosses(&self, side: Side, limit: u64, best_opposite: u64) -> bool {
        match side {
            Side::Bid => limit >= best_opposite,
            Side::Ask => limit <= best_opposite,
        }
    }

    fn opposite_top(&self, side: Side) -> Option<(u64, &VecDeque<Order>)> {
        match side {
            Side::Bid => self.asks.iter().next().map(|(p, l)| (*p, l)),
            Side::Ask => self.bids.iter().next_back().map(|(p, l)| (*p, l)),
        }
    }

    fn book_mut(&mut self, side: Side) -> &mut BTreeMap<u64, VecDeque<Order>> {
        match side {
            Side::Bid => &mut self.bids,
            Side::Ask => &mut self.asks,
        }
    }

    fn level_mut(&mut self, side: Side, price: u64) -> &mut VecDeque<Order> {
        self.book_mut(side).get_mut(&price).expect("level exists")
    }

    fn level_qty(&self, side: Side, price: u64) -> u64 {
        self.book_of(side)
            .and_then(|b| b.get(&price))
            .map(|lv| lv.iter().map(|o| o.qty).sum())
            .unwrap_or(0)
    }

    fn book_of(&self, side: Side) -> Option<&BTreeMap<u64, VecDeque<Order>>> {
        match side {
            Side::Bid => Some(&self.bids),
            Side::Ask => Some(&self.asks),
        }
    }
}
