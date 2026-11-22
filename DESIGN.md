# Design

Pinned by the user: follow the "Policy Network Dashboard" Dribbble concept (handshake). The codebase map translates it one-to-one; do not drift from it.

## Scene

Developers and leads at a desk, long sessions, often next to an editor in dark theme. Dark ink-navy ground.

## World

- Ground: deep navy slate (`--bg #141821`), canvas a step darker than panels; panels are flat slabs separated by 1px hairlines, no card soup.
- Network: colored dots per service (gold, teal, rose, violet, steel blue, amber), large pale hubs with bold two-line labels, thin cool-grey links, faint dashed orbit rings.
- Ring nodes are hollow (stroke only) for files, filled for services/modules, matching the reference's filled vs ringed dots.
- Accent: electric blue `#1aa3ff` for selection, the scrubber, the focused path. Lime `#8fd14f` up, coral `#ff5d5d` down, always with arrow glyphs drawn as SVG.
- Floating rail on the left: rounded slabs with dot, two-line label, value and delta; the selected one gets a blue 1.5px outline.
- Right panel: title + period select, KPI tabs (number, delta, label; active tab raised), teal area chart with dot markers, ranked table.
- Bottom: month ruler with minor ticks, blue scrubber knob and a date bubble above it, year stepper and prev/next round buttons.
- Focus view: selected node as a big blue disc with soft halo; "Depends on" on the left arc and "Used by" on the right arc, joined by long bezier curves; hovered curve turns blue.

## Type

Sora for UI (wide geometric, close to the reference). Tabular numerals in all figures. **Mono only for code:** paths, SHAs, symbols and code blocks. Never for labels, counts or buttons.

| Token | Size | Use |
| --- | --- | --- |
| `--fs-xs` | 0.75rem (12px, the floor) | Chips, captions, badges |
| `--fs-sm` | 0.8125rem | Secondary text, table cells, tab labels |
| `--fs-md` | 0.875rem | Body |
| `--fs-lg` | 1.0625rem | Section titles, brand |
| `--fs-xl` | 1.375rem | Page titles, big KPI |
| `--fs-2xl` | 1.75rem | Empty-state and welcome headings |

Nothing under 12px, ever. Write sizes in `rem`. Spacing is `--space-1` to `--space-8` (4, 8, 12, 16, 20, 24, 32, 40px). Radii: `--radius-sm` 6px, `--radius-md` 10px, `--radius-lg` 14px. Layout tokens: `--sb-w` 400px (sidebar), `--rail-w` 280px (rail).

### Target sizes

- 24px minimum for any control, 32px for primary controls (tabs, buttons, inputs, tree rows). Small icons get padding to reach it.
- Focus is a 2px `--accent` ring with 2px offset (`:focus-visible`), never removed. A `.skip-link` ("Skip to map") is the first tab stop and shows on focus.
- Links styled as buttons (`a.btn`) keep the button colour, not link blue or visited purple.

## Motion

One authored moment: switching Network ⇄ Ontology morphs every node to its new position (transform transition, exponential ease-out). Scrubbing time fades links in/out; newly added links flash lime, removed ones ghost coral.

## Themes (decision D13, board S00)

Dark is the default and lives on bare `:root` in `app/globals.css`. `:root[data-theme="light"]` overrides it, and `data-theme="system"` applies the light values under `prefers-color-scheme: light`. `lib/theme.ts` reads and writes `cartograph:appearance` and sets the `data-*` attributes; an inline script in `app/layout.tsx` sets them before paint. Every colour in CSS is a token. No hex literals in components, except domain hues that come from data.

