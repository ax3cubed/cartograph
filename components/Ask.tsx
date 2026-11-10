"use client";

import { useState } from "react";
import { ArrowUp, Route, X } from "lucide-react";
import { SUGGESTIONS, ask, type Answer, type Part } from "@/lib/ask";
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
  const run = (text: string) => {
    if (!text.trim()) return;
    onAnswer(ask(text, t));
    setQ("");
  };

  return (
    <div className="ask">
      {answer && (
        <article className="answer" aria-live="polite">
          <header>
            <p className="answer-q">{answer.q}</p>
            <button className="icon-btn sm" onClick={() => onAnswer(null)} aria-label="Close answer"><X size={14} /></button>
          </header>
          <h2>{answer.title}</h2>
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
        <button className="send" type="submit" aria-label="Ask" disabled={!q.trim()}><ArrowUp size={16} /></button>
      </form>
      {!answer && (
        <div className="chips" aria-label="Suggested questions">
          {SUGGESTIONS.map((s) => <button key={s} onClick={() => run(s)}>{s}</button>)}
        </div>
      )}
    </div>
  );
}
