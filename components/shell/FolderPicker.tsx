"use client";

import { useEffect, useState, type FormEvent } from "react";
import { ArrowUp, Folder } from "lucide-react";
import type { FsEntry } from "@/lib/api";
import { RequestError, get } from "@/lib/api/client";

/** One line from the server's E1 error (or a Problem): what, why, and what to try. */
export const say = (e: { what: string; why: string; try_?: string; try?: string }) => `${e.what}: ${e.why}. Try: ${e.try_ ?? e.try}.`;

const HOME = "~/";
const WS_FILE = ".code-workspace";

/**
 * Ruling R25: a typed path completed by GET /api/v1/fs/list, which returns folder names, never contents.
 * The browser can't hand over an absolute path, so the binary lists folders and runs discovery. Typing "~/code/ac" lists
 * ~/code/ and keeps the names that start with "ac". A path ending in .code-workspace names that file.
 * The list never looks inside a child folder (on macOS that raises privacy prompts for Desktop, Documents and
 * Downloads), so a folder is read only when it is opened; "Find repositories" says what it holds.
 */
export default function FolderPicker({ busy, onPick }: { busy: boolean; onPick: (path: string, file: string | null) => void }) {
  const [path, setPath] = useState(HOME);
  const [list, setList] = useState<{ dir: string; entries: FsEntry[] } | null>(null);
  const [err, setErr] = useState("");
  const cut = path.lastIndexOf("/") + 1;
  const dir = path.slice(0, cut) || HOME;
  const prefix = path.slice(cut).toLowerCase();
  const isFile = path.endsWith(WS_FILE);

  useEffect(() => {
    if (isFile) return;
    const ac = new AbortController();
    const t = setTimeout(() => {
      get<FsEntry[]>("/api/v1/fs/list", { path: dir }, { signal: ac.signal })
        .then((r) => { setList({ dir, entries: r.data }); setErr(""); })
        .catch((e) => { if (e instanceof RequestError) { setList(null); setErr(say(e)); } });
    }, 150);
    return () => { clearTimeout(t); ac.abort(); };
  }, [dir, isFile]);

  const shown = list?.dir === dir ? list.entries.filter((e) => e.name.toLowerCase().startsWith(prefix)) : [];
  const submit = (e: FormEvent) => {
    e.preventDefault();
    const p = path.trim();
    if (!p) return;
    const trim = (d: string) => d.replace(/(.)\/$/, "$1"); // "~/code/acme/" -> "~/code/acme", "/" stays
    const at = p.lastIndexOf("/");
    if (p.endsWith(WS_FILE)) onPick(at < 0 ? "~" : trim(p.slice(0, at + 1)), p.slice(at + 1));
    else onPick(trim(p), null);
  };
  const up = dir.replace(/[^/]+\/$/, "");

  return (
    <form className="aw-fp" onSubmit={submit}>
      <label htmlFor="aw-path">Folder or workspace file</label>
      <div className="aw-fp-row">
        <input id="aw-path" value={path} onChange={(e) => setPath(e.target.value)} spellCheck={false} autoComplete="off" autoCapitalize="off" placeholder="~/code/acme" aria-describedby="aw-path-hint" />
        <button type="submit" className="btn primary" disabled={busy || !path.trim()}>{busy ? "Looking…" : "Find repositories"}</button>
      </div>
      <p id="aw-path-hint" className="aw-sub">Type a path or pick a folder below. To use one workspace file, type its full path, ending in <code>{WS_FILE}</code>.</p>
      {err ? <p className="aw-err" role="alert">{err}</p> : !isFile && (
        <div className="aw-fs">
          <div className="aw-fs-head">
            <code>{dir}</code>
            <button type="button" className="btn" disabled={!up || dir === HOME} onClick={() => setPath(up)}><ArrowUp size={14} aria-hidden />Up</button>
          </div>
          <ul aria-label={`Folders in ${dir}`}>
            {shown.map((e) => (
              <li key={e.name}>
                <button type="button" onClick={() => setPath(`${dir}${e.name}/`)}>
                  <Folder size={16} aria-hidden /><span>{e.name}</span>
                </button>
              </li>
            ))}
            {list?.dir === dir && !shown.length && <li className="aw-fs-none">{prefix ? `No folder here starts with "${path.slice(cut)}".` : "No folders here."}</li>}
          </ul>
        </div>
      )}
    </form>
  );
}