| Token | Used for | Dark | Light |
| --- | --- | --- | --- |
| `--bg` | App ground | `#141821` | `#f4f6fa` |
| `--canvas` | Map and page canvas | `#191e29` | `#eef1f6` |
| `--panel` | Panels, header | `#1f2533` | `#ffffff` |
| `--slab`, `--slab-hi` | Rows, tiles, inputs; raised | `#252c3c`, `#2c3447` | `#eef1f6`, `#e3e8f0` |
| `--line`, `--line2`, `--line3` | Hairlines, stronger, strongest | `#2e3648`, `#3a4359`, `#4a5369` | `#d8deea`, `#c3cbdb`, `#aab3c5` |
| `--text`, `--body`, `--muted`, `--faint` | Headings; running text; labels; hints | `#e8ebf3`, `#c9cedb`, `#9aa3b8`, `#949db3` | `#141821`, `#363e52`, `#556074`, `#5e677c` |
| `--accent`, `--on-accent` | Selection, scrubber; text on accent | `#1aa3ff`, `#062338` | `#08649f`, `#ffffff` |
| `--accent-soft`, `--accent-line` | Selected row wash; outline | `rgba(26,163,255,.12)`, `#1d4a6e` | `rgba(8,100,159,.09)`, `#a9cdee` |
| `--up`, `--up-line`, `--up-bg` | Added, rising | `#8fd14f`, `#3c5a24`, `rgba(143,209,79,.06)` | `#316c0c`, `#bcd9a4`, `rgba(49,108,12,.06)` |
| `--down`, `--down-line`, `--down-text`, `--down-bg` | Removed, falling, errors | `#ff6a64`, `#5e2b2a`, `#f3c6c4`, `rgba(255,106,100,.07)` | `#bf322c`, `#efb7b4`, `#8a1f1a`, `rgba(191,50,44,.06)` |
| `--warn`, `--warn-line`, `--warn-bg`, `--on-warn` | Stale, expiring | `#ffb26b`, `#6b4a2a`, `#241e18`, `#2a1a08` | `#94500b`, `#ebc9a2`, `#fff6ec`, `#ffffff` |
| `--warn-soft`, `--warn-text`, `--warn-body`, `--warn-muted` | Text and washes on a warn banner | `rgba(255,178,107,.12)`, `#f3e2cf`, `#d9c3ad`, `#b89a7c` | `rgba(148,80,11,.1)`, `#4a2a06`, `#5e3a12`, `#7a5530` |
| `--demo-bg`, `--demo-line`, `--demo-text` | Demo-data banner | `#2a2617`, `#4a4020`, `#f3e6bd` | `#fbf3d9`, `#e6d28f`, `#5a4512` |
| `--hue-dim` | Darkening for text set in a hue | `0%` | `42%` |
| `--edge` | Map links and spokes | `#9aa3b8` | `#7d869b` |
| `--shadow-1` to `--shadow-4` | Raised, row, popover, dialog | black at .35 to .6 | ink at .12 to .2 |

Also tokens: `--seg`, `--seg-on`, `--bar`, `--tick`, `--tick-major`, `--chart`, `--chart-line`, `--hair`, `--scrim`, `--on-node`, `--node-mix`, `--stroke-keep`, `--demo-hue`. `--slab2` and `--accent-ink` are aliases of `--slab-hi` and `--on-accent`.

Other `data-*` modes (board S07), all set on `<html>`:

- `data-contrast="more"` darkens `--line*`, `--muted` and `--edge`, and sets `--link-w: 1.75`. Links read `stroke-width: var(--link-w)`.
- `data-motion="reduce"` makes transitions and animations instant. `follow` (default) honours `prefers-reduced-motion`, `full` ignores it.
- `data-size="90|100|115|130"` sets `--ts` and the root font size. Size new text in `rem`; map labels multiply by `var(--ts)`.
- `data-density="compact"` sets `--row-y` (row padding) from 12px to 6px. Keep 32px minimum targets.
- `data-hues="safe"` switches to the colour-blind-safe domain set. The attribute is set; the map reads it.

### Theme rules

