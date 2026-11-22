"use client";

import Link from "next/link";
import { usePathname } from "next/navigation";
import { ChevronLeft } from "lucide-react";
import { SETTINGS_GROUPS, SETTINGS_SECTIONS } from "@/lib/settings";
import { getCurrentWorkspace } from "@/lib/source";
import "./shell.css";

type NavItem = { slug: string; label: string; group: string };
/** "Models and MCP" is now "Models"; the MCP setup lives on its own page, "Use from your editor", right after it. */
const NAV: NavItem[] = (() => {
  const items: NavItem[] = SETTINGS_SECTIONS.map((s) => (s.slug === "models" ? { ...s, label: "Models" } : { ...s }));
  if (!items.some((s) => s.slug === "editor")) {
    const at = items.findIndex((s) => s.slug === "models");
    items.splice(at + 1, 0, { slug: "editor", label: "Use from your editor", group: "ai" });
  }
  return items;
})();

export default function SettingsNav() {
  const path = usePathname().replace(/\/$/, "");
  const ws = getCurrentWorkspace();
  return (
    <nav className="sh-snav" aria-label="Settings">
      <Link href="/" className="sh-snav-back"><ChevronLeft size={16} aria-hidden />Back to the map</Link>
      {(Object.keys(SETTINGS_GROUPS) as (keyof typeof SETTINGS_GROUPS)[]).map((g) => (
        <div key={g} className="sh-snav-group">
          <span className="sh-k">{SETTINGS_GROUPS[g]}{g === "workspace" && ws ? ` · ${ws.name}` : ""}</span>
          {NAV.filter((s) => s.group === g).map((s) => (
            <Link key={s.slug} href={`/settings/${s.slug}`} aria-current={path === `/settings/${s.slug}` ? "page" : undefined}>{s.label}</Link>
          ))}
        </div>
      ))}
    </nav>
  );
}
