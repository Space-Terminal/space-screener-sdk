# Space Terminal screener ABI v1

This is the wire contract between a screener plugin and Space Terminal, including the v1.1
additions of terminal 0.104.72 (aggregator signals, opening several markets, smart levels). The Rust PDK
(`crates/space-screener`) and the TypeScript PDK (`crates/st/ts/pdk`, written into TypeScript projects
by `st init --lang ts`) implement the plugin side; you only need this file if you write a PDK for
another language or debug the raw protocol. The rules below are code in `crates/space-screener-check`:
the terminal, `st` and the Space Market registry all check plugins with it.

## Transport

- A plugin is a WebAssembly module built for `wasm32-unknown-unknown` **without WASI**, run by
  the terminal with [Extism](https://extism.org) in a sandbox: no files, no sockets, no clock,
  no threads. Everything goes through the host functions below.
- Every export and every host function exchanges one UTF-8 JSON document through Extism memory.
- Both sides **ignore unknown fields**; a missing optional field means its default. New fields and
  new host functions are added without changing `abi`; a breaking change gets a new `abi` number
  and the terminal keeps supporting the previous one.

### Allowed imports

The terminal rejects a module (`forbidden_import`) that imports anything outside this list.

| Module | Functions |
|---|---|
| `extism:host/env` | `alloc`, `free`, `length`, `length_unsafe`, `load_u8`, `load_u64`, `store_u8`, `store_u64`, `input_length`, `input_load_u8`, `input_load_u64`, `output_set`, `error_set`, `config_get`, `var_get`, `var_set` |
| `extism:host/user` | the host functions of this document |

A host function newer than the plugin's `min_terminal` is refused too (`invalid_manifest`, path
`min_terminal`): `signals` and `open_markets` need `min_terminal: 0.104.72`, since an older terminal
would refuse the whole module. The Rust PDK imports only the host functions a plugin calls; for
TypeScript `st build` declares only those of the manifest's `min_terminal`.

Extism's own `http_request`, `log_*`, `get_log_level` and any `wasi_*` import are refused. With the
Rust PDK use `extism-pdk = { version = "1.4.1", default-features = false }` (the PDK crate already does).

Only functions may be imported (no memories, tables, globals or tags), each with its exact signature,
or the module is refused (`forbidden_import`) — a mismatch would otherwise fail only when the terminal
links it:

- every `extism:host/user` host function: `(i64) -> i64` — an Extism memory offset in, an offset out;
- `extism:host/env` (the Extism 1.30 kernel, as extism-pdk 1.4.1 declares it):

| Signature | Functions |
|---|---|
| `(i64) -> i64` | `alloc`, `length`, `length_unsafe`, `load_u64`, `input_load_u64`, `config_get`, `var_get` |
| `(i64) -> i32` | `load_u8`, `input_load_u8` |
| `(i64) -> ()` | `free`, `error_set` |
| `() -> i64` | `input_length` |
| `(i64, i32) -> ()` | `store_u8` |
| `(i64, i64) -> ()` | `store_u64`, `output_set`, `var_set` |

### Module shape

- The module validates with **WebAssembly 2.0** features plus **tail calls** and **extended constant
  expressions** (they allocate nothing). GC types, threads and shared memory, memory64,
  multi-memory, exceptions, relaxed SIMD and components are refused (`invalid_wasm`).
- At most 20 tables holding at most 50 000 elements in total — tables live outside `limits.memory_mb`.
  A table without a declared maximum counts with its initial size and must never be grown with
  `table.grow`.
- Size and shape of the code — compiling takes memory in proportion to the function being compiled:

| Limit | Value | Error |
|---|---|---|
| `screener.wasm` | ≤ 10 MiB | `too_large` |
| Code section, Space Market catalog only | ≤ 4 MiB (the registry compiles every version for its trial run) | `too_large` |
| Functions (defined in the module) | ≤ 10 000 | `invalid_wasm`, path `functions` |
| Body of one function | ≤ 256 KiB | `invalid_wasm`, path `function N` |
| Nesting of `block` / `loop` / `if` / `try_table` | ≤ 2 000 deep | `invalid_wasm`, path `function N` |
| Targets of one `br_table` | ≤ 10 000 | `invalid_wasm`, path `function N` |

  Real screeners stay far below: a Rust one is 150–300 KiB of code in total, the TypeScript engine
  (QuickJS) about 1 MiB in 1 566 functions, its largest body 50 KiB. A function over the limits is
  usually generated code — a huge `match` or table built inline, or an unrolled chain; move the data
  into a `static` or split the function.

