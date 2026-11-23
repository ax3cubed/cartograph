// FE-05 live events. One stream per browser, GET /api/v1/events, read with fetch (EventSource can't send the bearer
// header). The tab holding the Web Lock "cartograph-events" owns the connection and rebroadcasts each event on a
// BroadcastChannel, so 8 tabs hold 1 of the browser's 6 HTTP/1.1 connections. Reconnects resume with Last-Event-ID,
// backing off from 0.5 s to 15 s with jitter. Events reach the app coalesced per animation frame.
//
// Payloads carry ids and counts; any text in them (commit messages, names) is untrusted and renders as text only.
import type { ActivityItem, Actor, Freshness, ServerEvent } from "./types.gen.ts";
import { authHeaders } from "./client.ts";
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

// ---------- activity (FE-16), built here from the stream ----------
//
// ponytail: SRV-13's store and /api/v1/activity aren't in the binary yet, so rows are made in the browser from the
// events every tab receives, and kept in localStorage (shared by this origin's tabs). Row ids are derived from the
// event, so tabs that each record the same event write one row, and a sync's job.finished and its index.changed
// collapse into one row per workspace per run. Read state rides in the same key; the storage event tells other tabs.

export interface Row extends ActivityItem { ws: string; href?: string }
const ACT_KEY = "cartograph.activity";
const PER_WS = 500;
const WHO: Record<Actor, string> = { app: "from the app", cli: "from the CLI", mcp: "by an agent over MCP", hook: "by a git hook", background: "by the background sync" };
const started = new Map<string, { ws: string; by: Actor; kind: string }>();
/** Untrusted text (names, error messages) is cut at 120 characters and rendered as text only. */
export const cut = (t: string) => (t.length > 120 ? `${t.slice(0, 119)}…` : t);

let rows: Row[] = [];
let loaded = false;
const actSubs = new Set<() => void>();
const readRows = (): Row[] => { try { return JSON.parse(localStorage.getItem(ACT_KEY) ?? "[]"); } catch { return []; } };
function ensure() {
  if (loaded || typeof window === "undefined") return;
  loaded = true;
  rows = readRows();
  window.addEventListener("storage", (e) => { if (e.key === ACT_KEY) { rows = readRows(); actSubs.forEach((f) => f()); } });
}
function write(next: Row[]) {
  rows = next;
  try { localStorage.setItem(ACT_KEY, JSON.stringify(next)); } catch { /* storage blocked: this tab keeps its rows */ }
  actSubs.forEach((f) => f());
}
export const subscribeActivity = (f: () => void) => { ensure(); actSubs.add(f); return () => { actSubs.delete(f); }; };
export const activityRows = () => { ensure(); return rows; };
/** Marks `ids` read, or every row when none are given; other tabs follow through the storage event. */
export function markRead(ids?: string[]) {
  ensure();
  rows = readRows(); // another tab may have added rows since
  write(rows.map((r) => (!ids || ids.includes(r.id) ? { ...r, unread: false } : r)));
}
/** New or changed rows: the newest first, at most 500 per workspace. A row seen before keeps its read state. */
function upsert(r: Omit<Row, "unread" | "at">, opts: { replace?: boolean } = {}) {
  ensure();
  const cur = readRows();
  const old = cur.find((x) => x.id === r.id);
  if (old && !opts.replace) { if (!toasted.has(old.id) && Date.now() - Date.parse(old.at) < 60_000) { toasted.add(old.id); newRow.forEach((f) => f(old)); } return; }
  const row: Row = { ...r, text: cut(r.text), at: old?.at ?? new Date().toISOString(), unread: old ? old.unread : true };
  const rest = cur.filter((x) => x.id !== r.id);
  const next = [row, ...rest];
  let n = 0;
  write(next.filter((x) => x.ws !== r.ws || ++n <= PER_WS));
  if (!toasted.has(row.id)) { toasted.add(row.id); newRow.forEach((f) => f(row)); }
}
/** Rows this tab has announced: every tab toasts each row once, whichever tab stored it first. */
const toasted = new Set<string>();
const newRow = new Set<(r: Row) => void>();
/** Each row this tab records for the first time (the toast's cue). */
export const onActivity = (f: (r: Row) => void) => { newRow.add(f); return () => { newRow.delete(f); }; };

function record(e: ServerEvent) {
  switch (e.event) {
    case "job.started": started.set(e.data.job_id, { ws: e.data.ws, by: e.data.by, kind: e.data.kind }); break;
    case "job.finished": {
      const j = started.get(e.data.job_id);
      started.delete(e.data.job_id);
      if (!j) break;
      const what = j.kind === "rebuild" ? "rebuilt" : "synced";
      if (!e.data.ok) {
        upsert({ id: `job:${e.data.job_id}`, ws: j.ws, kind: "warning", href: "/settings/indexing",
          text: `${j.ws} ${j.kind} failed${e.data.error ? `: ${e.data.error.what}` : ""} · ${WHO[j.by]}` });
        break;
      }
      if (e.data.generation == null) break; // nothing changed: no row
      upsert({ id: `${j.ws}:gen:${e.data.generation}`, ws: j.ws, kind: j.kind === "rebuild" ? "index" : "sync", href: `/#ws=${encodeURIComponent(j.ws)}`,
        text: `${j.ws} ${what} to generation ${e.data.generation} · ${WHO[j.by]}` }, { replace: true });
      break;
    }
    case "index.changed":
      // A job of this tab's names its actor above; otherwise the index changed outside the app.
      upsert({ id: `${e.data.ws}:gen:${e.data.generation}`, ws: e.data.ws, kind: "sync", href: `/#ws=${encodeURIComponent(e.data.ws)}`,
        text: `${e.data.ws} synced to generation ${e.data.generation} · outside the app (CLI, hook or schedule)` });
      break;
    case "freshness": {
      const f = e.data.freshness;
      if (f.state === "failed" || f.state === "rewritten") {
        upsert({ id: `${e.data.ws}:${f.state}:${f.built_at ?? ""}`, ws: e.data.ws, kind: "warning", href: "/settings/indexing",
          text: f.state === "failed" ? `${e.data.ws}: the index can't be read${f.error ? `. ${f.error.what}` : ""}` : `${e.data.ws}: a repository's history was rewritten. The next sync replays it.` });
      }
      break;
    }
  }
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
  if (e.event === "_up") { setUp(e.data.up); return; }
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
