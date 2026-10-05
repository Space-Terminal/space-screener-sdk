use std::collections::BTreeSet;

use space_screener::prelude::*;
use space_screener::{BookSide, DensitySignal, DensityStatus};

const FAVORITES: &str = "favorites";
const BLACKLIST: &str = "blacklist";
/// Rows above this with a full snapshot (~20000 levels) do not fit the plugin's memory.
const ROWS_MAX: i64 = 5000;

/// Comma- or space-separated exchange slugs, lower case.
fn slugs(text: Option<&str>) -> Vec<String> {
    text.unwrap_or_default()
        .split([',', ';', ' '])
        .map(|s| s.trim().to_ascii_lowercase())
        .filter(|s| !s.is_empty())
        .collect()
}

/// Exchange, symbol and market of a level: the blacklist hides an instrument, not one price.
fn instrument(signal: &Signal, density: &DensitySignal) -> String {
    format!("{}:{}:{}", density.exchange, density.market, signal.symbol)
}

#[derive(Debug, Clone, PartialEq)]
struct Filter {
    search: String,
    include: Vec<String>,
    exclude: Vec<String>,
    market: Option<Market>,
    side: Option<BookSide>,
    hide_dead: bool,
    min_current: f64,
    min_initial: f64,
    /// 0 is "no limit".
    max_eaten: f64,
    /// Of the distance's absolute value; 0 is "no limit".
    max_distance: f64,
    min_touches: u32,
    min_lifetime_s: u64,
    /// current / average ≥ this, as in the terminal's own screener: the default 1 keeps
    /// levels at least as large as the book's average; 0 is off, levels without an average pass.
    avg_multiplier: f64,
}

impl Filter {
    fn from_params(params: &Params) -> Self {
        Self {
            search: params
                .str("search")
                .unwrap_or_default()
                .trim()
                .to_ascii_uppercase(),
            include: slugs(params.str("exchanges")),
            exclude: slugs(params.str("exclude_exchanges")),
            market: match params.str("market") {
                Some("spot") => Some(Market::Spot),
                Some("futures") => Some(Market::Futures),
                _ => None,
            },
            side: match params.str("side") {
                Some("bid") => Some(BookSide::Bid),
                Some("ask") => Some(BookSide::Ask),
                _ => None,
            },
            hide_dead: params.bool_or("hide_dead", false),
            min_current: params.f64_or("min_current", 0.0),
            min_initial: params.f64_or("min_initial", 0.0),
            max_eaten: params.f64_or("max_eaten", 0.0),
            max_distance: params.f64_or("max_distance", 0.0),
            min_touches: params.i64_or("min_touches", 0).max(0) as u32,
            min_lifetime_s: params.i64_or("min_lifetime_s", 0).max(0) as u64,
            avg_multiplier: params.f64_or("avg_multiplier", 1.0),
        }
    }

    fn passes(&self, signal: &Signal, d: &DensitySignal) -> bool {
        let avg_ok = match d.notional_avg_usd {
            Some(avg) if self.avg_multiplier > 0.0 => {
                avg > 0.0 && d.notional_current_usd / avg >= self.avg_multiplier
            }
            _ => true,
        };
        (self.search.is_empty() || signal.symbol.contains(&self.search))
            && self.market.is_none_or(|m| m == d.market)
            && (self.include.is_empty() || self.include.contains(&d.exchange))
            && !self.exclude.contains(&d.exchange)
            && self.side.is_none_or(|s| s == d.side)
            && !(self.hide_dead && d.status == DensityStatus::Dead)
            && d.notional_current_usd >= self.min_current
            && d.notional_initial_usd >= self.min_initial
            && (self.max_eaten <= 0.0 || d.eaten_pct <= self.max_eaten)
            && (self.max_distance <= 0.0 || d.distance_pct.abs() <= self.max_distance)
            && d.touch_count >= self.min_touches
            && d.lifetime_s >= self.min_lifetime_s
            && avg_ok
    }
}

/// Favourite levels (on top, by signal id) and blacklisted instruments (at the bottom or
/// hidden); kept in `kv`.
#[derive(Debug, Clone, Default)]
struct Curation {
    favorites: BTreeSet<String>,
    blacklist: BTreeSet<String>,
}

impl Curation {
    fn load() -> Self {
        let set = |key| {
            kv_get::<BTreeSet<String>>(key)
                .ok()
                .flatten()
                .unwrap_or_default()
        };
        Self {
            favorites: set(FAVORITES),
            blacklist: set(BLACKLIST),
        }
    }

