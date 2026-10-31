import { dataCloneError, field, isIndex, list } from './values.ts';

const encoder = new TextEncoder();
const decoder = new TextDecoder('utf-8', { fatal: true, ignoreBOM: false });

/** A frame's metadata: what it carries, the stream it concerns, an encoded value and the ids of the streams in that value. */
export interface Frame {
  type: string;
  id?: number;
  data: string;
  streams: readonly number[];
}

/** A length-prefixed metadata record followed by its binary buffers. */
export function packFrame(metadata: Frame, buffers: readonly ArrayBuffer[]): ArrayBuffer {
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

export function unpackFrame(frame: unknown): Frame & { buffers: ArrayBuffer[] } {
  if (!(frame instanceof ArrayBuffer) || frame.byteLength < 4) throw dataCloneError();
  const length = new DataView(frame).getUint32(0);
  if (length > frame.byteLength - 4) throw dataCloneError();
  const metadata: unknown = JSON.parse(decoder.decode(new Uint8Array(frame, 4, length)));
  const type = field(metadata, 'type'), id = field(metadata, 'id'), data = field(metadata, 'data');
  const streams = list(field(metadata, 'streams'));
  if (typeof type !== 'string' || !(id === undefined || isIndex(id)) || typeof data !== 'string') throw dataCloneError();
  if (!streams.every(isIndex)) throw dataCloneError();
  let offset = 4 + length;
  const buffers = list(field(metadata, 'buffers')).map(size => {
    if (!isIndex(size) || size > frame.byteLength - offset) throw dataCloneError();
    const buffer = frame.slice(offset, offset + size);
    offset += size;
    return buffer;
  });
  if (offset !== frame.byteLength) throw dataCloneError();
  return { type, id, data, streams, buffers };
}
