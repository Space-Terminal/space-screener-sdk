use std::collections::{BTreeMap, HashMap};

use space_screener::MouseButton;
use space_screener::prelude::*;

/// Snapshots older than this are left out: their price no longer compares.
const STALE_MS: i64 = secs(60);

#[derive(Debug, Clone, PartialEq)]
struct Leg {
    exchange: String,
    /// The exchange's own symbol, for `open_market` and `open_spread`.
    symbol: String,
    last: f64,
}

#[derive(Debug, Clone, PartialEq)]
struct Gap {
    /// Canonical `BASEQUOTE`.
    pair: String,
    cheap: Leg,
    dear: Leg,
    spread_pct: f64,
    venues: Vec<String>,
}

/// KuCoin futures call bitcoin XBT; everyone else BTC.
fn canonical_base(base: &str) -> String {
    match base.to_ascii_uppercase().as_str() {
        "XBT" => "BTC".to_string(),
        other => other.to_string(),
    }
}

/// The cheapest and the dearest leg of every pair listed on two or more exchanges.
fn gaps(legs: BTreeMap<String, Vec<Leg>>, min_spread: f64, max_spread: f64) -> Vec<Gap> {
    legs.into_iter()
        .filter_map(|(pair, legs)| {
            if legs.len() < 2 {
                return None;
            }
            let cheap = legs.iter().min_by(|a, b| a.last.total_cmp(&b.last))?;
            let dear = legs.iter().max_by(|a, b| a.last.total_cmp(&b.last))?;
            let spread_pct = (dear.last - cheap.last) / cheap.last * 100.0;
            if spread_pct < min_spread || spread_pct > max_spread {
                return None;
            }
            let mut venues: Vec<String> = legs.iter().map(|l| l.exchange.clone()).collect();
            venues.sort();
            venues.dedup();
            Some(Gap {
                pair,
                cheap: cheap.clone(),
                dear: dear.clone(),
                spread_pct,
                venues,
            })
        })
        .collect()
}

/// What a click on a row opens. `open_market` goes to the book linked with the screener pane, like
/// a click in the built-in screener; `open_spread` always opens a new tab.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ClickTarget {
    /// Left click: the cheaper exchange, where to buy.
    Cheap,
    /// Right click: the dearer exchange, where to sell.
    Dear,
    /// Middle or Shift+left click: both legs as a spread.
    Spread,
}

fn click_target(click: &Click) -> ClickTarget {
    match click.button {
        MouseButton::Middle => ClickTarget::Spread,
        MouseButton::Left if click.modifiers.shift => ClickTarget::Spread,
        MouseButton::Left => ClickTarget::Cheap,
        MouseButton::Right => ClickTarget::Dear,
    }
}

/// The same pair across connected exchanges: the cheapest and the dearest last price.
#[derive(Default)]
struct CrossSpread {
    /// Row key → (cheap leg, dear leg), for clicks.
    legs: HashMap<String, (MarketRef, MarketRef)>,
}

impl Screener for CrossSpread {
    fn on_timer(&mut self, now_ms: i64) -> ScreenerResult {
        let params = params();
        let market = match params.str("market") {
            Some("spot") => Market::Spot,
            _ => Market::Futures,
        };
        let min_volume = params.f64_or("min_volume", 1_000_000.0);
        let min_spread = params.f64_or("min_spread", 0.1);
        let max_spread = params.f64_or("max_spread", 20.0);
        let limit = params.i64_or("limit", 200).max(1) as usize;

        let mut by_pair: BTreeMap<String, Vec<Leg>> = BTreeMap::new();
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
            if snapshot.ts_ms > 0 && now_ms - snapshot.ts_ms > STALE_MS {
                continue;
            }
            sources += 1;
            for t in snapshot.tickers {
                if t.last <= 0.0 || t.volume_quote < min_volume || t.quote.is_empty() {
                    continue;
                }
                let pair = format!(
                    "{}{}",
                    canonical_base(&t.base),
                    t.quote.to_ascii_uppercase()
                );
                by_pair.entry(pair).or_default().push(Leg {
                    exchange: ex.exchange.clone(),
                    symbol: t.symbol,
                    last: t.last,
                });
            }
        }

