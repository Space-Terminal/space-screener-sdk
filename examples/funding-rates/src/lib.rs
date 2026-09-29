use std::collections::HashMap;

use serde_json::Value;
use space_screener::prelude::*;

const HOUR_MS: i64 = hours(1);
/// Binance lists only the symbols whose funding interval differs from the default.
const BINANCE_DEFAULT_HOURS: f64 = 8.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Source {
    BinanceIntervals,
    Binance,
    Bybit,
    Bitget,
    Gate,
    Mexc,
    Hyperliquid,
}

impl Source {
    fn slug(self) -> &'static str {
        match self {
            Self::BinanceIntervals | Self::Binance => "binance",
            Self::Bybit => "bybit",
            Self::Bitget => "bitget",
            Self::Gate => "gate",
            Self::Mexc => "mexc",
            Self::Hyperliquid => "hyperliquid",
        }
    }

    fn request(self) -> HttpRequest {
        match self {
            Self::BinanceIntervals => {
                HttpRequest::get("https://fapi.binance.com/fapi/v1/fundingInfo")
            }
            Self::Binance => HttpRequest::get("https://fapi.binance.com/fapi/v1/premiumIndex"),
            Self::Bybit => {
                HttpRequest::get("https://api.bybit.com/v5/market/tickers?category=linear")
            }
            Self::Bitget => HttpRequest::get(
                "https://api.bitget.com/api/v2/mix/market/current-fund-rate?productType=USDT-FUTURES",
            ),
            Self::Gate => HttpRequest::get("https://api.gateio.ws/api/v4/futures/usdt/contracts"),
            Self::Mexc => {
                HttpRequest::get("https://contract.mexc.com/api/v1/contract/funding_rate")
            }
            Self::Hyperliquid => HttpRequest::post(
                "https://api.hyperliquid.xyz/info",
                r#"{"type":"metaAndAssetCtxs"}"#,
            ),
        }
    }
}

/// Funding of one perpetual: the rate of one period, the period and the next payment.
#[derive(Debug, Clone, PartialEq)]
struct Funding {
    exchange: &'static str,
    /// Canonical `BASEQUOTE`.
    symbol: String,
    rate: f64,
    period_hours: f64,
    next_ms: Option<i64>,
}

impl Funding {
    fn apr_pct(&self) -> f64 {
        self.rate * 100.0 * 24.0 / self.period_hours * 365.0
    }
}

fn num(v: &Value) -> Option<f64> {
    match v {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
}

fn items<'a>(v: &'a Value, path: &[&str]) -> &'a [Value] {
    path.iter()
        .fold(Some(v), |v, key| v?.get(key))
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default()
}

fn usdt(symbol: &str) -> Option<String> {
    let base = symbol
        .strip_suffix("_USDT")
        .or_else(|| symbol.strip_suffix("USDT"))?;
    (!base.is_empty() && !base.contains(['-', '_'])).then(|| format!("{base}USDT"))
}

fn funding(
    exchange: &'static str,
    symbol: Option<String>,
    rate: Option<f64>,
    period_hours: Option<f64>,
    next_ms: Option<i64>,
) -> Option<Funding> {
    let period_hours = period_hours.filter(|h| *h > 0.0)?;
    Some(Funding {
        exchange,
        symbol: symbol?,
        rate: rate?,
        period_hours,
        next_ms: next_ms.filter(|t| *t > 0),
    })
}

fn parse_binance_intervals(body: &Value) -> HashMap<String, f64> {
    items(body, &[])
        .iter()
        .filter_map(|i| {
            Some((
                i.get("symbol")?.as_str()?.to_string(),
                num(i.get("fundingIntervalHours")?)?,
            ))
        })
        .collect()
}

fn parse_binance(body: &Value, intervals: &HashMap<String, f64>) -> Vec<Funding> {
    items(body, &[])
        .iter()
        .filter_map(|i| {
            let native = i.get("symbol")?.as_str()?;
            let hours = intervals
                .get(native)
                .copied()
                .unwrap_or(BINANCE_DEFAULT_HOURS);
            funding(
                "binance",
                usdt(native),
                i.get("lastFundingRate").and_then(num),
                Some(hours),
                i.get("nextFundingTime").and_then(num).map(|t| t as i64),
            )
        })
        .collect()
}

fn parse_bybit(body: &Value) -> Vec<Funding> {
    items(body, &["result", "list"])
        .iter()
        .filter_map(|i| {
            funding(
                "bybit",
                usdt(i.get("symbol")?.as_str()?),
                i.get("fundingRate").and_then(num),
                i.get("fundingIntervalHour").and_then(num),
                i.get("nextFundingTime").and_then(num).map(|t| t as i64),
            )
        })
        .collect()
}

