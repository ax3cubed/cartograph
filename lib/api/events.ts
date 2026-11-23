// FE-05 live events. One stream per browser, GET /api/v1/events, read with fetch (EventSource can't send the bearer
// header). The tab holding the Web Lock "cartograph-events" owns the connection and rebroadcasts each event on a
// BroadcastChannel, so 8 tabs hold 1 of the browser's 6 HTTP/1.1 connections. Reconnects resume with Last-Event-ID,
// backing off from 0.5 s to 15 s with jitter. Events reach the app coalesced per animation frame.
//
// Payloads carry ids and counts; any text in them (commit messages, names) is untrusted and renders as text only.
import type { Freshness, ServerEvent } from "./types.gen.ts";
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
    // ponytail: activity.new and config.changed wait on the activity store (FE-14) and settings (FE-13).
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
