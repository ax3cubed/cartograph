"use client";

import { useRef, useState } from "react";
import Link from "next/link";
import { Copy, Database, FolderPlus, Play } from "lucide-react";
import { copy } from "./IndexStatus";
import { toast } from "./toast";
import { rememberOrigin } from "./origin";
import Toasts from "./Toasts";
import "./shell.css";
import "./firstRun.css";

/** Decision D2 / board G01: three ways in. Add a workspace, open an existing index, or try the demo. */
export default function FirstRun() {
  const file = useRef<HTMLInputElement>(null);
  const [over, setOver] = useState(false);
  const open = (f?: File) => {
    if (!f) return;
    toast(f.name.endsWith(".cartograph")
      ? `Can't open ${f.name} yet. This version of Cartograph can't read index files. The northwind demo is available.`
      : `${f.name} isn't an index. Choose a file that ends in .cartograph.`);
  };
  return (
    <div className="fr">
      <header className="fr-top">
        <span className="fr-brand"><svg viewBox="0 0 48 48" aria-hidden><path d="M35.31 12.69A16 16 0 1 0 35.31 35.31" className="ring" /><path d="M24 24 35.31 12.69" className="spoke" /><circle cx="24" cy="24" r="5.2" /><circle cx="35.31" cy="12.69" r="4.4" className="dot" /></svg>cartograph</span>
        <span className="fr-none">No index open</span>
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
            <section className={`fr-card fr-drop${over ? " is-over" : ""}`} aria-labelledby="fr-b"
              onDragOver={(e) => { e.preventDefault(); setOver(true); }} onDragLeave={() => setOver(false)}
              onDrop={(e) => { e.preventDefault(); setOver(false); open(e.dataTransfer.files[0]); }}>
              <span className="sh-k">Already have an index</span>
              <Database size={28} aria-hidden />
              <h2 id="fr-b">Open an existing index</h2>
              <p>Drop a <code>.cartograph</code> file here, or choose one from disk. This version can&apos;t read index files yet, so for now it only tells you why.</p>
              <input ref={file} type="file" accept=".cartograph" hidden onChange={(e) => { open(e.target.files?.[0]); e.target.value = ""; }} />
              <button type="button" className="fr-btn" onClick={() => file.current?.click()}>Choose index file</button>
              <div className="fr-code"><code><span aria-hidden>$ </span>cartograph open &lt;file&gt;</code><button type="button" aria-label="Copy the open command" onClick={() => copy("cartograph open <file>")}><Copy size={13} aria-hidden />Copy</button></div>
            </section>
            <section className="fr-card" aria-labelledby="fr-c">
              <span className="sh-k">No repositories at hand</span>
              <Play size={28} aria-hidden />
              <h2 id="fr-c">Try the demo</h2>
              <p>Explore northwind, a made-up workspace with six services. None of it is your code.</p>
              <Link href="/" className="fr-btn">Explore the demo workspace</Link>
            </section>
          </div>
        </div>
      </main>
      <Toasts />
    </div>
  );
}
