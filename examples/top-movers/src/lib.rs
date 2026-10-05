use std::collections::HashMap;
use std::fmt::Write;

use space_screener::prelude::*;

const WINDOW_MS: i64 = mins(5);
const FORGET_AFTER_MS: i64 = mins(10);
/// History keeps a point per this interval, not per timer: tens of thousands of tickers must fit
/// the plugin memory. The 5 minute change is measured against the point at or just before
/// 5 minutes ago: with this interval and the terminal's snapshot age the window comes out up to
/// ~20 s longer.
const SAMPLE_MS: i64 = 15_000;

/// Biggest 5 minute price moves on connected exchanges, from the terminal's ticker snapshots.
///
/// Parameter changes keep the price history: `export_screener!` exports `on_params`, so the
/// terminal does not restart the plugin, and the series are keyed by market.
#[derive(Default)]
struct TopMovers {
    prices: HashMap<String, Series>,
    started_ms: Option<i64>,
    /// Market the warm-up countdown belongs to: switching the market starts it over, the series
    /// of the other market stay.
    market: Option<Market>,
}

/// One ticker of the round, before it becomes a row: rows are built only for the ones that make
/// the table.
struct Mover {
    rank: (bool, f64),
    exchange: usize,
    symbol: String,
    chg5: Option<f64>,
    chg24: f64,
    volume: f64,
    last: f64,
}

impl Screener for TopMovers {
    fn on_timer(&mut self, now_ms: i64) -> ScreenerResult {
        let params = params();
        let market = match params.str("market") {
            Some("spot") => Market::Spot,
            _ => Market::Futures,
        };
        let min_volume = params.f64_or("min_volume", 5_000_000.0);
        let limit = params.i64_or("limit", 100).max(1) as usize;
        if self.market != Some(market) {
            self.market = Some(market);
            self.started_ms = None;
        }
        let started_ms = *self.started_ms.get_or_insert(now_ms);

        let mut names = Vec::new();
        let mut movers = Vec::new();
        let mut key = String::new();
        for ex in exchanges()? {
            if !ex.connected || ex.market != market {
                continue;
            }
            let snapshot = match tickers(&ex.exchange, market) {
                Ok(snapshot) => snapshot,
                Err(e) => {
                    warn!("{} {market}: {e}", ex.exchange);
                    continue;
                }
            };
            let ts = if snapshot.ts_ms > 0 {
                snapshot.ts_ms
            } else {
                now_ms
            };
            for t in snapshot.tickers {
                // Only liquid tickers keep a history: spot lists reach tens of thousands of
                // symbols, and a series for each of them would not fit the plugin memory.
                if t.volume_quote < min_volume {
                    continue;
                }
                key.clear();
                let _ = write!(key, "{}:{market}:{}", ex.exchange, t.symbol);
                if !self.prices.contains_key(key.as_str()) {
                    self.prices
                        .insert(key.clone(), Series::new(WINDOW_MS + mins(1)));
                }
                let Some(series) = self.prices.get_mut(key.as_str()) else {
                    continue;
                };
                let past = series.value_at(ts - WINDOW_MS);
                if series.last_ts().is_none_or(|last| ts - last >= SAMPLE_MS) {
                    series.push(ts, t.last);
                }
                let chg5 = past
                    .filter(|past| *past != 0.0)
                    .map(|past| (t.last - past) / past * 100.0);
                movers.push(Mover {
                    rank: rank(chg5, t.change_pct),
                    exchange: names.len(),
                    symbol: t.symbol,
                    chg5,
                    chg24: t.change_pct,
                    volume: t.volume_quote,
                    last: t.last,
                });
            }
            names.push(ex.exchange);
        }
        self.prices
            .retain(|_, s| s.last_ts().is_some_and(|t| now_ms - t < FORGET_AFTER_MS));

        let sources = names.len();
        let warming_ms = WINDOW_MS - (now_ms - started_ms);
        if sources == 0 {
            set_status(StatusTone::Warn, format!("no connected {market} exchanges"))?;
        } else if warming_ms > 0 {
            set_status(
                StatusTone::Neutral,
                format!(
                    "collecting history: {}s left, sorted by 24h change meanwhile",
                    warming_ms / 1000
                ),
            )?;
        } else {
            set_status(StatusTone::Ok, format!("{sources} exchanges"))?;
        }
        let rows = top(movers, limit).into_iter().filter_map(|(m, row_rank)| {
            let exchange = names.get(m.exchange)?;
            Some(
                Row::new(format!("{exchange}:{market}:{}", m.symbol))
                    .market_ref(&MarketRef::new(exchange, market, &m.symbol))
                    .cell("symbol", m.symbol.as_str())
                    .cell("exchange", exchange.as_str())
                    .cell("chg5", m.chg5.map(Cell::signed))
                    .cell("chg24", Cell::signed(m.chg24))
                    .cell("volume", m.volume)
                    .cell("last", m.last)
                    .rank(row_rank),
            )
        });
        replace_rows(rows)?;
        Ok(())
    }
}

