# FE-12 audit: the live map against Chartability

FE-12 adds a table view and a keyboard walk to the live map (`components/map/LiveMap.tsx`, `components/map/TableView.tsx`). This records an audit against [Chartability](https://chartability.fizz.studio/)'s principles, made on 23 Nov 2026 with the northwind fixture served by the real binary.

How it was checked: axe (WCAG 2.2 A and AA) in `e2e/routes.spec.ts`, and the keyboard journeys in `e2e/journeys.spec.ts`, in Chromium, Firefox and WebKit. Screen reader output was not heard: the VoiceOver and NVDA passes in FE-12's "done when" are still open (see the last section).

| Principle | Check | Result |
| --- | --- | --- |
| Perceivable | Text alternative for the whole map | Pass. The map's label says how many files and links it shows; the table is the full alternative. |
| Perceivable | Colour is not the only signal | Pass. A folder's hue is repeated as its name in the table's Folder column. |
| Perceivable | Contrast of table text and focus rings | Pass by tokens (`--body`, `--text`, `--accent`), axe finds nothing. |
| Operable | Every element reachable without a pointer | Pass. Every file and link drawn is a table row (`table.check.ts` holds the two to the same function); the walk reaches every drawn file. |
| Operable | Keyboard walk | Pass. Arrows between siblings, Enter down to a folder's files, Escape back up, L through linked files. |
| Operable | No keyboard trap | Pass. The map and the table box are single tab stops; Escape in the details panel returns focus to the row it came from. |
| Understandable | Each move is announced | Pass in the DOM. A polite live region says "folder X, N files" or "file X, N incoming, M outgoing". |
| Understandable | Instructions | Pass. The map describes its keys (`aria-describedby`); the table shows a hint line. |
| Robust | Real table semantics | Pass. `<table>`, `<th scope="col">`, `aria-sort`, `aria-rowcount` and `aria-rowindex` on the windowed rows. |
| Compromising | Table as an alternative to the chart | Pass. Same slice, filter and date; for slices too big to draw, the table still lists every row. |
| Assistive | Data-to-text summary | Partial. Counts are announced; there is no written summary of structure yet (FE-08 brings services and modules). |
| Flexible | Respects user settings | Pass. Text size, contrast and reduced motion come from the app's appearance settings. |

## Open

- Hear the walk and the table with VoiceOver (Safari) and NVDA (Firefox), and record what each says on a move.
- The walk has two levels (folder, file) because the live slice is file-level. Services and modules join when FE-08 lands; the spec's data-navigator can be weighed then.
- Sorting and filtering run in the browser. Past 5,000 rows they should run on the server, which needs a route that isn't built yet.
