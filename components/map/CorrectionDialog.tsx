"use client";

// FE-14: correct one link. The verb, an optional target or level and a required reason go to the server, which checks
// them against the index; the preview is the server's diff of cartograph.overrides.yaml, and Save writes it. The map
// changes when the index has applied the entry, not before. The reason is the user's text and is shown as text.
import { useEffect, useRef, useState } from "react";
import type { LinkDetail, NodeDetail } from "@/lib/api";
import { RequestError } from "@/lib/api/client";
import { previewCorrection, saveCorrection } from "@/lib/api/corrections";
import { useStructure, useWsRead } from "@/lib/api/live";
import { toast } from "../shell/toast";
import { endOf, formProblem, keyOf, LEVELS, MAX_REASON, request, VERBS, type Form } from "./links";
import "./correction.css";

const said = (e: unknown) => (e instanceof RequestError ? `${e.what}. ${e.why}. ${e.try_}.` : "Something went wrong. Try again.");

export default function CorrectionDialog({ ws, d, onClose }: { ws: string; d: LinkDetail; onClose: () => void }) {
  const box = useRef<HTMLDialogElement>(null);
  const services = useStructure(ws).data?.services;
  const from = useWsRead<NodeDetail>(ws, `nodes/${encodeURIComponent(d.link.from)}`).data?.node;
  const to = useWsRead<NodeDetail>(ws, `nodes/${encodeURIComponent(d.link.to)}`).data?.node;
  const endFor = (n: typeof from) => (n?.service && services ? endOf(services.find((s) => s.id === n.service)?.name ?? "", n.path) : "");
  const [f, setF] = useState<Form>({ verb: "remove", from: "", to: "", level: "exact", reason: "" });
  const [target, setTarget] = useState("");
  const [diff, setDiff] = useState<string | null>(null);
  const [err, setErr] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const ends = { from: endFor(from), to: endFor(to) };
  const set = (p: Partial<Form>) => setF((x) => ({ ...x, ...p }));

  useEffect(() => {
    const el = box.current;
    if (el && !el.open) el.showModal();
    return () => el?.close();
  }, []);

  // `from` and `to` of an added link start as this link's ends; the other verbs name the link by them.
  const form: Form = { ...f, from: f.from || ends.from, to: f.verb === "retarget" ? target : f.to || ends.to };
  const problem = formProblem(form);
  const ready = !problem && !!ends.from && !!ends.to;
  const key = keyOf(d, ends.from, ends.to);
  const body = ready ? JSON.stringify(request(form, key)) : "";

  // The preview: the server's diff for what the form says now, a moment after the last keystroke.
  useEffect(() => {
    setDiff(null);
    if (!body) { setErr(null); return; }
    let live = true;
    const t = setTimeout(() => {
      previewCorrection(ws, JSON.parse(body)).then(
        (r) => { if (live) { setDiff(r.diff); setErr(null); } },
        (e) => { if (live) { setDiff(null); setErr(said(e)); } },
      );
    }, 300);
    return () => { live = false; clearTimeout(t); };
  }, [ws, body]);

  async function save() {
    setBusy(true);
    try {
      await saveCorrection(ws, JSON.parse(body));
      toast("Saved to cartograph.overrides.yaml. The map updates when the index has applied it.");
      onClose();
    } catch (e) {
      setErr(said(e));
      setBusy(false);
    }
  }

  const reasonLen = [...f.reason].length;
  return (
    <dialog ref={box} className="cd" aria-labelledby="cd-title" onCancel={(e) => { e.preventDefault(); onClose(); }} onClose={onClose}>
      <form method="dialog" onSubmit={(e) => { e.preventDefault(); if (ready && !busy) void save(); }}>
        <h2 id="cd-title">Correct this link</h2>
        <p className="cd-sub"><code>{ends.from || "…"}</code> to <code>{ends.to || "…"}</code>{d.operation && <> · <code>{keyOf(d, "", "").op}</code></>}</p>
        <fieldset className="cd-verbs">
          <legend>What is wrong</legend>
          {VERBS.map((v) => (
            <label key={v.verb}>
              <input type="radio" name="verb" value={v.verb} checked={f.verb === v.verb} onChange={() => set({ verb: v.verb })} />
              <span><b>{v.label}</b> <span className="cd-help">{v.help}</span></span>
            </label>
          ))}
        </fieldset>
        {f.verb === "retarget" && (
          <label className="cd-field">New target
            <input type="text" value={target} onChange={(e) => setTarget(e.target.value)} placeholder="service:path/to/file.ts" spellCheck={false} />
          </label>
        )}
        {f.verb === "add" && (
          <>
            <label className="cd-field">From
              <input type="text" value={form.from} onChange={(e) => set({ from: e.target.value })} spellCheck={false} />
            </label>
            <label className="cd-field">To
              <input type="text" value={form.to} onChange={(e) => set({ to: e.target.value })} spellCheck={false} />
            </label>
          </>
        )}
        {(f.verb === "set" || f.verb === "add") && (
          <label className="cd-field">Level
            <select value={f.level} onChange={(e) => set({ level: e.target.value as Form["level"] })}>
              {LEVELS.map((l) => <option key={l} value={l}>{l[0].toUpperCase() + l.slice(1)}</option>)}
            </select>
          </label>
        )}
        <label className="cd-field">Reason
          <textarea rows={3} value={f.reason} onChange={(e) => set({ reason: e.target.value })} aria-describedby="cd-count" required />
        </label>
        <p id="cd-count" className={reasonLen > MAX_REASON ? "cd-count is-over" : "cd-count"}>{reasonLen} / {MAX_REASON}. Written to the file for the next reader.</p>
        {err && <p className="cd-err" role="alert">{err}</p>}
        <div className="cd-preview" aria-live="polite">
          <span className="cd-k">Change to cartograph.overrides.yaml</span>
          {diff ? <pre>{diff.split("\n").map((l, i) => <span key={i} className={l.startsWith("+") ? "is-add" : l.startsWith("-") ? "is-del" : undefined}>{l}{"\n"}</span>)}</pre>
            : <p className="cd-none">{problem && f.reason ? problem : "The change shows here once the form is complete."}</p>}
        </div>
        <div className="cd-actions">
          <button type="button" className="btn" onClick={onClose}>Cancel</button>
          <button type="submit" className="btn primary" disabled={!ready || !diff || busy}>Save to the file</button>
        </div>
      </form>
    </dialog>
  );
}
