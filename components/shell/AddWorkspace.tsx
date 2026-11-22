"use client";

import { useEffect, useMemo, useRef, useState } from "react";
import Link from "next/link";
import { useRouter } from "next/navigation";
import { Check, Copy, FileJson, FolderSearch, FolderX, X } from "lucide-react";
import { getCurrentWorkspace } from "@/lib/source";
import { SERVICES } from "@/lib/workspace";
import { FOUND_VIA, NORTHWIND_FILE, NORTHWIND_FOLDERS, NORTHWIND_ID } from "@/lib/demo/shell";
import type { FolderRow } from "@/lib/model";
import { copy } from "./IndexStatus";
import { queueToast, toast } from "./toast";
import { takeOrigin } from "./origin";
import Toasts from "./Toasts";
import "./shell.css";
import "./addWorkspace.css";

/** Decision D1 / board S01: pick a workspace file or folder, review discovery, review services, build the first index. */
interface Source { file: string; name: string; rows: FolderRow[]; demo: boolean }
interface Svc { id: string; name: string; repo: string; via: string; color?: string; lang?: string }
const STEPS = ["Sources", "Repositories", "Services", "First index"];
const STEPS_OWN = ["Sources", "Repositories", "Services", "Build command"]; // a real folder gets a command, not a build
const INDEX_STAGES = ["Find each repository's default branch", "Replay first-parent history", "Parse files and link them", "Write the index"];

/** Parse a .code-workspace file the way VS Code does: comments and trailing commas are allowed. */
function parseWorkspace(text: string): string[] | Error {
  try {
    const json = text.replace(/("(?:\\.|[^"\\])*")|\/\/[^\n]*|\/\*[\s\S]*?\*\//g, (_m, s) => s ?? "").replace(/,(\s*[}\]])/g, "$1");
    const folders = (JSON.parse(json).folders ?? []) as { path?: string; uri?: string }[];
    return folders.map((f) => f.uri ?? f.path ?? "");
  } catch (e) { return e as Error; }
}
/** Resolve folders as far as a browser can: remote URIs are skipped here, the git check runs when the index is built. */
const rowsFrom = (folders: string[]): FolderRow[] => folders.filter(Boolean).map((f) =>
  /^[a-z][\w+.-]*:\/\//i.test(f) && !f.startsWith("file:")
    ? { folder: f, result: "skipped", detail: "Skipped: not on this machine" }
    : { folder: f, result: "repo", repo: f.replace(/\/$/, "").split("/").pop(), detail: "Repository. The git check runs when the index is built" });

const servicesFor = (src: Source): Svc[] => {
  const ids = [...new Set(src.rows.filter((r) => r.repo).map((r) => r.repo!))];
  return ids.map((id) => {
    const s = SERVICES.find((x) => x.id === id);
    return src.demo && s ? { id, name: s.name, repo: s.name, via: FOUND_VIA[id], color: s.color, lang: s.lang } : { id, name: id, repo: id, via: "No Dockerfile or chart checked yet: one service per repository" };
  });
};

