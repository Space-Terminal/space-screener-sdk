---
name: space-screener
description: Build Space Terminal screener plugins (Rust or TypeScript → wasm, run in the terminal's sandbox) with the space-screener PDK and the `st` CLI, test them on recordings and publish them to the Space Market catalog. Use when asked to write, fix or extend a screener, scanner or table of symbols for Space Terminal — for example open interest, funding, volume spikes, top movers — or when working in a folder with manifest.yaml and a space-screener dependency.
---

# Space Terminal screeners

A screener is a small Rust library compiled to `wasm32-unknown-unknown` (or a TypeScript module that
`st` compiles to the same kind of wasm — see "TypeScript" below). Space Terminal runs it in a
sandbox and shows its rows as a table pane; clicking a row opens that market's order book. The plugin
has **no network, files, clock or threads of its own** — it asks the terminal (the *host*) for data
and HTTP, and hands rows back. The full wire contract is `ABI.md` in the SDK repository.

## Workflow

1. **Tooling.** `st --version`. If missing:
   `cargo install --git https://github.com/Space-Terminal/space-screener-sdk st`. Projects created by this
   `st` depend on the SDK by git. When developing the SDK itself, install from your checkout instead —
   `cargo install --path <sdk-checkout>/crates/st` — and `st init` links new projects to that checkout.
   `st build` installs the `wasm32-unknown-unknown` target through rustup on first use.
2. **Scaffold.** `st init <folder> --id <author>.<name>` → `Cargo.toml`, `manifest.yaml`,
   `src/lib.rs` (a working "top movers" starter). Work inside that folder.
   `--sdk-path <sdk>/crates/space-screener` points the dependency at a specific checkout.
3. **Code.** Edit `src/lib.rs` (the screener) and `manifest.yaml` (id, columns, params, `http` hosts).
4. **Build.** `st build` — compiles, writes `screener.wasm`, validates the manifest and wasm imports
   exactly like the terminal does. Fix every error it prints before going on.
5. **Run.** With Space Terminal running: `st dev` — installs the plugin, asks the terminal to open a
   pane for it, then rebuilds and reinstalls on every save and prints the plugin log. Leave it running
   in the background. Every reinstall restarts the plugin and wipes its in-memory state, so 5/15-minute
   windows start over after each save; keep small state that must survive in `kv_set`.
6. **Check.** `st rows` (what the pane shows; `--json` prints the terminal's response as is:
   `{ok, version, status, status_text?, rows}`), `st logs` (plugin log; a call that used ≥ 50 % of its
   CPU budget adds `cpu … ms (… ms in host functions) of … ms, wall … ms`), `st list`
   (status: running / stopped / error / limit). Iterate until rows look right, then tell the user to
   look at the pane and click a row.
7. **Record and test.** While the plugin runs in a pane: `st record --seconds 120` restarts it and saves
   its run (the terminal's data replies) to `recordings/<UTC time>.json`. `st test` replays every
   recording headless — no terminal needed — and compares rows, alerts, status and clicks with
   `recordings/<file>.expected.json` (the first run writes it; after an intended change
   `st test --update`). Use it for fast iterations and before publishing; keep recordings small
   (the registry takes recordings up to 4 MiB). `st test` also fails when a call returns an error,
   traps or goes over its CPU budget, and prints the plugin's error lines.
8. **Publish — only when the user asks.** Fill the catalog fields (`categories`, `description`, optional
   `source`); `st validate` prints `catalog: ready`. The user makes a publish token on the author page
   of the store (`https://store.space-terminal.com/author`); `st login` saves it once (or `ST_TOKEN`).
   `st publish --recording recordings/<file>.json` builds, checks, runs a local trial and uploads.
   Every version waits for a human moderator; its status is on the author page. Raise `version` for
   every upload.

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
    // on_params: default does nothing (params() is always current)
}

export_screener!(MyScreener);
```

`on_timer` runs every `timer_ms` (from the end of the previous call). Errors returned from the trait
methods are logged; use `?` freely — any `std::error::Error` converts. A panic is logged with its
message and location (`st logs`) before the terminal restarts the plugin.

Clicks: rows with `symbol`/`exchange`/`market` open their order book on click — the terminal does it
at once, no code needed. Only for custom click logic implement `on_click` and export with
`export_screener!(MyScreener, on_click)`; such clicks wait while `on_timer` runs (calls never overlap),
so keep rounds short.

## Host API (all return `space_screener::Result`, errors carry a `code()`)

| Call | What it does |
|---|---|
| `http(&HttpRequest::get(url))` | HTTPS GET/POST to a host listed in manifest `http`; `resp.error_for_status()?.json::<T>()?` |
| `http_batch(&[req…])` | many requests concurrently (8 in flight) → `Vec<Result<HttpResponse>>` in order. **Use for per-symbol endpoints** |
| `tickers("binance", Market::Futures)` | terminal's 24h ticker snapshot: `symbol, base, quote, last, change_pct (%), volume_quote` |
| `symbols(exchange, market)` / `exchanges()` | symbol universe / exchanges the user connected (`connected: bool`) |
| `replace_rows(rows)` / `emit_rows(rows)` / `expire(keys)` | replace the table / upsert by key / remove |
| `set_status(StatusTone::Ok, "8/8 exchanges")` | short status line in the pane |
| `alert(AlertLevel::Warn, title, body)` / `alert_row(…, key)` | toast + notification (≤ 6/min); `Warn`/`Urgent` also play a sound, `Info` is silent |
| `open_market(&MarketRef)` / `open_spread(&a, &b, None)` | only inside `on_click` (needs `export_screener!(T, on_click)`) |
| `open_markets(&[MarketRef])` / `open_market_with_level(&m, &SmartLevel::new(id))` | v1.1, inside `on_click`: up to 16 order books at once / a book with a smart level at a density signal |
| `SignalFeed::new(SignalSource::Density)` → `feed.poll()?`, `feed.iter()` | v1.1: live aggregator signals (`activity`, `density`, `prints`) the terminal shares; needs `signals: [density]` in the manifest |
| `kv_get::<T>(key)` / `kv_set(key, &v)` / `kv_delete(key)` | state that survives restarts, ≤ 1 MiB total |
| `history_cluster(&req)` / `history_replay(&req)` | cloud history; needs `history: [cluster]` / `[replay]` in the manifest |
| `now_ms()` | current time (there is no `std::time` in wasm) |
| `info!`, `warn!`, `error!`, `debug!` | plugin log (`st logs`) |
| `params()`, `lang()` | current parameters (`params().f64_or("k", 1.0)`, `i64_or`, `bool_or`, `str("k")`), UI language |

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

Every `cells` key must be a column `key` in the manifest. Each row ≤ 16 KiB of JSON. At most 10000 rows or 32 MiB per plugin: beyond either cap the host evicts the least recently updated rows, so `replace: true` with more rows keeps the last ones of the batch.
So sort and truncate yourself (keep the top N) before `replace_rows`.

Symbols: put the canonical `BASEQUOTE` in upper case into rows and `MarketRef` (`BTCUSDT`, Hyperliquid
`BTCUSDC`); the exchange's native symbol (`BTC-USDT-SWAP`, `BTC_USDT`) also works — the terminal
resolves both. `tickers()`/`symbols()` return the terminal's symbol with `base` and `quote`.

Helpers:
- `Series::new(mins(16))` — per-key history: `push(ts, v)`; `value_at(ts)` is the latest value at or
  before `ts` (`None` if history starts later); `change_pct(now, mins(5))` compares the last value with
  `value_at(now - 5 min)` (`None` until enough history). Keep one per key in a `HashMap<String, Series>`;
  drop stale keys.
- `secs(n)`, `mins(n)`, `hours(n)` → milliseconds.
- `space_screener::oi` — open interest (one side, USD) of USDT perpetuals on 8 exchanges (Hyperliquid:
  USDC perpetuals). Only those contracts come back, no need to filter the quote.
  `Collector::new()` polls all of `oi::EXCHANGES` (`binance bybit okx bitget gate mexc kucoin
  hyperliquid`); `Collector::only(&["bybit", "okx"])` polls a subset (see rotation below).
  `collector.collect(now_ms)?` → one `Snapshot` per exchange: `result: Result<Vec<OpenInterest>>` (items
  that arrived), `is_complete()`, `summary()` (`"binance 180/200"`, `"mexc: no data"`), `first_error`.
  `OpenInterest { exchange: &'static str, symbol: String, native_symbol: String, base: String,
  quote: String, oi_usd: f64, price: f64 }` plus `market_ref()`; `symbol` is canonical (`BTCUSDT`) — use
  it for rows and clicks. The manifest `http` must list the 8 hosts explicitly (see the example below).
  Parsers per exchange (`oi::bybit::parse(body)` …) are public if you need only part of it.
- Binance has no bulk open interest: one request per symbol, and the terminal allows 5 requests/s to
  `fapi.binance.com`, so a round of N symbols takes about N/5 s (200 → 40 s). The collector polls the
  top `BINANCE_TOP_N` = 200 USDT perpetuals by 24h volume; change it with
  `collector.set_binance_top_n(n)`, n in 1..=600 (600 covers every USDT perpetual, ≈ 2 min).
  Keep `timer_ms` ≥ 60000 when one call polls Binance; with rotation shorter timers are fine.

## Manifest essentials

```yaml
abi: 1
id: author.oi-8-exchanges        # [a-z0-9._-], 3..64, starts/ends with a letter or digit; not `install`/`sync`; no con/prn/aux/nul/com1-9/lpt1-9 segment
version: 0.1.0
name: {ru: "Открытый интерес", en: "Open interest"}
lang: rust                       # or ts (see TypeScript)
min_terminal: 0.104.70
http: [fapi.binance.com, api.bybit.com]   # every host you call, bare names, no https://
timer_ms: 60000                  # 250..=3600000
columns:
  - {key: symbol, type: symbol, title: {ru: "Тикер", en: "Symbol"}}
  - {key: exchange, type: exchange, title: {ru: "Биржа", en: "Exchange"}}
  - {key: oi, type: usd, title: {en: "OI, $"}, sort: desc}
  - {key: chg5, type: percent, title: {en: "OI 5m"}}
params:
  - {key: min_oi, type: number, title: {en: "Min OI, $"}, default: 5000000, min: 0}
limits: {cpu_ms_per_call: 1000}  # parse-heavy screeners (default 250 ms of CPU per call, allowed 50–1000)
# catalog only (the terminal ignores them; st publish requires them):
description: {en: "One-side open interest of USDT perpetuals"}   # ≤ 2000 chars, the card text
categories: [open-interest]      # 1..3 of: volume open-interest funding spread movers listings orderbook other
source: https://github.com/me/oi # optional link to the code
```

`install` and `sync` are reserved ids. A host, column or parameter listed twice is refused; column
`width` is 1..2000. A column with `show_if: <bool param key>` is hidden while that parameter is false
(terminal 0.104.73+, no reinstall; older terminals show it) — use it for «show column X» checkboxes
instead of rebuilding the manifest. Versions for the catalog have no build metadata (`1.2.0`, not `1.2.0+b1`), and the
catalog takes at most 4 MiB of wasm code (`st validate` says so; typical Rust screeners are far below).

Column types: `text number integer percent usd price time duration countdown symbol exchange exchanges bool`.
`percent` values are already percents (1.5 = 1.5 %); `time`/`countdown` are ms since epoch;
`exchanges` cells are `Cell::markets([ExchangeMarket::new("bybit", Market::Spot)])`.
Param types: `number integer bool text select` (select needs `options`, `default` must fit).
`signals: [activity, density, prints]` (v1.1, `min_terminal: 0.104.72`) gives the Space aggregator's
signals through `SignalFeed`; `activity` and `prints` need the terminal's Pro build (`unavailable`
otherwise — show it with `set_status`). `feeds` must stay empty (trade/order-book streams come later) —
poll in `on_timer`.

Exchange slugs (lowercase): `binance bybit okx bitget gate mexc kucoin bingx hyperliquid`, and others the
user connected — check `exchanges()`. Markets: `spot`, `futures`.

## Limits

- 64 MB memory. CPU budget `limits.cpu_ms_per_call` (default 250, allowed 50–1000) counts the plugin
  thread's CPU for the whole call — wasm plus host work done for it (JSON, kv, rows); waiting for the
  network is free. Past the budget the next host call aborts the call; a pure wasm loop is cancelled
  after max(5 × budget, 5 s) wall time outside host functions. Overrun, abort, runaway cancel and the
  600 s wall timeout are violations; 3 in the last 10 calls stop the plugin (status `limit`). Calls using
  ≥ 50 % of the budget log `cpu … ms (… ms in host functions) of … ms, wall … ms`. `http` timeout
  1..10 s, responses ≤ 8 MiB.
- CPU share: at most 30 s of CPU (half a core) over any sliding 60 s window, summed over every call that
  ended in it (aborted and cancelled ones too); above that the plugin stops with `limit` ("average CPU
  over 50 % of a core"). A short `timer_ms` with heavy parsing hits this first.
- One call (`on_timer` with all its HTTP waits) must finish within 600 s of wall time. Budget
  per-symbol rounds: `fapi.binance.com` allows 5 requests/s, so N symbols ≈ N/5 s.
- After a crash the terminal restarts the plugin (`init` again): in-memory history is lost, `kv` stays.

## Pitfalls in wasm

| Symptom | Fix |
|---|---|
| `forbidden_import: wasi_snapshot_preview1::…` | built for wasip1 or used `std::fs`/`std::net`/`std::thread`; use `st build` (wasm32-unknown-unknown) and host calls |
| panic "time not implemented" | `std::time::SystemTime`/`Instant` do not work; use `now_ms` from `on_timer` or `now_ms()` |
| `forbidden_import: …__wbindgen…` | a dependency pulls wasm-bindgen (`getrandom` with `js`, `chrono` `wasmbind`, `uuid` `js`): drop it |
| `forbidden_import: extism:host/env::log_info` / `http_request` | extism-pdk used directly with default features; use `space_screener::{info!, http}` |
| `println!` prints nothing | use `info!` and `st logs` |
| `host_not_allowed` | add the exact host to manifest `http` (DNS names only: no IP addresses, no localhost) |
| status 301/302 with an empty body | redirects are not followed; call the final URL directly |
| `bad_request` on http | only `https://`, no `user:pass@` in the URL, `timeout_ms` 1000..=10000 |
| `forbidden_export` / start section | build a `cdylib` for wasm32-unknown-unknown with `st build`; WASI/C runtimes (`_start`, `_initialize`) are refused |
| `invalid_wasm: function N: … KiB of code, the limit is 256 KiB per function` (or nesting / br_table) | one enormous function — usually a giant `match` or a table built inline: move the data into a `static`, split the function |
| `limit` status / cpu overrun | host work for your calls counts too: parse only the fields you need (`#[derive(Deserialize)]` structs, not `serde_json::Value`), emit fewer/smaller rows, raise `limits.cpu_ms_per_call`, cache slow-changing metadata |
| slow round with hundreds of requests | `http_batch`, not a loop of `http`; keep `timer_ms` ≥ 60000 for per-symbol endpoints or rotate groups of exchanges (exchange rate limits are shared with the user's trading IP; `fapi.binance.com` is capped at 5 requests/s) |
| history (5/15 min changes) resets | every `st dev` reinstall and every crash restarts the plugin; windows fill again from scratch |
| clicks feel slow | do not export `on_click` unless needed — without it the terminal opens the row's market at once |
| `not_in_click` | `open_market` only from `on_click` exported with `export_screener!(T, on_click)` |
| `invalid_manifest: min_terminal: the module imports signals` | v1.1 calls (`signals`, `open_markets`) need `min_terminal: 0.104.72` |
| plugin restarts, `st logs` shows `panic: …` | fix the panic at the logged location (index out of bounds, `unwrap` on `None`, …) |
| numbers as strings in exchange JSON | parse strings (`"83890.5".parse::<f64>()`) or deserialize with a string-or-number helper |
| `st test` shows `N not recorded` | the code now calls data functions with inputs the recording does not have (another URL, symbol or exchange): record again |
| `st test` differs after an intended change | review the diff, then `st test --update` |
| `st publish`: `not ready for the catalog` | add `categories` and `description` (see Manifest essentials) |
| `st publish`: `version_exists` / `version_not_greater` | raise `version` in manifest.yaml |
| `st publish`: `too_many_pending` | the moderator has not looked at the previous uploads yet; wait |
| `st publish`: `unauthorized` / `token_invalid` | the token was revoked; the user makes a new one on the author page, then `st login` |

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
        let mut incomplete = Vec::new();
        for snapshot in self.collector.collect(now_ms)? {
            if !snapshot.is_complete() {
                incomplete.push(snapshot.summary());
            }
            let Ok(items) = snapshot.result else {
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
                        .cell("symbol", oi.symbol.as_str())
                        .cell("exchange", oi.exchange)
                        .cell("oi", oi.oi_usd)
                        .cell("chg5", series.change_pct(now_ms, mins(5)).map(Cell::signed))
                        .cell("chg15", series.change_pct(now_ms, mins(15)).map(Cell::signed)),
                );
            }
        }
        self.history.retain(|_, s| s.last_ts().is_some_and(|t| now_ms - t < mins(30)));
        if incomplete.is_empty() {
            set_status(StatusTone::Ok, "8/8 exchanges")?;
        } else {
            set_status(StatusTone::Warn, incomplete.join(" · "))?;
        }
        replace_rows(rows)?;
        Ok(())
    }
}

