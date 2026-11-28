// FE-03 client: one GET over the local API with the bearer header, no cookies, and one error type carrying the
// server's E1 shape {code, what, why, try}. The token is read here and nowhere else in the app.
import type { Envelope } from "./index.ts";
import type { ErrorCode } from "./types.gen.ts";
import { expire, setUp, token } from "./session.ts";

/** `offline` is the client's own code: the binary didn't answer at all. */
export type ClientCode = ErrorCode | "offline";

export class RequestError extends Error {
  constructor(readonly code: ClientCode, readonly what: string, readonly why: string, readonly try_: string, readonly status = 0) {
    super(what); // what only: never the URL's token, never headers
    this.name = "RequestError";
  }
}

export interface Got<T> extends Envelope<T> { state: string | null; bytes: number }

export type Params = Record<string, string | number | undefined | null>;

export const url = (route: string, params?: Params) => {
  const q = new URLSearchParams();
  for (const [k, v] of Object.entries(params ?? {})) if (v !== undefined && v !== null && v !== "") q.set(k, String(v));
  const s = q.toString();
  return s ? `${route}?${s}` : route;
};

/** Headers every API call carries. Exported for the event stream, which reads with fetch for this reason. */
export const authHeaders = (): HeadersInit => ({ Authorization: `Bearer ${token() ?? ""}`, Accept: "application/json" });

export const OFFLINE = new RequestError("offline", "Cartograph isn't running", "the app couldn't reach the local server", "start it again with cartograph open");

export async function get<T>(route: string, params?: Params, opts: { signal?: AbortSignal } = {}): Promise<Got<T>> {
  let res: Response;
  try {
    res = await fetch(url(route, params), { headers: authHeaders(), credentials: "omit", cache: "no-store", referrerPolicy: "no-referrer", signal: opts.signal });
  } catch (e) {
    if ((e as Error).name === "AbortError") throw e;
    setUp(false);
    throw OFFLINE;
  }
  setUp(true);
  // The server never sends CORS headers; one here means something other than the binary answered.
  if (res.headers.has("access-control-allow-origin")) throw new RequestError("internal", "the API answered with a CORS header", "the local server never sends one", "open the address cartograph open printed");
  const text = await res.text();
  let body: unknown;
  try { body = JSON.parse(text); } catch { body = null; }
  if (!res.ok) {
    if (res.status === 401) expire();
    const e = (body as { error?: { code: ErrorCode; what: string; why: string; try: string } } | null)?.error;
    throw e ? new RequestError(e.code, e.what, e.why, e.try, res.status) : new RequestError("internal", `the server answered ${res.status}`, "the response wasn't the API's error shape", "retry, then check the server log", res.status);
  }
  if (!body) throw new RequestError("internal", "the server's answer isn't JSON", "the response couldn't be parsed", "retry, then check the server log", res.status);
  return { ...(body as Envelope<T>), state: res.headers.get("cartograph-state"), bytes: text.length * 2 };
}

/** A read whose answer is text, not the envelope (impact as Markdown). The text is the caller's to show as text. */
export async function getText(route: string): Promise<string> {
  let res: Response;
  try {
    res = await fetch(route, { headers: { ...authHeaders(), Accept: "text/plain" }, credentials: "omit", cache: "no-store", referrerPolicy: "no-referrer" });
  } catch {
    setUp(false);
    throw OFFLINE;
  }
  setUp(true);
  const text = await res.text();
  if (res.ok) return text;
  if (res.status === 401) expire();
  const e = (() => { try { return JSON.parse(text)?.error; } catch { return null; } })();
  throw e ? new RequestError(e.code, e.what, e.why, e.try, res.status) : new RequestError("internal", `the server answered ${res.status}`, "the response wasn't the API's error shape", "retry, then check the server log", res.status);
}

/** A write: POST, PUT or DELETE with a JSON body. The X-Cartograph header and the JSON content type force a preflight
 *  on any cross-origin attempt, which the server refuses (CSRF). Returns the envelope's data. */
export async function send<T>(method: "POST" | "PUT" | "DELETE", route: string, body?: unknown): Promise<T> {
  let res: Response;
  try {
    res = await fetch(route, {
      method, credentials: "omit", cache: "no-store", referrerPolicy: "no-referrer",
      headers: { ...authHeaders(), "X-Cartograph": "1", ...(body === undefined ? {} : { "Content-Type": "application/json" }) },
      body: body === undefined ? undefined : JSON.stringify(body),
    });
  } catch {
    setUp(false);
    throw OFFLINE;
  }
  setUp(true);
  const parsed = await res.json().catch(() => null);
  if (res.ok && parsed) return (parsed as Envelope<T>).data;
  if (res.status === 401) expire();
  const e = parsed?.error;
  throw e ? new RequestError(e.code, e.what, e.why, e.try, res.status) : new RequestError("internal", `the server answered ${res.status}`, "the response wasn't the API's error shape", "retry, then check the server log", res.status);
}
