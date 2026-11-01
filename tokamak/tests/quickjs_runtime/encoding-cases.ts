// The responses encoding.ts makes, which encoding-reference.mts requests by index.
export const cases: { encoding: string; mode: "automatic" | "manual"; body: string; copy?: "clone" | "init" }[] = [
  ...["gzip", "br", "deflate", "zstd", "identity", "unknown", "gzip, br", "GZIP"].flatMap(encoding =>
    (["automatic", "manual"] as const).map(mode => ({ encoding, mode, body: "text" }))),
  ...["gzip", "br"].flatMap(encoding => ["stream", "empty", "null"].map(body => ({ encoding, mode: "automatic" as const, body }))),
  { encoding: "gzip", mode: "manual", body: "text", copy: "clone" },
  { encoding: "gzip", mode: "manual", body: "text", copy: "init" },
];