    fn rank(&self, id: &str, instrument: &str) -> i32 {
        if self.favorites.contains(id) {
            1
        } else if self.blacklist.contains(instrument) {
            -1
        } else {
            0
        }
    }

    fn toggle_favorite(&mut self, id: &str) -> Result<(), space_screener::Error> {
        if !self.favorites.remove(id) {
            self.favorites.insert(id.to_string());
        }
        kv_set(FAVORITES, &self.favorites)
    }

    fn toggle_blacklist(&mut self, instrument: &str) -> Result<(), space_screener::Error> {
        if !self.blacklist.remove(instrument) {
            self.blacklist.insert(instrument.to_string());
        }
        kv_set(BLACKLIST, &self.blacklist)
    }

    /// Favourites of levels that are gone would pile up in `kv`.
    fn forget_favorites_except(
        &mut self,
        alive: &BTreeSet<&str>,
    ) -> Result<(), space_screener::Error> {
        let before = self.favorites.len();
        self.favorites.retain(|id| alive.contains(id.as_str()));
        if self.favorites.len() == before {
            return Ok(());
        }
        kv_set(FAVORITES, &self.favorites)
    }
}

fn shown_symbol(symbol: &str, hide_usdt: bool) -> Cell {
    match symbol.strip_suffix("USDT") {
        Some(base) if hide_usdt && !base.is_empty() => Cell::new(symbol).text(base),
        _ => Cell::new(symbol),
    }
}

fn side_cell(side: BookSide, lang: Lang) -> Cell {
    match (side, lang) {
        (BookSide::Bid, Lang::Ru) => Cell::new("бид").tone(Tone::Pos),
        (BookSide::Bid, Lang::En) => Cell::new("bid").tone(Tone::Pos),
        (BookSide::Ask, Lang::Ru) => Cell::new("аск").tone(Tone::Neg),
        (BookSide::Ask, Lang::En) => Cell::new("ask").tone(Tone::Neg),
    }
}

fn status_cell(status: DensityStatus, lang: Lang) -> Cell {
    match (status, lang) {
        (DensityStatus::Alive, Lang::Ru) => Cell::new("живой").tone(Tone::Pos),
        (DensityStatus::Alive, Lang::En) => Cell::new("alive").tone(Tone::Pos),
        (DensityStatus::Reduced, Lang::Ru) => Cell::new("уменьш."),
        (DensityStatus::Reduced, Lang::En) => Cell::new("reduced"),
        (DensityStatus::Dead, Lang::Ru) => Cell::muted("мёртвый"),
        (DensityStatus::Dead, Lang::En) => Cell::muted("dead"),
        (DensityStatus::Other, _) => Cell::muted("—"),
    }
}

fn favorite_cell(on: bool) -> Cell {
    if on {
        Cell::new("★").tone(Tone::Accent)
    } else {
        Cell::muted("☆")
    }
}

fn blacklist_cell(on: bool) -> Cell {
    if on {
        Cell::new("⊘").tone(Tone::Neg)
    } else {
        Cell::muted("⊘")
    }
}

/// A dead level stays in the table until the server drops it, greyed out.
fn faded(cell: impl Into<Cell>, dead: bool) -> Cell {
    let cell = cell.into();
    if dead { cell.tone(Tone::Muted) } else { cell }
}

/// Large resting orders in exchange order books, from the aggregator's `density` signals.
struct Density {
    feed: SignalFeed,
    curation: Curation,
    /// Rows must be rebuilt: new signals, other parameters or a ★/⊘ click.
    dirty: bool,
    unavailable: bool,
}

impl Default for Density {
    fn default() -> Self {
        Self {
            feed: SignalFeed::new(SignalSource::Density),
            curation: Curation::default(),
            dirty: true,
            unavailable: false,
        }
    }
}

impl Density {
    /// Levels that pass the filter, favourites first, then the newest, at most `limit`.
    fn visible(
        &self,
        filter: &Filter,
        hide_blacklisted: bool,
        limit: usize,
    ) -> Vec<(&Signal, &DensitySignal)> {
        let mut levels: Vec<(&Signal, &DensitySignal)> = self
            .feed
            .iter()
            .filter_map(|signal| Some((signal, signal.density.as_deref()?)))
            .filter(|(signal, d)| {
                filter.passes(signal, d)
                    && !(hide_blacklisted
                        && self.curation.blacklist.contains(&instrument(signal, d)))
            })
            .collect();
        levels.sort_by_cached_key(|(signal, d)| {
            (
                std::cmp::Reverse(self.curation.rank(&signal.id, &instrument(signal, d))),
                std::cmp::Reverse(signal.ts_ms),
            )
        });
        levels.truncate(limit);
        levels
    }