fn parse_bitget(body: &Value) -> Vec<Funding> {
    items(body, &["data"])
        .iter()
        .filter_map(|i| {
            funding(
                "bitget",
                usdt(i.get("symbol")?.as_str()?),
                i.get("fundingRate").and_then(num),
                i.get("fundingRateInterval").and_then(num),
                i.get("nextUpdate").and_then(num).map(|t| t as i64),
            )
        })
        .collect()
}

fn parse_gate(body: &Value) -> Vec<Funding> {
    items(body, &[])
        .iter()
        .filter(|c| c.get("status").and_then(Value::as_str) == Some("trading"))
        .filter(|c| c.get("in_delisting").and_then(Value::as_bool) != Some(true))
        .filter_map(|c| {
            funding(
                "gate",
                usdt(c.get("name")?.as_str()?),
                c.get("funding_rate").and_then(num),
                c.get("funding_interval").and_then(num).map(|s| s / 3600.0),
                c.get("funding_next_apply")
                    .and_then(num)
                    .map(|s| s as i64 * 1000),
            )
        })
        .collect()
}

fn parse_mexc(body: &Value) -> Vec<Funding> {
    items(body, &["data"])
        .iter()
        .filter_map(|i| {
            funding(
                "mexc",
                usdt(i.get("symbol")?.as_str()?),
                i.get("fundingRate").and_then(num),
                i.get("collectCycle").and_then(num),
                i.get("nextSettleTime").and_then(num).map(|t| t as i64),
            )
        })
        .collect()
}

/// Hyperliquid pays funding every hour; `funding` is the hourly rate.
fn parse_hyperliquid(body: &Value, now_ms: i64) -> Vec<Funding> {
    let universe = body
        .get(0)
        .and_then(|meta| meta.get("universe"))
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();
    let contexts = body
        .get(1)
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();
    let next_hour = now_ms - now_ms.rem_euclid(HOUR_MS) + HOUR_MS;
    universe
        .iter()
        .zip(contexts)
        .filter(|(asset, _)| asset.get("isDelisted").and_then(Value::as_bool) != Some(true))
        .filter_map(|(asset, ctx)| {
            let name = asset.get("name")?.as_str()?;
            funding(
                "hyperliquid",
                Some(format!("{}USDC", name.to_ascii_uppercase())),
                ctx.get("funding").and_then(num),
                Some(1.0),
                Some(next_hour),
            )
        })
        .collect()
}

/// Funding of USDT perpetuals on six exchanges, one bulk request per exchange and round.
#[derive(Default)]
struct FundingRates {
    binance_intervals: HashMap<String, f64>,
    binance_intervals_at: Option<i64>,
}

impl Screener for FundingRates {
    fn on_timer(&mut self, now_ms: i64) -> ScreenerResult {
        let params = params();
        let min_abs_apr = params.f64_or("min_abs_apr", 0.0);
        let limit = params.i64_or("limit", 1000).max(1) as usize;

        let mut sources = vec![
            Source::Binance,
            Source::Bybit,
            Source::Bitget,
            Source::Gate,
            Source::Mexc,
            Source::Hyperliquid,
        ];
        if self
            .binance_intervals_at
            .is_none_or(|at| now_ms - at >= HOUR_MS)
        {
            sources.insert(0, Source::BinanceIntervals);
        }
        let requests: Vec<HttpRequest> = sources.iter().map(|s| s.request()).collect();
        let responses = http_batch(&requests)?;

        let mut all = Vec::new();
        let mut failed = Vec::new();
        for (source, response) in sources.iter().zip(responses) {
            let body = match response
                .and_then(HttpResponse::error_for_status)
                .and_then(|r| r.json::<Value>())
            {
                Ok(body) => body,
                Err(e) => {
                    warn!("{}: {e}", source.slug());
                    if *source != Source::BinanceIntervals {
                        failed.push(source.slug());
                    }
                    continue;
                }
            };
            match source {
                Source::BinanceIntervals => {
                    self.binance_intervals = parse_binance_intervals(&body);
                    self.binance_intervals_at = Some(now_ms);
                }
                Source::Binance => all.extend(parse_binance(&body, &self.binance_intervals)),
                Source::Bybit => all.extend(parse_bybit(&body)),
                Source::Bitget => all.extend(parse_bitget(&body)),
                Source::Gate => all.extend(parse_gate(&body)),
                Source::Mexc => all.extend(parse_mexc(&body)),
                Source::Hyperliquid => all.extend(parse_hyperliquid(&body, now_ms)),
            }
        }

        all.retain(|f| f.apr_pct().abs() >= min_abs_apr);
        all.sort_by(|a, b| b.apr_pct().abs().total_cmp(&a.apr_pct().abs()));
        all.truncate(limit);
        let rows = all.iter().map(|f| {
            Row::new(format!("{}:{}", f.exchange, f.symbol))
                .market_ref(&MarketRef::new(f.exchange, Market::Futures, &f.symbol))
                .cell("symbol", f.symbol.as_str())
                .cell("exchange", f.exchange)
                .cell("rate", Cell::signed(f.rate * 100.0))
                .cell("apr", Cell::signed(f.apr_pct()))
                .cell("period", f.period_hours * HOUR_MS as f64)
                .cell("next", f.next_ms.map(|t| t as f64))
        });
        if failed.is_empty() {
            set_status(StatusTone::Ok, format!("{} perpetuals", all.len()))?;
        } else {
            set_status(
                StatusTone::Warn,
                format!("{} perpetuals; no data: {}", all.len(), failed.join(", ")),
            )?;
        }
        replace_rows(rows)?;
        Ok(())
    }
}

