import { StringDecoder } from "node:string_decoder";

// Each kind of value an API may take as bytes, made fresh for every use.
export const byteInputs = {
  arrayBuffer: () => new Uint8Array([104, 105]).buffer,
  view: () => new Uint8Array([0, 104, 105, 0]).subarray(1, 3),
  dataView: () => new DataView(new Uint8Array([0, 104, 105]).buffer, 1),
  wideView: () => new Uint16Array([0x6968]),
  shared: () => { const buffer = new SharedArrayBuffer(2); new Uint8Array(buffer).set([104, 105]); return buffer; },
  sharedView: () => { const view = new Uint8Array(new SharedArrayBuffer(2)); view.set([104, 105]); return view; },
  detached: () => { const buffer = new ArrayBuffer(2); buffer.transfer(); return buffer; },
  detachedView: () => { const buffer = new ArrayBuffer(2); const view = new Uint8Array(buffer); buffer.transfer(); return view; },
  string: () => "5",
  number: () => 5,
  object: () => ({ length: 2 }),
};

export function streamOf(chunk) {
  return new ReadableStream({ start(controller) { controller.enqueue(chunk); controller.close(); } });
}

export async function bytesOf(buffer) {
  return [...new Uint8Array(await buffer)];
}

// What each input becomes in `use`, or the error it raises, by name alone unless `messages`.
export async function byteOutcomes(use, messages = true) {
  const outcomes = {};
  for (const [name, input] of Object.entries(byteInputs)) {
    try { outcomes[name] = await use(input()); }
    catch (error) { outcomes[name] = messages ? { error: error.name, message: error.message } : { error: error.name }; }
  }
  return outcomes;
}

// The bytes `value` becomes, written directly to an IdentityTransformStream.
async function written(value) {
  const stream = new IdentityTransformStream();
  const read = bytesOf(new Response(stream.readable).arrayBuffer());
  read.catch(() => {});
  const writer = stream.writable.getWriter();
  await writer.write(value);
  await writer.close();
  return read;
}

export async function bytesContracts() {
  return {
    response: await byteOutcomes(value => bytesOf(new Response(value).arrayBuffer())),
    blob: await byteOutcomes(value => bytesOf(new Blob([value]).arrayBuffer())),
    streamBody: await byteOutcomes(value => bytesOf(new Response(streamOf(value)).arrayBuffer())),
    decode: await byteOutcomes(value => new TextDecoder().decode(value)),
    decoderStream: await byteOutcomes(async value => (await Array.fromAsync(streamOf(value).pipeThrough(new TextDecoderStream()))).join("")),
    compression: await byteOutcomes(value => bytesOf(new Response(streamOf(value).pipeThrough(new CompressionStream("gzip")).pipeThrough(new DecompressionStream("gzip"))).arrayBuffer())),
    identity: await byteOutcomes(written),
    html: await byteOutcomes(value => new HTMLRewriter().transform(new Response(streamOf(value))).text()),
    digest: await byteOutcomes(async value => (await crypto.subtle.digest("SHA-1", value)).byteLength, false),
    stringDecoder: await byteOutcomes(value => new StringDecoder().write(value), false),
  };
}
