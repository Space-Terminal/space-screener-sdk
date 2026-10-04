// @space-terminal/screener — TypeScript PDK for Space Terminal screener plugins (ABI v1.1).
//
// `st build` bundles src/index.ts with this file and compiles it to screener.wasm with extism-js
// (QuickJS). The top level of every module runs ONCE, at build time, and is snapshotted: keep
// only declarations and assignments there. Host functions, Date.now() and Math.random() belong
// in init / on_timer / on_click / on_params. See ABI.md in the SDK repository for the contract.

declare const Host: {
  getFunctions(): Record<string, (ptr: bigint | number) => any>;
  inputString(): string;
};
declare const Memory: {
  fromString(s: string): { offset: any; free(): void };
  find(offset: any): { readString(): string; free(): void };
};

export type Market = "spot" | "futures";
export type Lang = "ru" | "en";
export type Tone = "pos" | "neg" | "muted" | "warn" | "accent";
export type AlertLevel = "info" | "warn" | "urgent";
export type StatusTone = "neutral" | "ok" | "warn" | "error";
export type LogLevel = "debug" | "info" | "warn" | "error";
export type ParamValue = number | string | boolean;
export type Params = Record<string, ParamValue>;

export interface MarketRef {
  exchange: string;
  market: Market;
  symbol: string;
}
export interface ExchangeMarket {
  exchange: string;
  market: Market;
}
export interface ExchangeInfo {
  exchange: string;
  market: Market;
  connected: boolean;
}
export interface Ticker {
  symbol: string;
  base: string;
  quote: string;
  last: number;
  change_pct: number;
  volume_quote: number;
}
export interface TickerSnapshot {
  ts_ms: number;
  tickers: Ticker[];
}
export interface SymbolInfo {
  symbol: string;
  base: string;
  quote: string;
  trading: boolean;
}
export interface HttpRequest {
  method?: "GET" | "POST";
  url: string;
  headers?: Record<string, string>;
  body?: string;
  timeout_ms?: number;
}
export interface HttpResponse {
  status: number;
  headers: Record<string, string>;
  body: string;
}
export type HostResult<T> = { ok: T } | { err: { code: string; message: string } };
export interface ClusterRequest {
  exchange: string;
  symbol: string;
  from_ms: number;
  to_ms: number;
  tf_s: number;
  max_cells?: number;
}
export interface ClusterCell {
  t_ms: number;
  price: number;
  bid_vol: number;
  ask_vol: number;
  trades: number;
}
export interface ClusterHistory {
  cells: ClusterCell[];
  truncated: boolean;
}
export interface ReplayRequest {
  exchange: string;
  symbol: string;
  from_ms: number;
  to_ms: number;
  max_trades?: number;
}

export type SignalSource = "activity" | "density" | "prints";
export interface ActivitySignal {
  /** yorsh, non_yorsh, unique_ticker, flat, has_futures */
  tags: string[];
  spread_pct: number;
  volume_per_min_usd: number;
  pnl_per_min_usd: number;
  trades_per_min?: number;
  liquidity_up_10pct_usd?: number;
  liquidity_down_10pct_usd?: number;
  token_age_days?: number;
  print_gaps?: number[];
}
export interface DensitySignal {
  exchange: string;
  market: Market;
  side: "bid" | "ask";
  price: number;
  qty: number;
  notional_initial_usd: number;
  notional_current_usd: number;
  notional_avg_usd?: number;
  eaten_pct: number;
  distance_pct: number;
  touch_count: number;
  lifetime_s: number;
  /** alive, reduced, dead */
  status: string;
  /** new, update, touched, reduced, dead, reappeared */
  event: string;
  prev_lifetime_s?: number;
  trade_qty?: number;
}
export interface PrintsSignal {
  exchange: string;
  market: Market;
  side: "buy" | "sell";
  volume_usd: number;
  batches: number;
  prints_per_batch: number;
}
/** One aggregator signal; exactly one of activity / density / prints is set. */
export interface Signal {
  id: string;
  source: SignalSource;
  ts_ms: number;
  symbol: string;
  exchanges: ExchangeMarket[];
  link?: string;
  expires_at_ms?: number;
  activity?: ActivitySignal;
  density?: DensitySignal;
  prints?: PrintsSignal;
}
export interface SignalsDelta {
  /** Pass it as `since` next time. */
  seq: number;
  /** `upserts` is the whole current snapshot: drop everything else. */
  reset: boolean;
  connected: boolean;
  upserts: Signal[];
  removed: string[];
}
export interface SmartLevel {
  signal_id: string;
  sound?: boolean;
}