- Domain hues are fixed. They are the same in both themes; only the surface behind them changes.
- Text set in a hue uses `color-mix(in oklab, <hue>, black var(--hue-dim))`, so it stays readable on light surfaces.
- On the map, light-mode fills get lighter (`--node-mix`) and strokes darker (`--stroke-keep`), so a node keeps 3:1 against the canvas.
- Colour is never the only signal. Lime and coral come with arrows; uncertain links are dotted.
- Text tokens pass 4.5:1 on `--bg`, `--panel`, `--slab` and `--slab-hi` in both themes (checked; lowest is `--faint` dark on `--slab-hi`, 4.58). Lines are decorative.
- `--ph` is the placeholder colour and equals `--muted`. The base layer sets `::placeholder`, so inputs need no rule of their own.

## Colour rule (decision I8)

Domains take the six pinned hues (gold, teal, rose, violet, steel blue, amber). Services are tints of their domain's hue: the busiest service by fan-in gets the base hue, the others a lighter and a darker tint. Everything past six domains is steel (`#6f7a90`, at 100%, 70% and 45%). The tree, chips and timeline ticks use the same colour as the map. A domain comes from the repo's top folder, or a `domain:` key in the override file.

## Link encoding and confidence chips

Confidence shows as line style, and time as colour, so the two never compete.

| Level | Score | Line | Chip |
| --- | --- | --- | --- |
| Exact | 0.90 to 1.00 | solid | `Exact · 0.96` |
| Likely | 0.60 to 0.89 | dotted | `Likely · 0.62` |
| Guessed | 0.30 to 0.59 | dotted, fainter | `Guessed · 0.41` |

Links under 0.30 are hidden by default. An overridden link has a diamond at its middle and the chip `Overridden`. Links that cross a service boundary draw at 2.25px, a bundle of N links has width proportional to the square root of N. Added in the last 30 days is lime, removed is coral and dashed, selected or on a path is accent.

State chips share one vocabulary across header, panel and Ask: `Demo data` (demo hue), `14 commits behind HEAD` (warn), `Index v3 · built 2h ago` (muted), `Citation checked` (up, with a check), `Citation dropped` (down, with a cross), `Written by a model`, `Likely · 0.62`, `Overridden`. Chips are pills, 12px, 1px border in the chip's colour.

## App shell (decision I9)

- **Header** (60px): brand, workspace switcher with a `Demo data` chip, view tabs, search (`Cmd+K`), index status chip, pins slot, Ask, activity bell with unread count, settings. Components: `components/shell/AppShell.tsx`.
- **Header tabs:** Map, Entrypoints, Contracts, Concepts, Coverage, Impact. Real links to `/`, `/entrypoints`, `/contracts`, `/concepts`, `/coverage`, `/impact`. The current tab has `aria-current="page"`, `--text` colour, weight 600 and a 2px `--accent` underline; the others are `--muted`. Map modes (Network, Adaptive, Ontology) live inside the Map view, not in the tabs. Ask is not a tab: its one header entry is the Ask button beside notifications, which opens the chat in its current mode and is marked current on `/ask`.
- **Settings nav:** a fixed 248px left column on `--panel`, with a `Back to the map` link, then three groups with 11px uppercase `--muted` labels. Workspace (named after the workspace): Sources and connections, Repositories, Indexing, Services and domains, Call matching. AI: Models, Use from your editor, Retrieval and privacy. This app: Appearance, Keyboard. Items are 40px high with a 10px radius; the current one has `aria-current="page"`, a `--slab` fill, `--text` colour and weight 600. Routes are `/settings/<section>`. Below 900px the nav becomes a wrapped row above the page.
- **Empty state pattern:** a centred icon, a title that says what will appear, one sentence on why it isn't there yet, and one action that fills it. Use `components/shell/EmptyState.tsx` and never leave a blank panel.
- **Prefixes:** shell `.sh-`, sidebar `.sb-`, map `.map-`, timeline `.tl-`, views `.view-`, settings `.set-`, ask `.ask-`. The older unprefixed classes in `globals.css` (`.chip`, `.row`, `.seg`, `.badge`, `.btn` and so on) belong to the map and panel; do not reuse their names.