### Refused exports

Nothing may run at instantiation, outside the terminal's call limits: a module with a start section or
an export named `_start`, `_initialize`, `__wasm_call_ctors` or `hs_init` is refused
(`forbidden_export`). A Rust `cdylib` built for `wasm32-unknown-unknown` has none of them.

## Manifest (`manifest.yaml`)

```yaml
abi: 1                                  # required
id: author.oi-8-exchanges               # required, see below
version: 0.1.0                          # required, semver
name: {ru: "Открытый интерес", en: "Open interest"}   # required, at least one language
description: {ru: "...", en: "..."}     # optional
lang: rust                              # required: rust | ts
categories: [open-interest]             # catalog only, see below
source: https://github.com/me/oi        # catalog only, optional
min_terminal: 0.104.72                  # required, semver; an older terminal refuses: terminal_too_old
http: [fapi.binance.com, api.bybit.com] # exact host names http() may call, https only
history: [cluster, replay]              # optional, cloud history access
signals: [density]                      # optional (v1.1): activity | density | prints, see Signals
timer_ms: 60000                         # optional, 250..=3600000, default 1000
feeds: []                               # reserved for trade/order-book streams; non-empty -> unsupported_feed
columns:                                # table columns, in display order
  - {key: symbol, type: symbol, title: {ru: "Тикер", en: "Symbol"}}
  - {key: oi, type: usd, title: {en: "OI, $"}, sort: desc, width: 120}
params:                                 # user-editable parameters
  - {key: min_oi, type: number, title: {en: "Min OI, $"}, default: 5000000, min: 0}
  - {key: side, type: select, title: {en: "Side"}, default: both, options: [both, long, short]}
limits: {memory_mb: 64, cpu_ms_per_call: 250}          # optional; cpu_ms_per_call 50..=1000
```

- `id` matches `^[a-z0-9][a-z0-9._-]{1,62}[a-z0-9]$` (3..64 chars, starts and ends with a letter or digit),
  is not `install` or `sync` (local API routes), and no segment between dots is a Windows device name: `con`, `prn`, `aux`, `nul`, `com1`..`com9`,
  `lpt1`..`lpt9` (`ivan.con` is refused, `ivan.console` is fine) — the id is a folder name on every OS.
- `http` hosts are bare DNS names; IP literals and `localhost` are refused. A host, a column key or a
  parameter key listed twice is refused.
- `signals` lists each source at most once; a non-empty list needs `min_terminal: 0.104.72` or newer.
- `width` of a column is 1..=2000 px.
- The first column with `sort` is the default sort of the pane.
- `pricing`, `hosting`, `alerts` and other fields are ignored in v1.

### Catalog fields

The terminal ignores them; the Space Market registry requires them on publish (`st validate` says
whether the manifest is ready, `st publish` refuses it otherwise):

- `categories`: 1..=3 of `volume`, `open-interest`, `funding`, `spread`, `movers`, `listings`,
  `orderbook`, `other`.
- `description`: required in `ru` or `en`, each at most 2000 characters (it is the catalog card text).
- `source`: optional `https://` link to the source code (host rules as for `http`, no user info).
- `version`: no build metadata (`1.2.0+b1` is refused — it does not order versions).

### Column types

| Type | Cell value | Typical display (the terminal decides) |
|---|---|---|
| `text` | string | as is |
| `number` | number | plain number |
| `integer` | number | rounded |
| `percent` | number, already in percent (`1.5` = 1.5 %) | `+1.50%` |
| `usd` | number, US dollars | `1.2M` |
| `price` | number | price precision |
| `time` | number, ms since Unix epoch | local time |
| `duration` | number, ms | `5m 12s` |
| `countdown` | number, target ms since epoch | time left, ticking |
| `symbol` | string | ticker |
| `exchange` | string, exchange slug | exchange name/logo |
| `exchanges` | `[{exchange, market}]` | exchange badges (S/F) |
| `bool` | bool | check mark |

