"use client";

import { useEffect, useRef } from "react";
import { ChevronLeft } from "lucide-react";
import type { Answer, Handle } from "@/lib/ask";
import { fmtDay } from "./format";
import "./ask.css";

const WHY: Record<string, string> = {
  impact: "the question asks what breaks if something changes",
  path: "the question names two things and asks how they connect",
  describe: "the question names one thing and asks what it is",
  history: "the question asks what changed over time",
  none: "no file, module or service was named",
};

/** Board A06: every tool call, the tokens per block against the budget, which handles were cited or dropped, and the system prompt. */
export default function Inspector({ answer, onClose }: { answer: Answer; onClose: () => void }) {
  const ref = useRef<HTMLDialogElement>(null);
  const ctx = answer.context!;
  useEffect(() => { ref.current?.showModal(); }, []);

  const cited = new Map(answer.checked?.paragraphs.flat().flatMap((s) => s.cites).map((c) => [c.handle, c]) ?? []);
  const outcome = (h: Handle) => (!answer.checked ? "Context" : cited.get(h.handle)?.ok ? "Cited" : cited.has(h.handle) ? "Dropped" : "Context");
  const chipClass = (o: string) => (o === "Cited" ? "ok" : o === "Dropped" ? "warn" : "");
  const stray = [...cited.values()].filter((c) => !ctx.handles.some((h) => h.handle === c.handle));
  const reachedBy = (handle: string) => ctx.tools.find((t) => t.rows.includes(handle))?.tool ?? "";
  const nodes = ctx.handles.filter((h) => h.kind === "node").length;
  const links = ctx.handles.filter((h) => h.kind === "link").length;
  const st = answer.checked?.stats;

  return (
    <dialog ref={ref} className="ask-insp" aria-labelledby="ask-insp-h" onClose={onClose}>
      <div className="ask-insp-in">
        <div className="ask-insp-top">
          <div>
            <span className="ask-k">Retrieval inspector · how this answer was built</span>
            <h1 id="ask-insp-h">{answer.q}</h1>
            <div className="ask-meta">
              <span className="ask-chip acc">Plan: {answer.plan ?? "none"}</span>
              <span className="ask-chip">As of {fmtDay(answer.asOf ?? 0)}</span>
              {answer.by && <span className="ask-chip">{answer.by}</span>}
              {st && <span className="ask-chip ok">{st.kept} of {st.cited} citations kept</span>}
            </div>
          </div>
          <button type="button" className="btn" onClick={() => ref.current?.close()}><ChevronLeft size={14} aria-hidden /> Back to the answer</button>
        </div>

        <div className="ask-grid">
          <section className="ask-sec" aria-labelledby="ask-i-steps">
            <h2 id="ask-i-steps">Steps</h2>
            <div className="ask-step"><span>1</span><span><b>Plan.</b> Classed as {answer.plan ?? "none"}, because {WHY[answer.plan ?? "none"]}.</span></div>
            <div className="ask-step"><span>2</span><span><b>Seed.</b> {ctx.seeds.length ? `${ctx.seeds.length} named in the question, matched by exact name.` : "Nothing in the question matched a name."}</span></div>
            <div className="ask-step"><span>3</span><span><b>Expand.</b> {ctx.tools.length ? ctx.tools.map((t) => t.tool).join(", ") : "No tools ran"}. Found {nodes} nodes and {links} links.</span></div>
            <div className="ask-step"><span>4</span><span><b>Assemble.</b> Sent {ctx.tokens.toLocaleString("en-GB")} of {ctx.budget.toLocaleString("en-GB")} tokens. {ctx.leftOut} nodes left out.</span></div>
            <div className="ask-step"><span>5</span><span><b>Check.</b> {st ? `${st.kept} citations kept, ${st.dropped} dropped, ${st.unsupported} of ${st.sentences} sentences with no checked source.` : "Not run. This is the map's own wording."}</span></div>
          </section>

          <section className="ask-sec" aria-labelledby="ask-i-tools">
            <h2 id="ask-i-tools">Tool calls</h2>
            {ctx.tools.length === 0 && <p className="ask-text">No tool returned rows for this question.</p>}
            <table className="ask-tab">
              <thead><tr><th className="ask-k">Tool</th><th className="ask-k">Arguments</th><th className="ask-k">Rows</th></tr></thead>
              <tbody>
                {ctx.tools.map((t, i) => (
                  <tr key={i}>
                    <td className="ask-mono">{t.tool}</td>
                    <td className="ask-mono" style={{ overflowWrap: "anywhere" }}>{t.args}</td>
                    <td className="ask-mono">{t.rows.length}</td>
                  </tr>
                ))}
              </tbody>
            </table>
            <h2 style={{ marginTop: 8 }}>Budget (tokens)</h2>
            <div className="ask-bud" role="progressbar" aria-label="Context budget used, in tokens" aria-valuemin={0} aria-valuemax={ctx.budget} aria-valuenow={ctx.tokens}>
              {ctx.blocks.map((b, i) => <span key={b.name} className={`ask-b${i}`} style={{ width: `${(b.tokens / ctx.budget) * 100}%` }} />)}
            </div>
            <ul className="ask-legend">
              {ctx.blocks.map((b, i) => <li key={b.name}><span className={`ask-sw ask-b${i}`} />{b.name} {b.tokens.toLocaleString("en-GB")}</li>)}
              <li>{ctx.tokens.toLocaleString("en-GB")} of {ctx.budget.toLocaleString("en-GB")} tokens</li>
            </ul>
            <details>
              <summary>System prompt</summary>
              <pre className="ask-prompt">{ctx.system}</pre>
            </details>
          </section>

          <section className="ask-sec" aria-labelledby="ask-i-ctx">
            <h2 id="ask-i-ctx">Context sent to the model</h2>
            <table className="ask-tab">
              <thead><tr><th className="ask-k">Handle</th><th className="ask-k">Row</th><th className="ask-k">Reached by</th><th className="ask-k">Outcome</th></tr></thead>
              <tbody>
                {ctx.handles.map((h) => {
                  const o = outcome(h);
                  return (
                    <tr key={h.handle}>
                      <td className="ask-mono">[{h.handle}]</td>
                      <td className="l" title={`${h.kind}: ${h.label}`}>{h.label}</td>
                      <td>{reachedBy(h.handle)}</td>
                      <td><span className={`ask-chip ${chipClass(o)}`}>{o}</span></td>
                    </tr>
                  );
                })}
                {stray.map((c) => (
                  <tr key={c.handle}>
                    <td className="ask-mono">[{c.handle}]</td>
                    <td className="l">{c.why}</td>
                    <td />
                    <td><span className="ask-chip warn">Dropped</span></td>
                  </tr>
                ))}
              </tbody>
            </table>
            <p className="ask-text" style={{ margin: 0, color: "var(--muted)" }}>{ctx.leftOut} nodes left out of this context{ctx.ultra ? ", to stay inside the budget" : ""}.</p>
          </section>
        </div>
      </div>
    </dialog>
  );
}
