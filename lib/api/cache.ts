// FE-03 cache: answers keyed by (workspace, generation, route, params). A past generation's answer never changes,
// so nothing revalidates; a new generation (a response header, or index.changed over the event stream) makes every
// visible resource fetch again while it keeps showing the old answer (stale-while-revalidate, D1).
// Capped at 64 MB or 500 entries, least recently used first. In-flight requests for one key are shared.
import { useEffect, useState, useSyncExternalStore } from "react";
import { get, url, type Params, RequestError } from "./client.ts";

const MAX_BYTES = 64 * 1024 * 1024;
const MAX_ENTRIES = 500;

/** Workspace "" is the global routes (workspaces, meta). `epochs` is a local counter, bumped when an answer
 *  changes without a new generation (freshness after a job, workspaces.changed). */
const gens = new Map<string, number>();
const epochs = new Map<string, number>();
const entries = new Map<string, { value: unknown; bytes: number }>(); // insertion order = recency
let bytes = 0;
const inflight = new Map<string, Promise<unknown>>();
const subs = new Set<() => void>();
let tick = 0; // bumps on any generation change, so hooks re-key
const notify = () => { tick++; subs.forEach((f) => f()); };

export const generationOf = (ws: string) => gens.get(ws) ?? 0;
export function setGeneration(ws: string, g: number) {
  if (gens.get(ws) === g) return;
  gens.set(ws, g);
  notify();
}
/** Every route of `ws` fetches again, without a new generation. */
export function bump(ws: string) { epochs.set(ws, (epochs.get(ws) ?? 0) + 1); notify(); }

/** A 401 or a new session: nothing cached may outlive the token it was read with. */
export function clear() { entries.clear(); inflight.clear(); gens.clear(); epochs.clear(); bytes = 0; notify(); }

function put(key: string, value: unknown, size: number) {
  const old = entries.get(key);
  if (old) { bytes -= old.bytes; entries.delete(key); }
  entries.set(key, { value, bytes: size });
  bytes += size;
  for (const [k, e] of entries) {
    if (bytes <= MAX_BYTES && entries.size <= MAX_ENTRIES) break;
    entries.delete(k); bytes -= e.bytes;
  }
}

const keyOf = (ws: string, g: number, u: string) => `${ws}\u0000${g}.${epochs.get(ws) ?? 0}\u0000${u}`;

/**
 * One resource. `ws` "" is for global routes. Per-workspace routes may carry a newer generation than the one asked
 * for; the answer is stored under the generation it carries, and that becomes the workspace's generation.
 */
export function load<T>(ws: string, route: string, params?: Params): Promise<T> {
  const u = url(route, params);
  const key = keyOf(ws, generationOf(ws), u);
  const hit = entries.get(key);
  if (hit) { entries.delete(key); entries.set(key, hit); return Promise.resolve(hit.value as T); }
  const pending = inflight.get(key);
  if (pending) return pending as Promise<T>;
  const p = get<T>(route, params).then((r) => {
    put(key, r.data, r.bytes);
    if (ws && r.generation > generationOf(ws)) {
      put(keyOf(ws, r.generation, u), r.data, 0); // the same object; count its bytes once
      setGeneration(ws, r.generation);
    }
    return r.data;
  }).finally(() => inflight.delete(key));
  inflight.set(key, p);
  return p;
}

const subscribe = (f: () => void) => { subs.add(f); return () => { subs.delete(f); }; };
/** Re-renders on any generation change. */
export const useGenerationTick = () => useSyncExternalStore(subscribe, () => tick, () => 0);

export interface Resource<T> { data?: T; error?: RequestError; loading: boolean }

/** Reads `route` for workspace `ws`; `route` null means "not yet" (no workspace chosen). */
export function useResource<T>(ws: string, route: string | null, params?: Params): Resource<T> {
  const t = useGenerationTick();
  const k = route ? keyOf(ws, generationOf(ws), url(route, params)) : "";
  // `of`: the route the data answers. The route names the workspace, so another one's answer is never shown
  // under this one's name, not even while this one loads or after it fails.
  const [state, setState] = useState<Resource<T> & { of?: string }>({ loading: !!route });
  useEffect(() => {
    if (!route) { setState({ loading: false }); return; }
    let live = true;
    const mine = (s: { data?: T; of?: string }) => (s.of === route ? s.data : undefined);
    // keep the last answer on screen while the next loads: a new generation or new params of the same route
    setState((s) => ({ data: mine(s), loading: true, of: route }));
    load<T>(ws, route, params).then(
      (data) => live && setState({ data, loading: false, of: route }),
      (error: RequestError) => live && setState((s) => ({ data: mine(s), error, loading: false, of: route })),
    );
    return () => { live = false; };
    // k covers ws, route, params and the generation
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [k, t]);
  return state;
}
