import * as zlib from "node:zlib";
import { Buffer } from "node:buffer";
import { asyncOutcome, outcome } from "./outcome.ts";

export async function zlibContracts() {
  const result: Record<string, unknown> = {};
  result.autoDetect = (["gzipSync", "deflateSync"] as const).map(encode => outcome(() => zlib.unzipSync(zlib[encode]("content")).toString()));
  result.dictionary = outcome(() => {
    const dictionary = Buffer.from("a shared phrase that should be reused");
    const data = zlib.deflateSync(dictionary, { dictionary });
    return { missing: outcome(() => zlib.inflateSync(data).toString()), restored: zlib.inflateSync(data, { dictionary }).toString() };
  });
  result.options = [
    { level: 10 }, { level: -2 }, { windowBits: 16 }, { memLevel: 0 },
    { strategy: 5 }, { chunkSize: 0 }, { flush: 9 }, { maxOutputLength: 0 },
  ].map(options => outcome(() => {
    const stream = zlib.createDeflate(options);
    stream.destroy();
    return true;
  }));
  result.parameterOptions = [
    // @ts-expect-error Brotli has no level option.
    () => new zlib.BrotliCompress({ level: 99 }),
    () => new zlib.BrotliCompress({ params: { 4: 2 } }),
    () => new zlib.BrotliDecompress({ params: { 2: 1 } }), () => new zlib.ZstdCompress({ params: { 0: 1 } }),
    () => new zlib.ZstdCompress({ params: { 201: true } }),
    // @ts-expect-error A dictionary is bytes.
    () => new zlib.Gzip({ dictionary: "invalid" }),
    // @ts-expect-error "01" is not a parameter number.
    () => new zlib.BrotliCompress({ params: { 1: 1, "01": 2 } }),
    () => new zlib.Deflate({ windowBits: 8 }),
  ].map(create => outcome(() => { const stream = create(); stream.destroy(); return true; }));
  const distances: zlib.BrotliOptions["params"][] = [{ 7: 99 }, { 7: -1 }, { 7: 2, 8: 12 }];
  result.brotliDistanceOptions = distances.map(params => outcome(() => {
    const data = zlib.brotliCompressSync("distance data ".repeat(100), { params });
    return zlib.brotliDecompressSync(data).toString() === "distance data ".repeat(100);
  }));
  result.outputLimit = outcome(() => zlib.gunzipSync(zlib.gzipSync("x".repeat(1000)), { maxOutputLength: 10 }).toString());
  result.levelZero = outcome(() => {
    const data = zlib.deflateSync("x".repeat(1000), { level: 0 });
    return { uncompressed: data.length > 1000, length: zlib.inflateSync(data).length };
  });
  result.info = outcome(() => {
    const { buffer, engine } = zlib.gzipSync("abc", { info: true });
    return { text: zlib.gunzipSync(buffer).toString(), bytesWritten: engine.bytesWritten, kind: engine.constructor.name };
  });
  result.trailing = outcome(() => zlib.inflateSync(Buffer.concat([zlib.deflateSync("abc"), Buffer.from("trailing")])).toString());
  result.members = outcome(() => zlib.gunzipSync(Buffer.concat([zlib.gzipSync("first"), zlib.gzipSync("second")])).toString());
  result.truncated = outcome(() => zlib.gunzipSync(zlib.gzipSync("abc").subarray(0, 15)).toString());
  result.partialFinish = outcome(() => zlib.gunzipSync(zlib.gzipSync("abc").subarray(0, 15), { finishFlush: zlib.constants.Z_SYNC_FLUSH }).toString());
  const streaming: Record<string, unknown> = {};
  result.streaming = streaming;
  for (const [name, create, decode, flush] of [
    ["gzip", "createGzip", "gunzipSync", zlib.constants.Z_SYNC_FLUSH],
    ["deflate", "createDeflate", "inflateSync", zlib.constants.Z_SYNC_FLUSH],
    ["raw", "createDeflateRaw", "inflateRawSync", zlib.constants.Z_SYNC_FLUSH],
    ["brotli", "createBrotliCompress", "brotliDecompressSync", zlib.constants.BROTLI_OPERATION_FLUSH],
    ["zstd", "createZstdCompress", "zstdDecompressSync", zlib.constants.ZSTD_e_flush],
  ] as const) streaming[name] = await asyncOutcome(async () => {
    const stream = zlib[create]();
    const chunks: Buffer[] = [];
    const completed = new Promise((resolve, reject) => { stream.on("data", chunk => chunks.push(Buffer.from(chunk))); stream.on("error", reject); stream.on("end", resolve); });
    try {
      await new Promise<void>((resolve, reject) => stream.write("first", error => error ? reject(error) : resolve()));
      // A flush passes its callback to a write, which reports any error.
      await new Promise<void>((resolve, reject) => stream.flush(flush, (error?: Error | null) => error ? reject(error) : resolve()));
      const beforeEnd = chunks.some(chunk => chunk.length > 0);
      stream.end("second");
      await completed;
      return { beforeEnd, text: zlib[decode](Buffer.concat(chunks)).toString(), bytesWritten: stream.bytesWritten };
    } finally { stream.destroy(); }
  });
  result.constants = (["DEFLATE", "INFLATE", "GZIP", "GUNZIP", "DEFLATERAW", "INFLATERAW", "UNZIP"] as const).map(name => [name, zlib.constants[name]]);
  result.closeAfterClosed = await Promise.all([false, true].map(completed => asyncOutcome(async () => {
    const stream = zlib.createGzip();
    if (completed) {
      stream.resume();
      const closed = new Promise(resolve => stream.once("close", resolve));
      stream.end("done");
      await closed;
    } else await new Promise<void>(resolve => stream.close(resolve));
    let synchronous = true;
    const callback = new Promise(resolve => stream.close(() => resolve({ synchronous })));
    synchronous = false;
    return Promise.race([callback, new Promise(resolve => setTimeout(() => resolve("missing"), 100))]);
  })));
  const chunkedDecoding: Record<string, unknown> = {};
  result.chunkedDecoding = chunkedDecoding;
  for (const [encode, create] of [["gzipSync", "createGunzip"], ["deflateSync", "createInflate"], ["deflateRawSync", "createInflateRaw"], ["brotliCompressSync", "createBrotliDecompress"], ["zstdCompressSync", "createZstdDecompress"]] as const) {
    chunkedDecoding[create] = await asyncOutcome(async () => {
      const plain = "incremental data ".repeat(8192);
      const encoded = zlib[encode](plain);
      // Workerd recursively drains Brotli output; 64-byte buffers overflow its stack here.
      const stream = zlib[create]({ chunkSize: 1024 });
      const chunks: Buffer[] = [];
      const finished = new Promise((resolve, reject) => { stream.on("data", chunk => chunks.push(chunk)); stream.once("end", resolve); stream.once("error", reject); });
      finished.catch(() => {});
      try {
        for (const byte of encoded) await new Promise<void>((resolve, reject) => stream.write(Buffer.from([byte]), error => error ? reject(error) : resolve()));
        stream.end();
        await finished;
        return { restored: Buffer.concat(chunks).toString() === plain, consumed: stream.bytesWritten === encoded.length };
      } finally { stream.destroy(); }
    });
  }
  result.backpressure = await asyncOutcome(async () => {
    const plain = "a".repeat(131072);
    const stream = zlib.createInflate({ chunkSize: 64, highWaterMark: 64 });
    let called = false;
    stream.end(zlib.deflateSync(plain), () => { called = true; });
    await new Promise(resolve => setTimeout(resolve, 1));
    const blocked = !called;
    const chunks: Buffer[] = [];
    for await (const chunk of stream) chunks.push(chunk);
    return { blocked, restored: Buffer.concat(chunks).toString() === plain };
  });
  result.params = await asyncOutcome(async () => {
    const stream = zlib.createDeflate({ level: 0 });
    const chunks: Buffer[] = [];
    const finished = new Promise((resolve, reject) => { stream.on("data", chunk => chunks.push(chunk)); stream.once("end", resolve); stream.once("error", reject); });
    finished.catch(() => {});
    try {
      stream.write("first");
      await new Promise<void>((resolve, reject) => stream.params(9, zlib.constants.Z_FILTERED, (error?: Error | null) => error ? reject(error) : resolve()));
      const suffix = "compressible suffix ".repeat(2048);
      stream.end(suffix);
      await finished;
      const data = Buffer.concat(chunks);
      return { restored: zlib.inflateSync(data).toString() === "first" + suffix,
        compressed: data.length < zlib.deflateSync("first" + suffix, { level: 0 }).length / 10 };
    } finally { stream.destroy(); }
  });
  const reset: Record<string, unknown> = {};
  result.reset = reset;
  for (const [create, decode, flush] of [["createDeflate", "inflateSync", 2], ["createBrotliCompress", "brotliDecompressSync", 1], ["createZstdCompress", "zstdDecompressSync", 1]] as const) {
    reset[create] = await asyncOutcome(async () => {
      const stream = zlib[create]();
      const chunks: Buffer[] = [];
      const done = new Promise((resolve, reject) => { stream.on("data", chunk => chunks.push(chunk)); stream.once("end", resolve); stream.once("error", reject); });
      done.catch(() => {});
      try {
        await new Promise<void>((resolve, reject) => stream.write("discarded", error => error ? reject(error) : resolve()));
        await new Promise<void>((resolve, reject) => stream.flush(flush, (error?: Error | null) => error ? reject(error) : resolve()));
        stream.reset();
        chunks.length = 0;
        stream.end("reused");
        await done;
        return { text: zlib[decode](Buffer.concat(chunks)).toString(), bytesWritten: stream.bytesWritten };
      } finally { stream.destroy(); }
    });
  }
  const decoderFailures: Record<string, unknown> = {};
  result.decoderFailures = decoderFailures;
  for (const [encode, create] of [["brotliCompressSync", "createBrotliDecompress"], ["zstdCompressSync", "createZstdDecompress"]] as const) {
    decoderFailures[create] = await Promise.all(["malformed", "truncated"].map(kind => asyncOutcome(async () => {
      const encoded = zlib[encode]("failure data ".repeat(512));
      const stream = zlib[create]();
      let ended = false, error: { name: string; code: string | null } | undefined;
      const closed = new Promise(resolve => stream.once("close", resolve));
      stream.on("error", (value: NodeJS.ErrnoException) => { error = { name: value.name, code: value.code ?? null }; });
      stream.on("end", () => { ended = true; });
      stream.resume();
      stream.end(kind === "malformed" ? Buffer.from("invalid compressed bytes") : encoded.subarray(0, Math.floor(encoded.length / 2)));
      await closed;
      return { error, ended, closed: stream.closed };
    })));
  }
  result.callbackForms = await Promise.all([
    decodedText<Buffer>(callback => zlib.gzip("callback", {}, callback), value => zlib.gunzipSync(value)),
    decodedText<{ buffer: Buffer }>(callback => zlib.brotliCompress("callback", { info: true }, callback), value => zlib.brotliDecompressSync(value.buffer)),
    decodedText<Buffer>(callback => zlib.zstdCompress("callback", {}, callback), value => zlib.zstdDecompressSync(value)),
  ]);
  // @ts-expect-error A string is not a starting value.
  result.crcValidation = [undefined, -1, 2 ** 32, 1.5, "0"].map(value => outcome(() => zlib.crc32("abc", value)));
  return result;
}

// The text that `decode` makes of what the callback form `encode` produces.
function decodedText<T>(encode: (callback: (error: Error | null, value: T) => void) => void, decode: (value: T) => Buffer) {
  return asyncOutcome(() => new Promise((resolve, reject) => {
    encode((error, value) => {
      if (error) { reject(error); return; }
      try { resolve(decode(value).toString()); }
      catch (error) { reject(error); }
    });
  }));
}
