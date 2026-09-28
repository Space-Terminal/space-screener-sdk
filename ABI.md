# Space Terminal screener ABI v1

This is the wire contract between a screener plugin and Space Terminal. The Rust PDK
(`crates/space-screener`) implements the plugin side; you only need this file if you write a
PDK for another language or debug the raw protocol.

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

Extism's own `http_request`, `log_*`, `get_log_level` and any `wasi_*` import are refused. With the
Rust PDK use `extism-pdk = { version = "1.4.1", default-features = false }` (the PDK crate already does).

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
min_terminal: 0.104.70                  # required, semver; an older terminal refuses: terminal_too_old
http: [fapi.binance.com, api.bybit.com] # exact host names http() may call, https only
history: [cluster, replay]              # optional, cloud history access
timer_ms: 60000                         # optional, 250..=3600000, default 1000
feeds: []                               # reserved for v1.1; non-empty in v1 -> unsupported_feed
columns:                                # table columns, in display order
  - {key: symbol, type: symbol, title: {ru: "Тикер", en: "Symbol"}}
  - {key: oi, type: usd, title: {en: "OI, $"}, sort: desc, width: 120}
params:                                 # user-editable parameters
  - {key: min_oi, type: number, title: {en: "Min OI, $"}, default: 5000000, min: 0}
  - {key: side, type: select, title: {en: "Side"}, default: both, options: [both, long, short]}
limits: {memory_mb: 64, cpu_ms_per_call: 250}          # optional
```

- `id` matches `^[a-z0-9][a-z0-9._-]{1,62}[a-z0-9]$` (3..64 chars, starts and ends with a letter or digit),
  is not `install` (a local API route), and no segment between dots is a Windows device name: `con`, `prn`, `aux`, `nul`, `com1`..`com9`,
  `lpt1`..`lpt9` (`ivan.con` is refused, `ivan.console` is fine) — the id is a folder name on every OS.
- `http` hosts are bare DNS names; IP literals and `localhost` are refused.
- The first column with `sort` is the default sort of the pane.
- `pricing`, `hosting`, `alerts` and other fields are ignored in v1.

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
| `on_batch` | — | reserved for v1.1 feeds | never called in v1 |

`params` always carries every declared parameter (user value or `default`).

Calls of one plugin never overlap: a click routed to `on_click` waits while `on_timer` runs. Export
`on_click` only when the click needs custom logic, and keep `on_timer` rounds short; without the export
the terminal opens the row's market at once. The Rust PDK exports `on_click` only with
`export_screener!(T, on_click)`. A panic in the PDK is logged (message and location) before the trap.

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
| `kv_get` | `{key}` | stored value or `null` | — |
| `kv_set` | `{key, value}` (`null` deletes) | `null` | `too_large` |
| `emit_rows` | `{rows: [Row], replace?: bool}` | `null` | — |
| `expire` | `{keys: [string]}` | `null` | — |
| `emit_alert` | `{level: "info"\|"warn"\|"urgent", title, body, row_key?}` | `null` | — |
| `set_status` | `{text, tone: "neutral"\|"ok"\|"warn"\|"error"}` | `null` | — |
| `open_market` | `{exchange, market, symbol}` | `null` | `not_in_click` |
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

### Key-value store

Survives restarts of the plugin and the terminal. The whole store is at most 1 MiB (`too_large`).

### Rows

```json
{"key": "binance:BTCUSDT", "symbol": "BTCUSDT", "exchange": "binance", "market": "futures",
 "cells": {"oi": 7.8e9, "chg5": {"v": 1.2, "tone": "pos"}, "note": {"v": null, "text": "n/a"}},
 "rank": 0, "ttl_s": 120}
```

- `emit_rows` upserts by `key` (≤ 128 chars); `replace: true` replaces the whole table. `expire` removes
  rows. `ttl_s` removes a row that was not re-emitted in time. At most 5000 rows per plugin, each at most
  16 KiB as serialized JSON, 32 MiB of rows in total.
- `symbol`, `exchange`, `market` make the row clickable: the default click opens that market.
- `rank` (default 0): higher ranks stay above lower ones whatever the sort (pins, favourites).
- A cell is a JSON number, string, bool or `null`, or `{"v": value, "tone"?: "pos"|"neg"|"muted"|"warn"|"accent", "text"?: "shown instead of v"}`.
  `exchanges` cells use `{"v": [{"exchange": "bybit", "market": "spot"}]}`.

### Alerts, status, opening markets

- `emit_alert` adds a notification and shows a toast. `info` is a quiet toast without sound; `warn` and
  `urgent` also play the screener alert sound. At most 6 alerts per minute per plugin, the rest is
  dropped with a warning in the log.
- `set_status` sets the short status line of the pane.
- `open_market` / `open_spread` work only inside `on_click`, once per click (`not_in_click` otherwise).
  Plain row clicks need neither: without an `on_click` export the terminal opens the row's market.
  The terminal routes the market to the pane's link group or a new order book, like its own screeners.

## Limits

| Limit | Value |
|---|---|
| Memory | `limits.memory_mb`, at most 64 MB |
| CPU per call | `limits.cpu_ms_per_call` (default 250, at most 1000) of wasm time; time spent inside host functions (HTTP) does not count. `init` gets 2000 ms |
| Wall time per call | 600 s: one call, HTTP waits included, must finish within it; a round of N requests to `fapi.binance.com` takes about N/5 s |
| Violations | 3 CPU overruns in a row stop the plugin (pane status "limit exceeded") |
| After a trap | the terminal recreates the plugin and calls `init` again; in-memory state is lost, `kv` survives |
| `screener.wasm` | at most 10 MiB |
| HTTP | `timeout_ms` 1000..=10000; body ≤ 8 MiB per response, ≤ 48 MiB per `http_batch` |
| Rows | ≤ 5000 rows, ≤ 16 KiB each (serialized), ≤ 32 MiB in total |

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
| `GET /api/v1/screeners` | — | `{ok, screeners: [{id, version, name: {ru, en}, dev, status, status_text?, rows, panes}]}`; status: `running`, `stopped`, `error`, `limit` |
| `POST /api/v1/screeners/install` | `{manifest: "<yaml text>", wasm_b64, open_pane?: bool}` (≤ 32 MiB) | `{ok, id, version, running, pane: "exists"\|"opening"\|"none"}`; 400 `invalid_manifest`, `invalid_wasm`, `forbidden_import`, `forbidden_export`, `unsupported_feed`, `terminal_too_old`, `too_large` |
| `POST /api/v1/screeners/{id}/reload` | — | `{ok, running}` |
| `DELETE /api/v1/screeners/{id}` | — | `{ok}` |
| `GET /api/v1/screeners/{id}/logs` | `?since=<seq>` (pass the previous `next`; 0 = from the start) | `{ok, lines: [{seq, ts_ms, level, msg}], next}` |
| `GET /api/v1/screeners/{id}/rows` | — | `{ok, version, status, status_text?, rows: [Row]}` |

Reinstalling an id restarts its running instance. With `open_pane: true` and no pane showing the
screener, the terminal opens a new tab with it. Unknown ids give 404 `not_found`.