## Right sidebar (`components/sidebar/Sidebar.tsx`)

One right-hand column holds the Details and Ask tabs. Right side only: free docking was rejected because it adds states nobody needs.

| Aspect | Rule |
| --- | --- |
| Width | `--sb-w` 400px, resizable 340 to 560px by dragging its left edge (arrow keys too) |
| Collapsed | 44px strip of tab icons; Ask shows an unread dot |
| Persistence | `localStorage` `cartograph:sidebar` as `{open, tab, width}` |
| Below 1180px | Overlay drawer at the same width, no scrim |
| Below 640px | Bottom sheet with peek (title and KPIs), half and full heights |
| Content | Each tab fills the column (`height: 100%`) and scrolls inside; no absolute positioning |

Keys: `]` toggles the sidebar, `[` toggles the rail, `/` opens Ask and focuses the composer, `Esc` in the composer returns focus to the map. Keys are ignored while typing in a field.

- **Details:** sticky node header, then a jump bar (Overview, Connections, Changes, Corrections) over collapsible sections in that order. Inspectors push onto a back stack with a `‹ Back` crumb. Scroll resets on selection change.
- **Ask** (Cartograph AI, mark `components/brand/AiMark.tsx`, board B1): one chat in three modes sharing one store (`components/ask/store.ts`, `cartograph:ask`): **Sidebar** (this tab), **Floating** (a movable, resizable panel on any route, minimises to a launcher) and **Full page** (`/ask`, laid out like Claire: no sidebar, past chats in a header history menu, a 660px conversation column, the composer as the hero of a new chat). An "Open in" menu switches modes; leaving the sidebar closes it. The composer (`components/ask/Composer.tsx`) is assistant-ui's `ComposerPrimitive` with `@` and `/` suggestions, one focus ring on the container, Enter sends, Shift+Enter breaks a line, Stop replaces Send while streaming. In the sidebar: thread oldest first, "Past questions" folded at the top, composer pinned to the bottom with the model picker, a removable `@name` context chip, suggestion chips only while the thread is empty. A citation selects the node and switches to Details with a `‹ Back to answer` crumb.

## ViewSplit (views)

Entrypoints, Contracts, Concepts, Coverage and Impact share one layout: a list or table on the left, the selected item's detail on the right, one `<h1>` and a one-line purpose above. Rows are a single tab stop with arrow keys (roving tabindex). Selection lives in the URL hash. Below 900px the detail stacks under the list. A view links to the map as `/#focus=<id>&back=<route>` or `/#trace=<id>&back=<route>`, and the map shows a `‹ Back to <View>` chip.

## Saving settings

| Setting kind | Behaviour |
| --- | --- |
| App-only (appearance, keyboard, models, privacy) | Saves on change. Show "Saved" beside the control for 2 seconds. No Save button. |
| File-backed (writes `cartograph.yaml`) | Edits are drafts. A sticky bar at the bottom shows the change count, "Discard" and "Save to cartograph.yaml". Leaving with unsaved edits asks first. |

Never mix the two on one page without a heading that says which is which.

## Status chips

- Never hide a warn or down chip, at any width. Collapse the muted ones first, then truncate labels.
- One source of truth for freshness: the index status chip owns "built 2h ago" and "N commits behind HEAD". The canvas bar, panel and Ask read it; none computes its own.
- A chip has a text label, an icon or glyph, and its colour. Colour is never the only signal.

## Copy rules

- Sentence case everywhere. Plain words. No exclamation marks.
- A control names its action: "Save to cartograph.yaml", "Add repository", not "OK" or "Submit".
- Empty state: what will appear, why it is not there yet, one action that fills it.
- Never promise an action that does not exist. Cartograph is local only: no hosted sharing, webhooks or pull-request integrations.
