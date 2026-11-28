// FE-11: a paged read, 50 rows at a time (F2). The first page loads on its own; `more` loads the next one with the
// cursor the last answer carried. A new route or params (the filter and the sort are the server's, so they are params)
// starts again from the first page and clears the rows; a new generation starts again too, but keeps the rows on
// screen until the first page arrives. A cursor is never used past the answer it came from.
import { useCallback, useEffect, useRef, useState } from "react";
import { get, type Params, type RequestError } from "./client.ts";
import { generationOf, setGeneration, useGenerationTick } from "./cache.ts";

export const PAGE = 50;

export interface Paged<T> {
  rows: T[];
  /** The first page is on its way. */
  loading: boolean;
  /** A later page is on its way. */
  loadingMore: boolean;
  error?: RequestError;
  hasMore: boolean;
  more: () => void;
}

interface State<T> { base: string; rows: T[]; cursor: string | null; loading: boolean; loadingMore: boolean; error?: RequestError }

/** `route` null waits (no workspace yet). */
export function usePaged<T>(ws: string | undefined, route: string | null, params?: Params): Paged<T> {
  const tick = useGenerationTick();
  const base = ws && route ? `${ws}\u0000${route}\u0000${JSON.stringify(params ?? {})}` : "";
  const gen = ws ? generationOf(ws) : 0;
  const [state, setState] = useState<State<T>>({ base, rows: [], cursor: null, loading: !!base, loadingMore: false });
  const current = useRef("");

  useEffect(() => {
    const id = `${base}\u0000${gen}`;
    current.current = id;
    if (!base || !route || !ws) { setState({ base, rows: [], cursor: null, loading: false, loadingMore: false }); return; }
    setState((s) => ({ base, rows: s.base === base ? s.rows : [], cursor: null, loading: true, loadingMore: false }));
    get<T[]>(route, { ...params, limit: PAGE }).then(
      (r) => {
        if (current.current !== id) return;
        if (r.generation > generationOf(ws)) setGeneration(ws, r.generation);
        setState({ base, rows: r.data, cursor: r.next_cursor, loading: false, loadingMore: false });
      },
      (error: RequestError) => { if (current.current === id) setState((s) => ({ ...s, error, loading: false })); },
    );
    // base covers ws, route and params; gen and tick are the index changing under it
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [base, gen, tick]);

  const more = useCallback(() => {
    if (!state.cursor || !route || !ws || state.loading || state.loadingMore) return;
    const id = current.current;
    const { cursor } = state;
    setState((s) => ({ ...s, loadingMore: true }));
    get<T[]>(route, { ...params, limit: PAGE, cursor }).then(
      (r) => { if (current.current === id) setState((s) => ({ ...s, rows: [...s.rows, ...r.data], cursor: r.next_cursor, loadingMore: false })); },
      (error: RequestError) => { if (current.current === id) setState((s) => ({ ...s, error, loadingMore: false })); },
    );
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [state, route, ws, base]);

  const mine = state.base === base;
  return { rows: mine ? state.rows : [], loading: mine ? state.loading : !!base, loadingMore: state.loadingMore, error: mine ? state.error : undefined, hasMore: mine && !!state.cursor, more };
}