### Parameter types

`number`, `integer`, `bool`, `text`, `select` (value is one of `options`). `default` is required and
must fit the type; `min`/`max` bound numbers.

## Exports

Each export reads one JSON input and produces no output. Returning an error (Extism `error_set` and a
non-zero status) is logged as an error and counted as a failure.

| Export | Required | Input | When |
|---|---|---|---|
| `init` | yes | `{params: {key: value}, terminal: "0.104.70", lang: "ru"\|"en", now_ms}` | once per (re)start |
| `on_timer` | yes | `{now_ms}` | every `timer_ms`, counted from the end of the previous call (calls never overlap) |
| `on_click` | no | `{row: {key, symbol?, exchange?, market?}, column: string\|null, button: "left"\|"right"\|"middle", modifiers: {shift, ctrl, alt, logo}}` | the user clicked a row; without this export the terminal opens the row's market itself, immediately |
| `on_params` | no | `{params}` | the user changed parameters; without this export the terminal restarts the plugin with a new `init` |
| `on_batch` | — | reserved for `feeds` | never called yet |

`params` always carries every declared parameter (user value or `default`).

Calls of one plugin never overlap: a click routed to `on_click` waits while `on_timer` runs. Export
`on_click` only when the click needs custom logic, and keep `on_timer` rounds short; without the export
the terminal opens the row's market at once. The Rust PDK exports `on_click` only with
`export_screener!(T, on_click)`. A panic in the PDK is logged (message and location) before the trap.

### TypeScript (`lang: ts`)

`st build` bundles `src/index.ts` with esbuild and compiles it with extism-js 1.7.0 (QuickJS). js-pdk
imports WASI and Extism's logging/HTTP; `st` links stubs for them into the module (clock → `now_ms`,
random → a PRNG seeded from `now_ms`, no files or environment) and keeps only `memory` and the entry
points as exports, so the result passes the rules above like a Rust module. Consequences:

- `src/index.ts` exports only `init`, `on_timer`, `on_click`, `on_params` (the first two required).
- The top level of every module runs **once, at build time**, and is snapshotted: calling a host
  function there fails the build, and `Date.now()` there returns the build time.
- `Math.random` and `crypto.getRandomValues` are not cryptographically secure.
- ES2020; QuickJS is roughly ten times slower than Rust, and a module is about 2.4 MB.

## Host functions

Every host function takes one JSON input and **always** returns one JSON object:

```json
{"ok": <value>}
{"err": {"code": "host_not_allowed", "message": "…"}}
```

Errors are data, not traps: the plugin decides what to do. `null` input is used by functions without
arguments.

| Function | Input | `ok` value | Error codes |
|---|---|---|---|
| `http` | `HttpRequest` | `HttpResponse` | `host_not_allowed`, `bad_request`, `forbidden_header`, `timeout`, `transport`, `too_large` |
| `http_batch` | `{requests: [HttpRequest]}` (≤ 1000) | `[{"ok": HttpResponse} \| {"err": {…}}]`, request order | per item as `http` |
| `tickers` | `{exchange, market}` | `{ts_ms, tickers: [{symbol, base, quote, last, change_pct, volume_quote}]}` | `unknown_exchange`, `not_connected`, `unavailable` |
| `symbols` | `{exchange, market}` | `[{symbol, base, quote, trading}]` | `unknown_exchange`, `not_connected` |
| `exchanges` | `null` | `[{exchange, market, connected}]` | — |
| `history_cluster` | `{exchange, symbol, from_ms, to_ms, tf_s, max_cells?}` | `{cells: [{t_ms, price, bid_vol, ask_vol, trades}], truncated}` | `not_permitted`, `quota`, `transport` |
| `history_replay` | `{exchange, symbol, from_ms, to_ms, max_trades?}` | replay chunk (see below) | `not_permitted`, `quota`, `transport` |
| `signals` (v1.1) | `{source: "activity"\|"density"\|"prints", since?: u64}` | `SignalsDelta` (see Signals) | `not_permitted`, `unavailable`, `bad_request` |
| `kv_get` | `{key}` | stored value or `null` | — |
| `kv_set` | `{key, value}` (`null` deletes) | `null` | `too_large` |
| `emit_rows` | `{rows: [Row], replace?: bool}` | `null` | — |
| `expire` | `{keys: [string]}` | `null` | — |
| `emit_alert` | `{level: "info"\|"warn"\|"urgent", title, body, row_key?}` | `null` | — |
| `set_status` | `{text, tone: "neutral"\|"ok"\|"warn"\|"error"}` | `null` | — |
| `open_market` | `{exchange, market, symbol, smart_level?: {signal_id, sound?}}` (`smart_level` v1.1) | `null` | `not_in_click` |
| `open_markets` (v1.1) | `{markets: [MarketRef]}` (1..=16) | `null` | `not_in_click`, `bad_request` |
| `open_spread` | `{a: MarketRef, b: MarketRef, layout?: "vertical"\|"horizontal"}` | `null` | `not_in_click` |
| `log` | `{level: "debug"\|"info"\|"warn"\|"error", msg}` | `null` | — |
| `now_ms` | `null` | ms since Unix epoch | — |

