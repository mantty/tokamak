import { dataCloneError } from './values.mjs';

const encoder = new TextEncoder();
const decoder = new TextDecoder('utf-8', { fatal: true });

/** A length-prefixed metadata record followed by its binary buffers. */
export function packFrame(metadata, buffers) {
  const header = encoder.encode(JSON.stringify({ ...metadata, buffers: buffers.map(buffer => buffer.byteLength) }));
  const frame = new Uint8Array(4 + header.byteLength + buffers.reduce((size, buffer) => size + buffer.byteLength, 0));
  new DataView(frame.buffer).setUint32(0, header.byteLength);
  frame.set(header, 4);
  let offset = 4 + header.byteLength;
  for (const buffer of buffers) {
    frame.set(new Uint8Array(buffer), offset);
    offset += buffer.byteLength;
  }
  return frame.buffer;
}

export function unpackFrame(frame) {
  if (!(frame instanceof ArrayBuffer) || frame.byteLength < 4) throw dataCloneError();
  const length = new DataView(frame).getUint32(0);
  if (length > frame.byteLength - 4) throw dataCloneError();
  const metadata = JSON.parse(decoder.decode(new Uint8Array(frame, 4, length)));
  if (!Array.isArray(metadata.buffers) || !Array.isArray(metadata.streams)) throw dataCloneError();
  let offset = 4 + length;
  const buffers = metadata.buffers.map(size => {
    if (!Number.isSafeInteger(size) || size < 0 || size > frame.byteLength - offset) throw dataCloneError();
    const buffer = frame.slice(offset, offset + size);
    offset += size;
    return buffer;
  });
  if (offset !== frame.byteLength) throw dataCloneError();
  return { ...metadata, buffers };
}
