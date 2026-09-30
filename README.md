# space-screener-sdk

Build screeners for [Space Terminal](https://space-terminal.com) as small WebAssembly plugins.
The terminal runs them in a sandbox, feeds them data (tickers, symbols, HTTP to exchange APIs, cloud
history) and shows their rows as a table pane; a click on a row opens the market's order book.

| Path | What |
|---|---|
| `crates/space-screener` | Rust PDK: ABI types, host calls, `Screener` trait, `export_screener!`, rows/cells, `Series`, open interest helpers for 8 exchanges |
| `crates/st` | CLI: `st init`, `build`, `validate`, `dev`, `logs`, `rows`, `list`, `record`, `test`, `login`, `publish`; the TypeScript PDK (`crates/st/ts/pdk`) and build |
| `crates/space-screener-check` | the rules every plugin is checked with — by the terminal, `st` and the Space Market registry |
| `crates/space-screener-harness` | headless host: replays recordings (`st test`) and runs trial runs (`st publish`, the registry) |
| `examples/oi-8-exchanges` | open interest in USD on Binance, Bybit, OKX, Bitget, Gate, MEXC, KuCoin, Hyperliquid with 5/15 minute change |
| `examples/top-movers` | biggest 5 minute moves from the terminal's tickers |
| `examples/funding-rates` | funding rate, period, APR and next payment on Binance, Bybit, Bitget, Gate, MEXC, Hyperliquid |
| `examples/cross-exchange-spread` | the same pair on different exchanges: cheapest, dearest, gap; a click opens the spread of the two exchanges (two books, chart below), a right click the cheaper leg in the linked book; exchanges picked in the parameters |
| `examples/volume-spike` | TypeScript: turnover of the last minutes against the 24h average |
| `ABI.md` | wire contract (manifest, exports, host functions, limits, local API) |
| `SKILL.md`, `llms.txt` | guides for Claude Code and other LLM agents |

## Quick start

```sh
cargo install --git https://github.com/Space-Terminal/space-screener-sdk st
st init my-screener --id me.my-screener
cd my-screener
st build          # installs the wasm32-unknown-unknown target on first use
st dev            # with Space Terminal running: install, open a pane, reload on save
st rows           # what the pane shows
st logs -f        # plugin log
st record         # restart the plugin in the terminal and save its data to recordings/
st test           # replay recordings headless, compare with the expected rows
```

TypeScript: `st init my-screener --lang ts` (needs Node.js 18+; `st build` fetches extism-js and binaryen
once, checked by sha256).

Publishing to the [Space Market](https://store.space-terminal.com) catalog: add `categories` and a
`description` to `manifest.yaml`, make a publish token on the author page of the store, then

```sh
st login          # paste the token (or set ST_TOKEN)
st publish        # build, check, trial run, upload; a moderator reviews every version
```

`st` finds the terminal through `<data>/screeners/local_api_token` and `<data>/config/general.yaml`,
where `<data>` is `SPACE_TERMINAL_DIR` or the `Space Terminal` folder in the OS data directory.

## Working on the SDK

`cargo install --path crates/st` from a checkout builds `st` that links new projects (`st init`) to that
checkout instead of the git repository; `st init --sdk-path <path>` picks any other copy.

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or <https://www.apache.org/licenses/LICENSE-2.0>)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or <https://opensource.org/licenses/MIT>)

at your option.

Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in the work
by you, as defined in the Apache-2.0 license, shall be dual licensed as above, without any additional
terms or conditions.
