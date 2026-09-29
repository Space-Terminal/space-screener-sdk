# space-screener-sdk

Build screeners for [Space Terminal](https://space-terminal.com) as small WebAssembly plugins.
The terminal runs them in a sandbox, feeds them data (tickers, symbols, HTTP to exchange APIs, cloud
history) and shows their rows as a table pane; a click on a row opens the market's order book.

| Path | What |
|---|---|
| `crates/space-screener` | Rust PDK: ABI types, host calls, `Screener` trait, `export_screener!`, rows/cells, `Series`, open interest helpers for 8 exchanges |
| `crates/st` | CLI: `st init`, `build`, `validate`, `dev`, `logs`, `rows`, `list` |
| `examples/oi-8-exchanges` | open interest in USD on Binance, Bybit, OKX, Bitget, Gate, MEXC, KuCoin, Hyperliquid with 5/15 minute change |
| `examples/top-movers` | biggest 5 minute moves from the terminal's tickers |
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
