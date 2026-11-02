import { get } from "node:http";
import { member, present } from "./outcome.ts";

async function caught(callback: () => unknown) {
  try { await callback(); return "fulfilled"; }
  catch (error) { return error === null ? null : typeof error === "object" ? [member(error, "name"), member(error, "message")] : [typeof error, error]; }
}

// An upload of `limit` 1 KiB chunks and its state. `settled()` waits up to a second for the upload to
// end or be cancelled, which runtimes do at no fixed time after its fetch settles or its response ends.
function trackedUpload(limit = Infinity) {
  const state: { ended: boolean; cancelled: number; reason: unknown } = { ended: false, cancelled: 0, reason: null };
  const settled = Promise.withResolvers<void>();
  let count = 0;
  const body = new ReadableStream({
    async pull(controller) {
      await new Promise(resolve => setTimeout(resolve, 1));
      if (state.cancelled) return;
      if (count++ === limit) { state.ended = true; controller.close(); settled.resolve(); }
      else controller.enqueue(new Uint8Array(1024));
    },
    cancel(reason) {
      state.cancelled++;
      state.reason = reason instanceof Error ? [reason.name, reason.message, member(reason, "retryable")] : String(reason);
      settled.resolve();
    },
  });
  return { body, state, settled: () => Promise.race([settled.promise, new Promise(resolve => setTimeout(resolve, 1000))]) };
}

