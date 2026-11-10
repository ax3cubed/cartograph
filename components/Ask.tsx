"use client";

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { ArrowUp, Route, Settings2, X } from "lucide-react";
import { SUGGESTIONS, ask, type Answer, type Part } from "@/lib/ask";
import { DEFAULT_AI, linkify, loadAi, modelName, narrate, saveAi, type AiSettings } from "@/lib/ai";
import ModelPicker, { type PickerValue } from "./ui/model-picker";
import Settings from "./Settings";
import { byId, commitBySha, serviceColor } from "@/lib/workspace";

/** Renders answer text with nodes and commits as clickable evidence. */
export function Parts({ parts, onOpen }: { parts: Part[]; onOpen: (id: string) => void }) {
  return (
    <>
      {parts.map((p, i) => {
        if (typeof p === "string") return <span key={i}>{p}</span>;
        if ("node" in p) {
          const n = byId.get(p.node)!;
          return (
            <button key={i} className="ref" style={{ ["--c" as string]: serviceColor(n.service) }} onClick={() => onOpen(n.id)} title={n.path}>
              {n.name}
            </button>
          );
        }
        const c = commitBySha.get(p.commit)!;
        return <code key={i} className="ref-commit" title={`${c.message} · ${c.author} · ${c.date}`}>{c.sha}</code>;
      })}
    </>
  );
}

interface Props {
  t: number;
  answer: Answer | null;
  onAnswer: (a: Answer | null) => void;
  onOpen: (id: string) => void;
}

export default function Ask({ t, answer, onAnswer, onOpen }: Props) {
  const [q, setQ] = useState("");
  const [ai, setAi] = useState<AiSettings>(DEFAULT_AI);
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [busy, setBusy] = useState(false);
  const stream = useRef<AbortController | null>(null);

  useEffect(() => setAi(loadAi()), []);
  const update = useCallback((next: AiSettings) => { setAi(next); saveAi(next); }, []);
  const pick = useCallback((v: PickerValue) => setAi((s) => { const next = { ...s, ...v }; saveAi(next); return next; }), []);
  const models = useMemo(() => ai.models.map((id) => ({ id, name: modelName(id) })), [ai.models]);
  const model = ai.models.includes(ai.model) ? ai.model : ai.models[0];

  const close = () => { stream.current?.abort(); onAnswer(null); };
  const run = async (text: string) => {
    if (!text.trim()) return;
    stream.current?.abort();
    const base = ask(text, t); // the graph engine answers first: instant, and it is the evidence the model words
    onAnswer(base);
    setQ("");
    if (!model) return;
    const ctl = (stream.current = new AbortController());
    setBusy(true);
    try {
      let out = "";
      for await (const chunk of narrate({ ...ai, model }, base, t, ctl.signal)) {
        out += chunk;
        onAnswer({ ...base, lines: out.split(/\n+/).filter((l) => l.trim()).map(linkify), by: modelName(model) });
      }
    } catch (e) {
      if (!ctl.signal.aborted) {
        onAnswer({ ...base, steps: [...base.steps, `The model call failed, so this is the map's own answer. ${e instanceof Error ? e.message : e}`] });
      }
    } finally {
      if (stream.current === ctl) setBusy(false);
    }
  };

  return (
    <div className="ask">
      {answer && (
        <article className="answer" aria-live="polite">
          <header>
            <p className="answer-q">{answer.q}</p>
            <button className="icon-btn sm" onClick={close} aria-label="Close answer"><X size={14} /></button>
          </header>
          <h2>{answer.title}</h2>
          {(answer.by || busy) && <p className="answer-by">{answer.by ? `Worded by ${answer.by} from the map's evidence` : "Asking the model…"}</p>}
          <div className="answer-body">
            {answer.lines.map((l, i) => (
              <p key={i}><Parts parts={l} onOpen={onOpen} /></p>
            ))}
          </div>
          {answer.steps.length > 0 && (
            <details className="steps-trail">
              <summary><Route size={13} aria-hidden /> How I worked this out</summary>
              <ol>{answer.steps.map((s, i) => <li key={i}>{s}</li>)}</ol>
            </details>
          )}
        </article>
      )}
      <form className="ask-bar" onSubmit={(e) => (e.preventDefault(), run(q))}>
        <input
          value={q}
          onChange={(e) => setQ(e.target.value)}
          placeholder="Ask the map: what breaks if I change schema.ts?"
          aria-label="Ask a question about the codebase"
        />
        {models.length > 0 ? (
          <>
            <button type="button" className="icon-btn sm" onClick={() => setSettingsOpen(true)} aria-label="Model settings"><Settings2 size={14} /></button>
            <ModelPicker models={models} value={{ model: model!, tier: ai.tier, ultra: ai.ultra }} onChange={pick} />
          </>
        ) : (
          <button type="button" className="model-setup" onClick={() => setSettingsOpen(true)}><Settings2 size={14} aria-hidden /> Choose a model</button>
        )}
        <button className="send" type="submit" aria-label="Ask" disabled={!q.trim()}><ArrowUp size={16} /></button>
      </form>
      <Settings open={settingsOpen} settings={ai} onSave={update} onClose={() => setSettingsOpen(false)} />
      {!answer && (
        <div className="chips" aria-label="Suggested questions">
          {SUGGESTIONS.map((s) => <button key={s} onClick={() => run(s)}>{s}</button>)}
        </div>
      )}
    </div>
  );
}
