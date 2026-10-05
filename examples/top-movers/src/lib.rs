use std::collections::HashMap;

use space_screener::prelude::*;

const WINDOW_MS: i64 = mins(5);
const FORGET_AFTER_MS: i64 = mins(10);
/// History keeps a point per this interval, not per timer: tens of thousands of tickers must fit
/// the plugin memory. The 5 minute change is measured against the point at or just before
/// 5 minutes ago, so the window is up to this much longer.
const SAMPLE_MS: i64 = 15_000;

/// Biggest 5 minute price moves on connected exchanges, from the terminal's ticker snapshots.
///
/// Parameter changes keep the price history: `export_screener!` exports `on_params`, so the
/// terminal does not restart the plugin, and the series are keyed by market.
#[derive(Default)]
struct TopMovers {
    prices: HashMap<String, Series>,
    started_ms: Option<i64>,
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
        let started_ms = *self.started_ms.get_or_insert(now_ms);

        let mut movers = Vec::new();
        let mut sources = 0;
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
            sources += 1;
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
                let key = format!("{}:{market}:{}", ex.exchange, t.symbol);
                if !self.prices.contains_key(&key) {
                    self.prices
                        .insert(key.clone(), Series::new(WINDOW_MS + mins(1)));
                }
                let Some(series) = self.prices.get_mut(&key) else {
                    continue;
                };
                let past = series.value_at(ts - WINDOW_MS);
                if series.last_ts().is_none_or(|last| ts - last >= SAMPLE_MS) {
                    series.push(ts, t.last);
                }
                let chg5 = past
                    .filter(|past| *past != 0.0)
                    .map(|past| (t.last - past) / past * 100.0);
                let row = Row::new(key)
                    .market_ref(&MarketRef::new(&ex.exchange, market, &t.symbol))
                    .cell("symbol", t.symbol.as_str())
                    .cell("exchange", ex.exchange.as_str())
                    .cell("chg5", chg5.map(Cell::signed))
                    .cell("chg24", Cell::signed(t.change_pct))
                    .cell("volume", t.volume_quote)
                    .cell("last", t.last);
                movers.push((rank(chg5, t.change_pct), row));
            }
        }
        self.prices
            .retain(|_, s| s.last_ts().is_some_and(|t| now_ms - t < FORGET_AFTER_MS));

        movers.sort_unstable_by(|a, b| by_rank(&a.0, &b.0));
        movers.truncate(limit);
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
        replace_rows(movers.into_iter().map(|(_, row)| row))?;
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

export_screener!(TopMovers);

#[cfg(test)]
mod tests {
    use super::*;

    fn order(ranks: Vec<(&'static str, (bool, f64))>) -> Vec<&'static str> {
        let mut ranks = ranks;
        ranks.sort_unstable_by(|a, b| by_rank(&a.1, &b.1));
        ranks.into_iter().map(|(name, _)| name).collect()
    }

    #[test]
    fn warming_tickers_rank_by_the_24h_change_after_the_5_minute_ones() {
        let ranked = order(vec![
            ("aster_flat", rank(None, 0.5)),
            ("gate_hot", rank(None, -12.0)),
            ("bybit_5m_small", rank(Some(0.2), 30.0)),
            ("okx_5m_big", rank(Some(-3.0), 1.0)),
        ]);
        assert_eq!(
            ranked,
            ["okx_5m_big", "bybit_5m_small", "gate_hot", "aster_flat"]
        );
    }
}
