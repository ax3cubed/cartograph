// Every route of the static export, with what says it has rendered. Add a route here and it gets a smoke test, an
// axe scan and a baseline in both themes.
import type { Locator, Page } from "@playwright/test";

const h1 = (name: string | RegExp) => (p: Page) => p.getByRole("heading", { level: 1, name });
const SETTINGS: [string, string][] = [
  ["connections", "Sources and connections"], ["repositories", "Repositories"], ["indexing", "Indexing"], ["services", "Services and domains"],
  ["contracts", "Call matching"], ["models", "Models"], ["editor", "Use from your editor"], ["retrieval", "Retrieval and privacy"],
  ["appearance", "Appearance"], ["keyboard", "Keyboard"],
];

export const ROUTES: { path: string; name: string; ready: (p: Page) => Locator }[] = [
  { path: "/", name: "map", ready: (p) => p.locator("svg.lm-svg") },
  { path: "/entrypoints", name: "entrypoints", ready: h1(/^Entrypoints/) },
  { path: "/contracts", name: "contracts", ready: h1(/^Contracts/) },
  { path: "/concepts", name: "concepts", ready: h1(/^Concepts/) },
  { path: "/coverage", name: "coverage", ready: h1(/^Coverage/) },
  { path: "/impact", name: "impact", ready: h1(/^Impact/) },
  { path: "/ask", name: "ask", ready: h1("Ask") },
  { path: "/welcome", name: "welcome", ready: h1("Map your own workspace") },
  { path: "/welcome/add", name: "welcome-add", ready: h1("Pick a folder or workspace file") },
  ...SETTINGS.map(([slug, title]) => ({ path: `/settings/${slug}`, name: `settings-${slug}`, ready: h1(title) })),
];

/** WCAG 2.2 A and AA. Best-practice rules (one main landmark and the like) are left to review. */
export const WCAG = ["wcag2a", "wcag2aa", "wcag21a", "wcag21aa", "wcag22aa"];