export default {
  async fetch(request, env) {
    const base = `http://127.0.0.1:${env.FLAG}`;
    const results: Record<string, unknown> = {};
    const headerResponse = await fetch(`${base}/headers`, { headers: { "x-latin": "caf\u00e9", "x-unicode": "\u6771\u4eac" } });
    const headerRequest = await headerResponse.json<Record<string, string>>();
    results.headers = {
      statusText: headerResponse.statusText,
      latin: headerResponse.headers.get("x-latin"),
      utf8: headerResponse.headers.get("x-utf8"),
      repeat: headerResponse.headers.get("x-repeat"),
      cookies: headerResponse.headers.getSetCookie(),
      // tokamak deliberately sends no cf-worker header.
      sent: Object.entries(headerRequest).filter(([name]) => name !== "cf-worker").sort(),
    };
    results.nodeHeaders = await new Promise((resolve, reject) => {
      get(`${base}/headers`, response => {
        response.resume();
        response.on("error", reject);
        response.on("end", () => resolve({
          status: response.statusCode,
          statusText: response.statusMessage,
          cookies: response.headers["set-cookie"],
          repeated: response.headers["x-repeat"],
          rawCookies: response.rawHeaders.flatMap((name, index, pairs) => index % 2 === 0 && name.toLowerCase() === "set-cookie" ? [pairs[index + 1]] : []),
        }));
      }).on("error", reject);
    });
    const contentDecoding: unknown[] = [];
    results.contentDecoding = contentDecoding;
    for (const encoding of ["gzip", "br", "deflate", "zstd", "x-gzip", "gzip, br", "GZIP", "unknown"]) {
      try {
        const response = await fetch(`${base}/encoding?kind=${encodeURIComponent(encoding)}`);
        contentDecoding.push({ encoding, bytes: [...new Uint8Array(await response.arrayBuffer())] });
      } catch (error) { contentDecoding.push({ encoding, error: member(error, "name") }); }
    }
    let timerRan = false;
    const timer = setTimeout(() => { timerRan = true; }, 1);
    await (await fetch(`${base}/delay`)).text();
    clearTimeout(timer);
    results.timer = timerRan;

    const abort = new AbortController();
    const cancel = setTimeout(() => abort.abort("cancelled"), 1);
    results.abortBeforeHeaders = await caught(() => fetch(`${base}/delay`, { signal: abort.signal }));
    clearTimeout(cancel);
    const forwardedAbort = new AbortController();
    const pendingRequest = new Request(`${base}/delay`, { signal: forwardedAbort.signal });
    const forwardedTimer = setTimeout(() => forwardedAbort.abort("forwarded"), 1);
    results.abortForwarded = await caught(() => fetch(pendingRequest));
    clearTimeout(forwardedTimer);

    const abortReasons: unknown[] = [];
    results.abortReasons = abortReasons;
    for (const reason of [undefined, null, "reason", new Error("reason"), new TypeError("reason"), new DOMException("reason", "AbortError")]) {
      const early = new AbortController();
      early.abort(reason);
      const earlyResult = await caught(() => fetch(`${base}/delay`, { signal: early.signal }));
      const late = new AbortController();
      const timer = setTimeout(() => late.abort(reason), 1);
      const lateResult = await caught(() => fetch(`${base}/delay`, { signal: late.signal }));
      clearTimeout(timer);
      abortReasons.push([earlyResult, lateResult]);
    }

    const response = await fetch(`${base}/stream`);
    const reader = present(response.body).getReader();
    const first = await reader.read();
    const streaming: Record<string, unknown> = { first: new TextDecoder().decode(first.value), ended: await (await fetch(`${base}/state`)).json() };
    results.streaming = streaming;
    await (await fetch(`${base}/release`)).text();
    const rest: string[] = [];
    for (;;) {
      const chunk = await reader.read();
      if (chunk.done) break;
      rest.push(new TextDecoder().decode(chunk.value));
    }
    streaming.rest = rest.join("");

    const controller = new AbortController();
    const interrupted = await fetch(`${base}/stream`, { signal: controller.signal });
    const streamReader = present(interrupted.body).getReader();
    await streamReader.read();
    controller.abort("body cancelled");
    results.abortBody = await caught(() => streamReader.read());
    await (await fetch(`${base}/release`)).text();

    let next = 0;
    const upload = new ReadableStream({ async pull(controller) {
      await new Promise(resolve => setTimeout(resolve, 1));
      if (next === 3) controller.close();
      else controller.enqueue(new TextEncoder().encode(String(next++)));
    } });
    results.upload = await (await fetch(`${base}/echo`, { method: "POST", body: upload })).json();

    let cancelledSource = 0;
    const original = new Request(`${base}/echo`, { method: "POST", body: new ReadableStream({ cancel() { cancelledSource++; } }) });
    const transferred = new Request(original);
    const transfer: Record<string, unknown> = { originalUsed: original.bodyUsed, originalLocked: present(original.body).locked };
    results.transfer = transfer;
    transfer.settled = await Promise.race([
      present(transferred.body).cancel().then(() => true),
      new Promise(resolve => setTimeout(() => resolve(false), 5)),
    ]);
    transfer.cancelled = cancelledSource;

    {
      const abandoned = trackedUpload(200);
      await (await fetch(new Request(`${base}/redirect-early`, { method: "POST", body: abandoned.body }))).text();
      await abandoned.settled();
      results.abandonedUpload = abandoned.state;
    }

    const earlyUploads: unknown[] = [];
    results.earlyUploads = earlyUploads;
    for (const endpoint of ["early", "early-204", "disconnect"]) {
      const early = trackedUpload(endpoint === "disconnect" ? Infinity : 100);
      try {
        const response = await fetch(`${base}/${endpoint}`, { method: "POST", body: early.body });
        const headersBeforeEnd = !early.state.ended;
        const text = await response.text();
        await early.settled();
        earlyUploads.push({ headersBeforeEnd, text, ...early.state });
      } catch {
        await early.settled();
        earlyUploads.push({ failed: true, ...early.state });
      }
    }

    {
      const closed = trackedUpload();
      // The server closes the connection after its response, before the body ends, so the fetch
      // can resolve or reject; either way both runtimes cancel the body.
      await fetch(`${base}/close`, { method: "POST", body: closed.body }).then(response => response.text()).catch(() => {});
      await closed.settled();
      results.closedUpload = closed.state;
    }

    {
      const open = trackedUpload(10);
      const reader = present((await fetch(`${base}/stream`, { method: "POST", body: open.body })).body).getReader();
      await reader.read();
      await open.settled();
      results.openResponseUpload = { ...open.state };
      await reader.cancel();
      await (await fetch(`${base}/release`)).text();
      const cancelled = trackedUpload();
      await present((await fetch(`${base}/stream`, { method: "POST", body: cancelled.body })).body).cancel();
      await cancelled.settled();
      results.cancelledResponseUpload = cancelled.state;
      await (await fetch(`${base}/release`)).text();
    }

    {
      const aborted = trackedUpload();
      const controller = new AbortController();
      const timer = setTimeout(() => controller.abort("upload aborted"), 5);
      const outcome = await caught(() => fetch(`${base}/delay`, { method: "POST", body: aborted.body, signal: controller.signal }));
      clearTimeout(timer);
      await aborted.settled();
      results.abortedUpload = { outcome, ...aborted.state };
    }

    const streamRedirects: unknown[] = [];
    results.streamRedirects = streamRedirects;
    for (const status of [302, 307]) {
      const redirected = trackedUpload();
      const outcome = await caught(() => fetch(`${base}/redirect?status=${status}`, { method: "POST", body: redirected.body }));
      await redirected.settled();
      streamRedirects.push([status, outcome, redirected.state]);
    }

    const redirects: unknown[] = [];
    results.redirects = redirects;
    for (const status of [301, 302, 303, 307, 308]) {
      for (const method of ["POST", "PUT", "HEAD"]) {
        const response = await fetch(`${base}/redirect?status=${status}`, { method, body: method === "HEAD" ? undefined : "body" });
        redirects.push([status, method, response.redirected, method === "HEAD" ? response.body === null : await response.json()]);
      }
    }
    const manual = await fetch(`${base}/redirect?status=302`, { redirect: "manual" });
    results.manual = [manual.status, manual.redirected, manual.headers.get("location")];
    const reusable = new Request(`${base}/redirect?status=307`, { method: "POST", body: "reusable" });
    results.replayedForwarded = await (await fetch(reusable)).json();
    results.rejectRedirect = await caught(() => fetch(`${base}/redirect?status=302`, { redirect: "error" }));
    results.rejectRedirectRequest = await caught(() => new Request(`${base}/redirect?status=302`, { redirect: "error" }));
    results.redirectLimit = await caught(() => fetch(`${base}/loop`));
    const redirectedHeaders = await (await fetch(`${base}/redirect?status=303&to=/headers`, {
      method: "POST",
      body: "body",
      headers: { "content-type": "text/plain", "content-language": "en", "content-location": "/body", "content-encoding": "identity" },
    })).json<Record<string, string>>();
    // tokamak deliberately sends no cf-worker header.
    results.redirectedHeaders = Object.entries(redirectedHeaders).filter(([name]) => name !== "cf-worker").sort();
    const compressed = await fetch(`${base}/gzip`);
    results.compressed = { encoding: compressed.headers.get("content-encoding"), cookies: compressed.headers.getSetCookie(), text: await compressed.text() };
    return Response.json(results);
  },
} satisfies ExportedHandler<{ FLAG: string }>;