export_screener!(OiScreener);
```

Manifest for it (write the hosts out: YAML cannot reference `oi::HOSTS`; OKX is `app.okx.com`, not
`www.okx.com`, which is blocked in parts of the CIS):

```yaml
http:
  - fapi.binance.com
  - api.bybit.com
  - app.okx.com
  - api.bitget.com
  - api.gateio.ws
  - contract.mexc.com
  - api-futures.kucoin.com
  - api.hyperliquid.xyz
timer_ms: 60000
limits: {cpu_ms_per_call: 1000}
columns:
  - {key: symbol, type: symbol, title: {ru: "Тикер", en: "Symbol"}}
  - {key: exchange, type: exchange, title: {ru: "Биржа", en: "Exchange"}}
  - {key: oi, type: usd, title: {en: "OI, $"}, sort: desc}
  - {key: chg5, type: percent, title: {en: "OI 5m"}}
  - {key: chg15, type: percent, title: {en: "OI 15m"}}
params:
  - {key: min_oi, type: number, title: {en: "Min OI, $"}, default: 5000000, min: 0}
```

`min_oi` is a filter, not a cap: at 5M$ it leaves about 1800 of ~2100 contracts, and a lower value can
exceed the host's 10000-row cap, past which the least recently updated rows are evicted. Sort and truncate to a `limit` param, as
`examples/oi-8-exchanges` does. The full example also has a `binance_top_n` param
(`collector.set_binance_top_n(..)` in `on_timer`). Changes stay empty for the first 5/15 minutes — the
screener builds history from its own snapshots.

### Splitting heavy exchanges (rotation)

A call that polls Binance lasts about N/5 s (40 s for the default 200 symbols) and the other 7 exchanges
wait for it. Split the work when you want the fast exchanges refreshed more often, when `on_timer` must
stay short (custom clicks wait for it), or with `binance_top_n` near 600 (≈ 2 min per round): one
`Collector::only(..)` per group, one group per call, and the latest rows of every exchange kept so the
table stays whole. With rotation a `timer_ms` below 60000 is fine (for example 15000).

```rust
use std::collections::HashMap;