### HTTP

```json
HttpRequest  {"method": "GET" | "POST", "url": "https://fapi.binance.com/fapi/v1/premiumIndex",
              "headers": {"content-type": "application/json"}, "body": "…", "timeout_ms": 10000}
HttpResponse {"status": 200, "headers": {"content-type": "application/json"}, "body": "…"}
```

- Only `https://` URLs whose host is listed in the manifest `http`; otherwise `host_not_allowed`.
  URLs with user info (`https://user:pass@host/…`) are refused (`bad_request`).
- Redirects are not followed: a 3xx response is returned to the plugin as is.
- Headers `authorization`, `cookie`, `host` and `proxy-*` are refused (`forbidden_header`).
- `timeout_ms` is 1000..=10000 (default 10000). A per-host quota waits within that timeout.
- Response bodies are UTF-8 text (lossy), at most 8 MiB each (`too_large`).
- `http_batch` runs up to 8 requests at a time with the same quotas; use it for per-symbol endpoints.
  The bodies of one batch total at most 48 MiB; items past that get `too_large`.
- `fapi.binance.com` is limited to 5 requests/s (its weight limit is shared with the user's trading IP):
  a batch of N requests there takes about N/5 s.
- Traffic goes through the terminal's proxy routing and its own per-host quotas, separate from trading.

### Tickers, symbols, exchanges

- `exchange` is the terminal's exchange slug in lower case: `binance`, `bybit`, `okx`, `bitget`, `gate`,
  `mexc`, `kucoin`, `bingx`, `hyperliquid`, … `exchanges()` lists what the user has connected.
- `market` is `spot` or `futures`.
- `tickers` returns the terminal's latest 24h snapshot; the first call for an exchange may wait up to
  10 s while the terminal fetches it. `change_pct` is in percent, `volume_quote` in the quote currency.
- `tickers` and `symbols` give the terminal's own `symbol` plus `base` and `quote`.

### Symbols in rows and `open_market`

`symbol` of a row, of `open_market` and of `open_spread` legs is either the canonical `BASEQUOTE` in
upper case — `BTCUSDT`, `ETHUSDC` (recommended) — or the exchange's native symbol (`BTC-USDT-SWAP`,
`BTC_USDT`, `XBTUSDTM`). The terminal resolves both against the exchange's symbol list.

### History

Needs `history: [cluster]` / `[replay]` in the manifest (else `not_permitted`); quota 1 request/s per
plugin. `history_cluster` returns footprint cells of the cloud history (7 days). `history_replay` returns
the terminal's cloud replay chunk (`from_ms`, `to_ms`, `next_from_ms`, `snapshots`, `trades`, `events`,
…); its shape follows the terminal and is **not frozen in v1** — treat it as JSON.

### Signals (v1.1)

The terminal keeps a live connection to the Space screener aggregator and shares its signals with
plugins that list the source in `signals` of the manifest (else `not_permitted`). `signals` reads the
terminal's buffers — no network, no quota — so call it every `on_timer`:

```json
SignalsDelta {"seq": 1842, "reset": false, "connected": true,
              "upserts": [Signal], "removed": ["binance|BTCUSDT|bid|60000"]}
```

