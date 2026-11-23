"use client";

import { useEffect, useRef, useState } from "react";
import { useRouter } from "next/navigation";
import { Check, FolderX, X } from "lucide-react";
import type { CreatedWorkspace, Discovery, Problem, ServerEvent } from "@/lib/api";
import { getCurrentWorkspace } from "@/lib/source";
import { SERVICES } from "@/lib/workspace";
import { FOUND_VIA, NORTHWIND_FILE, NORTHWIND_FOLDERS } from "@/lib/demo/shell";
import { openWorkspace, useLive } from "@/lib/api/live";
import { queueToast, toast } from "./toast";
import { takeOrigin } from "./origin";
import FolderPicker from "./FolderPicker";
import { RequestError, send } from "@/lib/api/client";
import { onEvent } from "@/lib/api/events";
import { say } from "./FolderPicker";
import Toasts from "./Toasts";
import "./shell.css";
import "./addWorkspace.css";

/** Decision D1 / board S01, item FE-15: pick a folder or workspace file, review discovery, review services, build the first index. */
interface Row { folder: string; result: "repo" | "scope" | "skipped"; repo?: string; detail: string }
interface Svc { id: string; name: string; repo: string; via: string; color?: string; lang?: string }
interface Source { path: string; file: string | null; name: string; rows: Row[]; services: Svc[]; estimate?: number | null; demo: boolean }
interface Job { id: string; ws: string; done: number; total: number; stage: string; repo?: string | null; eta?: number | null }
const STEPS = ["Source", "Repositories", "Services", "First index"];
const DEMO_STAGES = ["Find each repository's default branch", "Replay first-parent history", "Parse files and link them", "Write the index"];
const CANCELLED = "Cancelled. Nothing was added. Files already parsed stay in the cache, so the next try starts where this one stopped.";
const base = (p: string) => p.replace(/\/$/, "").split("/").pop() || p;

const fromDiscovery = (path: string, file: string | null, d: Discovery): Source => ({
  path, file, name: file ? file.replace(/\.code-workspace$/, "") : base(path), estimate: d.estimate_s, demo: false,
  rows: d.folders.map((f) => ({
    folder: f.folder, result: f.result, repo: f.repo ?? undefined,
    detail: f.reason ?? (f.result === "scope" ? `A scope of ${f.repo}` : "Repository"),
  })),
  // Service detection lands with IDX-20; until then /discover names none, and a repo with none is one service.
  services: d.services.length
    ? d.services.map((s) => ({ id: s.id, name: s.name, repo: s.repo, via: s.paths.length ? s.paths.join(", ") : "The whole repository" }))
    : [...new Set(d.folders.filter((f) => f.repo).map((f) => f.repo!))].map((id) => ({ id, name: id, repo: id, via: "No Dockerfile or chart checked yet: one service per repository" })),
});

const northwind = (empty: boolean): Source => {
  const rows: Row[] = empty ? [{ folder: "notes", result: "skipped", detail: "Skipped: not a git repository" }] : NORTHWIND_FOLDERS;
  const ids = [...new Set(rows.filter((r) => r.repo).map((r) => r.repo!))];
  return {
    path: NORTHWIND_FILE, file: null, name: "northwind", rows, demo: true,
    services: ids.map((id) => { const s = SERVICES.find((x) => x.id === id)!; return { id, name: s.name, repo: s.name, via: FOUND_VIA[id], color: s.color, lang: s.lang }; }),
  };
};