export_screener!(FundingRates);

#[cfg(test)]
mod tests {
    use super::*;

    fn json(text: &str) -> Value {
        serde_json::from_str(text).unwrap()
    }

    #[test]
    fn binance_uses_the_listed_interval_or_eight_hours() {
        let intervals =
            parse_binance_intervals(&json(r#"[{"symbol":"LPTUSDT","fundingIntervalHours":4}]"#));
        let rows = parse_binance(
            &json(
                r#"[{"symbol":"BTCUSDT","lastFundingRate":"0.0001","nextFundingTime":1790726400000},
                    {"symbol":"LPTUSDT","lastFundingRate":"-0.0002","nextFundingTime":1790712000000},
                    {"symbol":"BTCUSDT_251226","lastFundingRate":"","nextFundingTime":0}]"#,
            ),
            &intervals,
        );
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].period_hours, 8.0);
        assert!((rows[0].apr_pct() - 10.95).abs() < 1e-9);
        assert_eq!(rows[1].period_hours, 4.0);
    }

    #[test]
    fn exchanges_parse_rate_period_and_next_payment() {
        let bybit = parse_bybit(&json(
            r#"{"result":{"list":[{"symbol":"BTCUSDT","fundingRate":"0.00001","fundingIntervalHour":"8","nextFundingTime":"1790726400000"},
                                  {"symbol":"BTC-26DEC25","fundingRate":"","fundingIntervalHour":"","nextFundingTime":"0"}]}}"#,
        ));
        assert_eq!(bybit.len(), 1);
        assert_eq!(bybit[0].next_ms, Some(1_790_726_400_000));

        let bitget = parse_bitget(&json(
            r#"{"data":[{"symbol":"BTCUSDT","fundingRate":"0.0001","fundingRateInterval":"4","nextUpdate":"1790726400000"}]}"#,
        ));
        assert_eq!(bitget[0].period_hours, 4.0);

        let gate = parse_gate(&json(
            r#"[{"name":"BTC_USDT","status":"trading","funding_rate":"0.000082","funding_interval":28800,"funding_next_apply":1790726400},
                {"name":"OLD_USDT","status":"delisting","funding_rate":"0.1","funding_interval":28800,"funding_next_apply":1}]"#,
        ));
        assert_eq!(gate.len(), 1);
        assert_eq!(gate[0].symbol, "BTCUSDT");
        assert_eq!(gate[0].next_ms, Some(1_790_726_400_000));

        let mexc = parse_mexc(&json(
            r#"{"success":true,"data":[{"symbol":"BTC_USDT","fundingRate":2e-05,"collectCycle":8,"nextSettleTime":1790726400000}]}"#,
        ));
        assert_eq!(mexc[0].symbol, "BTCUSDT");

        let hl = parse_hyperliquid(
            &json(
                r#"[{"universe":[{"name":"BTC"},{"name":"OLD","isDelisted":true}]},[{"funding":"0.0000125"},{"funding":"0.1"}]]"#,
            ),
            1_790_702_090_000,
        );
        assert_eq!(hl.len(), 1);
        assert_eq!(hl[0].symbol, "BTCUSDC");
        assert_eq!(hl[0].period_hours, 1.0);
        assert_eq!(hl[0].next_ms, Some(1_790_704_800_000));
    }
}