- Without `since` (or with `0`, a cursor older than the terminal keeps, or after the terminal restarted
  the feed) the reply has `reset: true` and `upserts` is the whole current snapshot. Otherwise
  `upserts` holds the signals changed after `since` (latest version of each) and `removed` the ids
  dropped after it. An id is never in both `upserts` and `removed` of one reply. Pass `seq` as the
  next `since`. `SignalFeed` (Rust and TypeScript PDK) does this.
- `connected`: the terminal's connection to the aggregator for this source is alive — data or a
  server keepalive within the last 90 s. A sparse source (`activity`) can stay silent for long
  while `connected` is `true`.
- `unavailable`: this build of the terminal does not provide the source (`activity` and `prints` come
  with the Pro build).
- A density snapshot can hold up to ~20000 signals: filter before `emit_rows` (10000 rows at most).

```json
Signal {"id": "…", "source": "density", "ts_ms": 1790700000000, "symbol": "BTCUSDT",
        "exchanges": [{"exchange": "binance", "market": "futures"}],
        "link": "…", "expires_at_ms": 1790700300000,
        "density": {…}}
```

Exactly one of `activity`, `density`, `prints` is set, matching `source`. `symbol` is the canonical
`BASEQUOTE`; `exchanges` uses the terminal's exchange slugs (exchanges it does not know are left out);
`link` and `expires_at_ms` are optional. Numbers are JSON numbers, for display:

| Source | Fields |
|---|---|
| `activity` | `tags` (`yorsh`, `non_yorsh`, `unique_ticker`, `flat`, `has_futures`), `spread_pct`, `volume_per_min_usd`, `pnl_per_min_usd`; optional `trades_per_min`, `liquidity_up_10pct_usd`, `liquidity_down_10pct_usd`, `token_age_days`, `print_gaps` |
| `density` | `exchange`, `market`, `side` (`bid`/`ask`), `price`, `qty`, `notional_initial_usd`, `notional_current_usd`, `eaten_pct`, `distance_pct` (signed), `touch_count`, `lifetime_s`, `status` (`alive`, `reduced`, `dead`), `event` (`new`, `update`, `touched`, `reduced`, `dead`, `reappeared`); optional `notional_avg_usd`, `prev_lifetime_s`, `trade_qty` |
| `prints` | `exchange`, `market`, `side` (`buy`/`sell`, the aggressor), `volume_usd`, `batches`, `prints_per_batch` |

New tags, statuses and events may appear: treat unknown values as "other".

### Key-value store

Survives restarts of the plugin and the terminal. The whole store is at most 1 MiB (`too_large`).

### Rows

```json
{"key": "binance:BTCUSDT", "symbol": "BTCUSDT", "exchange": "binance", "market": "futures",
 "cells": {"oi": 7.8e9, "chg5": {"v": 1.2, "tone": "pos"}, "note": {"v": null, "text": "n/a"}},
 "rank": 0, "ttl_s": 120}
```

- `emit_rows` upserts by `key` (≤ 128 chars); `replace: true` replaces the whole table. `expire` removes
  rows. `ttl_s` removes a row that was not re-emitted in time. Each row is at most 16 KiB as serialized
  JSON. At most 10000 rows or 32 MiB per plugin: beyond either cap the host evicts the least recently updated rows, so `replace: true` with more rows keeps the last ones of the batch. Sort and truncate in the plugin so the rows you want are the ones kept.
- `symbol`, `exchange`, `market` make the row clickable: the default click opens that market.
- `rank` (default 0): higher ranks stay above lower ones whatever the sort — the plugin's own order
  (pins of its data, a blacklist at the bottom). Favourite tickers are the terminal's: from 0.104.73 it
  shows its own ★ in every screener and keeps favourites above all other rows, so a plugin needs no
  favourites column of its own.
- A cell is a JSON number, string, bool or `null`, or `{"v": value, "tone"?: "pos"|"neg"|"muted"|"warn"|"accent", "text"?: "shown instead of v"}`.
  `exchanges` cells use `{"v": [{"exchange": "bybit", "market": "spot"}]}`.

### Alerts, status, opening markets

- `emit_alert` adds a notification and shows a toast. `info` is a quiet toast without sound; `warn` and
  `urgent` also play the screener alert sound. At most 6 alerts per minute per plugin, the rest is
  dropped with a warning in the log.