/// Sort key: tickers with a 5 minute change rank by it; the rest — still collecting history,
/// right after a start or a new listing — follow by the 24h change. Without the fallback every
/// ticker would tie while warming up and the first exchanges in `exchanges()` order would fill
/// the whole table.
fn rank(chg5: Option<f64>, chg24: f64) -> (bool, f64) {
    match chg5 {
        Some(chg5) => (true, chg5.abs()),
        None => (false, chg24.abs()),
    }
}

/// Biggest first.
fn by_rank(a: &(bool, f64), b: &(bool, f64)) -> std::cmp::Ordering {
    b.0.cmp(&a.0).then_with(|| b.1.total_cmp(&a.1))
}

/// The table: the best `limit` movers, biggest first, each with its row rank. The terminal orders
/// rows by rank before the sort column, and the default sort column (5m) is empty while warming
/// up, so tickers without a 5 minute change get ranks -1, -2, … in 24h order below the others
/// (rank 0), which the 5m column then sorts.
fn top(mut movers: Vec<Mover>, limit: usize) -> Vec<(Mover, i32)> {
    if movers.len() > limit {
        movers.select_nth_unstable_by(limit - 1, |a, b| by_rank(&a.rank, &b.rank));
        movers.truncate(limit);
    }
    movers.sort_unstable_by(|a, b| by_rank(&a.rank, &b.rank));
    let mut warming = 0;
    movers
        .into_iter()
        .map(|m| {
            let row_rank = if m.chg5.is_some() {
                0
            } else {
                warming -= 1;
                warming
            };
            (m, row_rank)
        })
        .collect()
}

export_screener!(TopMovers);

#[cfg(test)]
mod tests {
    use super::*;

    fn mover(exchange: usize, symbol: &str, chg5: Option<f64>, chg24: f64) -> Mover {
        Mover {
            rank: rank(chg5, chg24),
            exchange,
            symbol: symbol.to_owned(),
            chg5,
            chg24,
            volume: 1.0,
            last: 1.0,
        }
    }

    #[test]
    fn warming_tickers_follow_the_5_minute_ones_with_ranks_in_24h_order() {
        let movers = vec![
            mover(0, "ASTER_FLAT", None, 0.5),
            mover(1, "GATE_HOT", None, -12.0),
            mover(2, "BYBIT_5M_SMALL", Some(0.2), 30.0),
            mover(3, "OKX_5M_BIG", Some(-3.0), 1.0),
            mover(4, "MEXC_WARM", None, 4.0),
        ];
        let table: Vec<_> = top(movers, 4)
            .into_iter()
            .map(|(m, rank)| (m.symbol, rank))
            .collect();
        assert_eq!(
            table,
            [
                ("OKX_5M_BIG".to_owned(), 0),
                ("BYBIT_5M_SMALL".to_owned(), 0),
                ("GATE_HOT".to_owned(), -1),
                ("MEXC_WARM".to_owned(), -2),
            ]
        );
    }
}
