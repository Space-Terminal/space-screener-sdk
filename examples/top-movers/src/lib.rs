use std::collections::HashMap;

use space_screener::prelude::*;

const WINDOW_MS: i64 = mins(5);
const FORGET_AFTER_MS: i64 = mins(10);

/// Biggest 5 minute price moves on connected exchanges, from the terminal's ticker snapshots.
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
                let key = format!("{}:{market}:{}", ex.exchange, t.symbol);
                let series = self
                    .prices
                    .entry(key.clone())
                    .or_insert_with(|| Series::new(WINDOW_MS + mins(1)));
                series.push(ts, t.last);
                if t.volume_quote < min_volume {
                    continue;
                }
                let chg5 = series.change_pct(ts, WINDOW_MS);
                let row = Row::new(key)
                    .market_ref(&MarketRef::new(&ex.exchange, market, &t.symbol))
                    .cell("symbol", t.symbol.as_str())
                    .cell("exchange", ex.exchange.as_str())
                    .cell("chg5", chg5.map(Cell::signed))
                    .cell("chg24", Cell::signed(t.change_pct))
                    .cell("volume", t.volume_quote)
                    .cell("last", t.last);
                movers.push((chg5.map_or(-1.0, f64::abs), row));
            }
        }
        self.prices
            .retain(|_, s| s.last_ts().is_some_and(|t| now_ms - t < FORGET_AFTER_MS));

        movers.sort_by(|a, b| b.0.total_cmp(&a.0));
        movers.truncate(limit);
        let warming_ms = WINDOW_MS - (now_ms - started_ms);
        if sources == 0 {
            set_status(StatusTone::Warn, format!("no connected {market} exchanges"))?;
        } else if warming_ms > 0 {
            set_status(
                StatusTone::Neutral,
                format!("collecting history: {}s left", warming_ms / 1000),
            )?;
        } else {
            set_status(StatusTone::Ok, format!("{sources} exchanges"))?;
        }
        replace_rows(movers.into_iter().map(|(_, row)| row))?;
        Ok(())
    }
}

export_screener!(TopMovers);