export type CellValue = number | string | boolean | null | ExchangeMarket[];
export type Cell = CellValue | { v: CellValue; tone?: Tone; text?: string };
export interface Row {
  key: string;
  symbol?: string;
  exchange?: string;
  market?: Market;
  cells: Record<string, Cell>;
  /** Higher ranks stay above lower ones whatever the sort (default 0). */
  rank?: number;
  /** Remove the row when it is not emitted again within this many seconds. */
  ttl_s?: number;
}

export interface InitInput {
  params: Params;
  terminal: string;
  lang: Lang;
  now_ms: number;
}
export interface TimerInput {
  now_ms: number;
}
export interface ClickInput {
  row: { key: string; symbol?: string; exchange?: string; market?: Market };
  column: string | null;
  button: "left" | "right" | "middle";
  modifiers: { shift: boolean; ctrl: boolean; alt: boolean; logo: boolean };
}
export interface ParamsInput {
  params: Params;
}

/** A host function answered `{"err": {code, message}}`: `host_not_allowed`, `timeout`, … */
export class HostError extends Error {
  constructor(
    public readonly code: string,
    message: string,
  ) {
    super(`${code}: ${message}`);
    this.name = "HostError";
  }
}

function call<T>(name: string, input: unknown): T {
  const fn = Host.getFunctions()[name];
  // `st build` declares only the host functions of the manifest's min_terminal.
  if (!fn) throw new HostError("unavailable", `${name} needs a newer min_terminal in manifest.yaml`);
  const arg = Memory.fromString(JSON.stringify(input ?? null));
  const out = Memory.find(fn(arg.offset));
  arg.free();
  const reply = JSON.parse(out.readString());
  out.free();
  if (reply.err) throw new HostError(reply.err.code, reply.err.message);
  return reply.ok as T;
}

/** HTTPS request to a host listed in `http` of manifest.yaml. Non-2xx statuses are not errors. */
export const http = (req: HttpRequest): HttpResponse => call("http", { method: "GET", ...req });
/** Up to 1000 requests, 8 in flight; results keep the request order. */
export const httpBatch = (requests: HttpRequest[]): HostResult<HttpResponse>[] =>
  call("http_batch", { requests: requests.map((r) => ({ method: "GET", ...r })) });
export const tickers = (exchange: string, market: Market): TickerSnapshot =>
  call("tickers", { exchange, market });
export const symbols = (exchange: string, market: Market): SymbolInfo[] =>
  call("symbols", { exchange, market });
/** Exchanges and markets the user has in the terminal. */
export const exchanges = (): ExchangeInfo[] => call("exchanges", null);
export const historyCluster = (req: ClusterRequest): ClusterHistory => call("history_cluster", req);
/** The terminal's replay chunk; its shape is not frozen in ABI v1. */
export const historyReplay = (req: ReplayRequest): unknown => call("history_replay", req);
export const kvGet = <T = unknown>(key: string): T | null => call("kv_get", { key });
/** `null` deletes the key. The whole store is at most 1 MiB. */
export const kvSet = (key: string, value: unknown): void => call("kv_set", { key, value: value ?? null });
/** Upserts rows by key. */
export const emitRows = (rows: Row[]): void => call("emit_rows", { rows });
/** Replaces the whole table. */
export const replaceRows = (rows: Row[]): void => call("emit_rows", { rows, replace: true });
export const expire = (keys: string[]): void => call("expire", { keys });
/** A notification and a toast; `warn` and `urgent` also play the alert sound. ≤ 6 per minute. */
export const alert = (level: AlertLevel, title: string, body: string, rowKey?: string): void =>
  call("emit_alert", rowKey === undefined ? { level, title, body } : { level, title, body, row_key: rowKey });
