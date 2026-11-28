// FE-14: corrections over the local API. The browser sends the entry as data; the server picks the file (the workspace's
// cartograph.overrides.yaml), checks the entry against the index and answers with the diff it made or would make.
import type { Correction, CorrectionRequest, FileDiff } from "./types.gen.ts";
import { get, send } from "./client.ts";

export type { Correction, CorrectionRequest, FileDiff };

const route = (ws: string, tail = "") => `/api/v1/w/${encodeURIComponent(ws)}/corrections${tail}`;

/** Fired on `window` when this tab wrote the file, so a list on screen reads it again. */
export const CORRECTIONS_CHANGED = "cartograph:corrections";
const wrote = <T,>(r: T): T => { if (typeof window !== "undefined") window.dispatchEvent(new Event(CORRECTIONS_CHANGED)); return r; };

/** The entries of the override file, in file order, each with the index's verdict (`stale`). */
export const getCorrections = (ws: string) => get<Correction[]>(route(ws)).then((r) => r.data);

/** What saving would change in the file. Writes nothing. */
export const previewCorrection = (ws: string, req: Omit<CorrectionRequest, "dry_run">) => send<FileDiff>("POST", route(ws), { ...req, dry_run: true });

/** Appends the entry to the file; the index applies it on the sync this starts. */
export const saveCorrection = (ws: string, req: Omit<CorrectionRequest, "dry_run">) => send<FileDiff>("POST", route(ws), { ...req, dry_run: false }).then(wrote);

/** Removes entry `id` (`c<position>`) from the file; the next sync restores its link. */
export const undoCorrection = (ws: string, id: string) => send<FileDiff>("DELETE", route(ws, `/${encodeURIComponent(id)}`)).then(wrote);
