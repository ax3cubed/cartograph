// FE-05 tests for the event-stream parser: run with `pnpm check`. Any chunking of a byte stream yields the same events.
import assert from "node:assert/strict";
import { SseParser, type SseEvent } from "./sse.ts";

const stream =
  ": hello\r\n\r\nid: 1\r\nevent: freshness\r\ndata: {\"ws\":\"a\"}\r\n\r\n" +
  "id: 2\nevent: job.progress\ndata: {\"done\":1,\ndata: \"total\":2}\n\n" +
  ": heartbeat\n\n" +
  "id: 3\revent: index.changed\rdata:{\"generation\":7}\r\r" +
  "data: no event name\n\n" +
  "event: dropped\n"; // unfinished: never emitted

const run = (chunks: string[]) => { const p = new SseParser(); const out: SseEvent[] = []; for (const c of chunks) out.push(...p.push(c)); return { out, last: p.lastId }; };

const whole = run([stream]);
assert.deepEqual(whole.out, [
  { id: "1", event: "freshness", data: "{\"ws\":\"a\"}" },
  { id: "2", event: "job.progress", data: "{\"done\":1,\n\"total\":2}" },
  { id: "3", event: "index.changed", data: "{\"generation\":7}" },
  { id: "3", event: "message", data: "no event name" },
]);
assert.equal(whole.last, "3");

// every split into two, and a seeded run of random splits into many
for (let i = 0; i <= stream.length; i++) assert.deepEqual(run([stream.slice(0, i), stream.slice(i)]).out, whole.out, `split at ${i}`);
let seed = 42;
const rnd = () => ((seed = (seed * 1103515245 + 12345) % 2 ** 31) / 2 ** 31);
for (let n = 0; n < 500; n++) {
  const chunks: string[] = [];
  for (let i = 0; i < stream.length;) { const k = 1 + Math.floor(rnd() * 8); chunks.push(stream.slice(i, i + k)); i += k; }
  assert.deepEqual(run(chunks).out, whole.out, `chunking ${n}`);
}
console.log("sse parser: ok");
