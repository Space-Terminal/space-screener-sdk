use std::collections::HashMap;

use space_screener::Market;
use space_screener::oi::{
    OpenInterest, binance, bitget, bybit, gate, hyperliquid, kucoin, mexc, okx,
};

fn fixture(name: &str) -> String {
    let path = format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"));
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"))
}

fn by_symbol(items: Vec<OpenInterest>) -> HashMap<String, OpenInterest> {
    items.into_iter().map(|i| (i.symbol.clone(), i)).collect()
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() <= b.abs() * 1e-9
}

#[test]
fn binance_keeps_trading_usdt_perpetuals_and_prices_them() {
    let symbols = binance::parse_exchange_info(&fixture("binance_exchange_info.json")).unwrap();
    let names: Vec<&str> = symbols.iter().map(|s| s.symbol.as_str()).collect();
    assert_eq!(names, ["BTCUSDT", "ETHUSDT"]);

    let days = binance::parse_ticker_24h(&fixture("binance_ticker_24hr.json")).unwrap();
    let top: Vec<&str> = binance::select_top(&symbols, &days, 1)
        .iter()
        .map(|s| s.symbol.as_str())
        .collect();
    assert_eq!(top, ["BTCUSDT"]);
    assert_eq!(binance::select_top(&symbols, &days, 600).len(), 2);

    let oi = binance::parse_open_interest(&fixture("binance_open_interest.json")).unwrap();
    assert_eq!(oi, ("BTCUSDT".to_string(), 93294.408));

    let rows = by_symbol(binance::assemble(&symbols, &days, [oi]));
    let btc = &rows["BTCUSDT"];
    assert_eq!((btc.base.as_str(), btc.quote.as_str()), ("BTC", "USDT"));
    assert_eq!(btc.native_symbol, "BTCUSDT");
    assert!(close(btc.oi_usd, 93294.408 * 83980.40));
    assert_eq!(btc.market_ref().market, Market::Futures);
}

#[test]
fn bybit_uses_single_side_value_and_skips_dated_and_usdc() {
    let rows = by_symbol(bybit::parse(&fixture("bybit_tickers.json")).unwrap());
    assert_eq!(rows.len(), 2);
    assert!(close(rows["BTCUSDT"].oi_usd, 2377366972.29));
    assert_eq!(rows["ETHUSDT"].base, "ETH");
}

#[test]
fn okx_keeps_usdt_swaps_with_usd_value() {
    let rows = by_symbol(okx::parse(&fixture("okx_open_interest.json")).unwrap());
    assert_eq!(rows.len(), 2);
    let btc = &rows["BTCUSDT"];
    assert_eq!(btc.native_symbol, "BTC-USDT-SWAP");
    assert!(close(btc.oi_usd, 2355939646.20393784318205));
    assert!(btc.price > 83_000.0 && btc.price < 84_000.0);
}

#[test]
fn bitget_multiplies_coins_by_mark() {
    let rows = by_symbol(bitget::parse(&fixture("bitget_tickers.json")).unwrap());
    assert!(close(rows["BTCUSDT"].oi_usd, 32741.626499999951 * 83919.3));
}

#[test]
fn gate_uses_one_side_position_size_and_skips_suspended() {
    let rows = by_symbol(gate::parse(&fixture("gate_contracts.json")).unwrap());
    assert_eq!(rows.len(), 2);
    assert!(close(
        rows["BTCUSDT"].oi_usd,
        263560370.0 * 0.0001 * 83896.34
    ));
    assert_eq!(rows["BTCUSDT"].native_symbol, "BTC_USDT");
}

#[test]
fn mexc_halves_hold_volume_and_needs_contract_size() {
    let sizes = mexc::parse_contract_sizes(&fixture("mexc_detail.json")).unwrap();
    assert_eq!(sizes.len(), 2);
    let rows = by_symbol(mexc::parse(&fixture("mexc_ticker.json"), &sizes).unwrap());
    assert_eq!(rows.len(), 2);
    assert!(close(
        rows["BTCUSDT"].oi_usd,
        515344711.0 * 0.0001 * 83893.6 / 2.0
    ));
}

#[test]
fn kucoin_normalizes_xbt_and_skips_inverse_and_usdc() {
    let rows = by_symbol(kucoin::parse(&fixture("kucoin_contracts.json")).unwrap());
    assert_eq!(rows.len(), 2);
    let btc = &rows["BTCUSDT"];
    assert_eq!(btc.native_symbol, "XBTUSDTM");
    assert!(close(btc.oi_usd, 10335648.0 * 0.001 * 83895.52));
}

// Synthetic fixture: shape from the Hyperliquid docs and the terminal's types, not a live capture.
#[test]
fn hyperliquid_aligns_contexts_and_quotes_in_usdc() {
    let rows =
        by_symbol(hyperliquid::parse(&fixture("hyperliquid_meta_and_asset_ctxs.json")).unwrap());
    assert_eq!(rows.len(), 3);
    assert!(close(rows["BTCUSDC"].oi_usd, 25000.5 * 83890.0));
    assert_eq!(rows["BTCUSDC"].native_symbol, "BTC");
    assert_eq!(rows["KPEPEUSDC"].native_symbol, "kPEPE");
}

#[test]
fn error_payloads_are_reported() {
    assert!(bybit::parse(r#"{"retCode":10001,"retMsg":"bad"}"#).is_err());
    assert!(okx::parse(r#"{"code":"50011","msg":"rate","data":[]}"#).is_err());
    assert!(kucoin::parse(r#"{"code":"429000","data":[]}"#).is_err());
    assert!(mexc::parse_contract_sizes(r#"{"success":false,"code":510}"#).is_err());
}