    fn render(&mut self) -> ScreenerResult {
        let params = params();
        let lang = lang();
        let filter = Filter::from_params(&params);
        let hide_usdt = params.bool_or("hide_usdt", false);
        let limit = params.i64_or("limit", 2000).clamp(1, ROWS_MAX) as usize;
        let rows: Vec<Row> = self
            .visible(&filter, params.bool_or("hide_blacklisted", false), limit)
            .into_iter()
            .map(|(signal, d)| {
                let instrument = instrument(signal, d);
                let dead = d.status == DensityStatus::Dead;
                let favorite = self.curation.favorites.contains(&signal.id);
                let blacklisted = self.curation.blacklist.contains(&instrument);
                let market = MarketRef::new(d.exchange.as_str(), d.market, signal.symbol.as_str());
                Row::new(signal.id.as_str())
                    .market_ref(&market)
                    .rank(self.curation.rank(&signal.id, &instrument))
                    .cell("fav", favorite_cell(favorite))
                    .cell("ban", blacklist_cell(blacklisted))
                    .cell("time", faded(signal.ts_ms, dead))
                    .cell("exchange", faded(d.exchange.as_str(), dead))
                    .cell(
                        "symbol",
                        faded(shown_symbol(&signal.symbol, hide_usdt), dead),
                    )
                    .cell("distance", faded(d.distance_pct, dead))
                    .cell("side", faded(side_cell(d.side, lang), dead))
                    .cell("price", faded(d.price, dead))
                    .cell("current", faded(d.notional_current_usd, dead))
                    .cell("initial", faded(d.notional_initial_usd, dead))
                    .cell("avg", faded(d.notional_avg_usd, dead))
                    .cell("eaten", faded(d.eaten_pct, dead))
                    .cell("lifetime", faded(d.lifetime_s.saturating_mul(1000), dead))
                    .cell("status", status_cell(d.status, lang))
                    .cell("touches", faded(d.touch_count, dead))
            })
            .collect();
        let count = rows.len();
        replace_rows(rows)?;
        self.status(count)
    }

    fn status(&self, rows: usize) -> ScreenerResult {
        let ru = lang() == Lang::Ru;
        if self.unavailable {
            let text = if ru {
                "Этот терминал не отдаёт плотности: обновите его"
            } else {
                "This terminal does not share densities: update it"
            };
            return Ok(set_status(StatusTone::Warn, text)?);
        }
        if !self.feed.connected() {
            let text = if ru {
                "Нет свежих данных от сервера сигналов"
            } else {
                "No fresh data from the signal server"
            };
            return Ok(set_status(StatusTone::Warn, text)?);
        }
        let total = self.feed.len();
        let text = if ru {
            format!("Плотностей: {rows} из {total}")
        } else {
            format!("{rows} of {total} densities")
        };
        Ok(set_status(StatusTone::Ok, text)?)
    }
}

impl Screener for Density {
    fn init(&mut self, _init: &Init) -> ScreenerResult {
        self.curation = Curation::load();
        Ok(())
    }

    fn on_timer(&mut self, _now_ms: i64) -> ScreenerResult {
        let connected = self.feed.connected();
        match self.feed.poll() {
            Ok(update) => {
                // A reset while the terminal is still connecting is empty: it says nothing
                // about which favourite levels are gone.
                if update.reset && self.feed.connected() && !self.feed.is_empty() {
                    let alive: BTreeSet<&str> = self.feed.iter().map(|s| s.id.as_str()).collect();
                    self.curation.forget_favorites_except(&alive)?;
                }
                self.dirty |= !update.is_empty() || self.unavailable;
                self.unavailable = false;
            }
            Err(e) if e.code() == Some("unavailable") => {
                if !self.unavailable {
                    warn!("density signals: {e}");
                    self.dirty = true;
                }
                self.unavailable = true;
            }
            Err(e) => return Err(e.into()),
        }
        if connected != self.feed.connected() {
            self.dirty = true;
        }
        if !self.dirty {
            return Ok(());
        }
        self.dirty = false;
        self.render()
    }

    fn on_params(&mut self, _params: &Params) -> ScreenerResult {
        self.dirty = true;
        Ok(())
    }

