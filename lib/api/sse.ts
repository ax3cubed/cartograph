// FE-05: a text/event-stream parser (the WHATWG rules the app needs). Feed it chunks split anywhere, even inside a
// line or a CRLF pair; it returns whole events. Comments (heartbeats) and fields it doesn't know are skipped.

export interface SseEvent { id: string | null; event: string; data: string }

export class SseParser {
  private buf = "";
  private cr = false; // the last chunk ended in \r: a \n opening the next one belongs to it
  private data: string[] = [];
  private event = "";
  /** The last id seen, kept across events (the Last-Event-ID to resume with). */
  lastId: string | null = null;

  push(chunk: string): SseEvent[] {
    if (!chunk) return [];
    if (this.cr && chunk.startsWith("\n")) chunk = chunk.slice(1);
    this.cr = chunk.endsWith("\r");
    this.buf += chunk;
    const out: SseEvent[] = [];
    const lines = this.buf.split(/\r\n|\r|\n/);
    this.buf = lines.pop()!; // the unfinished tail
    for (const line of lines) {
      if (line === "") {
        if (this.data.length) out.push({ id: this.lastId, event: this.event || "message", data: this.data.join("\n") });
        this.data = []; this.event = "";
        continue;
      }
      if (line.startsWith(":")) continue;
      const i = line.indexOf(":");
      const field = i < 0 ? line : line.slice(0, i);
      let value = i < 0 ? "" : line.slice(i + 1);
      if (value.startsWith(" ")) value = value.slice(1);
      if (field === "data") this.data.push(value);
      else if (field === "event") this.event = value;
      else if (field === "id" && !value.includes("\0")) this.lastId = value;
    }
    return out;
  }
}