        let mut gaps = gaps(by_pair, min_spread, max_spread);
        gaps.sort_by(|a, b| b.spread_pct.total_cmp(&a.spread_pct));
        gaps.truncate(limit);
        self.legs = gaps
            .iter()
            .map(|g| {
                (
                    g.pair.clone(),
                    (
                        MarketRef::new(&g.cheap.exchange, market, &g.cheap.symbol),
                        MarketRef::new(&g.dear.exchange, market, &g.dear.symbol),
                    ),
                )
            })
            .collect();
        let rows = gaps.iter().map(|g| {
            Row::new(g.pair.clone())
                .cell("symbol", g.pair.as_str())
                .cell("spread", g.spread_pct)
                .cell("buy", g.cheap.exchange.as_str())
                .cell("buy_price", g.cheap.last)
                .cell("sell", g.dear.exchange.as_str())
                .cell("sell_price", g.dear.last)
                .cell(
                    "venues",
                    Cell::markets(
                        g.venues
                            .iter()
                            .map(|exchange| ExchangeMarket::new(exchange, market)),
                    ),
                )
        });
        if sources < 2 {
            set_status(
                StatusTone::Warn,
                format!("connect two or more {market} exchanges to compare prices"),
            )?;
        } else {
            set_status(
                StatusTone::Ok,
                format!("{sources} exchanges, {} pairs", gaps.len()),
            )?;
        }
        replace_rows(rows)?;
        Ok(())
    }

    fn on_click(&mut self, click: &Click) -> ScreenerResult {
        let Some((cheap, dear)) = self.legs.get(&click.row.key) else {
            return Ok(());
        };
        match click_target(click) {
            ClickTarget::Cheap => open_market(cheap)?,
            ClickTarget::Dear => open_market(dear)?,
            ClickTarget::Spread => open_spread(cheap, dear, None)?,
        }
        Ok(())
    }
}

export_screener!(CrossSpread, on_click);

#[cfg(test)]
mod tests {
    use space_screener::Modifiers;

    use super::*;

    #[test]
    fn click_opens_a_leg_and_shift_or_middle_the_spread() {
        let click = |button, shift| Click {
            button,
            modifiers: Modifiers {
                shift,
                ..Modifiers::default()
            },
            ..Click::default()
        };
        assert_eq!(
            click_target(&click(MouseButton::Left, false)),
            ClickTarget::Cheap
        );
        assert_eq!(
            click_target(&click(MouseButton::Right, false)),
            ClickTarget::Dear
        );
        assert_eq!(
            click_target(&click(MouseButton::Right, true)),
            ClickTarget::Dear
        );
        assert_eq!(
            click_target(&click(MouseButton::Middle, false)),
            ClickTarget::Spread
        );
        assert_eq!(
            click_target(&click(MouseButton::Left, true)),
            ClickTarget::Spread
        );
    }

    fn leg(exchange: &str, last: f64) -> Leg {
        Leg {
            exchange: exchange.to_string(),
            symbol: "X".to_string(),
            last,
        }
    }

    #[test]
    fn keeps_pairs_on_two_exchanges_within_the_band() {
        let mut legs = BTreeMap::new();
        legs.insert(
            "BTCUSDT".to_string(),
            vec![
                leg("okx", 100.5),
                leg("binance", 100.0),
                leg("bybit", 100.2),
            ],
        );
        legs.insert("SOLOUSDT".to_string(), vec![leg("gate", 1.0)]);
        legs.insert(
            "TWINUSDT".to_string(),
            vec![leg("mexc", 1.0), leg("bitget", 3.0)],
        );
        let gaps = gaps(legs, 0.1, 20.0);
        assert_eq!(gaps.len(), 1);
        let btc = &gaps[0];
        assert_eq!(btc.cheap.exchange, "binance");
        assert_eq!(btc.dear.exchange, "okx");
        assert!((btc.spread_pct - 0.5).abs() < 1e-9);
        assert_eq!(btc.venues, ["binance", "bybit", "okx"]);
    }

    #[test]
    fn xbt_is_bitcoin() {
        assert_eq!(canonical_base("xbt"), "BTC");
        assert_eq!(canonical_base("ETH"), "ETH");
    }
}
