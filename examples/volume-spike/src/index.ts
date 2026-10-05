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
 * History keeps a point per this interval, not per timer: tens of thousands of tickers must fit
 * the plugin memory. Growth is measured against the point at or just before the window start, so
 * the window is up to this much longer.
 */
const SAMPLE_MS = 30_000;

/** Rolling 24h quote volume of every symbol, sampled on each timer. */
let volumes = new Map<string, Series>();
/** Row keys above the threshold on the previous round, so an alert fires once per spike. */
let spiking = new Set<string>();
let startedMs = 0;
/** Window the series are sized for: a longer one needs a longer history, so it starts over. */
let seriesWindowMin = 0;

function windowMin(): number {
  return Math.max(1, num("window_min", 5));
}

function reset(nowMs: number) {
  volumes = new Map();
  spiking = new Set();
  startedMs = nowMs;
  seriesWindowMin = windowMin();
}

export function init(input: InitInput) {
  reset(input.now_ms);
}

/** Other parameters keep the collected volumes: the series are keyed by market too. */
export function on_params(_input: ParamsInput) {
  if (windowMin() !== seriesWindowMin) reset(0);
}

/**
 * Exchanges report only a rolling 24h volume V. Its growth over a window W is the turnover of
 * the last W minus the turnover of the same W a day ago; adding the average W turnover (V / (24h / W))
 * estimates the last W alone. ratio = estimate / average.
 */
export function on_timer(input: TimerInput) {
  const now = input.now_ms;
  if (startedMs === 0) startedMs = now;
  const market: Market = str("market", "futures") === "spot" ? "spot" : "futures";
  const windowMs = mins(windowMin());
  const threshold = num("threshold", 3);
  const minVolume = num("min_volume", 5_000_000);
  const limit = Math.max(1, num("limit", 100));

  const spikes: [number, Row][] = [];
  const next = new Set<string>();
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
        series = new Series(windowMs + mins(2));
        volumes.set(key, series);
      }
      const past = series.valueAt(ts - windowMs);
      const lastTs = series.lastTs();
      if (lastTs === undefined || ts - lastTs >= SAMPLE_MS) series.push(ts, t.volume_quote);
      if (past === undefined) continue;
      const growth = t.volume_quote - past;
      const average = (t.volume_quote * windowMs) / mins(DAY_MINUTES);
      if (average <= 0) continue;
      const recent = Math.max(0, growth + average);
      const ratio = recent / average;
      if (ratio < threshold) continue;
      next.add(key);
      if (bool("alerts") && !spiking.has(key)) {
        alert("warn", `${t.symbol} ${ex.exchange}`, `turnover ×${ratio.toFixed(1)} of the 24h average`, key);
      }
      spikes.push([
        ratio,
        {
          key,
          exchange: ex.exchange,
          market,
          symbol: t.symbol,
          cells: {
            symbol: t.symbol,
            exchange: ex.exchange,
            ratio: { v: Math.round(ratio * 10) / 10, tone: "accent" },
            recent,
            volume: t.volume_quote,
            change: signed(t.change_pct),
            last: t.last,
          },
          ttl_s: 60,
        },
      ]);
    }
  }
  for (const [key, series] of volumes) {
    const last = series.lastTs();
    if (last === undefined || now - last > FORGET_AFTER_MS) volumes.delete(key);
  }
  spiking = next;

  spikes.sort((a, b) => b[0] - a[0]);
  const rows = spikes.slice(0, limit).map(([, row]) => row);
  const warming = windowMs - (now - startedMs);
  if (sources === 0) setStatus("warn", `no connected ${market} exchanges`);
  else if (warming > 0) setStatus("neutral", `collecting volume history: ${Math.ceil(warming / 1000)} s left`);
  else setStatus("ok", `${sources} exchange${sources === 1 ? "" : "s"}, ${rows.length} spikes`);
  replaceRows(rows);
}
