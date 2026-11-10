"use client";

import { createElement, useEffect, useRef, useState } from "react";

export interface PickerModel { id: string; name: string; color?: string }
export interface PickerValue { model: string; tier: number; ultra: boolean }

interface Props {
  models: PickerModel[];
  value: PickerValue;
  onChange: (v: PickerValue) => void;
  className?: string;
}

type PickerEl = HTMLElement & { models: PickerModel[] };

/** React face of the <model-picker> web component. The element extends HTMLElement, so it is
 *  imported on the client only; props are pushed imperatively once it has upgraded. */
export default function ModelPicker({ models, value, onChange, className }: Props) {
  const ref = useRef<PickerEl>(null);
  const [ready, setReady] = useState(false);

  useEffect(() => {
    let live = true;
    import("./model-picker-element.js").then(() => live && setReady(true));
    return () => { live = false; };
  }, []);

  useEffect(() => {
    const el = ref.current;
    if (!ready || !el) return;
    el.models = models; // before `model`, which is validated against the list
    el.setAttribute("model", value.model);
    el.setAttribute("tier", String(value.tier));
    el.toggleAttribute("ultra", value.ultra);
  }, [ready, models, value.model, value.tier, value.ultra]);

  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    const h = (e: Event) => {
      const d = (e as CustomEvent<{ model: string; tierIndex: number; ultra: boolean }>).detail;
      if (d) onChange({ model: d.model, tier: d.tierIndex, ultra: d.ultra });
    };
    el.addEventListener("change", h);
    return () => el.removeEventListener("change", h);
  }, [onChange]);

  return createElement("model-picker", { ref, className, inline: "" });
}