    fn on_click(&mut self, click: &Click) -> ScreenerResult {
        let id = click.row.key.as_str();
        let Some(signal) = self.feed.get(id) else {
            return Ok(());
        };
        let Some(d) = &signal.density else {
            return Ok(());
        };
        match click.column.as_deref() {
            Some("fav") => {
                self.curation.toggle_favorite(id)?;
                return self.render();
            }
            Some("ban") => {
                self.curation.toggle_blacklist(&instrument(signal, d))?;
                return self.render();
            }
            _ => {}
        }
        let market = MarketRef::new(d.exchange.as_str(), d.market, signal.symbol.as_str());
        let params = params();
        if params.bool_or("smart_level", true) {
            let level = SmartLevel::new(id).with_sound(params.bool_or("smart_level_sound", false));
            open_market_with_level(&market, &level)?;
        } else {
            open_market(&market)?;
        }
        Ok(())
    }
}

export_screener!(Density, on_click);

#[cfg(test)]
mod tests {
    use serde_json::json;
    use space_screener::SignalsDelta;

    use super::*;

    fn level(id: &str, ts_ms: i64, status: &str, avg: Option<f64>) -> Signal {
        serde_json::from_value(json!({
            "id": id, "source": "density", "ts_ms": ts_ms, "symbol": "BTCUSDT",
            "exchanges": [{"exchange": "binance", "market": "futures"}],
            "density": {
                "exchange": "binance", "market": "futures", "side": "bid", "price": 60000,
                "qty": 10, "notional_initial_usd": 900000, "notional_current_usd": 600000,
                "notional_avg_usd": avg, "eaten_pct": 33, "distance_pct": -0.4,
                "touch_count": 2, "lifetime_s": 90, "status": status, "event": "touched"
            }
        }))
        .unwrap()
    }

    fn filter(params: serde_json::Value) -> Filter {
        Filter::from_params(&Params::new(
            params.as_object().cloned().unwrap_or_default(),
        ))
    }

    #[test]
    fn filter_follows_the_parameters() {
        let s = level("d", 1, "alive", Some(100000.0));
        let d = s.density.as_ref().unwrap();
        assert!(filter(json!({})).passes(&s, d));
        assert!(
            filter(json!({"side": "bid", "market": "futures", "exchanges": "binance"}))
                .passes(&s, d)
        );
        assert!(!filter(json!({"side": "ask"})).passes(&s, d));
        assert!(!filter(json!({"exclude_exchanges": "Binance"})).passes(&s, d));
        assert!(filter(json!({"max_distance": 0.5})).passes(&s, d));
        assert!(!filter(json!({"max_distance": 0.3})).passes(&s, d));
        assert!(!filter(json!({"max_eaten": 30})).passes(&s, d));
        assert!(!filter(json!({"min_touches": 3})).passes(&s, d));
        assert!(filter(json!({"avg_multiplier": 6})).passes(&s, d));
        assert!(!filter(json!({"avg_multiplier": 7})).passes(&s, d));
        let no_avg = level("n", 1, "alive", None);
        assert!(
            filter(json!({"avg_multiplier": 7})).passes(&no_avg, no_avg.density.as_ref().unwrap())
        );
        let thin = level("t", 1, "alive", Some(1_000_000.0));
        let thin_d = thin.density.as_ref().unwrap();
        assert!(!filter(json!({})).passes(&thin, thin_d));
        assert!(filter(json!({"avg_multiplier": 0})).passes(&thin, thin_d));
        let dead = level("x", 1, "dead", None);
        assert!(!filter(json!({"hide_dead": true})).passes(&dead, dead.density.as_ref().unwrap()));
    }

    #[test]
    fn favourites_first_then_newest_within_the_limit() {
        let mut density = Density::default();
        density.feed.apply(SignalsDelta {
            reset: true,
            connected: true,
            upserts: vec![
                level("old", 1, "alive", None),
                level("mid", 2, "alive", None),
                level("new", 3, "alive", None),
            ],
            ..SignalsDelta::default()
        });
        density.curation.favorites.insert("old".to_string());
        let ids = |levels: Vec<(&Signal, &DensitySignal)>| {
            levels
                .into_iter()
                .map(|(s, _)| s.id.clone())
                .collect::<Vec<_>>()
        };
        assert_eq!(
            ids(density.visible(&filter(json!({})), false, 2)),
            ["old", "new"]
        );
        density
            .curation
            .blacklist
            .insert("binance:futures:BTCUSDT".to_string());
        assert_eq!(
            ids(density.visible(&filter(json!({})), true, 10)),
            Vec::<String>::new()
        );
    }
}
