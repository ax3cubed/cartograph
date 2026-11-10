# Design

Pinned by the user: follow the "Policy Network Dashboard" Dribbble concept (handshake). The codebase map translates it one-to-one; do not drift from it.

## Scene

Developers and leads at a desk, long sessions, often next to an editor in dark theme. Dark ink-navy ground.

## World

- Ground: deep navy slate (`--bg #171b26`), canvas a step darker than panels; panels are flat slabs separated by 1px hairlines, no card soup.
- Network: colored dots per service (gold, teal, rose, violet, steel blue, amber), large pale hubs with bold two-line labels, thin cool-grey links, faint dashed orbit rings.
- Ring nodes are hollow (stroke only) for files, filled for services/modules, matching the reference's filled vs ringed dots.
- Accent: electric blue `#1aa3ff` for selection, the scrubber, the focused path. Lime `#8fd14f` up, coral `#ff5d5d` down, always with arrow glyphs drawn as SVG.
- Floating rail on the left: rounded slabs with dot, two-line label, value and delta; the selected one gets a blue 1.5px outline.
- Right panel: title + period select, KPI tabs (number, delta, label; active tab raised), teal area chart with dot markers, ranked table.
- Bottom: month ruler with minor ticks, blue scrubber knob and a date bubble above it, year stepper and prev/next round buttons.
- Focus view: selected node as a big blue disc with soft halo; "Depends on" on the left arc and "Used by" on the right arc, joined by long bezier curves; hovered curve turns blue.

## Type

Sora for UI (wide geometric, close to the reference). JetBrains Mono only for paths and commit SHAs. Tabular numerals in all figures.

## Motion

One authored moment: switching Network ⇄ Ontology morphs every node to its new position (transform transition, exponential ease-out). Scrubbing time fades links in/out; newly added links flash lime, removed ones ghost coral.