- `set_status` sets the short status line of the pane.
- `open_market` / `open_markets` / `open_spread` work only inside `on_click`, once per click in total
  (`not_in_click` otherwise).
- `open_markets` (v1.1) opens 1..=16 markets at once, like a click on a multi-exchange row; a pane in a
  link group sends the first market to the group.
- `smart_level` of `open_market` (v1.1) also places a smart level in the opened order book at the price
  of the density signal `signal_id` (the terminal takes the exact price and live metrics from its own
  buffer; an unknown id opens the market without a level). `sound` alerts on touches and fills.
  A terminal older than 0.104.72 ignores `smart_level` and opens the market without a level.
  Plain row clicks need neither: without an `on_click` export the terminal opens the row's market.
  The terminal routes the market to the pane's link group or a new order book, like its own screeners.

## Limits

| Limit | Value |
|---|---|
| Memory | `limits.memory_mb`, at most 64 MB |
| CPU per call | `limits.cpu_ms_per_call` (default 250, allowed 50–1000) of the plugin thread's CPU time for the whole call, wasm plus host work done for it (JSON, kv, rows); waiting for the network costs nothing. Once the budget is spent, the next host call aborts the call. A pure wasm loop with no host calls is cancelled after `max(5 × budget, 5 s)` wall time outside host functions. Going over the budget, an abort, a runaway cancel and the 600 s wall timeout each count as a violation; 3 violations in the last 10 calls stop the plugin (`limit`). A call that used at least 50 % of its budget writes `cpu … ms (… ms in host functions) of … ms, wall … ms` to the screener log. |
| CPU share | besides the per-call budget, a plugin may use at most 30 s of CPU (half a core) over any sliding 60 s window, counted from the full CPU of every call that ended in it, including aborted and cancelled ones; above that the plugin stops with status `limit` ("average CPU over 50 % of a core"). |
| Wall time per call | 600 s: one call, HTTP waits included, must finish within it; a round of N requests to `fapi.binance.com` takes about N/5 s |
| After a trap | the terminal recreates the plugin and calls `init` again; in-memory state is lost, `kv` survives |
| `screener.wasm` | at most 10 MiB |
| HTTP | `timeout_ms` 1000..=10000; body ≤ 8 MiB per response, ≤ 48 MiB per `http_batch` |
| Rows | ≤ 16 KiB each (serialized); ≤ 10000 rows or 32 MiB per plugin, beyond either cap the least recently updated rows are evicted |

## Local API for tooling (`st`)

Space Terminal serves a local HTTP API (default port 5055). The screener routes require
`Authorization: Bearer <token>`; the token (64 hex chars) is in `<data>/screeners/local_api_token`,
created by the terminal on start. `<data>` is `SPACE_TERMINAL_DIR` or the OS data folder
`Space Terminal` (macOS `~/Library/Application Support/Space Terminal`, Windows `%APPDATA%\Space Terminal`,
Linux `~/.local/share/Space Terminal`). The port is `SPACE_TERMINAL_PORT`, else
`general.local_api.port` in `<data>/config/general.yaml`, else 5055.

Errors are `{ok: false, error: <code>, message}`; a missing or wrong token gives HTTP 401 `unauthorized`.

| Route | Body / query | Response |
|---|---|---|
| `GET /api/v1/screeners` | — | `{ok, screeners: [{id, version, name: {ru, en}, dev, source, status, status_text?, rows, panes}]}`; `source`: `dev` (installed by `st`) or `catalog` (from the Space Market library); status: `running`, `stopped`, `error`, `limit` |
| `POST /api/v1/screeners/install` | `{manifest: "<yaml text>", wasm_b64, open_pane?: bool}` (≤ 32 MiB) | `{ok, id, version, running, pane: "exists"\|"opening"\|"none"}`; 400 `invalid_manifest`, `invalid_wasm`, `forbidden_import`, `forbidden_export`, `unsupported_feed`, `terminal_too_old`, `too_large` |
| `POST /api/v1/screeners/{id}/reload` | — | `{ok, running}` |
| `DELETE /api/v1/screeners/{id}` | — | `{ok}` |
| `GET /api/v1/screeners/{id}/logs` | `?since=<seq>` (pass the previous `next`; 0 = from the start) | `{ok, lines: [{seq, ts_ms, level, msg}], next}` |
| `GET /api/v1/screeners/{id}/rows` | — | `{ok, version, status, status_text?, rows: [Row]}` |
| `POST /api/v1/screeners/{id}/record` | `{seconds}` (≤ 600, default 60) | `{ok, state: "recording", remaining_ms}`; restarts the plugin and records its run from `init`; 409 `not_running` (no pane shows it) |
| `GET /api/v1/screeners/{id}/record` | — | `{ok, state: "recording"\|"done"\|"none", remaining_ms?, recording?}`; `recording` (below) once `done` |