export default function AddWorkspace() {
  const router = useRouter();
  const [step, setStep] = useState(0);
  const [src, setSrc] = useState<Source | null>(null);
  const [err, setErr] = useState("");
  const [off, setOff] = useState<Set<string>>(new Set());
  const [manifest, setManifest] = useState(true);
  const [pct, setPct] = useState(0);
  const title = useRef<HTMLHeadingElement>(null);
  const fileIn = useRef<HTMLInputElement>(null);
  const dirIn = useRef<HTMLInputElement>(null);
  const ws = getCurrentWorkspace();
  // Cancel and close go back to where the person came from, not always /welcome
  const leave = () => {
    const to = takeOrigin();
    if (to) router.push(to);
    else if (history.length > 1) router.back();
    else router.push("/welcome");
  };

  const svcs = useMemo(() => (src ? servicesFor(src) : []), [src]);
  const on = svcs.filter((s) => !off.has(s.id));
  const repoCount = new Set(src?.rows.filter((r) => r.repo).map((r) => r.repo)).size;
  const skippedRows = src?.rows.filter((r) => r.result === "skipped") ?? [];
  const skipped = skippedRows.length;
  const demo = !!src?.demo;
  const steps = demo ? STEPS : STEPS_OWN;
  const target = src?.file.startsWith("~/") ? src.file : `<path to ${src?.file ?? "the workspace"}>`;
  const initCmd = `cartograph init ${target}`;
  useEffect(() => { title.current?.focus(); }, [step]);
  // step 4 runs the first index; the demo's progress is simulated
  useEffect(() => {
    if (step !== 3 || !demo || pct >= 100) return;
    const t = setTimeout(() => setPct((p) => Math.min(100, p + 4)), 120);
    return () => clearTimeout(t);
  }, [step, pct, demo]);

  const take = (s: Source) => { setErr(""); setOff(new Set()); setSrc(s); setStep(1); };
  const readFile = async (f?: File) => {
    if (!f) return;
    const folders = parseWorkspace(await f.text());
    if (folders instanceof Error) return setErr(`Couldn't read ${f.name} as a workspace file: ${folders.message}. Check that it is valid JSON, comments allowed, then choose it again.`);
    take({ file: f.name, name: f.name.replace(/\.code-workspace$/, ""), rows: rowsFrom(folders), demo: false });
  };
  const readDir = async (list: FileList | null) => {
    if (!list?.length) return;
    const files = [...list];
    const depth = (f: File) => f.webkitRelativePath.split("/").length;
    const wsFile = files.filter((f) => f.name.endsWith(".code-workspace") && depth(f) <= 3).sort((a, b) => depth(a) - depth(b))[0];
    if (wsFile) return readFile(wsFile);
    const repos = [...new Set(files.filter((f) => /\/\.git\/HEAD$/.test(f.webkitRelativePath) && depth(f) <= 5).map((f) => f.webkitRelativePath.replace(/\/\.git\/HEAD$/, "")))];
    const root = files[0].webkitRelativePath.split("/")[0];
    take({ file: `~/${root}`, name: root, rows: rowsFrom(repos.map((r) => r.split("/").slice(1).join("/") || ".")), demo: false });
  };
  const northwind = (empty = false) => take({ file: NORTHWIND_FILE, name: "northwind", rows: empty ? [{ folder: "notes", result: "skipped", detail: "Skipped: not a git repository" }] : NORTHWIND_FOLDERS, demo: true });
  const back = () => { setPct(0); setStep((s) => Math.max(0, s - 1)); };

  const yaml = `workspace: ${src?.name ?? ""}\nrepos:\n${on.map((s) => `  - path: ${s.repo}`).join("\n")}\n`;
  const building = step === 3 && demo && pct < 100;
  const done = step === 3 && demo && pct >= 100;
  const none = step === 1 && repoCount === 0;
  const stage = INDEX_STAGES[Math.min(INDEX_STAGES.length - 1, Math.floor(pct / 26))];

  return (
    <div className="aw">
      <header className="aw-top">
        <b>Add a workspace</b>
        <nav aria-label="Steps"><ol>
          {steps.map((s, i) => (
            <li key={s} className={i < step ? "is-done" : i === step ? "is-cur" : undefined} aria-current={i === step ? "step" : undefined}><i>{i < step ? <Check size={14} aria-hidden /> : i + 1}</i><span>{s}</span></li>
          ))}
        </ol></nav>
        <button type="button" className="aw-x" aria-label="Close and go back" onClick={leave}><X size={18} aria-hidden /></button>
      </header>
      {src && !demo && step > 0 && (
        <p className="aw-preview" role="note"><b>Preview:</b> this version can&apos;t index real folders yet. You can review what Cartograph finds. The last step gives you the command that builds the index.</p>
      )}

      <main className="aw-main">
        {step === 0 && (
          <section className="aw-sec aw-narrow" aria-labelledby="aw-h">
            <h1 id="aw-h" ref={title} tabIndex={-1}>Pick a workspace file or folder</h1>
            <p className="aw-sub">Cartograph reads the folders listed in a <code>.code-workspace</code> file, the same file VS Code and Cursor open. With a folder instead, it looks for the file up to two levels down, then for repositories up to three. Nothing leaves this machine.</p>
            <input ref={fileIn} type="file" accept=".code-workspace" hidden onChange={(e) => { readFile(e.target.files?.[0]); e.target.value = ""; }} />
            <input ref={dirIn} type="file" hidden {...{ webkitdirectory: "" }} onChange={(e) => { readDir(e.target.files); e.target.value = ""; }} />
            <div className="aw-pick">
              <button type="button" onClick={() => fileIn.current?.click()}><FileJson size={24} aria-hidden /><b>Choose a workspace file</b><span>A <code>.code-workspace</code> file</span></button>
              <button type="button" onClick={() => dirIn.current?.click()}><FolderSearch size={24} aria-hidden /><b>Choose a folder</b><span>Cartograph finds the repositories under it</span></button>
            </div>
            {err && <p className="aw-err" role="alert">{err}</p>}
            <div className="aw-demo">
              <span className="sh-k">Demo</span>
              <p>Walk through the steps with <code>{NORTHWIND_FILE}</code>.</p>
              <div><button type="button" className="btn primary" onClick={() => northwind()}>Use the northwind example</button><button type="button" className="btn" onClick={() => northwind(true)}>Try a folder with no repositories</button></div>
            </div>
          </section>
        )}

        {step === 1 && src && (
          <div className="aw-two">
            <section className="aw-sec" aria-labelledby="aw-h">
              <h1 id="aw-h" ref={title} tabIndex={-1}>{none ? "No repositories found" : `${repoCount} ${repoCount === 1 ? "repository" : "repositories"} found in ${src.rows.length} ${src.rows.length === 1 ? "folder" : "folders"}`}</h1>
              <p className="aw-sub">From <code>{src.file}</code>.</p>
              {skipped > 0 && (
                <p className="aw-sub aw-skip">{skipped === 1 ? "Skipped: " : `${skipped} folders skipped: `}
                  {skippedRows.map((r, i) => <span key={r.folder}>{i > 0 && "; "}<code>{r.folder}</code> ({r.detail.replace(/^Skipped: /, "")})</span>)}.</p>
              )}
              {none ? (
                <div className="aw-empty" role="status">
                  <FolderX size={28} aria-hidden />
                  <b>Cartograph found nothing to index</b>
                  <p>It checked the folders in the workspace file, and when there is none, every folder up to three levels down. Each folder has to be inside a git work tree. Folders on another machine are skipped too.</p>
                  <p>Check that the path is right and that the folder has a <code>.git</code> inside it, then choose again. You can also look at northwind, which has repositories to find.</p>
                  <div><button type="button" className="btn primary" onClick={back}>Choose another file or folder</button><Link href="/" className="btn">Try the demo</Link></div>
                </div>
              ) : (
                <table className="aw-tab">
                  <caption className="sr-only">How each folder in the workspace file was resolved</caption>
                  <thead><tr><th scope="col">Folder</th><th scope="col">Result</th><th scope="col">Detail</th></tr></thead>
                  <tbody>
                    {src.rows.map((r) => (
                      <tr key={r.folder} className={r.result === "skipped" ? "is-skip" : undefined}>
                        <td><code>{r.folder}</code></td>
                        <td><span className={`sh-chip ${r.result === "repo" ? "sh-chip-up" : r.result === "scope" ? "" : "sh-chip-warn"}`}>{r.result === "repo" ? "Repo" : r.result === "scope" ? "Scope" : "Skipped"}</span></td>
                        <td>{r.detail}</td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              )}
            </section>
            <aside className="aw-sec" aria-labelledby="aw-how">
              <h2 id="aw-how">How discovery works</h2>
              <ol className="aw-how">
                <li>A relative path is resolved from the folder that holds the workspace file.</li>
                <li>Each folder must be inside a git work tree. A folder inside a repo becomes a scope of it.</li>
                <li>Several folders in one repo make one repo with several scopes. Its history is walked once.</li>
                <li>Folders on another machine, and folders that aren&apos;t git repositories, are listed as skipped with the reason.</li>
                <li>With no workspace file, Cartograph walks up to three levels down and never follows symlinks.</li>
              </ol>
            </aside>
          </div>
        )}

        {step === 2 && src && (
          <div className="aw-two">
            <section className="aw-sec" aria-labelledby="aw-h">
              <h1 id="aw-h" ref={title} tabIndex={-1}>{svcs.length} {svcs.length === 1 ? "service" : "services"} found</h1>
              <p className="aw-sub">{on.length} selected. A package with a Dockerfile, a Kubernetes Deployment or a Procfile counts as a service. A repo with none is one service.</p>
              <table className="aw-tab">
                <caption className="sr-only">Services found in the repositories</caption>
                <thead><tr><th scope="col"><span className="sr-only">Include</span></th><th scope="col">Service</th><th scope="col">Repository</th><th scope="col">Found from</th></tr></thead>
                <tbody>
                  {svcs.map((s) => (
                    <tr key={s.id} className={off.has(s.id) ? "is-skip" : undefined}>
                      <td><input type="checkbox" checked={!off.has(s.id)} aria-label={`Include ${s.name}`} onChange={() => setOff((o) => { const n = new Set(o); if (n.has(s.id)) n.delete(s.id); else n.add(s.id); return n; })} /></td>
                      <td><span className="aw-dot" style={s.color ? ({ "--c": s.color } as React.CSSProperties) : undefined} aria-hidden /><b>{s.name}</b>{s.lang && <span className="aw-lang">{s.lang}</span>}</td>
                      <td><code>{s.repo}</code></td>
                      <td>{s.via}</td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </section>
            <aside className="aw-sec" aria-labelledby="aw-mf">
              <div className="aw-sw"><h2 id="aw-mf">Save as a manifest</h2>
                <button type="button" role="switch" aria-checked={manifest} aria-labelledby="aw-mf" className="aw-switch" onClick={() => setManifest(!manifest)}><span /></button></div>
              <p className="aw-sub">Commit <code>cartograph.yaml</code> to a repository so teammates and CI index the same set.</p>
              {manifest && <pre className="aw-yaml">{yaml}</pre>}
            </aside>
          </div>
        )}

        {step === 3 && src && demo && (
          <section className="aw-sec aw-narrow" aria-labelledby="aw-h">
            <h1 id="aw-h" ref={title} tabIndex={-1}>{done ? `${src.name} is indexed` : `Building the first index of ${src.name}`}</h1>
            <p className="aw-sub">The index is written outside your repositories, to <code>~/.cartograph/workspaces/{NORTHWIND_ID}/index.cartograph</code>.</p>
            <div className="aw-prog" aria-live="polite">
              <div role="progressbar" aria-label="First index" aria-valuemin={0} aria-valuemax={100} aria-valuenow={pct}><span style={{ width: `${pct}%` }} /></div>
              <span>{done ? `${on.length} services${ws ? `, ${ws.commits} commits` : ""}` : `${stage} · ${pct}%`}</span>
            </div>
            {done
              ? <div className="aw-act"><button type="button" className="btn primary" onClick={() => { queueToast(`${src.name} added. It is the demo workspace, built into the app.`); router.push("/#added=northwind"); }}>Open the map</button></div>
              : <p className="aw-sub">Cancel stops the build and writes nothing. You can start it again from the services step.</p>}
          </section>
        )}

        {step === 3 && src && !demo && (
          <section className="aw-sec aw-narrow" aria-labelledby="aw-h">
            <h1 id="aw-h" ref={title} tabIndex={-1}>Build the index from your terminal</h1>
            <p className="aw-sub">Nothing has been built or saved. Run this command in a terminal to index {src.name}. {initCmd.includes("<") && "Replace the part in angle brackets with the full path on your machine."}</p>
            <div className="aw-cmd"><code><span aria-hidden>$ </span>{initCmd}</code><button type="button" className="btn" aria-label="Copy command" onClick={() => copy(initCmd)}><Copy size={14} aria-hidden />Copy</button></div>
            <p className="aw-sub">When it finishes, open the index with <code>cartograph open &lt;file&gt;</code>. This version of Cartograph can&apos;t read index files yet, so the map still shows the demo.</p>
            <div className="aw-act"><button type="button" className="btn" onClick={leave}>Close</button><Link href="/" className="btn">Explore the demo</Link></div>
          </section>
        )}
      </main>

      <footer className="aw-foot">
        {step > 0 && !building && !done && <button type="button" className="btn" onClick={back}>Back</button>}
        {!building && !done && <button type="button" className="aw-cancel" onClick={leave}>Cancel</button>}
        <span className="aw-sum" aria-live="polite">
          {step === 1 && !none && `${repoCount} ${repoCount === 1 ? "repository" : "repositories"}${demo && ws ? ` · ${ws.commits} commits` : ""}`}
          {step === 2 && `${on.length} of ${svcs.length} services selected`}
        </span>
        {step === 1 && !none && <button type="button" className="btn primary" onClick={() => setStep(2)}>Continue to services</button>}
        {step === 2 && <button type="button" className="btn primary" disabled={!on.length} onClick={() => { setPct(0); setStep(3); }}>{demo ? "Build the first index" : "Show the build command"}</button>}
        {building && <button type="button" className="btn" onClick={() => { setPct(0); setStep(2); toast("Cancelled. Nothing was written."); }}>Cancel the build</button>}
        {step === 2 && manifest && <button type="button" className="btn" onClick={() => copy(yaml)}>Copy cartograph.yaml</button>}
      </footer>
      <Toasts />
    </div>
  );
}
