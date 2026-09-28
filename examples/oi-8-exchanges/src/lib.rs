use std::collections::HashMap;

use space_screener::oi::{Collector, EXCHANGES, OpenInterest};
use space_screener::prelude::*;

const HORIZON_MS: i64 = mins(16);
const FORGET_AFTER_MS: i64 = mins(30);
const ALERT_COOLDOWN_MS: i64 = mins(15);

/// One-side open interest of USDT perpetuals on 8 exchanges and its 5/15 minute change,
/// computed from the screener's own snapshots (one per `timer_ms`).
#[derive(Default)]
struct OiScreener {
    collector: Collector,
    history: HashMap<String, Series>,
    alerted_at: HashMap<String, i64>,
}

impl OiScreener {
    fn maybe_alert(&mut self, key: &str, oi: &OpenInterest, chg5: f64, now_ms: i64) {
        if self
            .alerted_at
            .get(key)
            .is_some_and(|&t| now_ms - t < ALERT_COOLDOWN_MS)
        {
            return;
        }
        self.alerted_at.insert(key.to_string(), now_ms);
        let title = format!("OI {chg5:+.1}% · {} {}", oi.symbol, oi.exchange);
        let body = format!("Open interest {:.1}M$ after 5 minutes", oi.oi_usd / 1e6);
        if let Err(e) = alert_row(AlertLevel::Warn, title, body, key) {
            warn!("alert failed: {e}");
        }
    }
}

impl Screener for OiScreener {
    fn on_timer(&mut self, now_ms: i64) -> ScreenerResult {
        let params = params();
        let min_oi = params.f64_or("min_oi", 5_000_000.0);
        let limit = params.i64_or("limit", 500).max(1) as usize;
        let alert_pct = params.f64_or("alert_pct", 0.0);

        let mut failed = Vec::new();
        let mut rows = Vec::new();
        for snapshot in self.collector.collect(now_ms)? {
            let items = match snapshot.result {
                Ok(items) => items,
                Err(e) => {
                    warn!("{}: {e}", snapshot.exchange);
                    failed.push(snapshot.exchange);
                    continue;
                }
            };
            for oi in items {
                let key = format!("{}:{}", oi.exchange, oi.symbol);
                let series = self
                    .history
                    .entry(key.clone())
                    .or_insert_with(|| Series::new(HORIZON_MS));
                series.push(now_ms, oi.oi_usd);
                let chg5 = series.change_pct(now_ms, mins(5));
                let chg15 = series.change_pct(now_ms, mins(15));
                if oi.oi_usd < min_oi {
                    continue;
                }
                if alert_pct > 0.0
                    && let Some(chg) = chg5
                    && chg.abs() >= alert_pct
                {
                    self.maybe_alert(&key, &oi, chg, now_ms);
                }
                let row = Row::new(key)
                    .market_ref(&oi.market_ref())
                    .cell("symbol", oi.symbol.as_str())
                    .cell("exchange", oi.exchange)
                    .cell("oi", oi.oi_usd)
                    .cell("chg5", chg5.map(Cell::signed))
                    .cell("chg15", chg15.map(Cell::signed))
                    .cell("price", oi.price);
                rows.push((oi.oi_usd, row));
            }
        }
        self.history
            .retain(|_, s| s.last_ts().is_some_and(|t| now_ms - t < FORGET_AFTER_MS));
        self.alerted_at
            .retain(|_, t| now_ms - *t < ALERT_COOLDOWN_MS);

        rows.sort_by(|a, b| b.0.total_cmp(&a.0));
        rows.truncate(limit);
        let ok = EXCHANGES.len() - failed.len();
        if failed.is_empty() {
            set_status(
                StatusTone::Ok,
                format!("{ok}/8 exchanges · {} rows", rows.len()),
            )?;
        } else {
            set_status(
                StatusTone::Warn,
                format!("{ok}/8 exchanges · no data: {}", failed.join(", ")),
            )?;
        }
        replace_rows(rows.into_iter().map(|(_, row)| row))?;
        Ok(())
    }
}

export_screener!(OiScreener);
