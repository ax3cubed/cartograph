"use client";

import { useEffect, type FocusEvent, type RefObject } from "react";

/**
 * Roving tabindex: a group of items is one tab stop. Render every item with tabIndex={-1};
 * this keeps exactly one at 0 (the last focused, else `prefer`, else the first) and gives you `move`.
 */
export function useRoving(root: RefObject<Element | null>, groups: string | string[], prefer?: string) {
  const list = typeof groups === "string" ? [groups] : groups;
  const selector = list.join(", ");
  // groups keep their own order: nodes before links, whatever the DOM order is
  const items = () => list.flatMap((g) => [...(root.current?.querySelectorAll<HTMLElement | SVGElement>(g) ?? [])]);
  useEffect(() => {
    const all = items();
    if (!all.length || all.some((el) => el.tabIndex === 0)) return;
    const first = (prefer && root.current?.querySelector<HTMLElement | SVGElement>(prefer)) || all[0];
    first.tabIndex = 0;
  });
  const make = (el: Element) => {
    for (const x of items()) x.tabIndex = x === el ? 0 : -1;
  };
  /** Focus the item `by` steps from `from` (or the first / last). */
  const move = (from: Element, by: number | "first" | "last") => {
    const all = items();
    if (!all.length) return;
    const i = all.indexOf(from as HTMLElement);
    const next = by === "first" ? all[0] : by === "last" ? all[all.length - 1] : all[Math.min(all.length - 1, Math.max(0, i + by))];
    make(next);
    next.focus();
  };
  return {
    selector,
    /** Arrow keys, Home and End over the items. `keys` maps a key to a step; returns true when it moved. */
    onKey: (e: { key: string; target: EventTarget; preventDefault: () => void }, keys: Record<string, number> = { ArrowDown: 1, ArrowRight: 1, ArrowUp: -1, ArrowLeft: -1 }) => {
      const el = e.target as Element;
      if (!el.matches?.(selector)) return false;
      const by = e.key === "Home" ? "first" : e.key === "End" ? "last" : keys[e.key];
      if (by === undefined) return false;
      e.preventDefault();
      move(el, by);
      return true;
    },
    /** Pass to the container's onFocus so a clicked or programmatically focused item becomes the tab stop. */
    onFocus: (e: FocusEvent) => { if ((e.target as Element).matches?.(selector)) make(e.target as Element); },
    move,
  };
}
