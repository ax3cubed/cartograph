// The local API's wire types (INT-01). Everything but the envelope is generated from
// crates/cartograph-api into types.gen.ts; the OpenAPI file is crates/cartograph-api/openapi.json.
export type * from "./types.gen.ts";

/** Every JSON read is wrapped in this. JSON Schema has no generics, so it is written here once. */
export interface Envelope<T> {
  schema: 1;
  generation: number;
  data: T;
  next_cursor: string | null;
  truncated: boolean;
}
