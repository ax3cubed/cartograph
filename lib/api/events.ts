// FE-05 live events. One stream per browser, GET /api/v1/events, read with fetch (EventSource can't send the bearer
// header). The tab holding the Web Lock "cartograph-events" owns the connection and rebroadcasts each event on a
// BroadcastChannel, so 8 tabs hold 1 of the browser's 6 HTTP/1.1 connections. Reconnects resume with Last-Event-ID,
// backing off from 0.5 s to 15 s with jitter. Events reach the app coalesced per animation frame.
//
// Payloads carry ids and counts; any text in them (commit messages, names) is untrusted and renders as text only.
import type { ActivityItem, Freshness, ServerEvent } from "./types.gen.ts";
import { authHeaders, get, send } from "./client.ts";
import { bump, setGeneration } from "./cache.ts";
import { expire, setUp } from "./session.ts";
import { SseParser } from "./sse.ts";

type Wire = ServerEvent | { event: "_up"; data: { up: boolean } };

// ---------- what events change: freshness pushed per workspace, the cache's generation ----------

/** Freshness as last pushed for each workspace; it wins over the fetched one until a job ends or the index changes. */
const pushed = new Map<string, Freshness>();
const jobs = new Map<string, string>(); // job id -> workspace
const subs = new Set<() => void>();
let version = 0;
export const subscribePushed = (f: () => void) => { subs.add(f); return () => { subs.delete(f); }; };
export const pushedVersion = () => version;
export const pushedFreshness = (ws: string) => pushed.get(ws);
const patch = (ws: string | undefined, f: (cur: Freshness | undefined) => Freshness | undefined) => {
  if (!ws) return;
  const next = f(pushed.get(ws));
  if (next) pushed.set(ws, next); else pushed.delete(ws);
};
const syncing = (cur: Freshness | undefined, progress: number): Freshness =>
  ({ ...(cur ?? { behind: 0, format_version: 0, repos: [] }), state: "syncing", progress });

function apply(e: ServerEvent) {
  switch (e.event) {
    case "freshness": patch(e.data.ws, () => e.data.freshness); break;
    case "job.started": jobs.set(e.data.job_id, e.data.ws); patch(e.data.ws, (c) => syncing(c, 0)); break;
    case "job.progress": patch(jobs.get(e.data.job_id), (c) => syncing(c, e.data.total ? e.data.done / e.data.total : 0)); break;
    case "job.finished": {
      const ws = jobs.get(e.data.job_id);
      jobs.delete(e.data.job_id);
      if (!ws) break;
      pushed.delete(ws); // fetch it again: the server's answer replaces "syncing"
      if (e.data.generation != null) setGeneration(ws, e.data.generation);
      bump(ws);
      break;
    }
    case "index.changed": pushed.delete(e.data.ws); setGeneration(e.data.ws, e.data.generation); break;
    case "workspaces.changed": bump(""); break;
    // config.changed reaches the settings pages through onEvent; it changes no cache entry here.
  }
  record(e);
}

// ---------- activity (FE-16): the server's rows, kept current by the stream ----------
//
// The server records a row per repo per run in the workspace's ask.db and sends each new one as activity.new. Read
// state lives there too (POST /api/v1/activity/read), and activity.read tells every tab, in any browser. Text is
// untrusted (repo and author names): cut at 120 characters and rendered as text only.

export type Row = ActivityItem;
/** Untrusted text (names, error messages) is cut at 120 characters and rendered as text only. */
export const cut = (t: string) => (t.length > 120 ? `${t.slice(0, 119)}…` : t);

let rows: Row[] = [];
let next: string | null = null;
let loading = false;
let loaded = false;
const actSubs = new Set<() => void>();
const changed = (r: Row[]) => { rows = r; actSubs.forEach((f) => f()); };
/** The first page, or the page after `cursor`. */
async function load(cursor?: string) {
  if (loading) return;
  loading = true;
  try {
    const got = await get<Row[]>("/api/v1/activity", { cursor });
    next = got.next_cursor ?? null;
    changed(cursor ? [...rows, ...got.data.filter((r) => !rows.some((x) => x.id === r.id))] : got.data);
  } catch { /* offline or an older binary: the list stays as it was */ }
  finally { loading = false; }
}
export const subscribeActivity = (f: () => void) => { actSubs.add(f); return () => { actSubs.delete(f); }; };
/** The first page, once per tab; called only against a live server. */
export const loadActivity = () => { if (!loaded) { loaded = true; void load(); } };
export const activityRows = () => rows;
export const hasMoreActivity = () => next !== null;
/** The next page of 50. */
export const moreActivity = () => (next ? load(next) : Promise.resolve());
const readLocally = (ids: string[]) => changed(rows.map((r) => (!ids.length || ids.includes(r.id) ? { ...r, unread: false } : r)));
/** Marks `ids` read on the server, or every row when none are given; other tabs follow through activity.read. */
export function markRead(ids?: string[]) {
  readLocally(ids ?? []);
  send("POST", "/api/v1/activity/read", { ids: ids ?? [] }).catch(() => { /* the next load shows the server's state */ });
}
const newRow = new Set<(r: Row) => void>();
/** Each new row as the stream announces it (the toast's cue). */
export const onActivity = (f: (r: Row) => void) => { newRow.add(f); return () => { newRow.delete(f); }; };

