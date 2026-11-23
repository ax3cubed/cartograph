// FE-03 session. `cartograph open` opens http://127.0.0.1:<port>/#k=<token>. The token moves into this tab's
// sessionStorage at boot and leaves the URL, so it stays out of history, Referer and share links. It is never put
// in localStorage, a cookie, a log line or error text. Every /api call sends it as a bearer header (merge note:
// no cookie, no session exchange).
//
// Modes: "checking" until boot decides; "live" with a token; "no-session" when the binary serves this tab but the
// tab has no token (a second tab, or a 401) — the "open from the CLI" screen; "demo" when nothing answers /api
// (pnpm dev, a static host), so the demo getters keep running.

export type Mode = "checking" | "demo" | "live" | "no-session";

const KEY = "cartograph.session";
let mode: Mode = "checking";
/** False once the binary stops answering; true again when a request or the event stream gets through. */
let up = true;
const subs = new Set<() => void>();
const emit = () => subs.forEach((f) => f());

export const subscribe = (f: () => void) => { subs.add(f); return () => { subs.delete(f); }; };
export const getMode = () => mode;
export const isLive = () => mode === "live";
export const isUp = () => up;
export const setUp = (v: boolean) => { if (v !== up) { up = v; emit(); } };

export function token(): string | null {
  try { return sessionStorage.getItem(KEY); } catch { return null; }
}

/** Takes `k` out of the fragment into sessionStorage, keeping the rest of the fragment. Runs before anything else reads it. */
function takeToken() {
  const p = new URLSearchParams(location.hash.slice(1));
  const k = p.get("k");
  if (k === null) return;
  p.delete("k");
  const rest = p.toString();
  history.replaceState(history.state, "", location.pathname + location.search + (rest ? `#${rest}` : ""));
  try { if (/^[\w-]{16,128}$/.test(k)) sessionStorage.setItem(KEY, k); } catch { /* storage blocked: the tab gets the CLI screen */ }
}
// At module load in the browser, before React reads the hash for #focus= and the like.
if (typeof window !== "undefined") takeToken();

let booted = false;
/** Decides the mode once per page load. Runs after hydration. */
export function boot() {
  // Again here: the router keeps the URL it read at start-up and can write it back after the first strip.
  takeToken();
  if (booted) return;
  booted = true;
  if (token()) { mode = "live"; emit(); return; }
  // No token: ask whether the binary serves this page. It refuses an unauthenticated /api call with its JSON error
  // body; next dev and static hosts answer 404 with HTML.
  fetch("/api/version", { credentials: "omit", cache: "no-store" })
    .then(async (r) => (r.status === 401 && (await r.json())?.error?.code === "unauthorized" ? "no-session" : "demo"))
    .catch(() => "demo" as const)
    .then((m) => { mode = m; emit(); });
}

/** A 401: the token is wrong (the binary restarted with a new one) or gone. Forget it and show the CLI screen. */
export function expire() {
  try { sessionStorage.removeItem(KEY); } catch { /* nothing to clear */ }
  if (mode !== "no-session") { mode = "no-session"; emit(); }
}