export const setStatus = (tone: StatusTone, text: string): void => call("set_status", { text, tone });
/** Only inside on_click, once per click. */
export const openMarket = (market: MarketRef): void => call("open_market", market);
/** Opens the market and places a smart level at a density signal's price. Only inside
 * on_click, once per click; min_terminal 0.104.72. */
export const openMarketWithLevel = (market: MarketRef, level: SmartLevel): void =>
  call("open_market", { ...market, smart_level: level });
/** 1..16 markets at once. Only inside on_click, once per click; min_terminal 0.104.72. */
export const openMarkets = (markets: MarketRef[]): void => call("open_markets", { markets });
/** Only inside on_click, once per click. */
export const openSpread = (a: MarketRef, b: MarketRef, layout?: "vertical" | "horizontal"): void =>
  call("open_spread", layout === undefined ? { a, b } : { a, b, layout });
/** Signals of `source` changed after `since` (omit it: the whole snapshot). Needs `signals` in
 * manifest.yaml and min_terminal 0.104.72; `SignalFeed` keeps the cursor. */
export const signals = (source: SignalSource, since?: number): SignalsDelta =>
  call("signals", since === undefined ? { source } : { source, since });
export const log = (level: LogLevel, msg: string): void => call("log", { level, msg });
export const nowMs = (): number => call("now_ms", null);
export const debug = (msg: string): void => log("debug", msg);
export const info = (msg: string): void => log("info", msg);
export const warn = (msg: string): void => log("warn", msg);
export const error = (msg: string): void => log("error", msg);

let currentParams: Params = {};
let currentLang: Lang = "en";
let currentTerminal = "";

/** Parameters from the last init / on_params: every declared key, user value or default. */
export const params = (): Params => currentParams;
export const lang = (): Lang => currentLang;
export const terminalVersion = (): string => currentTerminal;

export const num = (key: string, fallback = 0): number => {
  const v = currentParams[key];
  return typeof v === "number" ? v : fallback;
};
export const str = (key: string, fallback = ""): string => {
  const v = currentParams[key];
  return typeof v === "string" ? v : fallback;
};
export const bool = (key: string, fallback = false): boolean => {
  const v = currentParams[key];
  return typeof v === "boolean" ? v : fallback;
};

export const secs = (n: number): number => n * 1_000;
export const mins = (n: number): number => n * 60_000;
export const hours = (n: number): number => n * 3_600_000;

/** Green when positive, red when negative. */
export const signed = (v: number): Cell => ({ v, tone: v > 0 ? "pos" : v < 0 ? "neg" : "muted" });
export const muted = (v: CellValue): Cell => ({ v, tone: "muted" });
export const toned = (v: CellValue, tone: Tone, text?: string): Cell =>
  text === undefined ? { v, tone } : { v, tone, text };
export const row = (key: string, market?: MarketRef): Row =>
  market === undefined ? { key, cells: {} } : { key, ...market, cells: {} };

/** `(ts_ms, value)` points with just enough history to answer "what was it `horizonMs` ago". */
export class Series {
  private points: [number, number][] = [];
  constructor(private readonly horizonMs: number) {}

  /** Out-of-order points are dropped; the same timestamp replaces the last point. */
  push(tsMs: number, value: number): void {
    const last = this.points[this.points.length - 1];
    if (last && tsMs < last[0]) return;
    if (last && tsMs === last[0]) last[1] = value;
    else this.points.push([tsMs, value]);
    const cutoff = tsMs - this.horizonMs;
    let drop = 0;
    while (this.points.length - drop >= 2 && this.points[drop + 1][0] <= cutoff) drop++;
    if (drop > 0) this.points.splice(0, drop);
  }
  last(): number | undefined {
    return this.points[this.points.length - 1]?.[1];
  }
  lastTs(): number | undefined {
    return this.points[this.points.length - 1]?.[0];
  }
  /** Latest value at or before `tsMs`. */
  valueAt(tsMs: number): number | undefined {
    let found: number | undefined;
    for (const [t, v] of this.points) {
      if (t > tsMs) break;
      found = v;
    }
    return found;
  }
  change(nowMs: number, agoMs: number): number | undefined {
    const last = this.last();
    const past = this.valueAt(nowMs - agoMs);
    return last === undefined || past === undefined ? undefined : last - past;
  }
  changePct(nowMs: number, agoMs: number): number | undefined {
    const last = this.last();
    const past = this.valueAt(nowMs - agoMs);
    if (last === undefined || past === undefined || past === 0) return undefined;
    return ((last - past) / past) * 100;
  }
  get length(): number {
    return this.points.length;
  }
}