function record(e: ServerEvent) {
  if (e.event === "activity.new") {
    if (rows.some((r) => r.id === e.data.id)) return;
    changed([e.data, ...rows]);
    newRow.forEach((f) => f(e.data));
  } else if (e.event === "activity.read") readLocally(e.data.ids);
}

/** Every event, after the app's own state has taken it in. For callers that follow one job (the wizard). */
const raw = new Set<(e: ServerEvent) => void>();
export const onEvent = (f: (e: ServerEvent) => void) => { raw.add(f); return () => { raw.delete(f); }; };

let queue: ServerEvent[] = [];
let frame = 0;
function flush() {
  frame = 0;
  const q = queue; queue = [];
  q.forEach(apply);
  q.forEach((e) => raw.forEach((f) => f(e)));
  version++;
  subs.forEach((f) => f());
}
function receive(e: Wire) {
  if (e.event === "_up") {
    setUp(e.data.up);
    if (e.data.up && loaded) void load(); // a reconnect: rows sent while the stream was down
    return;
  }
  queue.push(e);
  // a hidden tab gets no animation frames; a timer still delivers
  if (!frame) frame = document.hidden ? (setTimeout(flush, 100) as unknown as number) : requestAnimationFrame(flush);
}

// ---------- transport ----------

const sleep = (ms: number, signal: AbortSignal) => new Promise<void>((r) => { const t = setTimeout(r, ms); signal.addEventListener("abort", () => { clearTimeout(t); r(); }); });

async function connect(signal: AbortSignal, deliver: (e: Wire) => void) {
  let delay = 500;
  const parser = new SseParser();
  while (!signal.aborted) {
    let res: Response | null = null;
    try {
      const h: Record<string, string> = { ...(authHeaders() as Record<string, string>), Accept: "text/event-stream" };
      if (parser.lastId) h["Last-Event-ID"] = parser.lastId;
      res = await fetch("/api/v1/events", { headers: h, credentials: "omit", cache: "no-store", referrerPolicy: "no-referrer", signal });
    } catch {
      if (signal.aborted) return;
      deliver({ event: "_up", data: { up: false } }); // nothing listens: "Cartograph isn't running"
    }
    if (res) {
      deliver({ event: "_up", data: { up: true } });
      if (res.status === 401) { expire(); return; }
      // ponytail: a binary without SRV-13's stream answers 404; the app then runs on fetch-once data.
      if (res.status === 404 || res.status === 405) return;
      if (res.ok && res.body) {
        delay = 500;
        try {
          const reader = res.body.pipeThrough(new TextDecoderStream()).getReader();
          for (;;) {
            const { value, done } = await reader.read();
            if (done) break;
            for (const ev of parser.push(value)) {
              try { deliver({ event: ev.event, data: JSON.parse(ev.data) } as ServerEvent); } catch { /* a malformed payload is skipped */ }
            }
          }
        } catch { /* the connection dropped: reconnect below */ }
      }
    }
    if (signal.aborted) return;
    await sleep(delay * (0.5 + Math.random()), signal);
    delay = Math.min(delay * 2, 15_000);
  }
}

let stop: (() => void) | null = null;
/** Starts listening, once per tab. Safe to call again; returns the stopper. */
export function startEvents(): () => void {
  if (stop) return stop;
  const ac = new AbortController();
  const ch = typeof BroadcastChannel === "undefined" ? null : new BroadcastChannel("cartograph-events");
  if (ch) ch.onmessage = (m: MessageEvent<Wire>) => receive(m.data);
  const own = () => connect(ac.signal, (e) => { ch?.postMessage(e); receive(e); });
  if (navigator.locks) navigator.locks.request("cartograph-events", { signal: ac.signal }, () => own()).catch(() => { /* aborted while waiting */ });
  else void own();
  stop = () => { ac.abort(); ch?.close(); stop = null; };
  return stop;
}