use space_screener::oi::{Collector, EXCHANGES};
use space_screener::prelude::*;

struct OiRotating {
    groups: Vec<Collector>,
    turn: usize,
    tables: HashMap<&'static str, Vec<(f64, Row)>>,
    history: HashMap<String, Series>,
}

impl Default for OiRotating {
    fn default() -> Self {
        let fast: Vec<&str> = EXCHANGES.into_iter().filter(|e| *e != "binance").collect();
        Self {
            groups: vec![Collector::only(&fast), Collector::only(&["binance"])],
            turn: 0,
            tables: HashMap::new(),
            history: HashMap::new(),
        }
    }
}

impl Screener for OiRotating {
    fn on_timer(&mut self, now_ms: i64) -> ScreenerResult {
        let group = self.turn % self.groups.len();
        self.turn += 1;
        for snapshot in self.groups[group].collect(now_ms)? {
            // A failed exchange keeps its previous rows.
            let Ok(items) = snapshot.result else {
                continue;
            };
            let rows = items
                .into_iter()
                .map(|oi| {
                    let key = format!("{}:{}", oi.exchange, oi.symbol);
                    let series = self.history.entry(key.clone()).or_insert_with(|| Series::new(mins(16)));
                    series.push(now_ms, oi.oi_usd);
                    let row = Row::new(key)
                        .market_ref(&oi.market_ref())
                        .cell("symbol", oi.symbol.as_str())
                        .cell("exchange", oi.exchange)
                        .cell("oi", oi.oi_usd)
                        .cell("chg5", series.change_pct(now_ms, mins(5)).map(Cell::signed));
                    (oi.oi_usd, row)
                })
                .collect();
            self.tables.insert(snapshot.exchange, rows);
        }
        self.history.retain(|_, s| s.last_ts().is_some_and(|t| now_ms - t < mins(30)));
        let mut all: Vec<(f64, Row)> = self.tables.values().flatten().cloned().collect();
        all.sort_by(|a, b| b.0.total_cmp(&a.0));
        all.truncate(params().i64_or("limit", 1000).max(1) as usize);
        replace_rows(all.into_iter().map(|(_, row)| row))?;
        Ok(())
    }
}

