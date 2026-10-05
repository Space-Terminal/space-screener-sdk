import {
  Series,
  alert,
  bool,
  exchanges,
  mins,
  num,
  replaceRows,
  setStatus,
  signed,
  str,
  tickers,
  warn,
} from "@space-terminal/screener";
import type { InitInput, Market, ParamsInput, Row, TimerInput } from "@space-terminal/screener";

// Top-level code runs once at build time: only declarations here.

const DAY_MINUTES = 24 * 60;
const FORGET_AFTER_MS = mins(10);
/**
 * History keeps a point per tenth of the window (at most 6 min), not per timer: tens of thousands
 * of tickers must fit the plugin memory. Growth is measured against the point at or just before
 * the window start, so the window comes out up to a tenth longer than the average it is compared
 * with.
 */
const SAMPLE_MAX_MS = mins(6);

/** Fields of a spiking ticker, copied out so the exchange's snapshot can go after its loop. */
interface Spike {
  ratio: number;
  key: string;
  exchange: string;
  symbol: string;
  recent: number;
  volume: number;
  change: number;
  last: number;
}

/** Rolling 24h quote volume of every symbol, sampled on each timer. */
let volumes = new Map<string, Series>();
/** Row keys above the threshold on the previous round, so an alert fires once per spike. */
let spiking = new Set<string>();
let startedMs = 0;
/**
 * Keys with a computed growth on the previous round. A ticker alerts only from its second computed
 * round: the first one after a warm-up or a market switch would otherwise alert everything that is
 * already spiking at once.
 */
let computed = new Set<string>();
/** Window the series are sized for: a longer one needs a longer history, so it starts over. */
let seriesWindowMin = 0;
/** Market the warm-up countdown belongs to: switching it starts the countdown over. */
let countdownMarket: Market = "futures";

function windowMin(): number {
  return Math.max(1, num("window_min", 5));
}

function marketParam(): Market {
  return str("market", "futures") === "spot" ? "spot" : "futures";
}

function restartCountdown(nowMs: number) {
  startedMs = nowMs;
  countdownMarket = marketParam();
}

function reset(nowMs: number) {
  volumes = new Map();
  spiking = new Set();
  computed = new Set();
  seriesWindowMin = windowMin();
  restartCountdown(nowMs);
}

export function init(input: InitInput) {
  reset(input.now_ms);
}

/**
 * Other parameters keep the collected volumes: the series are keyed by market too. A new market
 * only starts its own warm-up countdown.
 */
export function on_params(_input: ParamsInput) {
  if (windowMin() !== seriesWindowMin) reset(0);
  else if (marketParam() !== countdownMarket) restartCountdown(0);
}

/**
 * Exchanges report only a rolling 24h volume V. Its growth over a window W is the turnover of
 * the last W minus the turnover of the same W a day ago; adding the average W turnover (V / (24h / W))
 * estimates the last W alone. ratio = estimate / average.
 */
export function on_timer(input: TimerInput) {
  const now = input.now_ms;
  if (startedMs === 0) startedMs = now;
  const market = marketParam();
  const windowMs = mins(windowMin());
  const sampleMs = Math.min(SAMPLE_MAX_MS, windowMs / 10);
  // A point at or before the window start must survive: the window plus two samples, and at least
  // a minute of slack for timer and snapshot jitter.
  const horizonMs = windowMs + Math.max(2 * sampleMs, mins(1));
  const threshold = num("threshold", 3);
  const minVolume = num("min_volume", 5_000_000);
  const limit = Math.max(1, num("limit", 100));

  // Rows are built only for the spikes that make the table: a low threshold on a large market
  // marks tens of thousands of tickers.
  const spikes: Spike[] = [];
  const next = new Set<string>();
  const nextComputed = new Set<string>();
  let sources = 0;
  for (const ex of exchanges()) {
    if (!ex.connected || ex.market !== market) continue;
    let snapshot;
    try {
      snapshot = tickers(ex.exchange, market);
    } catch (e) {
      warn(`${ex.exchange} ${market}: ${e}`);
      continue;
    }
    sources++;
    const ts = snapshot.ts_ms > 0 ? snapshot.ts_ms : now;
    for (const t of snapshot.tickers) {
      // Only liquid tickers keep a history: spot lists reach tens of thousands of symbols, and a
      // series for each of them would not fit the plugin memory.
      if (t.volume_quote < minVolume) continue;
      const key = `${ex.exchange}:${market}:${t.symbol}`;
      let series = volumes.get(key);
      if (!series) {
        series = new Series(horizonMs);
        volumes.set(key, series);
      }
      const past = series.valueAt(ts - windowMs);
      const lastTs = series.lastTs();
      if (lastTs === undefined || ts - lastTs >= sampleMs) series.push(ts, t.volume_quote);
      if (past === undefined) continue;
      nextComputed.add(key);
      const growth = t.volume_quote - past;
      const average = (t.volume_quote * windowMs) / mins(DAY_MINUTES);
      if (average <= 0) continue;
      const recent = Math.max(0, growth + average);
      const ratio = recent / average;
      if (ratio < threshold) continue;
      next.add(key);
      spikes.push({
        ratio,
        key,
        exchange: ex.exchange,
        symbol: t.symbol,
        recent,
        volume: t.volume_quote,
        change: t.change_pct,
        last: t.last,
      });
    }
  }
  for (const [key, series] of volumes) {
    const last = series.lastTs();
    if (last === undefined || now - last > FORGET_AFTER_MS) volumes.delete(key);
  }

  spikes.sort((a, b) => b.ratio - a.ratio);
  const shown = spikes.slice(0, limit);
  const warming = windowMs - (now - startedMs);
  // Only rows that made the table, once per spike.
  if (bool("alerts")) {
    for (const spike of shown) {
      if (spiking.has(spike.key) || !computed.has(spike.key)) continue;
      alert(
        "warn",
        `${spike.symbol} ${spike.exchange}`,
        `turnover ×${spike.ratio.toFixed(1)} of the 24h average`,
        spike.key,
      );
    }
  }
  computed = nextComputed;
  spiking = next;
  const rows = shown.map((spike): Row => ({
    key: spike.key,
    exchange: spike.exchange,
    market,
    symbol: spike.symbol,
    cells: {
      symbol: spike.symbol,
      exchange: spike.exchange,
      ratio: { v: Math.round(spike.ratio * 10) / 10, tone: "accent" },
      recent: spike.recent,
      volume: spike.volume,
      change: signed(spike.change),
      last: spike.last,
    },
    ttl_s: 60,
  }));
  if (sources === 0) setStatus("warn", `no connected ${market} exchanges`);
  else if (warming > 0) setStatus("neutral", `collecting volume history: ${Math.ceil(warming / 1000)} s left`);
  else setStatus("ok", `${sources} exchange${sources === 1 ? "" : "s"}, ${rows.length} spikes`);
  replaceRows(rows);
}