Reinstalling an id restarts its running instance. With `open_pane: true` and no pane showing the
screener, the terminal opens a new tab with it. Unknown ids give 404 `not_found`.

`POST /api/v1/screeners/sync` (optional `?open=1`) needs no token and answers browsers (CORS): the
Space Market site calls it after "Add" so that a terminal with a linked account syncs its library at
once instead of within a minute. It carries no ids or files; with `open=1` the terminal opens a pane for
plugins that this sync installs for the first time.

## Recordings (`st record`, `st test`)

A recording is one run of a plugin in the terminal: the calls the terminal made and the replies of the
data host functions. `st test` replays it headless (`crates/space-screener-harness`) on the recorded
clock and compares rows, alerts, status and clicks with `<recording>.expected.json`.

```json
{"v": 1, "id": "me.oi", "version": "0.1.0", "terminal": "0.104.71", "lang": "en",
 "params": {"min_oi": 5000000}, "kv": {"seen": ["BTCUSDT"]}, "started_ms": 1790700000000,
 "truncated": false,
 "events": [
   {"kind": "init",  "t_ms": 1790700000000, "input": {"params": {…}, "terminal": "0.104.71", "lang": "en", "now_ms": 1790700000000}},
   {"kind": "call",  "t_ms": 1790700000012, "fn": "tickers", "input": {"exchange": "binance", "market": "futures"}, "output": {"ok": {…}}},
   {"kind": "timer", "t_ms": 1790700060000, "input": {"now_ms": 1790700060000}},
   {"kind": "click", "t_ms": 1790700061000, "input": {"row": {"key": "BTCUSDT"}, "column": null, "button": "left", "modifiers": {…}}}]}
```

- Recorded functions: `http`, `http_batch`, `tickers`, `symbols`, `exchanges`, `history_cluster`,
  `history_replay`, `signals`, `now_ms`. Local ones (kv, rows, alerts, status, log) run for real on replay.
- Replay answers a call with the first unused recorded call of the same function and the same JSON
  input; a call not in the recording gets `{"err": {"code": "transport", "message": "not recorded"}}`.
- `kv` is the plugin's store at `init`; `truncated` means the recording hit the terminal's 32 MiB cap.

## Publishing to the Space Market catalog

`st publish` sends `POST /api/v1/registry/publish` with `Authorization: Bearer stp_…` (a publish token
from the author page of the store) and `{manifest, wasm_b64, recording?}`:

- The registry checks the manifest, the module and the catalog fields with the rules above, then runs
  the plugin headless (`init` and three `on_timer`, 20 s of wall time at most) on the recording, or on a
  small offline data set without network when there is none (`signals` answers an empty snapshot there). The report (verdict ok/warn/fail, issues,
  sample rows, log lines ≤ 4096 bytes each, ≤ 256 KiB in total) goes to the moderator.
- Limits: manifest ≤ 64 KiB, module ≤ 10 MiB with a code section ≤ 4 MiB, recording ≤ 4 MiB; at most 3 versions of one screener
  and 10 of one author waiting for moderation (`too_many_pending`).
- An id belongs to the first author who publishes it (`id_taken`); versions only grow (`version_exists`,
  `version_not_greater`).
- Every version waits for a moderator. An approved version is signed with the registry key
  (Ed25519); terminals install only signed versions and check the signature and the SHA-256 of the
  files before every load.
- A terminal installs the version its account chose on the site, or else the newest approved stable
  version (a pre-release only when pinned or when there is no stable one). A revoked version or
  screener is removed from terminals at the next library sync.