export_screener!(OiRotating);
```

## TypeScript

`st init <folder> --lang ts --id <author>.<name>` → `package.json` (esbuild, typescript), `tsconfig.json`,
`manifest.yaml` (`lang: ts`), `src/index.ts`, `pdk/index.ts` (the PDK — do not edit, `st` writes it).
`st build` needs Node.js 18+ (it runs `npm install` once, then `tsc --noEmit`) and downloads extism-js and
binaryen once into the cache; the rest of the workflow (`dev`, `rows`, `logs`, `record`, `test`,
`publish`) is the same.

```ts
import { exchanges, num, replaceRows, setStatus, signed, tickers, warn } from "@space-terminal/screener";
import type { InitInput, Row, TimerInput } from "@space-terminal/screener";

let seen = new Map<string, number>();       // top level: declarations only — it runs at BUILD time

export function init(input: InitInput) {}    // params(), num("key"), str(), bool(), lang() are set here

export function on_timer(input: TimerInput) {
  const rows: Row[] = [];
  for (const ex of exchanges()) {
    if (!ex.connected || ex.market !== "futures") continue;
    for (const t of tickers(ex.exchange, ex.market).tickers) {
      rows.push({ key: `${ex.exchange}:${t.symbol}`, exchange: ex.exchange, market: ex.market, symbol: t.symbol,
                  cells: { symbol: t.symbol, change: signed(t.change_pct), last: t.last } });
    }
  }
  replaceRows(rows);
  setStatus("ok", `${rows.length} rows`);
}
```

- Export only `init` and `on_timer` (required), `on_click`, `on_params`; put helpers in other files.
- Host calls are synchronous and throw `HostError` (`e.code`): `http`, `httpBatch`, `tickers`, `symbols`,
  `exchanges`, `historyCluster`, `historyReplay`, `kvGet`, `kvSet`, `emitRows`, `replaceRows`, `expire`,
  `alert`, `setStatus`, `openMarket`, `openSpread`, `log`/`debug`/`info`/`warn`/`error`, `nowMs`;
  v1.1 (`min_terminal: 0.104.72`): `signals`, `openMarkets`, `openMarketWithLevel`;
  helpers `Series`, `SignalFeed`, `secs`/`mins`/`hours`, `signed`, `muted`, `toned`, `row`.
- `fetch(url)` works for hosts in manifest `http` (it goes through `http`); `console.log` goes to `st logs`.
  An `async` export is fine: a rejected promise is reported as a failed call.
- Top level runs once when `st build` snapshots the module: a host call there fails the build
  (`move host calls … into init/on_timer`), `Date.now()` there is the build time.
- `Math.random` is a seeded PRNG, not crypto. QuickJS is ~10× slower than Rust: keep per-call work small
  or raise `limits.cpu_ms_per_call`. Example: `examples/volume-spike`.

## Done checklist

- `st build` passes with no errors; `st dev` shows `installed …`. The first install prints
  `running: false, pane: opening`: the plugin starts as soon as the terminal opens the pane, about a
  second later — `st list` then shows `running`.
- `st rows` shows the expected rows and `st logs` has no repeating errors; `st list` status is `running`.
- Rows carry `symbol`/`exchange`/`market` so a click opens the order book; tell the user to click one.
- If the project has `recordings/`, `st test` passes.
