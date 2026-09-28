---
name: space-screener
description: Build Space Terminal screener plugins (Rust → wasm, run in the terminal's sandbox) with the space-screener PDK and the `st` CLI. Use when asked to write, fix or extend a screener, scanner or table of symbols for Space Terminal — for example open interest, funding, volume spikes, top movers — or when working in a folder with manifest.yaml and a space-screener dependency.
---

# Space Terminal screeners

A screener is a small Rust library compiled to `wasm32-unknown-unknown`. Space Terminal runs it in a
sandbox and shows its rows as a table pane; clicking a row opens that market's order book. The plugin
has **no network, files, clock or threads of its own** — it asks the terminal (the *host*) for data
and HTTP, and hands rows back. The full wire contract is `ABI.md` in the SDK repository.

## Workflow

1. **Tooling.** `st --version`. If missing: `cargo install --path <sdk>/crates/st` (local SDK checkout)
   or `cargo install --git https://github.com/EvgeniiKobelev/space-screener-sdk st`.
   `st build` installs the `wasm32-unknown-unknown` target through rustup on first use.
2. **Scaffold.** `st init <folder> --id <author>.<name>` → `Cargo.toml`, `manifest.yaml`,
   `src/lib.rs` (a working "top movers" starter). Work inside that folder.
3. **Code.** Edit `src/lib.rs` (the screener) and `manifest.yaml` (id, columns, params, `http` hosts).
4. **Build.** `st build` — compiles, writes `screener.wasm`, validates the manifest and wasm imports
   exactly like the terminal does. Fix every error it prints before going on.
5. **Run.** With Space Terminal running: `st dev` — installs the plugin, asks the terminal to open a
   pane for it, then rebuilds and reinstalls on every save and prints the plugin log. Leave it running
   in the background.
6. **Check.** `st rows` (what the pane shows, `--json` for raw), `st logs` (plugin log), `st list`
   (status: running / stopped / error / limit). Iterate until rows look right, then tell the user to
   look at the pane and click a row.

Terminal discovery: `st` reads the token from `<data>/screeners/local_api_token` and the port from
`<data>/config/general.yaml` (default 5055). `<data>` is `SPACE_TERMINAL_DIR` or the OS data folder
`Space Terminal`. For a terminal started with `cargo run` from its repository, set
`SPACE_TERMINAL_DIR=<that repository>`. `--port` / `SPACE_TERMINAL_PORT` override the port.

## Skeleton

```rust
use space_screener::prelude::*;

#[derive(Default)]
struct MyScreener {
    // your state; lives until the plugin restarts
}

impl Screener for MyScreener {
    fn init(&mut self, _init: &Init) -> ScreenerResult {
        Ok(())
    }

    fn on_timer(&mut self, now_ms: i64) -> ScreenerResult {
        let min_volume = params().f64_or("min_volume", 1e6);
        // fetch → compute → rows
        replace_rows(rows)?;
        Ok(())
    }
    // on_click: default opens the row's market; on_params: default does nothing (params() is live)
}

export_screener!(MyScreener);
```

`on_timer` runs every `timer_ms` (from the end of the previous call). Errors returned from the trait
methods are logged; use `?` freely — any `std::error::Error` converts.

## Host API (all return `space_screener::Result`, errors carry a `code()`)

| Call | What it does |
|---|---|
| `http(&HttpRequest::get(url))` | HTTPS GET/POST to a host listed in manifest `http`; `resp.error_for_status()?.json::<T>()?` |
| `http_batch(&[req…])` | many requests concurrently (8 in flight) → `Vec<Result<HttpResponse>>` in order. **Use for per-symbol endpoints** |
| `tickers("binance", Market::Futures)` | terminal's 24h ticker snapshot: `symbol, base, quote, last, change_pct (%), volume_quote` |
| `symbols(exchange, market)` / `exchanges()` | symbol universe / exchanges the user connected (`connected: bool`) |
| `replace_rows(rows)` / `emit_rows(rows)` / `expire(keys)` | replace the table / upsert by key / remove |
| `set_status(StatusTone::Ok, "8/8 exchanges")` | short status line in the pane |
| `alert(AlertLevel::Warn, title, body)` / `alert_row(…, key)` | toast + sound + notification (≤ 6/min) |
| `open_market(&MarketRef)` / `open_spread(&a, &b, None)` | only inside `on_click` |
| `kv_get::<T>(key)` / `kv_set(key, &v)` / `kv_delete(key)` | state that survives restarts, ≤ 1 MiB total |
| `history_cluster(&req)` / `history_replay(&req)` | cloud history; needs `history: [cluster]` / `[replay]` in the manifest |
| `now_ms()` | current time (there is no `std::time` in wasm) |
| `info!`, `warn!`, `error!`, `debug!` | plugin log (`st logs`) |
| `params()`, `lang()` | current parameters, UI language |

Rows:

```rust
Row::new(format!("{exchange}:{symbol}"))                    // stable unique key
    .market_ref(&MarketRef::new(exchange, Market::Futures, symbol))  // makes the row clickable
    .cell("symbol", "BTCUSDT")
    .cell("oi", 7.8e9)                                       // numbers, &str/String, bool, Option<_>
    .cell("chg5", Cell::signed(1.2))                         // green/red by sign
    .cell("note", Cell::new("stale").tone(Tone::Muted))
    .rank(1)                                                 // pinned above rank 0
```

Every `cells` key must be a column `key` in the manifest. At most 5000 rows.

Helpers:
- `Series::new(mins(16))` — per-key history: `push(ts, v)`, `value_at(ts)`, `change_pct(now, mins(5))`
  (`None` until enough history). Keep one per key in a `HashMap<String, Series>`; drop stale keys.
- `secs(n)`, `mins(n)`, `hours(n)` → milliseconds.
- `space_screener::oi` — open interest (one side, USD) for 8 exchanges: `Collector::new()` then
  `collector.collect(now_ms)?` → one `Snapshot { exchange, result }` per exchange. Items have
  `exchange`, native `symbol` (use for clicks), normalized `base`, `quote`, `oi_usd`, `price`,
  `market_ref()`, `pair()`. Add `space_screener::oi::HOSTS` to manifest `http`. Parsers per exchange
  (`oi::bybit::parse(body)` …) are public if you need only part of it.

## Manifest essentials

```yaml
abi: 1
id: author.oi-8-exchanges        # lowercase, [a-z0-9._-], 3..64
version: 0.1.0
name: {ru: "Открытый интерес", en: "Open interest"}
lang: rust
min_terminal: 0.104.70
http: [fapi.binance.com, api.bybit.com]   # every host you call, bare names, no https://
timer_ms: 60000                  # >= 250
columns:
  - {key: symbol, type: symbol, title: {ru: "Тикер", en: "Symbol"}}
  - {key: exchange, type: exchange, title: {ru: "Биржа", en: "Exchange"}}
  - {key: oi, type: usd, title: {en: "OI, $"}, sort: desc}
  - {key: chg5, type: percent, title: {en: "OI 5m"}}
params:
  - {key: min_oi, type: number, title: {en: "Min OI, $"}, default: 5000000, min: 0}
limits: {cpu_ms_per_call: 1000}  # parse-heavy screeners (default 250 ms of wasm CPU per call)
```

Column types: `text number integer percent usd price time duration countdown symbol exchange exchanges bool`.
`percent` values are already percents (1.5 = 1.5 %); `time`/`countdown` are ms since epoch;
`exchanges` cells are `Cell::markets([ExchangeMarket::new("bybit", Market::Spot)])`.
Param types: `number integer bool text select` (select needs `options`, `default` must fit).
`feeds` must stay empty in v1 (trade/order-book streams come later) — poll in `on_timer`.

Exchange slugs (lowercase): `binance bybit okx bitget gate mexc kucoin bingx hyperliquid`, and others the
user connected — check `exchanges()`. Markets: `spot`, `futures`.

## Limits

- 64 MB memory; `cpu_ms_per_call` of wasm CPU per call (default 250, max 1000; HTTP waiting does not
  count); 3 overruns in a row stop the plugin. `http` timeout ≤ 10 s, responses ≤ 8 MiB.
- After a crash the terminal restarts the plugin (`init` again): in-memory history is lost, `kv` stays.

## Pitfalls in wasm

| Symptom | Fix |
|---|---|
| `forbidden_import: wasi_snapshot_preview1::…` | built for wasip1 or used `std::fs`/`std::net`/`std::thread`; use `st build` (wasm32-unknown-unknown) and host calls |
| panic "time not implemented" | `std::time::SystemTime`/`Instant` do not work; use `now_ms` from `on_timer` or `now_ms()` |
| `forbidden_import: …__wbindgen…` | a dependency pulls wasm-bindgen (`getrandom` with `js`, `chrono` `wasmbind`, `uuid` `js`): drop it |
| `forbidden_import: extism:host/env::log_info` / `http_request` | extism-pdk used directly with default features; use `space_screener::{info!, http}` |
| `println!` prints nothing | use `info!` and `st logs` |
| `host_not_allowed` | add the exact host to manifest `http` |
| `limit` status / cpu overrun | parse only the fields you need (`#[derive(Deserialize)]` structs, not `serde_json::Value`), raise `limits.cpu_ms_per_call`, cache slow-changing metadata |
| slow round with hundreds of requests | `http_batch`, not a loop of `http`; keep `timer_ms` ≥ 60000 for per-symbol endpoints (exchange rate limits are shared with the user's trading IP) |
| `not_in_click` | `open_market` only from `on_click` |
| numbers as strings in exchange JSON | parse strings (`"83890.5".parse::<f64>()`) or deserialize with a string-or-number helper |

## Example: open interest on 8 exchanges

`examples/oi-8-exchanges` in the SDK is the reference. Core of it:

```rust
use std::collections::HashMap;

use space_screener::oi::Collector;
use space_screener::prelude::*;

#[derive(Default)]
struct OiScreener {
    collector: Collector,
    history: HashMap<String, Series>,
}

impl Screener for OiScreener {
    fn on_timer(&mut self, now_ms: i64) -> ScreenerResult {
        let min_oi = params().f64_or("min_oi", 5_000_000.0);
        let mut rows = Vec::new();
        for snapshot in self.collector.collect(now_ms)? {
            let Ok(items) = snapshot.result else {
                warn!("{}: no data", snapshot.exchange);
                continue;
            };
            for oi in items {
                let key = format!("{}:{}", oi.exchange, oi.symbol);
                let series = self.history.entry(key.clone()).or_insert_with(|| Series::new(mins(16)));
                series.push(now_ms, oi.oi_usd);
                if oi.oi_usd < min_oi {
                    continue;
                }
                rows.push(
                    Row::new(key)
                        .market_ref(&oi.market_ref())
                        .cell("symbol", oi.pair())
                        .cell("exchange", oi.exchange)
                        .cell("oi", oi.oi_usd)
                        .cell("chg5", series.change_pct(now_ms, mins(5)).map(Cell::signed))
                        .cell("chg15", series.change_pct(now_ms, mins(15)).map(Cell::signed)),
                );
            }
        }
        self.history.retain(|_, s| s.last_ts().is_some_and(|t| now_ms - t < mins(30)));
        replace_rows(rows)?;
        Ok(())
    }
}

export_screener!(OiScreener);
```

Manifest: the 8 hosts of `space_screener::oi::HOSTS` in `http`, `timer_ms: 60000`,
`limits: {cpu_ms_per_call: 1000}`, columns `symbol, exchange, oi, chg5, chg15`.
Changes stay empty for the first 5/15 minutes — the screener builds history from its own snapshots.

## Done checklist

- `st build` passes with no errors; `st dev` shows `installed … running: true`.
- `st rows` shows the expected rows and `st logs` has no repeating errors; `st list` status is `running`.
- Rows carry `symbol`/`exchange`/`market` so a click opens the order book; tell the user to click one.
