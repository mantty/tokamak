import { Readable, Writable, Duplex } from "node:stream";

async function outcome(callback) {
  try { return await callback(); }
  catch (error) { return { error: error.name, code: error.code ?? null }; }
}

async function compress(format, bytes) {
  const stream = new CompressionStream(format);
  const writer = stream.writable.getWriter();
  await writer.write(bytes);
  await writer.close();
  return new Response(stream.readable).bytes();
}

export async function streamContracts() {
  const result = {};
  result.encoding = await outcome(async () => {
    const stream = new TextEncoderStream();
    const output = new Response(stream.readable).text();
    const writer = stream.writable.getWriter();
    for (const chunk of ["a\ud83d", "\ude00b", "\ud800", "c", "\ud800"]) await writer.write(chunk);
    await writer.close();
    return output;
  });
  result.identity = {};
  for (const [name, make] of [["identity", () => new IdentityTransformStream()], ["fixed", () => new FixedLengthStream(3)]]) {
    result.identity[name] = await outcome(async () => {
      const stream = make();
      const reader = stream.readable.getReader({ mode: "byob" });
      const writer = stream.writable.getWriter();
      const input = new Uint8Array([1, 2, 3]);
      const read = reader.read(new Uint8Array(8));
      await writer.write(input);
      const { value } = await read;
      await writer.close();
      const end = await reader.read(new Uint8Array(8));
      return { bytes: [...value], inputLength: input.byteLength, done: end.done };
    });
  }
  result.identity.partialRead = await outcome(async () => {
    const stream = new IdentityTransformStream();
    const reader = stream.readable.getReader({ mode: "byob" });
    const writer = stream.writable.getWriter();
    let written = false;
    const writing = writer.write(new Uint8Array([1, 2, 3, 4])).then(() => { written = true; });
    const first = await reader.read(new Uint8Array(2));
    await new Promise(resolve => setTimeout(resolve, 0));
    const afterFirst = written;
    const second = await reader.read(new Uint8Array(2));
    await writing;
    await writer.close();
    return { first: [...first.value], second: [...second.value], afterFirst, written };
  });
  result.identity.cancelPending = await outcome(async () => {
    const stream = new IdentityTransformStream();
    const writer = stream.writable.getWriter();
    const writing = writer.write(new Uint8Array([1])).then(() => "written", error => error.message);
    await stream.readable.cancel(new Error("cancelled"));
    return writing;
  });
  result.identity.switchReaders = await outcome(async () => {
    const stream = new IdentityTransformStream();
    const writer = stream.writable.getWriter();
    const writing = writer.write(new Uint8Array([1, 2, 3, 4]));
    const byob = stream.readable.getReader({ mode: "byob" });
    const first = await byob.read(new Uint8Array(2));
    byob.releaseLock();
    const reader = stream.readable.getReader();
    const rest = await reader.read();
    await writing;
    await writer.close();
    return [...first.value, ...rest.value];
  });
  result.identity.fixedErrors = await Promise.all([1, 3].map(length => outcome(async () => {
    const stream = new FixedLengthStream(length);
    const body = new Response(stream.readable).bytes();
    body.catch(() => {});
    const writer = stream.writable.getWriter();
    await writer.write(new Uint8Array([1, 2]));
    await writer.close();
    return [...await body];
  })));
  result.decompression = {};
  for (const format of ["gzip", "deflate", "deflate-raw"]) {
    const bytes = await compress(format, new TextEncoder().encode("incremental codec"));
    result.decompression[format] = {};
    for (const variant of ["valid", "truncated", "trailing", "invalid"]) {
      result.decompression[format][variant] = await outcome(async () => {
        const input = variant === "truncated" ? bytes.subarray(0, bytes.length - 1)
          : variant === "trailing" ? new Uint8Array([...bytes, 1])
          : variant === "invalid" ? new Uint8Array([255, 255, 255, 255]) : bytes;
        const stream = new DecompressionStream(format);
        const writer = stream.writable.getWriter();
        const output = new Response(stream.readable).text();
        output.catch(() => {});
        await writer.write(input);
        await writer.close();
        return output;
      });
    }
    result.decompression[format].incremental = await outcome(async () => {
      const stream = new DecompressionStream(format);
      const reader = stream.readable.getReader({ mode: "byob" });
      const writer = stream.writable.getWriter();
      const first = reader.read(new Uint8Array(100));
      await writer.write(bytes);
      const chunk = await first;
      await writer.close();
      return { value: new TextDecoder().decode(chunk.value), done: (await reader.read(new Uint8Array(1))).done };
    });
    result.decompression[format].splitInput = await outcome(async () => {
      const stream = new DecompressionStream(format);
      const text = new Response(stream.readable).text();
      const writer = stream.writable.getWriter();
      for (const byte of bytes) await writer.write(new Uint8Array([byte]));
      await writer.close();
      return text;
    });
  }
  result.adapters = {
    duplexFrom: await outcome(async () => {
      const writes = [];
      const duplex = Duplex.fromWeb({
        readable: new ReadableStream({ start(c) { c.enqueue(new Uint8Array([1, 2])); c.close(); } }),
        writable: new WritableStream({ write(chunk) { writes.push([...chunk]); } }),
      });
      const finished = new Promise((resolve, reject) => { duplex.on("error", reject); duplex.end(new Uint8Array([3, 4]), resolve); });
      const reads = [];
      for await (const chunk of duplex) reads.push(...chunk);
      await finished;
      return { reads, writes, halfOpen: duplex.allowHalfOpen };
    }),
    duplexTo: await outcome(async () => {
      const writes = [];
      const duplex = new Duplex({
        read() { this.push(new Uint8Array([5, 6])); this.push(null); },
        write(chunk, encoding, callback) { writes.push([...chunk]); callback(); },
      });
      const pair = Duplex.toWeb(duplex);
      const output = new Response(pair.readable).bytes();
      const writer = pair.writable.getWriter();
      await writer.write(new Uint8Array([7, 8]));
      const finished = new Promise((resolve, reject) => { duplex.on("finish", resolve); duplex.on("error", reject); });
      writer.close().catch(() => {});
      await finished;
      return { reads: [...await output], writes };
    }),
    readableCancel: await outcome(async () => {
      let reason;
      const stream = Readable.fromWeb(new ReadableStream({ cancel(value) { reason = value.message; } }));
      const closed = new Promise(resolve => stream.on("close", resolve));
      stream.on("error", () => {});
      stream.destroy(new Error("cancelled"));
      await closed;
      return reason;
    }),
    writableAbort: await outcome(async () => {
      let reason;
      const stream = Writable.fromWeb(new WritableStream({ abort(value) { reason = value.message; } }));
      const closed = new Promise(resolve => stream.on("close", resolve));
      stream.on("error", () => {});
      stream.destroy(new Error("aborted"));
      await closed;
      return reason;
    }),
    invalid: await Promise.all([null, {}, 1].flatMap(value => [
      outcome(() => Readable.fromWeb(value)), outcome(() => Writable.fromWeb(value)),
      outcome(() => Duplex.fromWeb(value)), outcome(() => Readable.toWeb(value)),
      outcome(() => Writable.toWeb(value)), outcome(() => Duplex.toWeb(value)),
    ])),
  };
  return result;
}
