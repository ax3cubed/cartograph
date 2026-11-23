// FE-09: bin sizes by range and the knob's snapping. Run with `pnpm check`.
import assert from "node:assert/strict";
import { DAY, binFor, nearest } from "./bins.ts";

assert.equal(binFor(31), "day");
assert.equal(binFor(92), "day");
assert.equal(binFor(93), "week");
assert.equal(binFor(365), "week");
assert.equal(binFor(731), "week");
assert.equal(binFor(732), "month");
assert.equal(binFor(3650), "month");

const days = Int32Array.from([10, 10, 12, 20, 40]);
const at = (d: number) => d * DAY + DAY / 2;
assert.equal(nearest(new Int32Array(), at(5)), -1, "no commits");
assert.equal(nearest(days, at(0)), 1, "before the first: the first day, its last commit");
assert.equal(nearest(days, at(10)), 1, "the last commit of a day");
assert.equal(nearest(days, at(11)), 2, "a tie (11 is one day from 10 and from 12) goes to the later commit");
assert.equal(nearest(days, at(13)), 2);
assert.equal(nearest(days, at(17)), 3);
assert.equal(nearest(days, at(29)), 3);
assert.equal(nearest(days, at(31)), 4);
assert.equal(nearest(days, at(99)), 4, "after the last: the last");
// property: the pick is never farther than any other commit
let seed = 3;
const rnd = (n: number) => { seed = (seed * 1103515245 + 12345) & 0x7fffffff; return seed % n; };
for (let t = 0; t < 500; t++) {
  const d = Int32Array.from(Array.from({ length: 1 + rnd(40) }, () => rnd(400)).sort((a, b) => a - b));
  const x = rnd(420);
  const i = nearest(d, at(x));
  for (const y of d) assert.ok(Math.abs(d[i] - x) <= Math.abs(y - x), "nearest");
}
console.log("timeline bins ok");
