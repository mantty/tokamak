import * as zlib from "node:zlib";
import { Buffer } from "node:buffer";

function outcome(callback) {
  try { return callback(); }
  catch (error) { return { error: error.name, code: error.code ?? null }; }
}
async function asyncOutcome(callback) {
  try { return await callback(); }
  catch (error) { return { error: error.name, code: error.code ?? null }; }
}

export async function zlibContracts() {
  const result = {};
  result.autoDetect = ["gzipSync", "deflateSync"].map(encode => outcome(() => zlib.unzipSync(zlib[encode]("content")).toString()));
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
    ["BrotliCompress", { level: 99 }], ["BrotliCompress", { params: { 4: 2 } }],
    ["BrotliDecompress", { params: { 2: 1 } }], ["ZstdCompress", { params: { 0: 1 } }],
    ["ZstdCompress", { params: { 201: true } }], ["Gzip", { dictionary: "invalid" }],
    ["BrotliCompress", { params: { 1: 1, "01": 2 } }], ["Deflate", { windowBits: 8 }],
  ].map(([name, options]) => outcome(() => { const stream = new zlib[name](options); stream.destroy(); return true; }));
  result.brotliDistanceOptions = [{ 7: 99 }, { 7: -1 }, { 7: 2, 8: 12 }].map(params => outcome(() => {
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
  result.streaming = {};
  for (const [name, create, decode, flush] of [
    ["gzip", "createGzip", "gunzipSync", zlib.constants.Z_SYNC_FLUSH],
    ["deflate", "createDeflate", "inflateSync", zlib.constants.Z_SYNC_FLUSH],
    ["raw", "createDeflateRaw", "inflateRawSync", zlib.constants.Z_SYNC_FLUSH],
    ["brotli", "createBrotliCompress", "brotliDecompressSync", zlib.constants.BROTLI_OPERATION_FLUSH],
    ["zstd", "createZstdCompress", "zstdDecompressSync", zlib.constants.ZSTD_e_flush],
  ]) result.streaming[name] = await asyncOutcome(async () => {
    const stream = zlib[create]();
    const chunks = [];
    const completed = new Promise((resolve, reject) => { stream.on("data", chunk => chunks.push(Buffer.from(chunk))); stream.on("error", reject); stream.on("end", resolve); });
    try {
      await new Promise((resolve, reject) => stream.write("first", error => error ? reject(error) : resolve()));
      await new Promise((resolve, reject) => stream.flush(flush, error => error ? reject(error) : resolve()));
      const beforeEnd = chunks.some(chunk => chunk.length > 0);
      stream.end("second");
      await completed;
      return { beforeEnd, text: zlib[decode](Buffer.concat(chunks)).toString(), bytesWritten: stream.bytesWritten };
    } finally { stream.destroy(); }
  });
  result.constants = ["DEFLATE", "INFLATE", "GZIP", "GUNZIP", "DEFLATERAW", "INFLATERAW", "UNZIP"].map(name => [name, zlib.constants[name]]);
  result.closeAfterClosed = await Promise.all([false, true].map(completed => asyncOutcome(async () => {
    const stream = zlib.createGzip();
    if (completed) {
      stream.resume();
      const closed = new Promise(resolve => stream.once("close", resolve));
      stream.end("done");
      await closed;
    } else await new Promise(resolve => stream.close(resolve));
    let synchronous = true;
    const callback = new Promise(resolve => stream.close(() => resolve({ synchronous })));
    synchronous = false;
    return Promise.race([callback, new Promise(resolve => setTimeout(() => resolve("missing"), 100))]);
  })));
  result.chunkedDecoding = {};
  for (const [encode, create] of [["gzipSync", "createGunzip"], ["deflateSync", "createInflate"], ["deflateRawSync", "createInflateRaw"], ["brotliCompressSync", "createBrotliDecompress"], ["zstdCompressSync", "createZstdDecompress"]]) {
    result.chunkedDecoding[create] = await asyncOutcome(async () => {
      const plain = "incremental data ".repeat(8192);
      const encoded = zlib[encode](plain);
      // Workerd recursively drains Brotli output; 64-byte buffers overflow its stack here.
      const stream = zlib[create]({ chunkSize: 1024 });
      const chunks = [];
      const finished = new Promise((resolve, reject) => { stream.on("data", chunk => chunks.push(chunk)); stream.once("end", resolve); stream.once("error", reject); });
      finished.catch(() => {});
      try {
        for (const byte of encoded) await new Promise((resolve, reject) => stream.write(Buffer.from([byte]), error => error ? reject(error) : resolve()));
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
    const chunks = [];
    for await (const chunk of stream) chunks.push(chunk);
    return { blocked, restored: Buffer.concat(chunks).toString() === plain };
  });
  result.params = await asyncOutcome(async () => {
    const stream = zlib.createDeflate({ level: 0 });
    const chunks = [];
    const finished = new Promise((resolve, reject) => { stream.on("data", chunk => chunks.push(chunk)); stream.once("end", resolve); stream.once("error", reject); });
    finished.catch(() => {});
    try {
      stream.write("first");
      await new Promise((resolve, reject) => stream.params(9, zlib.constants.Z_FILTERED, error => error ? reject(error) : resolve()));
      const suffix = "compressible suffix ".repeat(2048);
      stream.end(suffix);
      await finished;
      const data = Buffer.concat(chunks);
      return { restored: zlib.inflateSync(data).toString() === "first" + suffix,
        compressed: data.length < zlib.deflateSync("first" + suffix, { level: 0 }).length / 10 };
    } finally { stream.destroy(); }
  });
  result.reset = {};
  for (const [create, decode, flush] of [["createDeflate", "inflateSync", 2], ["createBrotliCompress", "brotliDecompressSync", 1], ["createZstdCompress", "zstdDecompressSync", 1]]) {
    result.reset[create] = await asyncOutcome(async () => {
      const stream = zlib[create]();
      const chunks = [];
      const done = new Promise((resolve, reject) => { stream.on("data", chunk => chunks.push(chunk)); stream.once("end", resolve); stream.once("error", reject); });
      done.catch(() => {});
      try {
        await new Promise((resolve, reject) => stream.write("discarded", error => error ? reject(error) : resolve()));
        await new Promise((resolve, reject) => stream.flush(flush, error => error ? reject(error) : resolve()));
        stream.reset();
        chunks.length = 0;
        stream.end("reused");
        await done;
        return { text: zlib[decode](Buffer.concat(chunks)).toString(), bytesWritten: stream.bytesWritten };
      } finally { stream.destroy(); }
    });
  }
  result.decoderFailures = {};
  for (const [encode, create] of [["brotliCompressSync", "createBrotliDecompress"], ["zstdCompressSync", "createZstdDecompress"]]) {
    result.decoderFailures[create] = await Promise.all(["malformed", "truncated"].map(kind => asyncOutcome(async () => {
      const encoded = zlib[encode]("failure data ".repeat(512));
      const stream = zlib[create]();
      let ended = false, error;
      const closed = new Promise(resolve => stream.once("close", resolve));
      stream.on("error", value => { error = { name: value.name, code: value.code ?? null }; });
      stream.on("end", () => { ended = true; });
      stream.resume();
      stream.end(kind === "malformed" ? Buffer.from("invalid compressed bytes") : encoded.subarray(0, Math.floor(encoded.length / 2)));
      await closed;
      return { error, ended, closed: stream.closed };
    })));
  }
  result.callbackForms = await Promise.all([
    ["gzip", "gunzipSync", {}], ["brotliCompress", "brotliDecompressSync", { info: true }], ["zstdCompress", "zstdDecompressSync", {}],
  ].map(([encode, decode, options]) => asyncOutcome(() => new Promise((resolve, reject) => {
    zlib[encode]("callback", options, (error, value) => {
      if (error) { reject(error); return; }
      try { resolve(zlib[decode](options.info ? value.buffer : value).toString()); }
      catch (error) { reject(error); }
    });
  }))));
  result.crcValidation = [undefined, -1, 2 ** 32, 1.5, "0"].map(value => outcome(() => zlib.crc32("abc", value)));
  return result;
}
