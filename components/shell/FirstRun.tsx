"use client";

import { useEffect, useState } from "react";
import Link from "next/link";
import { Copy, Database, FolderPlus, Play } from "lucide-react";
import type { WorkspaceSummary } from "@/lib/api";
import { copy } from "./IndexStatus";
import { rememberOrigin } from "./origin";
import { RequestError, get } from "@/lib/api/client";
import { openWorkspace, useLive } from "@/lib/api/live";
import Toasts from "./Toasts";
import "./shell.css";
import "./firstRun.css";

/** Decision D2 / board G01, item FE-15: three ways in. Add a workspace, open one already in the registry, or try the demo. */
export default function FirstRun() {
  // null while loading; a RequestError when the binary isn't reachable
  const [list, setList] = useState<WorkspaceSummary[] | RequestError | null>(null);
  useEffect(() => {
    const ac = new AbortController();
    get<WorkspaceSummary[]>("/api/v1/workspaces", {}, { signal: ac.signal })
      .then((r) => setList(r.data.filter((x) => !x.demo)))
      .catch((e) => { if (e instanceof RequestError) setList(e); });
    return () => ac.abort();
  }, []);
  const cmd = list instanceof RequestError ? "cartograph open" : "cartograph init <folder>";
  // Served by the binary, "/" is the live map: there is no demo workspace to open there.
  const live = useLive();
  return (
    <div className="fr">
      <header className="fr-top">
        <span className="fr-brand"><svg viewBox="0 0 48 48" aria-hidden><path d="M35.31 12.69A16 16 0 1 0 35.31 35.31" className="ring" /><path d="M24 24 35.31 12.69" className="spoke" /><circle cx="24" cy="24" r="5.2" /><circle cx="35.31" cy="12.69" r="4.4" className="dot" /></svg>cartograph</span>
        <span className="fr-none">No workspace open</span>
      </header>
      <main className="fr-main">
        <div className="fr-rings" aria-hidden />
        <div className="fr-wrap">
          <div className="fr-head">
            <h1>Map your own workspace</h1>
            <p>Cartograph reads one index that it builds from your repositories and their git history. Nothing is uploaded. The index stays on this machine.</p>
          </div>
          <div className="fr-grid">
            <section className="fr-card fr-primary" aria-labelledby="fr-a">
              <span className="sh-k">Recommended</span>
              <FolderPlus size={28} aria-hidden />
              <h2 id="fr-a">Add a workspace</h2>
              <p>Pick a <code>.code-workspace</code> file or a folder. Cartograph finds the repositories and services, then builds the first index.</p>
              <Link href="/welcome/add" className="fr-btn is-accent" onClick={rememberOrigin}>Add a workspace</Link>
            </section>
            <section className="fr-card" aria-labelledby="fr-b">
              <span className="sh-k">Already added</span>
              <Database size={28} aria-hidden />
              <h2 id="fr-b">Open a workspace</h2>
              {list === null ? <p role="status">Looking for workspaces…</p>
                : list instanceof RequestError ? <p role="status">{list.what}. Start it from a terminal, then open the address it prints.</p>
                : list.length ? (
                  <ul className="fr-list">
                    {list.map((w) => <li key={w.id}><Link href={`/#ws=${w.id}`} onClick={() => openWorkspace(w.id)}><b>{w.name}</b><span title={w.path}>{w.repos} {w.repos === 1 ? "repo" : "repos"} · {w.path}</span></Link></li>)}
                  </ul>
                ) : <p>No workspaces yet. Ones you add here or with <code>cartograph init</code> are listed here.</p>}
              {!(Array.isArray(list) && list.length) && (
                <div className="fr-code"><code><span aria-hidden>$ </span>{cmd}</code><button type="button" aria-label={`Copy the command ${cmd}`} onClick={() => copy(cmd)}><Copy size={13} aria-hidden />Copy</button></div>
              )}
            </section>
            {!live && (
              <section className="fr-card" aria-labelledby="fr-c">
                <span className="sh-k">No repositories at hand</span>
                <Play size={28} aria-hidden />
                <h2 id="fr-c">Try the demo</h2>
                <p>Explore northwind, a made-up workspace with six services. None of it is your code.</p>
                <Link href="/" className="fr-btn">Explore the demo workspace</Link>
              </section>
            )}
          </div>
        </div>
      </main>
      <Toasts />
    </div>
  );
}