/** The current signals of one source, kept in sync with the terminal: call `poll()` in on_timer. */
export class SignalFeed {
  readonly signals = new Map<string, Signal>();
  connected = false;
  private seq = 0;
  constructor(readonly source: SignalSource) {}

  /** Applies the terminal's changes; `removed` includes the ids a reset left out. */
  poll(): { reset: boolean; upserted: string[]; removed: string[] } {
    const delta = signals(this.source, this.seq > 0 ? this.seq : undefined);
    this.seq = delta.seq;
    this.connected = delta.connected;
    const upserted: string[] = [];
    const removed: string[] = [];
    if (delta.reset) {
      const fresh = new Set(delta.upserts.map((signal) => signal.id));
      for (const id of this.signals.keys()) if (!fresh.has(id)) removed.push(id);
      this.signals.clear();
    }
    for (const signal of delta.upserts) {
      this.signals.set(signal.id, signal);
      upserted.push(signal.id);
    }
    if (!delta.reset) for (const id of delta.removed) if (this.signals.delete(id)) removed.push(id);
    return { reset: delta.reset, upserted, removed };
  }
}

// ---- runtime glue: assignments only, they run at build time ----

const g = globalThis as any;

g.__consoleWrite = (level: string, msg: string) =>
  log(level === "log" ? "info" : level === "trace" ? "debug" : (level as LogLevel), msg);

g.fetch = (input: string, init: any = {}) => {
  try {
    const r = http({ method: init.method ?? "GET", url: String(input), headers: init.headers, body: init.body });
    return Promise.resolve(new g.Response(r.body, { status: r.status, headers: r.headers }));
  } catch (e) {
    return Promise.reject(e);
  }
};

g.Http = {
  request() {
    throw new Error("Http.request is not available in screeners; use http() from @space-terminal/screener");
  },
};

// js-pdk snapshots Math.random's seed at build time; reseed lazily from the host clock.
let s0 = 0;
let s1 = 0;
Math.random = () => {
  if (s0 === 0 && s1 === 0) {
    const seed = new Uint32Array(2);
    g.crypto.getRandomValues(seed);
    s0 = seed[0] | 1;
    s1 = seed[1] | 1;
  }
  let x = s0;
  const y = s1;
  s0 = y;
  x ^= x << 23;
  x ^= x >>> 17;
  x ^= y ^ (y >>> 26);
  s1 = x;
  return ((s0 + s1) >>> 0) / 4294967296;
};

function reportError(e: unknown) {
  const err = e as any;
  const text = err && err.stack ? `${err}\n${err.stack}` : String(err);
  const m = Memory.fromString(text);
  Host.getFunctions()["error_set"](m.offset);
}

/**
 * Wraps one export for `st build`; not for plugin code. Records params/lang from init and
 * on_params, and turns a rejected Promise of an async export into a call error (js-pdk would
 * drop it silently).
 */
export function entry<I>(name: string, fn: (input: I) => void | Promise<void>): () => number {
  return () => {
    const text = Host.inputString();
    const input = (text ? JSON.parse(text) : null) as I;
    if (name === "init" || name === "on_params") {
      const p = input as unknown as Partial<InitInput>;
      if (p && p.params) currentParams = p.params;
      if (p && p.lang) currentLang = p.lang;
      if (p && p.terminal) currentTerminal = p.terminal;
    }
    const result = fn(input);
    if (result && typeof (result as Promise<void>).then === "function") {
      (result as Promise<void>).then(undefined, reportError);
    }
    return 0;
  };
}
