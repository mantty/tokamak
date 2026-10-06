import { readFileSync } from "node:fs";
import { mkdtemp, rm } from "node:fs/promises";
import { request as httpRequest } from "node:http";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { Miniflare, convertV4MiniflareOptions } from "miniflare";

const root = join(import.meta.dirname, "assets");
const configurations = [
  ["auto-trailing-slash", "none"],
  ["force-trailing-slash", "none"],
  ["drop-trailing-slash", "none"],
  ["none", "none"],
  ["auto-trailing-slash", "single-page-application"],
  ["auto-trailing-slash", "404-page"],
];
const paths = ["/", "/index.html", "/about", "/about/", "/about.html", "/docs", "/docs/", "/docs/index",
  "/blog/post", "/blog/post/", "/blog/post.html", "/app.css", "/missing"];
// Each request's method, path and whether it is a navigation.
const requests = [
  ...paths.map((path) => ["GET", path, false]),
  ["GET", "/missing", true], ["GET", "/docs/missing/page", true],
  ["HEAD", "/app.css", false], ["POST", "/about", false], ["POST", "/missing", false],
];

/** The status, location and body of the response to a request to `base`. */
function send(base, method, path, navigation) {
  const headers = navigation ? { "Sec-Fetch-Mode": "navigate" } : {};
  return new Promise((resolve, reject) => {
    const request = httpRequest(new URL(path, base), { method, headers }, (response) => {
      const chunks = [];
      response.on("data", (chunk) => chunks.push(chunk));
      response.on("end", () => resolve({
        status: response.statusCode,
        location: response.headers.location,
        body: Buffer.concat(chunks).toString(),
      }));
    });
    request.on("error", reject);
    request.end();
  });
}

const state = await mkdtemp(join(tmpdir(), "tokamak-assets-"));
process.chdir(state);
const results = [];
try {
  for (const [htmlHandling, notFoundHandling] of configurations) {
    const mf = new Miniflare(convertV4MiniflareOptions({
      modulesRoot: root,
      cf: false,
      compatibilityDate: "2026-08-25",
      modules: [{ type: "ESModule", path: join(root, "worker.mjs"), contents: readFileSync(join(root, "worker.mjs")) }],
      assets: {
        directory: join(root, "public"),
        binding: "STATIC",
        routerConfig: { has_user_worker: true },
        assetConfig: { html_handling: htmlHandling, not_found_handling: notFoundHandling },
      },
    }));
    try {
      const base = await mf.ready;
      const router = [];
      for (const [method, path, navigation] of requests) {
        let response = await send(base, method, path, navigation);
        // tokamak serves the asset Cloudflare redirects to.
        if (response.status === 307) response = await send(base, method, response.location, navigation);
        router.push({ status: response.status, body: response.body });
      }
      const binding = await (await mf.dispatchFetch(new URL("/binding", base), {
        method: "POST",
        body: JSON.stringify(requests),
      })).json();
      results.push({ htmlHandling, notFoundHandling, requests, router, binding });
    } finally {
      await mf.dispose();
    }
  }
  console.log(JSON.stringify(results));
} finally {
  await rm(state, { recursive: true, force: true });
}
