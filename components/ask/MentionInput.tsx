"use client";

import { useId, useState, type KeyboardEvent, type RefObject } from "react";
import { Box, FileCode2, Layers, Wrench } from "lucide-react";
import { TOOLS, type Ref, type ToolId } from "@/lib/ask";
import { search } from "@/lib/source";
import { NODES } from "@/lib/workspace";

export type { Ref } from "@/lib/ask";
export { parseComposer } from "@/lib/ask";

type Item =
  | { kind: "node"; id: string; name: string; detail: string; nodeKind: string }
  | { kind: "tool"; id: ToolId; name: string; detail: string; args: string };

const NODE_KINDS = new Set(["service", "module", "file"]);
const ICON = { service: Layers, module: Box, file: FileCode2 } as const;

/** The "@" or "/" being typed just before the caret, if any. "/" counts only at the start of the box. */
export function triggerAt(value: string, caret: number) {
  const m = /(^|\s)([@/])([^\s@/]*)$/.exec(value.slice(0, caret));
  if (!m || (m[2] === "/" && m.index !== 0)) return null;
  return { char: m[2] as "@" | "/", query: m[3], start: caret - m[3].length - 1 };
}
type Trigger = ReturnType<typeof triggerAt>;

function itemsFor(trig: Trigger): Item[] {
  if (!trig) return [];
  if (trig.char === "/") {
    return TOOLS.filter((t) => t.id.startsWith(trig.query.toLowerCase())).map((t) => ({ kind: "tool", id: t.id, name: t.label, detail: t.hint, args: t.args }));
  }
  // An empty "@" lists services, so the menu is never blank on the first keystroke.
  if (!trig.query) return NODES.filter((n) => n.kind === "service").slice(0, 8).map((n) => ({ kind: "node", id: n.id, name: n.name, detail: n.path, nodeKind: n.kind }));
  return search(trig.query, 8)
    .filter((r) => NODE_KINDS.has(r.kind))
    .map((r) => ({ kind: "node", id: r.id, name: r.name, detail: r.path, nodeKind: r.kind }));
}

type Field = HTMLTextAreaElement | HTMLInputElement;

/**
 * "@" (files, modules, services) and leading "/" (tools) suggestions for any text field.
 * `setText` must write the new text into the field's owner (here, the assistant-ui composer) and report
 * it with the refs; the hook only tracks the trigger, the active row and the caret.
 */
export function useMentions({ field, text, refs, setText }: {
  field: RefObject<Field | null>;
  text: string;
  refs: Ref[];
  setText: (text: string, refs: Ref[]) => void;
}) {
  const listId = useId();
  const [trig, setTrig] = useState<Trigger>(null);
  const [active, setActive] = useState(0);
  const shown = itemsFor(trig);
  const open = !!trig && shown.length > 0;

  const caretTo = (at: number) => requestAnimationFrame(() => { field.current?.focus(); field.current?.setSelectionRange(at, at); });

  /** Call after the field's text or caret moved. */
  const sync = (value: string, caret: number) => { setTrig(triggerAt(value, caret)); setActive(0); };

  const choose = (item: Item) => {
    if (!trig) return;
    const caret = field.current?.selectionStart ?? text.length;
    const token = item.kind === "tool" ? item.name : `@${item.name}`;
    const next = text.slice(0, trig.start) + token + " " + text.slice(caret);
    const nextRefs = item.kind === "node" ? [...refs.filter((r) => r.token !== token), { token, id: item.id }] : refs;
    setText(next, nextRefs);
    setTrig(null);
    caretTo(trig.start + token.length + 1);
  };

  /** Puts "@" at the caret (or "/" at the start) and opens its list, for the buttons in the action row. */
  const insert = (char: "@" | "/") => {
    const el = field.current;
    let next: string, at: number;
    if (char === "/") {
      const lead = /^\/\S*/.exec(text);
      next = lead ? text : "/" + text;
      at = lead ? lead[0].length : 1;
    } else {
      const caret = el && document.activeElement === el ? el.selectionStart ?? text.length : text.length;
      const pad = caret > 0 && !/\s/.test(text[caret - 1]) ? " " : "";
      next = text.slice(0, caret) + pad + "@" + text.slice(caret);
      at = caret + pad.length + 1;
    }
    if (next !== text) setText(next, refs);
    setTrig(triggerAt(next, at));
    setActive(0);
    caretTo(at);
  };

  /** Runs before the composer's own keydown: returning with preventDefault keeps Enter from sending. */
  const onKeyDown = (e: KeyboardEvent<Field>) => {
    if (!open) return;
    if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      e.preventDefault();
      setActive((a) => (a + (e.key === "ArrowDown" ? 1 : -1) + shown.length) % shown.length);
    } else if ((e.key === "Enter" && !e.shiftKey) || e.key === "Tab") {
      e.preventDefault(); // pick the suggestion instead of sending the question
      choose(shown[Math.min(active, shown.length - 1)]);
    } else if (e.key === "Escape") {
      e.preventDefault();
      e.stopPropagation();
      setTrig(null);
    }
  };

  const fieldProps = {
    role: "combobox",
    "aria-expanded": open,
    "aria-controls": listId,
    "aria-autocomplete": "list" as const,
    "aria-activedescendant": open ? `${listId}-${active}` : undefined,
    "aria-describedby": `${listId}-hint`,
    autoComplete: "off",
    onKeyDown,
    // Clicking into an "@sch" token reopens its list. Not onSelect: React fires it on later keydowns too, which would undo Esc.
    onClick: (e: React.MouseEvent<Field>) => {
      const el = e.currentTarget;
      if (el.selectionStart === el.selectionEnd) setTrig(triggerAt(el.value, el.selectionStart ?? el.value.length));
    },
    onBlur: () => setTrig(null),
  };

  const list = (
    <>
      <span id={`${listId}-hint`} className="ask-cmp-sr">Type @ to name a file, module or service. Type / at the start to run a tool.</span>
      {open && (
        <ul id={listId} role="listbox" className="ask-cmp-suggest" aria-label={trig!.char === "/" ? "Tools" : "Files, modules and services"}>
          <li role="presentation" className="ask-cmp-suggest-head">{trig!.char === "/" ? "Tools" : "Files, modules and services"}</li>
          {shown.map((it, i) => {
            const Icon = it.kind === "tool" ? Wrench : ICON[it.nodeKind as keyof typeof ICON];
            return (
              <li
                key={it.id}
                id={`${listId}-${i}`}
                role="option"
                aria-selected={i === active}
                className="ask-cmp-suggest-row"
                onMouseDown={(e) => e.preventDefault()} // keep focus in the field
                onClick={() => choose(it)}
                onMouseEnter={() => setActive(i)}
              >
                <Icon size={14} aria-hidden />
                <span className="ask-cmp-suggest-name">{it.name}{it.kind === "tool" && <i> {it.args}</i>}</span>
                {it.detail !== it.name && <span className={`ask-cmp-suggest-detail${it.kind === "node" ? " is-path" : ""}`}>{it.detail}</span>}
              </li>
            );
          })}
        </ul>
      )}
    </>
  );

  return { open, sync, insert, fieldProps, list };
}
