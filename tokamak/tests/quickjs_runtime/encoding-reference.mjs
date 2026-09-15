import { readFileSync } from "node:fs";
import { get } from "node:http";
import { gunzipSync, brotliDecompressSync } from "node:zlib";
import { Miniflare, convertV4MiniflareOptions } from "miniflare";
import { cases } from "./encoding.mjs";

const path = `${import.meta.dirname}/encoding.mjs`;
const mf = new Miniflare(convertV4MiniflareOptions({
  cf: false, modulesRoot: import.meta.dirname, compatibilityDate: "2026-08-25", compatibilityFlags: ["nodejs_compat"],
  modules: [{ type: "ESModule", path, contents: readFileSync(path, "utf8") }],
}));
try {
  const base = await mf.ready;
  const results = [];
  for (let index = 0; index < cases.length; index++) {
    results.push(await new Promise((resolve, reject) => {
      get(new URL(`/?case=${index}`, base), response => {
        const chunks = [];
        response.on("data", chunk => chunks.push(chunk));
        response.on("error", reject);
        response.on("end", () => {
          const raw = Buffer.concat(chunks);
          const encoding = response.headers["content-encoding"];
          let body = raw, encoded = false;
          try {
            const decode = { gzip: gunzipSync, br: brotliDecompressSync }[encoding];
            if (decode) { body = decode(raw); encoded = true; }
          } catch { /* Manual bodies need not be encoded. */ }
          resolve({ encoding, encoded, body: body.toString() });
        });
      }).on("error", reject);
    }));
  }
  console.log(JSON.stringify(results));
} finally { await mf.dispose(); }
