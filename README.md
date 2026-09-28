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
cargo install --path crates/st      # from this checkout; st init links new projects to it
st init my-screener --id me.my-screener
cd my-screener
st build          # installs the wasm32-unknown-unknown target on first use
st dev            # with Space Terminal running: install, open a pane, reload on save
st rows           # what the pane shows
st logs -f        # plugin log
```

`st` finds the terminal through `<data>/screeners/local_api_token` and `<data>/config/general.yaml`,
where `<data>` is `SPACE_TERMINAL_DIR` or the `Space Terminal` folder in the OS data directory.