export default function AddWorkspace() {
  const router = useRouter();
  const [step, setStep] = useState(0);
  const [src, setSrc] = useState<Source | null>(null);
  const [err, setErr] = useState("");
  const [busy, setBusy] = useState(false);
  const [job, setJob] = useState<Job | null>(null);
  const [failed, setFailed] = useState<Problem | null>(null);
  const [finished, setFinished] = useState(false);
  const [pct, setPct] = useState(0); // the demo's simulated progress
  const unsub = useRef<(() => void) | null>(null);
  // Set by cancel or close before POST /workspaces answers: that request's job is stopped as soon as its id is known.
  const cancelled = useRef(false);
  const title = useRef<HTMLHeadingElement>(null);
  const ws = getCurrentWorkspace();
  const demo = !!src?.demo;
  const live = useLive();

  const stop = () => { unsub.current?.(); unsub.current = null; };
  useEffect(() => stop, []);
  useEffect(() => { title.current?.focus(); }, [step]);
  useEffect(() => {
    if (step !== 3 || !demo || pct >= 100) return;
    const t = setTimeout(() => setPct((p) => Math.min(100, p + 4)), 120);
    return () => clearTimeout(t);
  }, [step, pct, demo]);

  const repoCount = new Set(src?.rows.filter((r) => r.repo).map((r) => r.repo)).size;
  const skipped = src?.rows.filter((r) => r.result === "skipped").length ?? 0;
  const none = step === 1 && repoCount === 0;
  const done = step === 3 && (demo ? pct >= 100 : finished);
  const building = step === 3 && !done && !failed;
  const progress = demo ? pct : job?.total ? Math.round((job.done / job.total) * 100) : 0;

  /** Cancel the first index: the server drops the half-built workspace, so nothing is left in the registry (E5). */
  const cancel = async () => {
    cancelled.current = true;
    stop();
    if (!demo && job) await send("DELETE", `/api/v1/jobs/${job.id}`).catch(() => {});
    setJob(null); setPct(0); setStep(2); toast(CANCELLED);
  };
  // Cancel and close go back to where the person came from, not always /welcome. Closing mid-build cancels the build first.
  const leave = async () => {
    if (building && !demo) {
      cancelled.current = true;
      if (job) await send("DELETE", `/api/v1/jobs/${job.id}`).catch(() => {});
    }
    stop();
    const to = takeOrigin();
    if (to) router.push(to);
    else if (history.length > 1) router.back();
    else router.push("/welcome");
  };

  const discover = async (path: string, file: string | null) => {
    setBusy(true); setErr("");
    try {
      const d = await send<Discovery>("POST", "/api/v1/discover", { path, workspace_file: file });
      setSrc(fromDiscovery(path, file, d)); setStep(1);
    } catch (e) {
      if (e instanceof RequestError) setErr(say(e));
    } finally { setBusy(false); }
  };

  const build = async () => {
    cancelled.current = false;
    setFailed(null); setFinished(false); setPct(0); setJob(null); setStep(3);
    if (!src || src.demo) return;
    // Listen before creating, so a small workspace that finishes at once isn't missed.
    const early = new Map<string, ServerEvent>();
    let id = "";
    const finish = (e: Extract<ServerEvent, { event: "job.finished" }>) => {
      stop();
      if (e.data.ok) { setJob((j) => j && { ...j, done: j.total }); setFinished(true); }
      else setFailed(e.data.error ?? { what: "The first index stopped", why: "the server didn't say why", try: "check cartograph doctor, then build again" });
    };
    unsub.current = onEvent((e) => {
      if (e.event !== "job.progress" && e.event !== "job.finished") return;
      if (!id) { early.set(e.event, e); return; }
      if (e.data.job_id !== id) return;
      if (e.event === "job.finished") finish(e);
      else setJob((j) => j && { ...j, done: e.data.done, total: e.data.total, stage: e.data.stage, repo: e.data.repo, eta: e.data.eta_s });
    });
    try {
      // manifest: the server writes cartograph.yaml beside the workspace file (C7), the file the aside tells you to commit.
      const c = await send<CreatedWorkspace>("POST", "/api/v1/workspaces", { path: src.path, workspace_file: src.file, name: null, manifest: true });
      if (cancelled.current) {
        // Cancelled while the server was still setting up: stop the first index it started, so nothing is added.
        await send("DELETE", `/api/v1/jobs/${c.job_id}`).catch(() => {});
        return;
      }
      id = c.job_id;
      setJob({ id, ws: c.workspace.id, done: 0, total: 0, stage: "Starting" });
      const f = early.get("job.finished");
      if (f?.event === "job.finished" && f.data.job_id === id) finish(f);
    } catch (e) {
      stop();
      setFailed(e instanceof RequestError ? { what: e.what, why: e.why, try: e.try_ } : { what: "Couldn't start the first index", why: "the request failed", try: "build again" });
    }
  };

  const open = () => {
    queueToast(demo ? `${src?.name} added. It is the demo workspace, built into the app.` : `${src?.name} added.`);
    // The router's push fires no hashchange, so the map wouldn't read #added: pick the workspace first.
    if (!demo && job?.ws) openWorkspace(job.ws);
    router.push(`/#added=${demo ? "northwind" : job?.ws}`);
  };
  const back = () => { setErr(""); setStep((s) => Math.max(0, s - 1)); };
  const stage = demo ? DEMO_STAGES[Math.min(DEMO_STAGES.length - 1, Math.floor(pct / 26))] : `${job?.stage ?? "Starting"}${job?.repo ? ` · ${job.repo}` : ""}`;
  const eta = !demo && job?.eta ? ` · about ${Math.ceil(job.eta / 60)} min left` : "";

  return (
    <div className="aw">
      <header className="aw-top">
        <b>Add a workspace</b>
        <nav aria-label="Steps"><ol>
          {STEPS.map((s, i) => (
            <li key={s} className={i < step ? "is-done" : i === step ? "is-cur" : undefined} aria-current={i === step ? "step" : undefined}><i>{i < step ? <Check size={14} aria-hidden /> : i + 1}</i><span>{s}</span></li>
          ))}
        </ol></nav>
        <button type="button" className="aw-x" aria-label="Close and go back" onClick={leave}><X size={18} aria-hidden /></button>
      </header>

      <main className="aw-main">
        {step === 0 && (
          <section className="aw-sec aw-narrow" aria-labelledby="aw-h">
            <h1 id="aw-h" ref={title} tabIndex={-1}>Pick a folder or workspace file</h1>
            <p className="aw-sub">Cartograph reads the folders listed in a <code>.code-workspace</code> file, the same file VS Code and Cursor open. In a folder, it looks for that file up to two levels down, then for repositories up to three. Nothing leaves this machine.</p>
            <FolderPicker busy={busy} onPick={discover} />
            {err && <p className="aw-err" role="alert">{err}</p>}
            {/* Served by the binary, the map shows registered workspaces only: the demo walkthrough would end nowhere. */}
            {!live && (
              <div className="aw-demo">
                <span className="sh-k">Demo</span>
                <p>Walk through the steps with <code>{NORTHWIND_FILE}</code>.</p>
                <div><button type="button" className="btn" onClick={() => { setSrc(northwind(false)); setStep(1); }}>Use the northwind example</button><button type="button" className="btn" onClick={() => { setSrc(northwind(true)); setStep(1); }}>Try a folder with no repositories</button></div>
              </div>
            )}
          </section>
        )}

        {step === 1 && src && (
          <div className="aw-two">
            <section className="aw-sec" aria-labelledby="aw-h">
              <h1 id="aw-h" ref={title} tabIndex={-1}>{none ? "No repositories found" : `${repoCount} ${repoCount === 1 ? "repository" : "repositories"} found in ${src.rows.length} ${src.rows.length === 1 ? "folder" : "folders"}`}</h1>
              <p className="aw-sub">From <code>{src.file ? `${src.path}/${src.file}` : src.path}</code>{skipped > 0 && `. ${skipped} skipped, each with the reason`}.</p>
              {none && (
                <div className="aw-empty" role="status">
                  <FolderX size={28} aria-hidden />
                  <b>Cartograph found nothing to index</b>
                  <p>Each folder has to be inside a git work tree. {src.rows.length ? "The reason for each folder is below." : "There were no folders to check: no workspace file, and no repository up to three levels down."} Check the path, then choose again.</p>
                </div>
              )}
              {src.rows.length > 0 && (
                <table className="aw-tab">
                  <caption className="sr-only">How each folder was resolved</caption>
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
              <h1 id="aw-h" ref={title} tabIndex={-1}>{src.services.length} {src.services.length === 1 ? "service" : "services"} found</h1>
              <p className="aw-sub">A package with a Dockerfile, a Kubernetes Deployment or a Procfile counts as a service. A repo with none is one service. You can merge or split services later in Settings.</p>
              <table className="aw-tab">
                <caption className="sr-only">Services found in the repositories</caption>
                <thead><tr><th scope="col">Service</th><th scope="col">Repository</th><th scope="col">Found from</th></tr></thead>
                <tbody>
                  {src.services.map((s) => (
                    <tr key={s.id}>
                      <td><span className="aw-dot" style={s.color ? ({ "--c": s.color } as React.CSSProperties) : undefined} aria-hidden /><b>{s.name}</b>{s.lang && <span className="aw-lang">{s.lang}</span>}</td>
                      <td><code>{s.repo}</code></td>
                      <td>{s.via}</td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </section>
            <aside className="aw-sec" aria-labelledby="aw-mf">
              <h2 id="aw-mf">Share the set with your team</h2>
              {/* The server writes the manifest itself (CreateWorkspace.manifest, C7), so it is the one cartograph init reads. */}
              <p className="aw-sub">Building the first index writes <code>cartograph.yaml</code> beside {src.file ? <code>{src.file}</code> : "the folder you picked"}, listing these repositories; one that is already there is kept. Commit it, so teammates and CI index the same set.</p>
            </aside>
          </div>
        )}

        {step === 3 && src && (
          <section className="aw-sec aw-narrow" aria-labelledby="aw-h">
            <h1 id="aw-h" ref={title} tabIndex={-1}>{failed ? `The first index of ${src.name} stopped` : done ? `${src.name} is indexed` : `Building the first index of ${src.name}`}</h1>
            <p className="aw-sub">The index is written outside your repositories, under <code>~/.cartograph/workspaces/</code>.{building && !demo && src.estimate ? ` It should take about ${Math.ceil(src.estimate / 60)} min.` : ""}</p>
            {failed ? <p className="aw-err" role="alert">{say(failed)}</p> : (
              <div className="aw-prog" aria-live="polite">
                <div role="progressbar" aria-label="First index" aria-valuemin={0} aria-valuemax={100} aria-valuenow={progress}><span style={{ width: `${progress}%` }} /></div>
                <span>{done ? `${src.services.length} services${demo && ws ? `, ${ws.commits} commits` : ""}` : `${stage} · ${progress}%${eta}`}</span>
              </div>
            )}
            {done && <div className="aw-act"><button type="button" className="btn primary" onClick={open}>Open the map</button></div>}
            {building && <p className="aw-sub">Cancel stops the build. Nothing is added to your workspaces, and files already parsed stay in the cache, so the next try starts where this one stopped.</p>}
          </section>
        )}
      </main>

      <footer className="aw-foot">
        {step > 0 && !building && !done && <button type="button" className="btn" onClick={back}>Back</button>}
        {!building && !done && <button type="button" className="aw-cancel" onClick={leave}>Cancel</button>}
        <span className="aw-sum" aria-live="polite">
          {step === 1 && !none && `${repoCount} ${repoCount === 1 ? "repository" : "repositories"}${demo && ws ? ` · ${ws.commits} commits` : ""}`}
          {step === 2 && `${src?.services.length ?? 0} services`}
        </span>
        {step === 1 && !none && <button type="button" className="btn primary" onClick={() => setStep(2)}>Continue to services</button>}
        {step === 1 && none && <button type="button" className="btn primary" onClick={back}>Choose another folder</button>}
        {step === 2 && <button type="button" className="btn primary" onClick={build}>Build the first index</button>}
        {failed && <button type="button" className="btn primary" onClick={build}>Build again</button>}
        {building && <button type="button" className="btn" onClick={cancel}>Cancel the build</button>}
      </footer>
      <Toasts />
    </div>
  );
}
